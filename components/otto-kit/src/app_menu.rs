//! A window's menus in the top bar.
//!
//! The bar shows the focused window's menus beside its name, the way a Mac
//! does: it reads them over `com.canonical.dbusmenu`, from wherever the
//! window said they are served with `org_kde_kwin_appmenu`. An app describes
//! its menus as [`Menu`]s, hands them to an [`AppMenu`], and attaches it to
//! its window; a pick comes back as the item's action id from
//! [`AppMenu::take_picked`], for the app to run as it would the same key.
//!
//! The menus are the app's to keep current: [`AppMenu::set`] with what they
//! should say now (an item that cannot run greyed out, a toggle checked) is
//! cheap when nothing changed, so an app can call it on every update. The
//! bar fetches them afresh each time a menu opens.
//!
//! Served on a connection of its own, on a thread of its own, so an app
//! needs no async runtime for it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wayland_client::protocol::wl_surface::WlSurface;
use zbus::zvariant::{OwnedValue, Structure, StructureBuilder, Value};

use crate::protocols::org_kde_kwin_appmenu::OrgKdeKwinAppmenu;
use crate::AppContext;

/// Where the menus are served on the app's connection.
const MENU_PATH: &str = "/MenuBar";

/// One menu in the bar: its title, and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuEntry>,
}

impl Menu {
    pub fn new(title: impl Into<String>, items: Vec<MenuEntry>) -> Self {
        Self {
            title: title.into(),
            items,
        }
    }
}

/// An entry in a [`Menu`].
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    /// Something to run.
    Item(MenuItem),
    Separator,
    /// A menu inside the menu.
    Submenu(Menu),
}

/// Something to run from a menu.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// What [`AppMenu::take_picked`] says when it is picked.
    pub id: String,
    pub label: String,
    /// Its key, as the app reads it: `Ctrl+Shift+Z`. See [`Shortcut`].
    pub shortcut: Option<Shortcut>,
    pub enabled: bool,
    /// `Some` for a toggle, checked or not.
    pub checked: Option<bool>,
}

impl MenuEntry {
    /// An item that runs `id`, labelled `label`.
    pub fn item(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::Item(MenuItem {
            id: id.into(),
            label: label.into(),
            shortcut: None,
            enabled: true,
            checked: None,
        })
    }

    /// The same item, with its key. Ignored on anything but an item.
    pub fn with_shortcut(mut self, shortcut: &str) -> Self {
        if let Self::Item(item) = &mut self {
            item.shortcut = Shortcut::parse(shortcut);
        }
        self
    }

    /// The same item, greyed out unless `enabled`.
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Item(item) = &mut self {
            item.enabled = enabled;
        }
        self
    }

    /// The same item as a toggle, checked when `checked`.
    pub fn checked(mut self, checked: bool) -> Self {
        if let Self::Item(item) = &mut self {
            item.checked = Some(checked);
        }
        self
    }
}

/// A key combination as menus show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The logo key, which Otto's menus write ⌘ and people call Cmd.
    pub cmd: bool,
    /// The key itself, as written: `Z`, `0`, `+`, `Space`, `Esc`.
    pub key: String,
}

impl Shortcut {
    /// `Ctrl+Shift+Z`, `Cmd+Y`, `Alt+Left`: modifiers then the key, joined
    /// by `+` (a `+` key is written last, `Ctrl++`). `None` with no key.
    pub fn parse(text: &str) -> Option<Self> {
        let (mods, key) = match text.strip_suffix("++") {
            Some(mods) => (mods, "+"),
            None => match text.rsplit_once('+') {
                Some((mods, key)) => (mods, key),
                None => ("", text),
            },
        };
        if key.is_empty() {
            return None;
        }
        let mut shortcut = Self {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: key.to_owned(),
        };
        for modifier in mods.split('+').filter(|m| !m.is_empty()) {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => shortcut.ctrl = true,
                "alt" | "option" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                "cmd" | "super" | "logo" | "meta" => shortcut.cmd = true,
                _ => return None,
            }
        }
        Some(shortcut)
    }

    /// As written for people, the way [`Self::parse`] reads it:
    /// `Ctrl+Shift+Z`.
    pub fn text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.cmd, "Cmd"),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// As `com.canonical.dbusmenu` writes it: modifier names, then the key.
    fn dbusmenu(&self) -> Vec<String> {
        let mut combo = Vec::new();
        for (on, name) in [
            (self.ctrl, "Control"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.cmd, "Super"),
        ] {
            if on {
                combo.push(name.to_owned());
            }
        }
        let key = match self.key.as_str() {
            "Esc" | "Escape" => "Escape".to_owned(),
            "Space" => "space".to_owned(),
            "Enter" | "Return" => "Return".to_owned(),
            "Backspace" => "BackSpace".to_owned(),
            "Delete" | "Del" => "Delete".to_owned(),
            "PageUp" => "Page_Up".to_owned(),
            "PageDown" => "Page_Down".to_owned(),
            "+" => "plus".to_owned(),
            "-" => "minus".to_owned(),
            "/" => "slash".to_owned(),
            key => key.to_owned(),
        };
        combo.push(key);
        combo
    }
}

/// What the menu thread and the app share.
#[derive(Default)]
struct Shared {
    /// The menus as last set, and how often they changed.
    menus: Vec<Menu>,
    revision: u32,
    /// The menus as served, with the ids [`Ids`] gave them.
    tree: Option<Node>,
    /// The action each item's id runs, and whether it can run now.
    actions: HashMap<i32, (String, bool)>,
    ids: Ids,
    /// What was picked since the app last asked.
    picked: Vec<String>,
}

/// The dbusmenu id of every entry ever served, by what it is: an item by its
/// action, a menu by its titles from the top, a separator by its place in
/// its menu. An entry keeps its id while the menus around it change, and an
/// id is never given to anything else, so a click on a menu the bar fetched
/// a moment ago runs what it said, or nothing.
#[derive(Default)]
struct Ids {
    given: HashMap<String, i32>,
    last: i32,
}

impl Ids {
    fn of(&mut self, key: String) -> i32 {
        if let Some(id) = self.given.get(&key) {
            return *id;
        }
        self.last += 1;
        self.given.insert(key, self.last);
        self.last
    }
}

/// A window's menus, served for the top bar.
///
/// Used on the thread that runs the app, like the window it is attached to.
pub struct AppMenu {
    shared: Arc<Mutex<Shared>>,
    /// The connection's name once it has one, which the window points the
    /// bar at.
    service: std::cell::OnceCell<String>,
    ready: std::sync::mpsc::Receiver<Option<String>>,
    /// Sends `LayoutUpdated` when the menus change.
    changed: tokio::sync::mpsc::UnboundedSender<u32>,
    /// One per attached window, and whether it has been given the address
    /// yet; dropping one would forget the address.
    attached: std::cell::RefCell<Vec<(OrgKdeKwinAppmenu, bool)>>,
}

impl AppMenu {
    /// Serve `menus` on a session-bus connection of their own, started on a
    /// thread of its own: this returns at once, and the windows attached are
    /// pointed at the menus once the connection is up. With no session bus
    /// the app simply has no menus in the bar. `None` only when the thread
    /// cannot be started.
    pub fn serve(menus: Vec<Menu>) -> Option<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (ready_tx, ready) = std::sync::mpsc::channel();
        let (changed, changes) = tokio::sync::mpsc::unbounded_channel();
        let served = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("app-menu".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        tracing::warn!(%err, "no runtime for the app menu");
                        let _ = ready_tx.send(None);
                        return;
                    }
                };
                runtime.block_on(run(served, ready_tx, changes));
            })
            .ok()?;
        let menu = Self {
            shared,
            service: std::cell::OnceCell::new(),
            ready,
            changed,
            attached: std::cell::RefCell::new(Vec::new()),
        };
        menu.set(menus);
        Some(menu)
    }

    /// Point the top bar at these menus while `surface`'s window has focus.
    /// Nothing happens on a compositor without `org_kde_kwin_appmenu`.
    pub fn attach(&self, surface: &WlSurface) {
        if let Some(appmenu) = AppContext::appmenu_for(surface) {
            self.attached.borrow_mut().push((appmenu, false));
            self.address();
        }
    }

    /// The menus as they should read now. The bar hears of a change only
    /// when there is one.
    pub fn set(&self, menus: Vec<Menu>) {
        self.address();
        let revision = {
            let mut shared = self.shared.lock().unwrap();
            if shared.menus == menus && shared.revision > 0 {
                return;
            }
            let mut actions = HashMap::new();
            let tree = layout(&menus, &mut shared.ids, &mut actions);
            shared.tree = Some(tree);
            shared.actions = actions;
            shared.menus = menus;
            shared.revision += 1;
            shared.revision
        };
        let _ = self.changed.send(revision);
    }

    /// The action ids of the items picked since the last call, in order.
    pub fn take_picked(&self) -> Vec<String> {
        self.address();
        std::mem::take(&mut self.shared.lock().unwrap().picked)
    }

    /// The connection's name, once it is up.
    fn service(&self) -> Option<&str> {
        if self.service.get().is_none() {
            if let Ok(Some(name)) = self.ready.try_recv() {
                let _ = self.service.set(name);
            }
        }
        self.service.get().map(String::as_str)
    }

    /// Give the windows attached so far the menus' address, once there is
    /// one.
    fn address(&self) {
        let Some(service) = self.service() else {
            return;
        };
        for (appmenu, addressed) in self.attached.borrow_mut().iter_mut() {
            if !*addressed {
                appmenu.set_address(service.to_owned(), MENU_PATH.to_owned());
                *addressed = true;
            }
        }
    }
}

impl Drop for AppMenu {
    /// The windows stop pointing the bar at menus that are no longer served.
    fn drop(&mut self) {
        for (appmenu, _) in self.attached.borrow_mut().drain(..) {
            appmenu.release();
        }
    }
}

/// One entry as the bar is served it: its id, what it says, and the
/// entries under it.
#[derive(Debug)]
struct Node {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<Node>,
}

impl Node {
    /// This node and everything under it, as `GetLayout` sends it.
    fn wire(&self) -> (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) {
        let props = self
            .props
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.try_clone().ok()?)))
            .collect();
        let children = self.children.iter().map(Node::value).collect();
        (self.id, props, children)
    }

    /// This node as one value in its parent's children.
    fn value(&self) -> OwnedValue {
        let (id, props, children) = self.wire();
        let structure: Structure<'_> = StructureBuilder::new()
            .add_field(id)
            .add_field(props)
            .add_field(children)
            .build()
            .expect("a menu node has its fields");
        OwnedValue::try_from(Value::from(structure)).expect("a menu node is a plain struct")
    }

    /// The node `id`, this one or one under it.
    fn find(&self, id: i32) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    /// Every node under this one, depth first.
    fn descendants(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        for child in &self.children {
            out.push(child);
            out.extend(child.descendants());
        }
        out
    }
}

/// The whole menu tree from the root (id 0), each entry with the id `ids`
/// keeps for it, and in `actions` what each item runs.
fn layout(menus: &[Menu], ids: &mut Ids, actions: &mut HashMap<i32, (String, bool)>) -> Node {
    fn entries(
        items: &[MenuEntry],
        path: &str,
        ids: &mut Ids,
        actions: &mut HashMap<i32, (String, bool)>,
    ) -> Vec<Node> {
        let mut separators = 0;
        items
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => {
                    let id = ids.of(format!("item\u{1f}{}", item.id));
                    actions.insert(id, (item.id.clone(), item.enabled));
                    Node {
                        id,
                        props: item_props(item),
                        children: Vec::new(),
                    }
                }
                MenuEntry::Separator => {
                    separators += 1;
                    let mut props = HashMap::new();
                    props.insert("type".to_owned(), text("separator"));
                    Node {
                        id: ids.of(format!("separator\u{1f}{path}\u{1f}{separators}")),
                        props,
                        children: Vec::new(),
                    }
                }
                MenuEntry::Submenu(menu) => submenu(menu, path, ids, actions),
            })
            .collect()
    }
    fn submenu(
        menu: &Menu,
        parent: &str,
        ids: &mut Ids,
        actions: &mut HashMap<i32, (String, bool)>,
    ) -> Node {
        let path = format!("{parent}\u{1f}{}", menu.title);
        Node {
            id: ids.of(format!("menu{path}")),
            props: submenu_props(&menu.title),
            children: entries(&menu.items, &path, ids, actions),
        }
    }
    let children = menus
        .iter()
        .map(|menu| submenu(menu, "", ids, actions))
        .collect();
    let mut props = HashMap::new();
    props.insert("children-display".to_owned(), text("submenu"));
    Node {
        id: 0,
        props,
        children,
    }
}

fn item_props(item: &MenuItem) -> HashMap<String, OwnedValue> {
    let mut props = HashMap::new();
    props.insert("label".to_owned(), text(&item.label));
    props.insert("enabled".to_owned(), OwnedValue::from(item.enabled));
    props.insert("visible".to_owned(), OwnedValue::from(true));
    if let Some(shortcut) = &item.shortcut {
        let combos = vec![shortcut.dbusmenu()];
        if let Ok(value) = OwnedValue::try_from(Value::from(combos)) {
            props.insert("shortcut".to_owned(), value);
        }
    }
    if let Some(checked) = item.checked {
        props.insert("toggle-type".to_owned(), text("checkmark"));
        props.insert(
            "toggle-state".to_owned(),
            OwnedValue::from(i32::from(checked)),
        );
    }
    props
}

fn submenu_props(title: &str) -> HashMap<String, OwnedValue> {
    let mut props = HashMap::new();
    props.insert("label".to_owned(), text(title));
    props.insert("enabled".to_owned(), OwnedValue::from(true));
    props.insert("visible".to_owned(), OwnedValue::from(true));
    props.insert("children-display".to_owned(), text("submenu"));
    props
}

fn text(s: &str) -> OwnedValue {
    OwnedValue::from(zbus::zvariant::Str::from(s.to_owned()))
}

struct Served {
    shared: Arc<Mutex<Shared>>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Served {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// Nothing to fill in: the menus are always current.
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[expect(clippy::type_complexity, reason = "the signature dbusmenu defines")]
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))> {
        let shared = self.shared.lock().unwrap();
        let node = shared
            .tree
            .as_ref()
            .and_then(|tree| tree.find(parent_id))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        Ok((shared.revision, node.wire()))
    }

    #[allow(
        clippy::type_complexity,
        reason = "the signature dbusmenu defines; complex enough to lint on some clippy versions only"
    )]
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let shared = self.shared.lock().unwrap();
        let Some(tree) = &shared.tree else {
            return Vec::new();
        };
        tree.descendants()
            .into_iter()
            .filter(|node| ids.is_empty() || ids.contains(&node.id))
            .map(|node| {
                let (id, props, _) = node.wire();
                (id, props)
            })
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let shared = self.shared.lock().unwrap();
        shared
            .tree
            .as_ref()
            .and_then(|tree| tree.find(id))
            .and_then(|node| node.wire().1.remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        // An item greyed out, or gone since the bar fetched the menus, does
        // nothing.
        let mut shared = self.shared.lock().unwrap();
        if let Some((action, true)) = shared.actions.get(&id).cloned() {
            shared.picked.push(action);
            drop(shared);
            AppContext::request_wakeup();
        }
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, &event_id, data, timestamp);
        }
        Vec::new()
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

/// Serve the menus until the app goes, saying so each time they change.
async fn run(
    shared: Arc<Mutex<Shared>>,
    ready: std::sync::mpsc::Sender<Option<String>>,
    mut changes: tokio::sync::mpsc::UnboundedReceiver<u32>,
) {
    let connection = async {
        zbus::connection::Builder::session()?
            .serve_at(MENU_PATH, Served { shared })?
            .build()
            .await
    }
    .await;
    let connection = match connection {
        Ok(connection) => connection,
        Err(err) => {
            tracing::warn!(%err, "no session bus for the app menu");
            let _ = ready.send(None);
            return;
        }
    };
    let name = connection.unique_name().map(|name| name.to_string());
    let _ = ready.send(name);
    while let Some(revision) = changes.recv().await {
        let Ok(emitter) = zbus::object_server::SignalEmitter::new(&connection, MENU_PATH) else {
            continue;
        };
        let _ = Served::layout_updated(&emitter, revision, 0).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menus() -> Vec<Menu> {
        vec![
            Menu::new(
                "Edit",
                vec![
                    MenuEntry::item("undo", "Undo").with_shortcut("Ctrl+Z"),
                    MenuEntry::item("redo", "Redo")
                        .with_shortcut("Ctrl+Y")
                        .enabled(false),
                    MenuEntry::Separator,
                    MenuEntry::item("pen", "Pen").checked(true),
                ],
            ),
            Menu::new(
                "View",
                vec![
                    MenuEntry::item("zoom_in", "Zoom In").with_shortcut("Ctrl++"),
                    MenuEntry::Submenu(Menu::new(
                        "Panels",
                        vec![MenuEntry::item("chat", "Chat").with_shortcut("Ctrl+K")],
                    )),
                ],
            ),
        ]
    }

    #[test]
    fn shortcuts_read_and_write_back_as_people_write_them() {
        let redo = Shortcut::parse("Ctrl+Shift+Z").unwrap();
        assert!(redo.ctrl && redo.shift && !redo.cmd);
        assert_eq!(redo.key, "Z");
        assert_eq!(redo.text(), "Ctrl+Shift+Z");
        assert_eq!(redo.dbusmenu(), ["Control", "Shift", "Z"]);
        let plus = Shortcut::parse("Ctrl++").unwrap();
        assert_eq!(plus.key, "+");
        assert_eq!(Shortcut::parse("Cmd+Y").unwrap().dbusmenu(), ["Super", "Y"]);
        assert_eq!(Shortcut::parse("Esc").unwrap().dbusmenu(), ["Escape"]);
        assert!(Shortcut::parse("Hyper+Z").is_none());
    }

    /// The id an item is served as, from the tree.
    fn id_of(tree: &Node, label: &str) -> i32 {
        tree.descendants()
            .into_iter()
            .find(|node| {
                node.props.get("label").is_some_and(|value| {
                    String::try_from(value.try_clone().unwrap()).unwrap() == label
                })
            })
            .unwrap()
            .id
    }

    #[test]
    fn every_entry_keeps_its_id_while_the_menus_change() {
        let mut ids = Ids::default();
        let mut actions = HashMap::new();
        let tree = layout(&menus(), &mut ids, &mut actions);
        let chat = id_of(&tree, "Chat");
        assert_eq!(actions[&chat], ("chat".to_owned(), true));
        assert_eq!(tree.find(id_of(&tree, "Panels")).unwrap().children.len(), 1);
        let (root, _, children) = tree.wire();
        assert_eq!((root, children.len()), (0, 2));

        // An item comes in at the top: everything keeps its id, the new one
        // gets one nothing had.
        let mut more = menus();
        more[0].items.insert(0, MenuEntry::item("paste", "Paste"));
        let mut actions = HashMap::new();
        let again = layout(&more, &mut ids, &mut actions);
        assert_eq!(id_of(&again, "Chat"), chat);
        assert_eq!(id_of(&again, "Undo"), id_of(&tree, "Undo"));
        let paste = id_of(&again, "Paste");
        assert!(tree.find(paste).is_none());
        // A greyed item is served, but would not run.
        assert!(!actions[&id_of(&again, "Redo")].1);
    }

    #[test]
    fn a_toggle_and_a_greyed_item_say_so() {
        let menus = menus();
        let item = |id: &str| {
            menus
                .iter()
                .flat_map(|menu| &menu.items)
                .find_map(|entry| match entry {
                    MenuEntry::Item(item) if item.id == id => Some(item.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let pen = item_props(&item("pen"));
        let kind: String = pen["toggle-type"].try_clone().unwrap().try_into().unwrap();
        let state: i32 = pen["toggle-state"].try_clone().unwrap().try_into().unwrap();
        assert_eq!((kind.as_str(), state), ("checkmark", 1));
        let redo = item_props(&item("redo"));
        let enabled: bool = redo["enabled"].try_clone().unwrap().try_into().unwrap();
        assert!(!enabled);
        assert!(!redo.contains_key("toggle-type"));
    }

    /// A `GetLayout` reply: the revision, then the node asked for.
    type Layout = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

    /// Over a real session bus, as the top bar reads it: the layout, then a
    /// click that comes back as the item's id.
    #[test]
    #[ignore = "needs a session bus"]
    fn the_bar_reads_the_menus_and_a_click_comes_back() {
        let menu = AppMenu::serve(menus()).expect("the menu thread");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let service = loop {
            if let Some(service) = menu.service() {
                break service.to_owned();
            }
            assert!(std::time::Instant::now() < deadline, "no session bus");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let (panels, chat) = {
            let shared = menu.shared.lock().unwrap();
            let tree = shared.tree.as_ref().unwrap();
            (id_of(tree, "Panels"), id_of(tree, "Chat"))
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let bus = zbus::Connection::session().await.unwrap();
            let reply = bus
                .call_method(
                    Some(service.as_str()),
                    MENU_PATH,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<String>::new()),
                )
                .await
                .unwrap();
            let (revision, (root, _, children)): Layout = reply.body().deserialize().unwrap();
            assert_eq!((revision, root, children.len()), (1, 0, 2));
            // The Panels submenu on its own.
            let reply = bus
                .call_method(
                    Some(service.as_str()),
                    MENU_PATH,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(panels, -1i32, Vec::<String>::new()),
                )
                .await
                .unwrap();
            let (_, (id, _, children)): Layout = reply.body().deserialize().unwrap();
            assert_eq!((id, children.len()), (panels, 1));
            bus.call_method(
                Some(service.as_str()),
                MENU_PATH,
                Some("com.canonical.dbusmenu"),
                "Event",
                &(chat, "clicked", Value::from(0i32), 0u32),
            )
            .await
            .unwrap();
        });
        assert_eq!(menu.take_picked(), ["chat"]);
    }
}
