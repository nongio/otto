//! Screen-sharing indicator: a StatusNotifierItem published while — and only
//! while — a remote party can actually see this screen.
//!
//! The bridge reuses the tray protocols the desktop already speaks
//! (`org.kde.StatusNotifierItem` plus `com.canonical.dbusmenu`) rather than
//! inventing an Otto-specific interface, so any SNI host shows the indicator.
//! See `docs/developer/remote-desktop-indicator.md` for the full contract.
//!
//! ## Why a dedicated D-Bus connection
//!
//! SNI hosts drop an item when its bus name loses its owner. There is no
//! reliable "unregister" in practice — the KDE spec's
//! `StatusNotifierItemUnregistered` is emitted by watchers, not items, and
//! hosts do not universally act on it. So the item lives on its own
//! `Connection` that is opened when sharing starts and dropped when it ends:
//! the indicator disappears on client disconnect, on `Stop`, and on a crash
//! or SIGKILL of the bridge, all through the same mechanism. A stale "you are
//! being recorded" icon is not possible.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zbus::zvariant::{OwnedValue, Structure, StructureBuilder, Value};
use zbus::{interface, Connection};

/// Object path of the item. Matches the SNI default.
const ITEM_PATH: &str = "/StatusNotifierItem";

/// Object path of the item's dbusmenu.
const MENU_PATH: &str = "/MenuBar";

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// dbusmenu item id of the "Stop Sharing" entry — the one actionable item.
const MENU_ID_STOP: i32 = 4;

/// Facts about the live session, shown in the indicator's menu.
#[derive(Clone, Debug)]
pub struct Session {
    /// Remote peer address (`host:port`).
    pub peer: String,
    /// Wayland output being shared, e.g. `eDP-1`.
    pub output: String,
    /// Local time the session started, preformatted as `HH:MM`.
    pub since: String,
}

impl Session {
    fn new(peer: &str, output: &str) -> Self {
        Self {
            peer: peer.to_string(),
            output: output.to_string(),
            since: local_hhmm(),
        }
    }

    /// Peer without the port — the part worth putting in a menu.
    fn peer_host(&self) -> &str {
        match self.peer.rfind(':') {
            Some(i) => &self.peer[..i],
            None => &self.peer,
        }
    }
}

// ── StatusNotifierItem ─────────────────────────────────────────────────────

struct Item {
    session: Session,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    /// `SystemServices` is the SNI category for things the session itself is
    /// doing to the user, as opposed to an application's own tray icon.
    #[zbus(property)]
    fn category(&self) -> &str {
        "SystemServices"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "otto-rdp"
    }

    #[zbus(property)]
    fn title(&self) -> String {
        format!("Screen shared with {}", self.session.peer_host())
    }

    /// Never `Passive`: hosts are allowed to hide passive items, and a privacy
    /// signal the user cannot see is worse than no signal at all.
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    /// Themes that ship it get the standard record glyph; the pixmap below is
    /// what actually renders in Otto, so the indicator does not depend on the
    /// icon theme having this name.
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "media-record"
    }

    /// A red dot, drawn here rather than looked up, at the sizes a bar is
    /// likely to want. ARGB32 in network byte order, per the SNI spec.
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![record_dot(22), record_dot(44)]
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    /// `(icon name, icon pixmaps, title, description)`.
    #[allow(clippy::type_complexity)]
    #[zbus(property, name = "ToolTip")]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        (
            String::new(),
            Vec::new(),
            format!("Screen shared with {}", self.session.peer_host()),
            format!(
                "{} is being shared since {}",
                self.session.output, self.session.since
            ),
        )
    }

    /// The item is a menu: hosts should open the dbusmenu instead of
    /// synthesising an activation.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        true
    }

    #[zbus(property, name = "Menu")]
    fn menu(&self) -> zbus::zvariant::ObjectPath<'_> {
        zbus::zvariant::ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    /// There is no window to raise, and no secondary action worth guessing at:
    /// everything the user can do lives in the menu.
    fn activate(&self, _x: i32, _y: i32) {}
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
}

// ── dbusmenu ───────────────────────────────────────────────────────────────

/// The indicator's menu: three read-only status lines, a separator, and the
/// one thing the user can actually do.
///
/// The layout is fixed for the life of a session, so hosts that cache it (Otto's
/// bar prefetches at registration time) always show something accurate. That is
/// why the "since" line is an absolute clock time rather than an elapsed
/// duration, and why the transport codec — which is only settled after the
/// client's capability exchange — is not shown at all.
struct Menu {
    session: Session,
    stop_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
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

    /// Nothing to refresh — the layout never changes within a session.
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
        if parent_id != 0 {
            // A flat menu: nothing but the root has children.
            return Ok((1, (parent_id, HashMap::new(), Vec::new())));
        }

        let children = vec![
            menu_entry(
                1,
                &format!(
                    "Sharing {} with {}",
                    self.session.output,
                    self.session.peer_host()
                ),
                false,
            ),
            menu_entry(2, &format!("Since {}", self.session.since), false),
            separator_entry(3),
            menu_entry(MENU_ID_STOP, "Stop Sharing", true),
        ];

        let mut root_props = HashMap::new();
        root_props.insert("children-display".to_string(), owned_str("submenu"));

        Ok((1, (0, root_props, children)))
    }

    #[allow(clippy::type_complexity)]
    fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        Vec::new()
    }

    fn get_property(&self, _id: i32, _name: &str) -> zbus::fdo::Result<OwnedValue> {
        Err(zbus::fdo::Error::InvalidArgs("no such property".into()))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if id == MENU_ID_STOP && event_id == "clicked" {
            tracing::info!("'Stop Sharing' clicked — ending the remote session");
            let _ = self.stop_tx.send(());
        }
    }

    fn event_group(&self, _events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        Vec::new()
    }
}

fn menu_entry(id: i32, label: &str, enabled: bool) -> OwnedValue {
    let mut props = HashMap::new();
    props.insert("label".to_string(), owned_str(label));
    props.insert("enabled".to_string(), owned_bool(enabled));
    props.insert("visible".to_string(), owned_bool(true));
    entry(id, props)
}

fn separator_entry(id: i32) -> OwnedValue {
    let mut props = HashMap::new();
    props.insert("type".to_string(), owned_str("separator"));
    props.insert("visible".to_string(), owned_bool(true));
    entry(id, props)
}

/// One `(id, a{sv}, av)` child, boxed into the `av` the protocol expects.
fn entry(id: i32, props: HashMap<String, OwnedValue>) -> OwnedValue {
    let structure: Structure<'_> = StructureBuilder::new()
        .add_field(id)
        .add_field(props)
        .add_field(Vec::<OwnedValue>::new())
        .build();
    OwnedValue::try_from(Value::from(structure)).expect("menu entry is a plain struct")
}

fn owned_str(s: &str) -> OwnedValue {
    OwnedValue::from(zbus::zvariant::Str::from(s.to_string()))
}

fn owned_bool(b: bool) -> OwnedValue {
    OwnedValue::from(b)
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

/// Publishes and retracts the indicator as the remote session comes and goes.
///
/// Cheap to clone; the connection-handler and display paths both hold one.
#[derive(Clone)]
pub struct Indicator {
    output: String,
    stop_tx: tokio::sync::mpsc::UnboundedSender<()>,
    /// The published item, live exactly while sharing.
    live: Arc<Mutex<Option<Live>>>,
    /// Whether an indicator *should* be up. Held separately from `live` so a
    /// `hide` that lands while `publish` is still in flight is not lost.
    wanted: Arc<AtomicBool>,
    /// Set once `Stop Sharing` has been requested, so the connection handler
    /// knows to end the accept loop rather than wait for the next client.
    pub stopping: Arc<AtomicBool>,
}

/// A published indicator and everything that must be undone to retract it.
struct Live {
    conn: Connection,
    name: String,
    /// The re-registration watcher. It holds its own `Connection` clone, so it
    /// has to be aborted before the name can actually be released.
    watcher: tokio::task::AbortHandle,
}

impl Indicator {
    pub fn new(output: &str) -> (Self, tokio::sync::mpsc::UnboundedReceiver<()>) {
        let (stop_tx, stop_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                output: output.to_string(),
                stop_tx,
                live: Arc::new(Mutex::new(None)),
                wanted: Arc::new(AtomicBool::new(false)),
                stopping: Arc::new(AtomicBool::new(false)),
            },
            stop_rx,
        )
    }

    /// Raise the indicator: a remote party can now see the screen.
    ///
    /// Callable from sync context (the ironrdp handlers are not async) and
    /// idempotent — a second call while an indicator is live does nothing.
    pub fn show(&self, peer: &str) {
        if self.wanted.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        let session = Session::new(peer, &self.output);
        tokio::spawn(async move {
            match this.publish(session).await {
                Ok(live) => {
                    // A `hide` may have landed while we were publishing. Retract
                    // immediately rather than storing an indicator nobody wants;
                    // the alternative is an icon that never goes away.
                    if !this.wanted.load(Ordering::SeqCst) {
                        retract(live);
                        return;
                    }
                    *this.live.lock().unwrap() = Some(live);
                }
                Err(e) => tracing::warn!("could not publish the sharing indicator: {e:#}"),
            }
        });
    }

    /// Drop the indicator: nobody is watching any more.
    pub fn hide(&self) {
        self.wanted.store(false, Ordering::SeqCst);
        if let Some(live) = self.live.lock().unwrap().take() {
            retract(live);
        }
    }

    async fn publish(&self, session: Session) -> anyhow::Result<Live> {
        // A well-known name (rather than just the unique one) is what most SNI
        // hosts expect to be handed, and it still vanishes with the connection.
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());

        let conn = zbus::ConnectionBuilder::session()?
            .name(name.as_str())?
            .serve_at(
                ITEM_PATH,
                Item {
                    session: session.clone(),
                },
            )?
            .serve_at(
                MENU_PATH,
                Menu {
                    session,
                    stop_tx: self.stop_tx.clone(),
                },
            )?
            .build()
            .await?;

        register(&conn, &name).await;

        // A host that starts later, or restarts, has never seen our
        // registration. Re-register whenever the watcher name changes owner.
        let watch_conn = conn.clone();
        let watch_name = name.clone();
        let watcher = tokio::spawn(async move {
            if let Err(e) = reregister_on_watcher_restart(watch_conn, watch_name).await {
                tracing::debug!("sharing indicator re-registration loop ended: {e}");
            }
        })
        .abort_handle();

        Ok(Live {
            conn,
            name,
            watcher,
        })
    }
}

/// Take the indicator down.
///
/// The name is released explicitly rather than by letting the connection drop:
/// `Connection` is reference-counted and the proxies and streams built from it
/// hold their own clones, so "drop the one we stored" is not the same as "close
/// the connection". Releasing the name is what hosts actually watch for, and it
/// does not depend on having accounted for every clone.
fn retract(live: Live) {
    live.watcher.abort();
    tokio::spawn(async move {
        if let Err(e) = live.conn.release_name(live.name.as_str()).await {
            tracing::warn!("could not release the sharing indicator's name: {e}");
        }
    });
}

async fn register(conn: &Connection, name: &str) {
    let result = conn
        .call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some(WATCHER_NAME),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await;
    match result {
        Ok(_) => tracing::info!("sharing indicator registered as {name}"),
        // No tray host running is normal, not an error worth shouting about —
        // the bridge still serves, there is simply nowhere to draw the icon.
        Err(e) => tracing::info!("no StatusNotifierWatcher to register the indicator with: {e}"),
    }
}

async fn reregister_on_watcher_restart(conn: Connection, name: String) -> zbus::Result<()> {
    // Via zbus rather than a direct futures-util dependency for one import.
    use zbus::export::futures_util::StreamExt;

    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.name.as_str() != WATCHER_NAME {
            continue;
        }
        if args.new_owner.is_some() {
            register(&conn, &name).await;
        }
    }
    Ok(())
}

// ── ironrdp connection lifecycle ───────────────────────────────────────────

/// Connection lifecycle → indicator visibility.
///
/// `on_accept` only records the peer. A bare TCP connect — a port scan, an
/// abandoned handshake — must not raise a "you are being watched" signal; the
/// indicator goes up when the display handler actually starts serving that
/// client (see `VirtualOutputDisplay::updates`).
pub struct ConnectionStatus {
    pub indicator: Indicator,
    pub peer: Arc<Mutex<String>>,
}

impl ironrdp_server::ConnectionHandler for ConnectionStatus {
    fn on_accept(&mut self, peer: std::net::SocketAddr) -> bool {
        *self.peer.lock().unwrap() = peer.to_string();
        true
    }

    fn on_disconnected(
        &mut self,
        peer: std::net::SocketAddr,
        duration: std::time::Duration,
        error: Option<&anyhow::Error>,
    ) -> ironrdp_server::PostConnectionAction {
        tracing::info!(
            "RDP client {peer} disconnected after {duration:?}{}",
            match error {
                Some(e) => format!(": {e:#}"),
                None => String::new(),
            }
        );
        self.peer.lock().unwrap().clear();
        self.indicator.hide();

        if self.indicator.stopping.load(Ordering::SeqCst) {
            ironrdp_server::PostConnectionAction::Stop
        } else {
            ironrdp_server::PostConnectionAction::Continue
        }
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn local_hhmm() -> String {
    // Seconds since local midnight, from the UTC clock plus the zone offset
    // the C library reports. Avoids pulling a date/time crate into the bridge
    // for one label.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let local = now + local_utc_offset_secs(now);
    let mins = local.rem_euclid(86_400) / 60;
    format!("{:02}:{:02}", mins / 60, mins % 60)
}

/// UTC offset in seconds for `unix_time`, via `localtime_r`.
fn local_utc_offset_secs(unix_time: i64) -> i64 {
    // SAFETY: `tm` is fully written by `localtime_r`, which takes a pointer to
    // caller-owned storage and is thread-safe.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = unix_time as libc::time_t;
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
}

/// An anti-aliased red disc as unpremultiplied ARGB32 in network byte order.
fn record_dot(size: i32) -> (i32, i32, Vec<u8>) {
    const RGB: (u8, u8, u8) = (0xFF, 0x3B, 0x30);
    const SUB: i32 = 4;

    let n = size as usize;
    let mut data = vec![0u8; n * n * 4];
    let centre = (size as f32 - 1.0) / 2.0;
    let radius = size as f32 * 0.34;

    for y in 0..size {
        for x in 0..size {
            // Supersample so the disc has a clean edge at any size.
            let mut inside = 0;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let px = x as f32 + (sx as f32 + 0.5) / SUB as f32 - 0.5 - centre;
                    let py = y as f32 + (sy as f32 + 0.5) / SUB as f32 - 0.5 - centre;
                    if px * px + py * py <= radius * radius {
                        inside += 1;
                    }
                }
            }
            let i = ((y as usize) * n + x as usize) * 4;
            data[i] = (inside * 255 / (SUB * SUB)) as u8;
            data[i + 1] = RGB.0;
            data[i + 2] = RGB.1;
            data[i + 3] = RGB.2;
        }
    }

    (size, size, data)
}
