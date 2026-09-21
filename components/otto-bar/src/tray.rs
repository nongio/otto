//! SNI (StatusNotifierItem) tray icon support.
//!
//! Implements the StatusNotifierWatcher D-Bus service and monitors registered
//! tray items. Icon data is fetched from each item and shared with the render
//! thread via `TrayState`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use otto_kit::AppContext;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, proxy, Connection, SignalContext};

/// Global shared tray state readable from the draw loop.
static TRAY_STATE: LazyLock<TrayState> = LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

/// Monotonic counter bumped every time TRAY_STATE changes.
static TRAY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Global D-Bus connection for calling methods on tray items.
static TRAY_CONNECTION: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));

/// Pending context menu waiting to be rendered by the UI.
static PENDING_MENU: LazyLock<Mutex<Option<PendingMenu>>> = LazyLock::new(|| Mutex::new(None));

/// Layouts refetched because an item said its menu changed, oldest first.
/// The UI applies the one for whichever menu it has open and drops the rest —
/// their caches are already updated.
static PENDING_REFRESHES: LazyLock<Mutex<Vec<PendingMenu>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Bumped when a refetched layout lands. Separate from `TRAY_GENERATION` so a
/// menu changing does not rebuild the bar's icons.
static MENU_GENERATION: AtomicU64 = AtomicU64::new(0);

/// How long to wait after a menu-changed signal before refetching. Applets
/// announce a change as a burst — nm-applet sends `LayoutUpdated` and
/// `ItemsPropertiesUpdated` back to back — and one fetch covers all of it.
const MENU_REFRESH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);

/// A context menu fetched from dbusmenu, ready for the UI to display.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PendingMenu {
    pub service: String,
    /// SNI object path — used to match back to the owning TrayItem.
    pub item_path: String,
    pub menu_path: String,
    pub layout: crate::dbusmenu::MenuLayout,
    pub anchor_x: i32,
    pub anchor_y: i32,
}

/// Thread-safe list of tray items shared between D-Bus tasks and renderer.
pub type TrayState = Arc<Mutex<Vec<TrayItem>>>;

/// A single tray icon's cached state.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TrayItem {
    /// D-Bus service name (e.g. `:1.42` or `org.kde.StatusNotifierItem-1234-1`)
    pub service: String,
    /// Object path of the SNI item.
    pub path: String,
    /// Icon name from icon theme.
    pub icon_name: Option<String>,
    /// Resolved path to an SVG/PNG icon file from the icon theme.
    pub icon_file: Option<String>,
    /// ARGB32 pixel data of the best available icon pixmap.
    pub icon_data: Option<Vec<u8>>,
    /// Icon width (if pixmap).
    pub icon_width: i32,
    /// Icon height (if pixmap).
    pub icon_height: i32,
    /// Tooltip text.
    pub tooltip: Option<String>,
    /// Status: Active, Passive, NeedsAttention.
    pub status: String,
    /// Object path of the dbusmenu interface (for apps that use dbusmenu instead of ContextMenu).
    pub menu_path: Option<String>,
    /// Pre-fetched dbusmenu layout (cached when item is discovered, refreshed on signals).
    pub cached_layout: Option<crate::dbusmenu::MenuLayout>,
}

/// Read current snapshot of tray items for rendering.
pub fn current_items() -> Vec<TrayItem> {
    TRAY_STATE.lock().unwrap().clone()
}

/// Current generation counter — changes whenever tray items are added/removed/updated.
pub fn generation() -> u64 {
    TRAY_GENERATION.load(Ordering::Relaxed)
}

/// Take the pending menu (if any) for rendering by the UI.
pub fn take_pending_menu() -> Option<PendingMenu> {
    PENDING_MENU.lock().unwrap().take()
}

/// Changes whenever a tray item's menu has been refetched.
pub fn menu_generation() -> u64 {
    MENU_GENERATION.load(Ordering::Relaxed)
}

/// Take every refetched layout since the last call.
pub fn take_pending_refreshes() -> Vec<PendingMenu> {
    std::mem::take(&mut *PENDING_REFRESHES.lock().unwrap())
}

/// Activate a dbusmenu item by sending a "clicked" event.
pub fn activate_menu_item(service: &str, menu_path: &str, item_id: i32, item_label: &str) {
    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };
    let service = service.to_string();
    let menu_path = menu_path.to_string();
    let label = item_label.to_string();

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        match crate::dbusmenu::activate_menu_item(&conn, &service, &menu_path, item_id, &label)
            .await
        {
            Ok(_) => {}
            Err(e) => tracing::warn!("dbusmenu activate failed: {e}"),
        }
    });
}

/// Open context menu for a tray item by index (right click).
/// Uses cached layout if available for instant display, otherwise fetches on demand.
pub fn context_menu_item(index: usize, x: i32, y: i32) {
    let items = TRAY_STATE.lock().unwrap();
    let Some(item) = items.get(index) else { return };
    let service = item.service.clone();
    let path = item.path.clone();
    let menu_path = item.menu_path.clone();
    let cached = item.cached_layout.clone();
    drop(items);

    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else {
        tracing::warn!("no D-Bus connection for context_menu");
        return;
    };

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        // Prefer our own dbusmenu-based menu when a menu path is available
        if let Some(ref mpath) = menu_path {
            let layout = if let Some(cached) = cached {
                cached
            } else {
                match crate::dbusmenu::fetch_menu(&conn, &service, mpath).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::warn!("dbusmenu fetch failed: {service}: {e}");
                        return;
                    }
                }
            };

            *PENDING_MENU.lock().unwrap() = Some(PendingMenu {
                service: service.clone(),
                item_path: path.clone(),
                menu_path: mpath.clone(),
                layout,
                anchor_x: x,
                anchor_y: y,
            });
            TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
            AppContext::request_wakeup();
            return;
        }

        // No dbusmenu path — activate the app via SNI Activate
        let proxy = StatusNotifierItemProxy::builder(&conn)
            .destination(service.as_str())
            .unwrap()
            .path(path.as_str())
            .unwrap()
            .build()
            .await;

        if let Ok(p) = proxy {
            let _ = p.activate(x, y).await;
        }
    });
}

/// Left-click: call SNI Activate on the tray item at `index`.
pub fn activate_item(index: usize, x: i32, y: i32) {
    let items = TRAY_STATE.lock().unwrap();
    let Some(item) = items.get(index) else { return };
    let service = item.service.clone();
    let path = item.path.clone();
    drop(items);

    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        let proxy = StatusNotifierItemProxy::builder(&conn)
            .destination(service.as_str())
            .unwrap()
            .path(path.as_str())
            .unwrap()
            .build()
            .await;
        if let Ok(p) = proxy {
            let _ = p.activate(x, y).await;
        }
    });
}

/// Middle-click: call SNI SecondaryActivate on the tray item at `index`.
pub fn secondary_activate_item(index: usize, x: i32, y: i32) {
    let items = TRAY_STATE.lock().unwrap();
    let Some(item) = items.get(index) else { return };
    let service = item.service.clone();
    let path = item.path.clone();
    drop(items);

    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        let proxy = StatusNotifierItemProxy::builder(&conn)
            .destination(service.as_str())
            .unwrap()
            .path(path.as_str())
            .unwrap()
            .build()
            .await;
        if let Ok(p) = proxy {
            let _ = p.secondary_activate(x, y).await;
        }
    });
}

/// Spawn the SNI watcher D-Bus service + item monitor.
pub fn spawn_tray_watcher() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = run_watcher().await {
            tracing::warn!("SNI tray watcher stopped: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// StatusNotifierWatcher D-Bus service implementation
// ---------------------------------------------------------------------------

/// State held by the watcher service on the bus.
struct WatcherService {
    items: Arc<Mutex<HashMap<String, String>>>,
    hosts: Vec<String>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl WatcherService {
    async fn register_status_notifier_item(
        &mut self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        // service may be a bus name or an object path. Normalise.
        let (bus_name, path) = if service.starts_with('/') {
            // Caller sent object path; use sender's bus name.
            let sender = header
                .sender()
                .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?
                .to_string();
            (sender, service.to_string())
        } else {
            (service.to_string(), "/StatusNotifierItem".to_string())
        };

        let key = format!("{bus_name}{path}");

        self.items
            .lock()
            .unwrap()
            .insert(key.clone(), bus_name.clone());

        // Emit signal
        Self::status_notifier_item_registered(&ctxt, &key).await?;

        // Fetch item properties in background
        let state = TRAY_STATE.clone();
        let items_map = self.items.clone();
        let conn = ctxt.connection().clone();
        tokio::spawn(async move {
            if let Err(e) = fetch_item(&conn, &bus_name, &path, state, items_map).await {
                tracing::warn!("failed to fetch SNI item {bus_name}: {e}");
            }
        });

        Ok(())
    }

    async fn register_status_notifier_host(
        &mut self,
        service: &str,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        self.hosts.push(service.to_string());
        Self::status_notifier_host_registered(&ctxt).await?;
        Ok(())
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().keys().cloned().collect()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        ctxt: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        ctxt: &SignalContext<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(ctxt: &SignalContext<'_>) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// StatusNotifierItem D-Bus proxy (to talk to tray apps)
// ---------------------------------------------------------------------------

#[proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
trait StatusNotifierItem {
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    #[zbus(property)]
    fn icon_theme_path(&self) -> zbus::Result<String>;

    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> zbus::Result<OwnedValue>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property, name = "Menu")]
    fn menu(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_icon(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_status(&self, status: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_tool_tip(&self) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// Watcher main loop
// ---------------------------------------------------------------------------

async fn run_watcher() -> Result<(), zbus::Error> {
    let conn = Connection::session().await?;

    // Store connection for later Activate/ContextMenu calls
    *TRAY_CONNECTION.lock().unwrap() = Some(conn.clone());

    let items_map = Arc::new(Mutex::new(HashMap::new()));
    let watcher = WatcherService {
        items: items_map.clone(),
        hosts: Vec::new(),
    };

    // Serve the watcher interface
    conn.object_server()
        .at("/StatusNotifierWatcher", watcher)
        .await?;

    // Request the well-known name
    conn.request_name("org.kde.StatusNotifierWatcher").await?;

    // Also register ourselves as a host
    // (we are both the watcher and the host in this compositor)

    // Monitor for name owner changes to detect items going away
    let state = TRAY_STATE.clone();
    let conn_clone = conn.clone();
    tokio::spawn(async move {
        monitor_disconnects(conn_clone, items_map, state).await;
    });

    // Keep alive
    std::future::pending::<()>().await;
    Ok(())
}

/// Watch for D-Bus name owner changes to remove items when their owner disconnects.
async fn monitor_disconnects(
    conn: Connection,
    items_map: Arc<Mutex<HashMap<String, String>>>,
    state: TrayState,
) {
    #[proxy(
        interface = "org.freedesktop.DBus",
        default_service = "org.freedesktop.DBus",
        default_path = "/org/freedesktop/DBus"
    )]
    trait DBus {
        #[zbus(signal)]
        fn name_owner_changed(
            &self,
            name: &str,
            old_owner: &str,
            new_owner: &str,
        ) -> zbus::Result<()>;
    }

    let Ok(proxy) = DBusProxy::new(&conn).await else {
        tracing::warn!("failed to create DBus proxy for disconnect monitoring");
        return;
    };

    let Ok(mut stream) = proxy.receive_name_owner_changed().await else {
        return;
    };

    while let Some(signal) = stream.next().await {
        let Ok(args) = signal.args() else { continue };

        // A name vanished (new_owner is empty)
        if !args.new_owner.is_empty() {
            continue;
        }

        let vanished = args.name;
        let mut removed = Vec::new();

        {
            let mut map = items_map.lock().unwrap();
            let keys_to_remove: Vec<String> = map
                .iter()
                .filter(|(_, bus)| bus.as_str() == vanished)
                .map(|(k, _)| k.clone())
                .collect();

            for key in &keys_to_remove {
                map.remove(key);
                removed.push(key.clone());
            }
        }

        if !removed.is_empty() {
            let mut items = state.lock().unwrap();
            items.retain(|item| {
                let key = format!("{}{}", item.service, item.path);
                !removed.contains(&key)
            });
            TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
            AppContext::request_wakeup();
        }
    }
}

// ---------------------------------------------------------------------------
// Fetch a single SNI item's properties and add to shared state
// ---------------------------------------------------------------------------

async fn fetch_item(
    conn: &Connection,
    bus_name: &str,
    path: &str,
    state: TrayState,
    _items_map: Arc<Mutex<HashMap<String, String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(bus_name)?
        .path(path)?
        .build()
        .await?;

    let icon_name = proxy.icon_name().await.ok();
    let status = proxy.status().await.unwrap_or_else(|_| "Active".into());
    let title = proxy.title().await.ok();

    // Try to get icon pixmap
    // Match the size the icon is actually painted at, so the pixmap we keep
    // needs the least rescaling.
    let pixmap_target = (crate::config::TRAY_ICON_SIZE as i32)
        * otto_kit::app_runner::context::AppContext::scale_factor().max(1);
    let (icon_data, icon_w, icon_h) = match proxy.icon_pixmap().await {
        Ok(pixmaps) if !pixmaps.is_empty() => {
            // Pick the best size (closest to target, prefer >= target when tied)
            let best = pixmaps
                .iter()
                .min_by(|(w1, h1, _), (w2, h2, _)| {
                    let d1 = (w1 - pixmap_target).abs() + (h1 - pixmap_target).abs();
                    let d2 = (w2 - pixmap_target).abs() + (h2 - pixmap_target).abs();
                    d1.cmp(&d2).then_with(|| {
                        // When equal distance, prefer the larger one
                        let s2 = w2 * h2;
                        let s1 = w1 * h1;
                        s2.cmp(&s1)
                    })
                })
                .unwrap();
            // SNI pixmaps are ARGB32 in network byte order (big-endian)
            let data = argb_network_to_native(&best.2);
            (Some(data), best.0, best.1)
        }
        _ => (None, 0, 0),
    };

    // Extract tooltip text
    let tooltip = match proxy.tool_tip().await {
        Ok(val) => extract_tooltip_text(val),
        Err(_) => title,
    };

    // Resolve icon file from theme if we have a name but no pixmap.
    // Search at logical size (24) — most themes top out at 32px and these
    // are SVGs anyway, so the renderer can scale them up for HiDPI.
    let scale = otto_kit::app_runner::context::AppContext::scale_factor().max(1);
    let icon_load_size = 24;
    let icon_file = if icon_data.is_none() {
        icon_name.as_deref().and_then(|name| {
            // Helper: try finding an icon at the current scale, then fall back to 1x
            let find = |n: &str| {
                let result = otto_kit::icons::find_icon(n, icon_load_size, scale);
                if result.is_some() {
                    return result;
                }
                if scale > 1 {
                    return otto_kit::icons::find_icon(n, icon_load_size, 1);
                }
                None
            };
            // Prefer symbolic variant — single-color SVGs that can be recolored
            // to match the bar's text color, ensuring visibility on any background.
            if !name.ends_with("-symbolic") {
                let symbolic = format!("{name}-symbolic");
                if let Some(path) = find(&symbolic) {
                    return Some(path);
                }
            }
            find(name)
        })
    } else {
        None
    };

    // Read the Menu property (dbusmenu object path)
    let menu_path = proxy.menu().await.ok().map(|p| p.to_string());

    let item = TrayItem {
        service: bus_name.to_string(),
        path: path.to_string(),
        icon_name,
        icon_file,
        icon_data,
        icon_width: icon_w,
        icon_height: icon_h,
        tooltip,
        status,
        menu_path: menu_path.clone(),
        cached_layout: None,
    };

    // Insert at front so newest items appear leftmost (per spec).
    state.lock().unwrap().insert(0, item);
    TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
    AppContext::request_wakeup();

    // Pre-fetch dbusmenu layout so the menu opens instantly on click
    if let Some(ref mpath) = menu_path {
        let service = bus_name.to_string();
        let mpath = mpath.clone();
        let state_for_prefetch = state.clone();
        let conn_for_prefetch = conn.clone();
        tokio::spawn(async move {
            prefetch_menu_layout(&conn_for_prefetch, &service, &mpath, state_for_prefetch).await;
        });

        // And keep it current: the prefetch is a snapshot, and a network
        // list or a toggle is stale the moment the applet changes it.
        let service = bus_name.to_string();
        let item_path = path.to_string();
        let watch_path = menu_path.clone().unwrap_or_default();
        let state_for_watch = state.clone();
        let conn_for_watch = conn.clone();
        tokio::spawn(async move {
            watch_menu_signals(
                &conn_for_watch,
                &service,
                &item_path,
                &watch_path,
                state_for_watch,
            )
            .await;
        });
    }

    // Watch for property changes
    let state_clone = state.clone();
    let bus = bus_name.to_string();
    let p = path.to_string();
    let conn = conn.clone();

    tokio::spawn(async move {
        watch_item_signals(&conn, &bus, &p, state_clone).await;
    });

    Ok(())
}

/// Pre-fetch a dbusmenu layout in the background and cache it on the TrayItem.
/// Also pre-loads menu item icons so the first open is instant.
async fn prefetch_menu_layout(conn: &Connection, service: &str, menu_path: &str, state: TrayState) {
    if let Ok(layout) = crate::dbusmenu::fetch_menu(conn, service, menu_path).await {
        // Pre-cache icons referenced in the menu
        let scale = otto_kit::app_runner::context::AppContext::scale_factor().max(1);
        let load_size = 16 * scale;
        precache_menu_icons(&layout.items, load_size);

        // Store the cached layout on the matching TrayItem
        let mut items = state.lock().unwrap();
        if let Some(item) = items.iter_mut().find(|i| i.service == service) {
            item.cached_layout = Some(layout);
        }
    }
}

/// Refetch an item's menu whenever it announces a change.
///
/// dbusmenu has two change signals: `LayoutUpdated` when items come or go,
/// `ItemsPropertiesUpdated` when a label, checkmark or enabled state moves.
/// Both are answered the same way — refetch the whole layout — because a
/// partial update would have to be merged into a tree the UI may be drawing,
/// and a GetLayout is a few kilobytes.
async fn watch_menu_signals(
    conn: &Connection,
    service: &str,
    item_path: &str,
    menu_path: &str,
    state: TrayState,
) {
    use futures_util::FutureExt;

    let Ok(builder) = crate::dbusmenu::DBusMenuProxy::builder(conn).destination(service) else {
        return;
    };
    let Ok(builder) = builder.path(menu_path) else {
        return;
    };
    let Ok(proxy) = builder.build().await else {
        return;
    };

    let (Ok(mut layout_stream), Ok(mut props_stream)) = (
        proxy.receive_layout_updated().await,
        proxy.receive_items_properties_updated().await,
    ) else {
        return;
    };

    loop {
        tokio::select! {
            ev = layout_stream.next() => if ev.is_none() { break },
            ev = props_stream.next() => if ev.is_none() { break },
        }

        // Let the rest of the burst arrive, then swallow it.
        tokio::time::sleep(MENU_REFRESH_DEBOUNCE).await;
        while let Some(Some(_)) = layout_stream.next().now_or_never() {}
        while let Some(Some(_)) = props_stream.next().now_or_never() {}

        // The item may have gone while we waited; its menu with it.
        let still_here = state
            .lock()
            .unwrap()
            .iter()
            .any(|i| i.service == service && i.path == item_path);
        if !still_here {
            break;
        }

        let layout = match crate::dbusmenu::fetch_menu(conn, service, menu_path).await {
            Ok(layout) => layout,
            Err(e) => {
                tracing::debug!("dbusmenu refetch failed: {service}: {e}");
                continue;
            }
        };

        let scale = otto_kit::app_runner::context::AppContext::scale_factor().max(1);
        precache_menu_icons(&layout.items, 16 * scale);

        {
            let mut items = state.lock().unwrap();
            if let Some(item) = items
                .iter_mut()
                .find(|i| i.service == service && i.path == item_path)
            {
                item.cached_layout = Some(layout.clone());
            }
        }

        PENDING_REFRESHES.lock().unwrap().push(PendingMenu {
            service: service.to_string(),
            item_path: item_path.to_string(),
            menu_path: menu_path.to_string(),
            layout,
            anchor_x: 0,
            anchor_y: 0,
        });
        MENU_GENERATION.fetch_add(1, Ordering::Relaxed);
        AppContext::request_wakeup();
    }
}

/// Recursively pre-cache named icons found in dbusmenu items.
fn precache_menu_icons(items: &[crate::dbusmenu::MenuItem], load_size: i32) {
    for item in items {
        if let Some(ref name) = item.icon_name {
            if !name.is_empty() {
                let _ = otto_kit::icons::named_icon_sized(name, load_size);
            }
        }
        if !item.children.is_empty() {
            precache_menu_icons(&item.children, load_size);
        }
    }
}

/// Watch NewIcon/NewStatus/NewToolTip signals and refresh the item.
async fn watch_item_signals(conn: &Connection, bus_name: &str, path: &str, state: TrayState) {
    let Ok(proxy) = StatusNotifierItemProxy::builder(conn)
        .destination(bus_name)
        .unwrap()
        .path(path)
        .unwrap()
        .build()
        .await
    else {
        return;
    };

    let mut icon_stream = match proxy.receive_new_icon().await {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut status_stream = proxy.receive_new_status().await.ok();
    let mut tooltip_stream = proxy.receive_new_tool_tip().await.ok();

    let bus = bus_name.to_string();
    let p = path.to_string();

    loop {
        tokio::select! {
            icon_ev = icon_stream.next() => {
                if icon_ev.is_none() { break; }

                // Re-fetch icon
                let (icon_data, icon_w, icon_h) = match proxy.icon_pixmap().await {
                    Ok(pixmaps) if !pixmaps.is_empty() => {
                        let target = 24 * otto_kit::app_runner::context::AppContext::scale_factor().max(1);
                        let best = pixmaps
                            .iter()
                            .min_by(|(w1, h1, _), (w2, h2, _)| {
                                let d1 = (w1 - target).abs() + (h1 - target).abs();
                                let d2 = (w2 - target).abs() + (h2 - target).abs();
                                d1.cmp(&d2).then_with(|| (w2 * h2).cmp(&(w1 * h1)))
                            })
                            .unwrap();
                        let data = argb_network_to_native(&best.2);
                        (Some(data), best.0, best.1)
                    }
                    _ => (None, 0, 0),
                };
                let icon_name = proxy.icon_name().await.ok();

                let mut items = state.lock().unwrap();
                if let Some(item) = items.iter_mut().find(|i| i.service == bus && i.path == p) {
                    item.icon_data = icon_data;
                    item.icon_width = icon_w;
                    item.icon_height = icon_h;
                    item.icon_name = icon_name;
                }
                TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
                AppContext::request_wakeup();
            }
            status_ev = async {
                match status_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if status_ev.is_none() { break; }

                let new_status = proxy.status().await.unwrap_or_default();
                let mut items = state.lock().unwrap();
                if let Some(item) = items.iter_mut().find(|i| i.service == bus && i.path == p) {
                    item.status = new_status;
                }
                TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
                AppContext::request_wakeup();
            }
            tooltip_ev = async {
                match tooltip_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if tooltip_ev.is_none() { break; }

                let tooltip = proxy.tool_tip().await.ok().and_then(extract_tooltip_text);
                let mut items = state.lock().unwrap();
                if let Some(item) = items.iter_mut().find(|i| i.service == bus && i.path == p) {
                    item.tooltip = tooltip;
                }
                TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
                AppContext::request_wakeup();
            }
        }
    }
}

/// Convert ARGB32 from network byte order (big-endian) to native RGBA premultiplied
/// for Skia (which expects native-endian).
fn argb_network_to_native(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        let a = chunk[0];
        let r = chunk[1];
        let g = chunk[2];
        let b = chunk[3];
        // Skia ColorType::RGBA_8888 (or we can use BGRA on little-endian)
        // Store as BGRA for skia_safe::ColorType::BGRA8888
        out.extend_from_slice(&[b, g, r, a]);
    }
    out
}

/// Extract the text portion of an SNI ToolTip.
/// ToolTip is (icon_name: s, icon_pixmap: a(iiay), title: s, description: s)
fn extract_tooltip_text(val: OwnedValue) -> Option<String> {
    let v: Value<'_> = val.into();
    match v {
        Value::Structure(s) => {
            let fields = s.into_fields();
            // title is field[2]
            if fields.len() >= 3 {
                if let Value::Str(title) = &fields[2] {
                    let t = title.to_string();
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
            }
            None
        }
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}
