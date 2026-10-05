//! Global application menu support (macOS-style menu bar).
//!
//! A menu is found one of three ways, for the window `org.otto.Shell1` says has
//! focus (its `WindowChanged` event, `GetTree` at startup):
//!
//! 1. `otto_appmenu` on the window's node: a Wayland-native Qt/KDE app told the
//!    compositor over `org_kde_kwin_appmenu` where its dbusmenu lives.
//! 2. Its X11 `window` id, registered with the `com.canonical.AppMenu.Registrar`
//!    this module serves (GTK through `appmenu-gtk-module`, Qt on xcb).
//! 3. Its `pid`, matched against the process behind a registration: an app on
//!    Wayland that registered anyway, with an id the bar cannot map to a window.
//!
//! The compositor's `topbar.show_app_menu` turns all of this off. The bar then
//! gives up the registrar's name as well, since an app that finds no registrar
//! keeps its menu bar in its own window rather than handing it to nobody.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use futures_util::StreamExt;
use otto_kit::dbus::settings::SettingsProxy;
use otto_kit::dbus::shell::ShellProxy;
use otto_kit::AppContext;
use zbus::{interface, Connection};

use crate::dbusmenu::MenuLayout;
use crate::settings::get_bool;

/// The setting that shows or hides application menus.
const SHOW_ID: &str = "topbar.show_app_menu";

/// The name apps look for to hand their menus over.
const REGISTRAR_NAME: &str = "com.canonical.AppMenu.Registrar";

/// Whether application menus are shown. On until the compositor says
/// otherwise, so a bar running without Otto still has them.
static SHOWN: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Global shared state
// ---------------------------------------------------------------------------

/// Map from window-id → registration info.
static REGISTRATIONS: LazyLock<Mutex<HashMap<u32, Registration>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The current app menu ready for the UI.
static CURRENT_MENU: LazyLock<Mutex<Option<AppMenu>>> = LazyLock::new(|| Mutex::new(None));

/// Generation counter — bumped whenever CURRENT_MENU changes.
static MENU_GENERATION: AtomicU64 = AtomicU64::new(0);

/// D-Bus connection shared for fetching menus.
static APPMENU_CONNECTION: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));

/// The focused window, as `org.otto.Shell1` last described it.
static FOCUSED: LazyLock<Mutex<Option<FocusedWindow>>> = LazyLock::new(|| Mutex::new(None));

/// Bumped on every lookup, so a fetch that answers after focus moved on is
/// dropped rather than drawn over the new window's menu.
static LOOKUP: AtomicU64 = AtomicU64::new(0);

/// Registration entry for one window.
#[derive(Clone, Debug)]
struct Registration {
    service: String,
    menu_path: String,
    /// The process that owns `service`, for apps whose window id means
    /// nothing to the bar.
    pid: Option<u32>,
}

/// What identifies the focused window's menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FocusedWindow {
    app_id: String,
    pid: Option<u32>,
    x11_window: Option<u32>,
    /// `(service, object_path)` from `org_kde_kwin_appmenu`.
    kde_appmenu: Option<(String, String)>,
}

impl FocusedWindow {
    /// Read a `GetTree` window node.
    fn from_node(node: &serde_json::Value) -> Option<Self> {
        if !node.is_object() {
            return None;
        }
        let u32_field = |key: &str| {
            node.get(key)
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())
        };
        let kde_appmenu = node.get("otto_appmenu").and_then(|m| {
            Some((
                m.get("service")?.as_str()?.to_string(),
                m.get("object_path")?.as_str()?.to_string(),
            ))
        });
        Some(Self {
            app_id: node
                .get("app_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            pid: u32_field("pid"),
            x11_window: u32_field("window"),
            kde_appmenu,
        })
    }

    /// Where this window's menu is, if it has one.
    fn menu_address(&self, regs: &HashMap<u32, Registration>) -> Option<(String, String)> {
        if let Some(address) = &self.kde_appmenu {
            return Some(address.clone());
        }
        let reg = self.x11_window.and_then(|id| regs.get(&id)).or_else(|| {
            let pid = self.pid?;
            regs.values().find(|r| r.pid == Some(pid))
        })?;
        Some((reg.service.clone(), reg.menu_path.clone()))
    }
}

/// The focused window in a `GetTree` answer: the focused leaf.
fn focused_in_tree(node: &serde_json::Value) -> Option<&serde_json::Value> {
    let is_window = node.get("app_id").is_some();
    if is_window && node.get("focused").and_then(|f| f.as_bool()) == Some(true) {
        return Some(node);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| node.get(*key)?.as_array())
        .flatten()
        .find_map(focused_in_tree)
}

/// A fetched app menu ready for the UI.
#[derive(Clone, Debug)]
pub struct AppMenu {
    pub app_id: String,
    pub service: String,
    pub menu_path: String,
    pub layout: MenuLayout,
}

// ---------------------------------------------------------------------------
// Public API for the UI thread
// ---------------------------------------------------------------------------

/// Read the generation counter.
pub fn generation() -> u64 {
    MENU_GENERATION.load(Ordering::Relaxed)
}

/// Take the current menu for rendering.
pub fn current_menu() -> Option<AppMenu> {
    CURRENT_MENU.lock().unwrap().clone()
}

fn clear_menu() {
    let mut current = CURRENT_MENU.lock().unwrap();
    if current.is_some() {
        *current = None;
        MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
        AppContext::request_wakeup();
    }
}

/// Fetch the focused window's menu again, or clear it when it has none.
fn refresh() {
    let lookup = LOOKUP.fetch_add(1, Ordering::SeqCst) + 1;
    if !SHOWN.load(Ordering::Relaxed) {
        clear_menu();
        return;
    }
    let conn = APPMENU_CONNECTION.lock().unwrap().clone();
    let focused = FOCUSED.lock().unwrap().clone();
    let (Some(conn), Some(focused)) = (conn, focused) else {
        clear_menu();
        return;
    };
    let Some((service, menu_path)) = focused.menu_address(&REGISTRATIONS.lock().unwrap()) else {
        clear_menu();
        return;
    };

    let app_id = focused.app_id;
    tokio::spawn(async move {
        let fetched = crate::dbusmenu::fetch_menu(&conn, &service, &menu_path).await;
        if LOOKUP.load(Ordering::SeqCst) != lookup {
            return;
        }
        match fetched {
            Ok(layout) => {
                *CURRENT_MENU.lock().unwrap() = Some(AppMenu {
                    app_id,
                    service,
                    menu_path,
                    layout,
                });
                MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
                AppContext::request_wakeup();
            }
            Err(e) => {
                tracing::warn!("appmenu: fetch failed for {app_id}: {e}");
                clear_menu();
            }
        }
    });
}

/// A new focused window, or new news about it.
fn set_focused(focused: Option<FocusedWindow>) {
    {
        let mut current = FOCUSED.lock().unwrap();
        if *current == focused {
            return;
        }
        *current = focused;
    }
    refresh();
}

/// Activate a menu item in the current app menu.
pub fn activate_menu_item(item_id: i32, item_label: &str) {
    let menu = CURRENT_MENU.lock().unwrap().clone();
    let Some(menu) = menu else { return };

    let conn = APPMENU_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };

    let service = menu.service;
    let menu_path = menu.menu_path;
    let label = item_label.to_string();

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        match crate::dbusmenu::activate_menu_item(&conn, &service, &menu_path, item_id, &label)
            .await
        {
            Ok(_) => {}
            Err(e) => tracing::warn!("appmenu activate failed: {e}"),
        }
    });
}

/// Fetch the submenu for a specific top-level item by index.
/// This calls `AboutToShow` then re-fetches the layout to get fresh children.
pub fn fetch_submenu_for_item(item_index: usize, anchor_x: i32) {
    let menu = CURRENT_MENU.lock().unwrap().clone();
    let Some(menu) = menu else { return };

    let conn = APPMENU_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };

    let top_level_id = menu
        .layout
        .items
        .iter()
        .filter(|i| i.visible && !i.label.is_empty())
        .nth(item_index)
        .map(|i| i.id);

    let Some(top_id) = top_level_id else { return };

    let service = menu.service.clone();
    let menu_path = menu.menu_path.clone();
    let app_id = menu.app_id.clone();

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        // Apps fill a submenu when told it is about to show; then re-fetch
        // the full layout so we get fresh children.
        crate::dbusmenu::about_to_show(&conn, &service, &menu_path, top_id).await;
        match crate::dbusmenu::fetch_menu(&conn, &service, &menu_path).await {
            Ok(layout) => {
                *PENDING_SUBMENU.lock().unwrap() = Some(PendingSubmenu {
                    app_id,
                    service,
                    menu_path,
                    item_index,
                    anchor_x,
                    layout,
                });
                MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
                AppContext::request_wakeup();
            }
            Err(e) => {
                tracing::warn!("appmenu: submenu fetch failed: {e}");
            }
        }
    });
}

/// A pending submenu ready for the UI to display as a popup.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PendingSubmenu {
    pub app_id: String,
    pub service: String,
    pub menu_path: String,
    pub item_index: usize,
    pub anchor_x: i32,
    pub layout: MenuLayout,
}

static PENDING_SUBMENU: LazyLock<Mutex<Option<PendingSubmenu>>> =
    LazyLock::new(|| Mutex::new(None));

/// Take the pending submenu (if any) for rendering.
pub fn take_pending_submenu() -> Option<PendingSubmenu> {
    PENDING_SUBMENU.lock().unwrap().take()
}

// ---------------------------------------------------------------------------
// D-Bus Registrar service
// ---------------------------------------------------------------------------

struct AppMenuRegistrar;

#[interface(name = "com.canonical.AppMenu.Registrar")]
impl AppMenuRegistrar {
    /// Called by apps to register their menu for a window.
    async fn register_window(
        &mut self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        window_id: u32,
        menu_object_path: &str,
    ) {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let pid = sender_pid(&sender).await;

        REGISTRATIONS.lock().unwrap().insert(
            window_id,
            Registration {
                service: sender,
                menu_path: menu_object_path.to_string(),
                pid,
            },
        );
        // The app may well have focus already: menus register after mapping.
        refresh();
    }

    /// Called by apps to unregister their window's menu.
    fn unregister_window(&mut self, window_id: u32) {
        REGISTRATIONS.lock().unwrap().remove(&window_id);
        refresh();
    }

    /// Query which service provides the menu for a given window.
    fn get_menu_for_window(
        &self,
        window_id: u32,
    ) -> zbus::fdo::Result<(String, zbus::zvariant::OwnedObjectPath)> {
        let regs = REGISTRATIONS.lock().unwrap();
        if let Some(reg) = regs.get(&window_id) {
            Ok((
                reg.service.clone(),
                zbus::zvariant::OwnedObjectPath::try_from(reg.menu_path.clone()).unwrap_or_else(
                    |_| zbus::zvariant::OwnedObjectPath::try_from("/MenuBar").unwrap(),
                ),
            ))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "no menu registered for window {window_id}"
            )))
        }
    }
}

/// The process behind a bus name.
async fn sender_pid(sender: &str) -> Option<u32> {
    let conn = APPMENU_CONNECTION.lock().unwrap().clone()?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
    let name = zbus::names::BusName::try_from(sender).ok()?;
    dbus.get_connection_unix_process_id(name).await.ok()
}

// ---------------------------------------------------------------------------
// Following focus on org.otto.Shell1
// ---------------------------------------------------------------------------

async fn read_focus_from_tree(proxy: &ShellProxy<'_>) {
    let focused = proxy
        .get_tree()
        .await
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|tree| focused_in_tree(&tree).and_then(FocusedWindow::from_node));
    set_focused(focused);
}

async fn follow_focus(conn: Connection) -> zbus::Result<()> {
    let proxy = ShellProxy::new(&conn).await?;
    let mut events = proxy.receive_window_changed().await?;
    // A compositor that starts after the bar, or restarts under it.
    let mut owners = proxy.inner().receive_owner_changed().await?;

    read_focus_from_tree(&proxy).await;

    loop {
        tokio::select! {
            Some(signal) = events.next() => {
                let Ok(args) = signal.args() else { continue };
                let Ok(event) = serde_json::from_str::<serde_json::Value>(args.event()) else {
                    continue;
                };
                set_focused(event.get("container").and_then(FocusedWindow::from_node));
            }
            Some(owner) = owners.next() => {
                if owner.is_some() {
                    read_focus_from_tree(&proxy).await;
                } else {
                    set_focused(None);
                }
            }
            else => break,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Spawn the registrar service
// ---------------------------------------------------------------------------

/// Spawn the AppMenu registrar on the session D-Bus.
pub fn spawn_appmenu_registrar() {
    use std::sync::atomic::{AtomicBool, Ordering as AO};
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, AO::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = run_registrar().await {
            tracing::warn!("AppMenu registrar stopped: {e}");
        }
    });
}

async fn run_registrar() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = Connection::session().await?;

    // Store the connection for fetching menus
    *APPMENU_CONNECTION.lock().unwrap() = Some(conn.clone());

    // Register the object
    conn.object_server()
        .at("/com/canonical/AppMenu/Registrar", AppMenuRegistrar)
        .await?;

    // Focus is followed for the life of the bar; the registrar itself lives
    // in zbus's loop.
    tokio::spawn({
        let conn = conn.clone();
        async move {
            if let Err(e) = follow_focus(conn).await {
                tracing::warn!("appmenu: focus no longer followed: {e}");
            }
        }
    });

    follow_setting(conn).await?;
    Ok(())
}

/// Follow `topbar.show_app_menu`, holding the registrar's name only while
/// menus are shown.
///
/// The setting is read before the name is first asked for, so an app that
/// starts with the bar never hides its menu bar for a bar that will not show
/// it.
async fn follow_setting(conn: Connection) -> zbus::Result<()> {
    let proxy = SettingsProxy::new(&conn).await?;
    // Subscribed before the first read, so a change landing between the two
    // is not lost.
    let mut changes = proxy.receive_changed().await?;
    // A compositor that starts after the bar, or restarts under it, is asked
    // again.
    let mut owners = proxy.inner().receive_owner_changed().await?;

    let mut holding = false;
    loop {
        let shown = get_bool(&proxy, SHOW_ID)
            .await
            .unwrap_or_else(|| SHOWN.load(Ordering::Relaxed));
        SHOWN.store(shown, Ordering::Relaxed);
        if shown != holding {
            let held = if shown {
                // No AllowReplacement: zbus 5's plain request_name would add it.
                conn.request_name_with_flags(
                    REGISTRAR_NAME,
                    zbus::fdo::RequestNameFlags::ReplaceExisting
                        | zbus::fdo::RequestNameFlags::DoNotQueue,
                )
                .await
                .map(|_| ())
            } else {
                conn.release_name(REGISTRAR_NAME).await.map(|_| ())
            };
            match held {
                Ok(()) => holding = shown,
                Err(e) => tracing::warn!("appmenu: registrar name: {e}"),
            }
            refresh();
        }

        // Wait for the next reason to read the setting again.
        loop {
            tokio::select! {
                Some(signal) = changes.next() => {
                    if signal.args().is_ok_and(|args| args.values.contains_key(SHOW_ID)) {
                        break;
                    }
                }
                Some(owner) = owners.next() => {
                    if owner.is_some() {
                        break;
                    }
                }
                else => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registration(service: &str, pid: Option<u32>) -> Registration {
        Registration {
            service: service.into(),
            menu_path: "/MenuBar/1".into(),
            pid,
        }
    }

    #[test]
    fn kde_appmenu_wins_over_registrations() {
        let window = FocusedWindow::from_node(&json!({
            "app_id": "org.kde.kate", "pid": 42, "window": 7,
            "otto_appmenu": {"service": ":1.5", "object_path": "/MenuBar/3"},
        }))
        .unwrap();
        let regs = HashMap::from([(7, registration(":1.9", Some(42)))]);
        assert_eq!(
            window.menu_address(&regs),
            Some((":1.5".into(), "/MenuBar/3".into()))
        );
    }

    #[test]
    fn x11_window_id_then_pid() {
        let regs = HashMap::from([
            (7, registration(":1.7", Some(1))),
            (9, registration(":1.9", Some(42))),
        ]);
        let x11 =
            FocusedWindow::from_node(&json!({"app_id": "gimp", "pid": 42, "window": 7})).unwrap();
        assert_eq!(x11.menu_address(&regs).unwrap().0, ":1.7");

        // A Wayland window registered under an id that is not its own.
        let wayland = FocusedWindow::from_node(&json!({"app_id": "app", "pid": 42})).unwrap();
        assert_eq!(wayland.menu_address(&regs).unwrap().0, ":1.9");

        let unknown = FocusedWindow::from_node(&json!({"app_id": "foot", "pid": 3})).unwrap();
        assert_eq!(unknown.menu_address(&regs), None);
    }

    #[test]
    fn finds_the_focused_window_in_a_tree() {
        let tree = json!({"type": "root", "focused": false, "nodes": [{
            "type": "workspace", "focused": false, "nodes": [
                {"app_id": "a", "focused": false},
            ],
            "floating_nodes": [{"app_id": "b", "focused": true, "pid": 5}],
        }]});
        let window = FocusedWindow::from_node(focused_in_tree(&tree).unwrap()).unwrap();
        assert_eq!((window.app_id.as_str(), window.pid), ("b", Some(5)));
    }
}
