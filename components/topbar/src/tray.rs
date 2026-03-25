//! SNI (StatusNotifierItem) tray icon support.
//!
//! Implements the StatusNotifierWatcher D-Bus service and monitors registered
//! tray items. Icon data is fetched from each item and shared with the render
//! thread via `TrayState`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, LazyLock};
use std::sync::atomic::{AtomicU64, Ordering};

use otto_kit::AppContext;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, proxy, Connection, SignalContext};

/// Global shared tray state readable from the draw loop.
static TRAY_STATE: LazyLock<TrayState> = LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

/// Monotonic counter bumped every time TRAY_STATE changes.
static TRAY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Global D-Bus connection for calling methods on tray items.
static TRAY_CONNECTION: LazyLock<Mutex<Option<Connection>>> =
    LazyLock::new(|| Mutex::new(None));

/// Pending context menu waiting to be rendered by the UI.
static PENDING_MENU: LazyLock<Mutex<Option<PendingMenu>>> =
    LazyLock::new(|| Mutex::new(None));

/// A context menu fetched from dbusmenu, ready for the UI to display.
#[derive(Clone, Debug)]
pub struct PendingMenu {
    pub service: String,
    pub menu_path: String,
    pub layout: crate::dbusmenu::MenuLayout,
    pub anchor_x: i32,
    pub anchor_y: i32,
}

/// Thread-safe list of tray items shared between D-Bus tasks and renderer.
pub type TrayState = Arc<Mutex<Vec<TrayItem>>>;

/// A single tray icon's cached state.
#[derive(Clone, Debug)]
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

/// Activate a dbusmenu item by sending a "clicked" event.
pub fn activate_menu_item(service: &str, menu_path: &str, item_id: i32, item_label: &str) {
    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else { return };
    let service = service.to_string();
    let menu_path = menu_path.to_string();
    let label = item_label.to_string();

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        match crate::dbusmenu::activate_menu_item(&conn, &service, &menu_path, item_id, &label).await {
            Ok(_) => tracing::info!("dbusmenu item activated: id={item_id} label={label:?}"),
            Err(e) => tracing::warn!("dbusmenu activate failed: {e}"),
        }
    });
}

/// Activate a tray item by index (left click).
pub fn activate_item(index: usize, x: i32, y: i32) {
    call_item_method(index, x, y, "activate");
}

/// Open context menu for a tray item by index (right click).
/// Tries SNI ContextMenu first, falls back to dbusmenu if available.
pub fn context_menu_item(index: usize, x: i32, y: i32) {
    let items = TRAY_STATE.lock().unwrap();
    let Some(item) = items.get(index) else { return };
    let service = item.service.clone();
    let path = item.path.clone();
    let menu_path = item.menu_path.clone();
    drop(items);

    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else {
        tracing::warn!("no D-Bus connection for context_menu");
        return;
    };

    tracing::info!("context_menu: {service}{path} at ({x},{y})");

    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        // Try SNI ContextMenu first
        let proxy = StatusNotifierItemProxy::builder(&conn)
            .destination(service.as_str())
            .unwrap()
            .path(path.as_str())
            .unwrap()
            .build()
            .await;

        let sni_ok = match proxy {
            Ok(p) => p.context_menu(x, y).await.is_ok(),
            Err(_) => false,
        };

        if sni_ok {
            tracing::info!("SNI context_menu success: {service}");
            return;
        }

        // Fall back to dbusmenu
        if let Some(ref mpath) = menu_path {
            tracing::debug!("falling back to dbusmenu: {service} {mpath}");
            match crate::dbusmenu::fetch_menu(&conn, &service, mpath).await {
                Ok(layout) => {
                    tracing::info!(
                        "dbusmenu fetched: {} items, revision {}",
                        layout.items.len(),
                        layout.revision
                    );
                    for item in &layout.items {
                        if item.visible {
                            tracing::info!("  menu: [{}] {:?}", item.id, item.label);
                        }
                    }
                    // Store the menu for the UI to render
                    *PENDING_MENU.lock().unwrap() = Some(PendingMenu {
                        service: service.clone(),
                        menu_path: mpath.clone(),
                        layout,
                        anchor_x: x,
                        anchor_y: y,
                    });
                    TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
                    AppContext::request_wakeup();
                }
                Err(e) => {
                    tracing::warn!("dbusmenu fetch failed: {service}: {e}");
                }
            }
        } else {
            tracing::warn!("no context menu available for {service} (no ContextMenu method, no Menu path)");
        }
    });
}

fn call_item_method(index: usize, x: i32, y: i32, method: &str) {
    let items = TRAY_STATE.lock().unwrap();
    let Some(item) = items.get(index) else { return };
    let service = item.service.clone();
    let path = item.path.clone();
    drop(items);

    let conn = TRAY_CONNECTION.lock().unwrap().clone();
    let Some(conn) = conn else {
        tracing::warn!("no D-Bus connection for {method}");
        return;
    };

    tracing::info!("SNI {method}: {service}{path} at ({x},{y})");

    let method = method.to_string();
    let handle = tokio::runtime::Handle::current();
    handle.spawn(async move {
        let proxy = StatusNotifierItemProxy::builder(&conn)
            .destination(service.as_str())
            .unwrap()
            .path(path.as_str())
            .unwrap()
            .build()
            .await;
        match proxy {
            Ok(p) => {
                let result = match method.as_str() {
                    "context_menu" => p.context_menu(x, y).await,
                    _ => p.activate(x, y).await,
                };
                match result {
                    Ok(_) => tracing::info!("SNI {method} success: {service}"),
                    Err(e) => tracing::warn!("SNI {method} failed: {service}: {e}"),
                }
            }
            Err(e) => tracing::warn!("SNI proxy build failed: {e}"),
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
        tracing::info!("SNI registered: {key}");

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
        tracing::info!("SNI host registered: {service}");
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
    async fn status_notifier_host_registered(
        ctxt: &SignalContext<'_>,
    ) -> zbus::Result<()>;
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
    conn.request_name("org.kde.StatusNotifierWatcher")
        .await?;

    tracing::info!("SNI watcher service running on org.kde.StatusNotifierWatcher");

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
            tracing::info!("SNI items removed (owner vanished: {vanished}): {removed:?}");
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
    let (icon_data, icon_w, icon_h) = match proxy.icon_pixmap().await {
        Ok(pixmaps) if !pixmaps.is_empty() => {
            // Pick the best size (closest to 24px, prefer larger)
            let target = 24;
            let best = pixmaps
                .iter()
                .min_by_key(|(w, _, _)| (w - target).abs())
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

    // Resolve icon file from theme if we have a name but no pixmap
    let icon_file = if icon_data.is_none() {
        icon_name.as_deref().and_then(|name| {
            // Try non-symbolic variant first (colored, visible on any background)
            let non_symbolic = name.trim_end_matches("-symbolic");
            if non_symbolic != name {
                if let Some(path) = otto_kit::icons::find_icon(non_symbolic, 24, 1) {
                    return Some(path);
                }
            }
            otto_kit::icons::find_icon(name, 24, 1)
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
        menu_path,
    };

    tracing::info!(
        "SNI item fetched: {bus_name}{path} icon_name={:?} icon_file={:?} has_pixmap={} menu={:?}",
        item.icon_name, item.icon_file, item.icon_data.is_some(), item.menu_path
    );
    state.lock().unwrap().push(item);
    TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
    AppContext::request_wakeup();

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

    let bus = bus_name.to_string();
    let p = path.to_string();

    while icon_stream.next().await.is_some() {
        // Re-fetch icon
        let (icon_data, icon_w, icon_h) = match proxy.icon_pixmap().await {
            Ok(pixmaps) if !pixmaps.is_empty() => {
                let target = 24;
                let best = pixmaps
                    .iter()
                    .min_by_key(|(w, _, _)| (w - target).abs())
                    .unwrap();
                let data = argb_network_to_native(&best.2);
                (Some(data), best.0, best.1)
            }
            _ => (None, 0, 0),
        };

        let icon_name = proxy.icon_name().await.ok();

        let mut items = state.lock().unwrap();
        if let Some(item) = items
            .iter_mut()
            .find(|i| i.service == bus && i.path == p)
        {
            item.icon_data = icon_data;
            item.icon_width = icon_w;
            item.icon_height = icon_h;
            item.icon_name = icon_name;
        }
        TRAY_GENERATION.fetch_add(1, Ordering::Relaxed);
        AppContext::request_wakeup();
        tracing::debug!("SNI icon updated: {bus}{p}");
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
