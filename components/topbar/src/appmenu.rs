//! Global application menu support (macOS-style menu bar).
//!
//! Implements the `com.canonical.AppMenu.Registrar` D-Bus service so that GTK
//! (via `unity-gtk-module`) and Qt apps can register their menus.  When the
//! focused toplevel changes the topbar fetches the registered menu and exposes
//! it to the left panel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use zbus::{interface, Connection};

use crate::dbusmenu::MenuLayout;

// ---------------------------------------------------------------------------
// Global shared state
// ---------------------------------------------------------------------------

/// Map from window-id → registration info.
static REGISTRATIONS: LazyLock<Mutex<HashMap<u32, Registration>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The current app menu ready for the UI.
static CURRENT_MENU: LazyLock<Mutex<Option<AppMenu>>> =
    LazyLock::new(|| Mutex::new(None));

/// Generation counter — bumped whenever CURRENT_MENU changes.
static MENU_GENERATION: AtomicU64 = AtomicU64::new(0);

/// D-Bus connection shared for fetching menus.
static APPMENU_CONNECTION: LazyLock<Mutex<Option<Connection>>> =
    LazyLock::new(|| Mutex::new(None));

/// Registration entry for one window.
#[derive(Clone, Debug)]
struct Registration {
    service: String,
    menu_path: String,
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

/// Request a menu fetch for the focused app.
///
/// `window_id` is the X11 window ID (for XWayland apps) or 0 for native
/// Wayland apps.  `app_id` is the Wayland app_id from the foreign toplevel
/// protocol, used as a fallback identifier.
pub fn request_menu_for_app(app_id: &str, window_id: u32) {
    let conn = APPMENU_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else {
        tracing::debug!("appmenu: no D-Bus connection yet");
        return;
    };

    // Look up registration by window_id first, then scan by service name
    let reg = {
        let regs = REGISTRATIONS.lock().unwrap();
        if window_id != 0 {
            regs.get(&window_id).cloned()
        } else {
            // For Wayland-native apps, try to find a registration whose service
            // name contains the app_id (heuristic).
            regs.values()
                .find(|r| r.service.contains(app_id))
                .cloned()
        }
    };

    let Some(reg) = reg else {
        tracing::debug!("appmenu: no registration for app_id={app_id} wid={window_id}");
        // Clear the current menu since focused app has no registered menu
        let mut current = CURRENT_MENU.lock().unwrap();
        if current.is_some() {
            *current = None;
            MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        return;
    };

    let app_id = app_id.to_string();
    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        tracing::debug!(
            "appmenu: fetching menu for {app_id} from {} {}",
            reg.service,
            reg.menu_path
        );
        match crate::dbusmenu::fetch_menu(&conn, &reg.service, &reg.menu_path).await {
            Ok(layout) => {
                tracing::info!(
                    "appmenu: fetched {} top-level items for {app_id}",
                    layout.items.len()
                );
                *CURRENT_MENU.lock().unwrap() = Some(AppMenu {
                    app_id,
                    service: reg.service,
                    menu_path: reg.menu_path,
                    layout,
                });
                MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!("appmenu: fetch failed for {app_id}: {e}");
                let mut current = CURRENT_MENU.lock().unwrap();
                if current.as_ref().map(|m| m.app_id == app_id).unwrap_or(false) {
                    *current = None;
                    MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });
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
            Ok(_) => tracing::info!("appmenu item activated: id={item_id} label={label:?}"),
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

    let Some(_top_id) = top_level_id else { return };

    let service = menu.service.clone();
    let menu_path = menu.menu_path.clone();
    let app_id = menu.app_id.clone();

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        // Re-fetch the full layout so we get fresh children
        match crate::dbusmenu::fetch_menu(&conn, &service, &menu_path).await {
            Ok(layout) => {
                tracing::info!(
                    "appmenu: refreshed menu for {app_id}, {} items",
                    layout.items.len()
                );
                *PENDING_SUBMENU.lock().unwrap() = Some(PendingSubmenu {
                    app_id,
                    service,
                    menu_path,
                    item_index,
                    anchor_x,
                    layout,
                });
                MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!("appmenu: submenu fetch failed: {e}");
            }
        }
    });
}

/// A pending submenu ready for the UI to display as a popup.
#[derive(Clone, Debug)]
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
        let sender = header
            .sender()
            .map(|s| s.to_string())
            .unwrap_or_default();

        tracing::info!(
            "AppMenu: RegisterWindow wid={window_id} path={menu_object_path} sender={sender}"
        );

        REGISTRATIONS.lock().unwrap().insert(
            window_id,
            Registration {
                service: sender,
                menu_path: menu_object_path.to_string(),
            },
        );
    }

    /// Called by apps to unregister their window's menu.
    fn unregister_window(&mut self, window_id: u32) {
        tracing::info!("AppMenu: UnregisterWindow wid={window_id}");
        REGISTRATIONS.lock().unwrap().remove(&window_id);
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
                zbus::zvariant::OwnedObjectPath::try_from(reg.menu_path.clone())
                    .unwrap_or_else(|_| {
                        zbus::zvariant::OwnedObjectPath::try_from("/MenuBar").unwrap()
                    }),
            ))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "no menu registered for window {window_id}"
            )))
        }
    }
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

    // Request the well-known name
    conn.request_name("com.canonical.AppMenu.Registrar")
        .await?;

    tracing::info!("AppMenu registrar running on com.canonical.AppMenu.Registrar");

    // Keep alive — the connection event loop runs inside zbus
    std::future::pending::<()>().await;

    Ok(())
}
