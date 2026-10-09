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
    /// Action ids by dbusmenu id, for the menus as they are now.
    ids: HashMap<i32, String>,
    /// What was picked since the app last asked.
    picked: Vec<String>,
}

/// A window's menus, served for the top bar.
pub struct AppMenu {
    shared: Arc<Mutex<Shared>>,
    /// The connection's name, which the window points the bar at.
    service: String,
    /// Sends `LayoutUpdated` when the menus change.
    changed: tokio::sync::mpsc::UnboundedSender<u32>,
    /// One per attached window; dropping it would forget the address.
    attached: Vec<OrgKdeKwinAppmenu>,
}

impl AppMenu {
    /// Serve `menus` on a session-bus connection of their own. `None` with
    /// no session bus, in which case the app simply has no menus in the bar.
    pub fn serve(menus: Vec<Menu>) -> Option<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
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
                        return;
                    }
                };
                runtime.block_on(run(served, ready_tx, changes));
            })
            .ok()?;
        let service = match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Some(service)) => service,
            _ => return None,
        };
        let menu = Self {
            shared,
            service,
            changed,
            attached: Vec::new(),
        };
        menu.set(menus);
        Some(menu)
    }

    /// Point the top bar at these menus while `surface`'s window has focus.
    /// Nothing happens on a compositor without `org_kde_kwin_appmenu`.
    pub fn attach(&mut self, surface: &WlSurface) {
        if let Some(appmenu) = AppContext::appmenu_for(surface) {
            appmenu.set_address(self.service.clone(), MENU_PATH.to_owned());
            self.attached.push(appmenu);
        }
    }

    /// The menus as they should read now. The bar hears of a change only
    /// when there is one.
    pub fn set(&self, menus: Vec<Menu>) {
        let revision = {
            let mut shared = self.shared.lock().unwrap();
            if shared.menus == menus && shared.revision > 0 {
                return;
            }
            shared.ids = numbered(&menus)
                .into_iter()
                .map(|(id, item)| (id, item.id.clone()))
                .collect();
            shared.menus = menus;
            shared.revision += 1;
            shared.revision
        };
        let _ = self.changed.send(revision);
    }

    /// The action ids of the items picked since the last call, in order.
    pub fn take_picked(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.lock().unwrap().picked)
    }
}

/// Every item in `menus` with the dbusmenu id it is served as: ids count up
/// from 1 in the order the bar walks them, menus and separators included,
/// so the same menus always get the same ids.
fn numbered(menus: &[Menu]) -> Vec<(i32, &MenuItem)> {
    fn walk<'a>(entries: &'a [MenuEntry], next: &mut i32, out: &mut Vec<(i32, &'a MenuItem)>) {
        for entry in entries {
            *next += 1;
            match entry {
                MenuEntry::Item(item) => out.push((*next, item)),
                MenuEntry::Separator => {}
                MenuEntry::Submenu(menu) => walk(&menu.items, next, out),
            }
        }
    }
    let mut out = Vec::new();
    let mut next = 0;
    for menu in menus {
        next += 1;
        walk(&menu.items, &mut next, &mut out);
    }
    out
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
}

/// The whole menu tree from the root (id 0), numbered as [`numbered`] does.
fn layout(menus: &[Menu]) -> Node {
    fn entries(items: &[MenuEntry], next: &mut i32) -> Vec<Node> {
        items
            .iter()
            .map(|entry| {
                *next += 1;
                let id = *next;
                match entry {
                    MenuEntry::Item(item) => Node {
                        id,
                        props: item_props(item),
                        children: Vec::new(),
                    },
                    MenuEntry::Separator => {
                        let mut props = HashMap::new();
                        props.insert("type".to_owned(), text("separator"));
                        Node {
                            id,
                            props,
                            children: Vec::new(),
                        }
                    }
                    MenuEntry::Submenu(menu) => Node {
                        id,
                        props: submenu_props(&menu.title),
                        children: entries(&menu.items, next),
                    },
                }
            })
            .collect()
    }
    let mut next = 0;
    let children = menus
        .iter()
        .map(|menu| {
            next += 1;
            let id = next;
            Node {
                id,
                props: submenu_props(&menu.title),
                children: entries(&menu.items, &mut next),
            }
        })
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

    #[allow(clippy::type_complexity)]
    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))> {
        let shared = self.shared.lock().unwrap();
        let tree = layout(&shared.menus);
        let node = tree
            .find(parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        Ok((shared.revision, node.wire()))
    }

    #[allow(clippy::type_complexity)]
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let shared = self.shared.lock().unwrap();
        numbered(&shared.menus)
            .into_iter()
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .map(|(id, item)| (id, item_props(item)))
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let shared = self.shared.lock().unwrap();
        numbered(&shared.menus)
            .into_iter()
            .find(|(item_id, _)| *item_id == id)
            .and_then(|(_, item)| item_props(item).remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let mut shared = self.shared.lock().unwrap();
        if let Some(action) = shared.ids.get(&id).cloned() {
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

    #[test]
    fn every_item_has_the_id_it_is_served_as() {
        let menus = menus();
        let ids: Vec<(i32, &str)> = numbered(&menus)
            .into_iter()
            .map(|(id, item)| (id, item.id.as_str()))
            .collect();
        // Edit 1: undo 2, redo 3, separator 4, pen 5; View 6: zoom 7,
        // Panels 8: chat 9.
        assert_eq!(
            ids,
            [
                (2, "undo"),
                (3, "redo"),
                (5, "pen"),
                (7, "zoom_in"),
                (9, "chat")
            ]
        );

        // The layout numbers them the same way.
        let tree = layout(&menus);
        let chat = tree.find(9).unwrap();
        let label: String = chat.props["label"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(label, "Chat");
        assert_eq!(tree.find(8).unwrap().children.len(), 1);
        // And it goes on the wire whole.
        let (id, _, children) = tree.wire();
        assert_eq!((id, children.len()), (0, 2));
    }

    #[test]
    fn a_toggle_and_a_greyed_item_say_so() {
        let menus = menus();
        let items = numbered(&menus);
        let pen = item_props(items.iter().find(|(_, i)| i.id == "pen").unwrap().1);
        let kind: String = pen["toggle-type"].try_clone().unwrap().try_into().unwrap();
        let state: i32 = pen["toggle-state"].try_clone().unwrap().try_into().unwrap();
        assert_eq!((kind.as_str(), state), ("checkmark", 1));
        let redo = item_props(items.iter().find(|(_, i)| i.id == "redo").unwrap().1);
        let enabled: bool = redo["enabled"].try_clone().unwrap().try_into().unwrap();
        assert!(!enabled);
        assert!(!redo.contains_key("toggle-type"));
    }

    /// Over a real session bus, as the top bar reads it: the layout, then a
    /// click that comes back as the item's id.
    #[test]
    #[ignore = "needs a session bus"]
    fn the_bar_reads_the_menus_and_a_click_comes_back() {
        let menu = AppMenu::serve(menus()).expect("a session bus");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let bus = zbus::Connection::session().await.unwrap();
            let reply = bus
                .call_method(
                    Some(menu.service.as_str()),
                    MENU_PATH,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<String>::new()),
                )
                .await
                .unwrap();
            let (revision, (root, _, children)): (
                u32,
                (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>),
            ) = reply.body().deserialize().unwrap();
            assert_eq!((revision, root, children.len()), (1, 0, 2));
            // The Panels submenu on its own.
            let reply = bus
                .call_method(
                    Some(menu.service.as_str()),
                    MENU_PATH,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(8i32, -1i32, Vec::<String>::new()),
                )
                .await
                .unwrap();
            let (_, (id, _, children)): (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) =
                reply.body().deserialize().unwrap();
            assert_eq!((id, children.len()), (8, 1));
            bus.call_method(
                Some(menu.service.as_str()),
                MENU_PATH,
                Some("com.canonical.dbusmenu"),
                "Event",
                &(9i32, "clicked", Value::from(0i32), 0u32),
            )
            .await
            .unwrap();
        });
        assert_eq!(menu.take_picked(), ["chat"]);
    }
}
