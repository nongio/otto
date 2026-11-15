//! The network indicator: Wi-Fi signal or wired, and a menu to join a
//! network.
//!
//! Drawn by the bar rather than left to a tray applet, for the reason the
//! battery is (`power.rs`): the tray carries an icon and a flat menu, and the
//! indicator needs signal bars and a menu that reads like the rest of the
//! bar's. The state is NetworkManager's, read through `otto-network` into a
//! snapshot whenever NetworkManager signals a change; whether the indicator
//! is there at all is the compositor's `topbar.show_network` setting. See
//! `specs/network.md`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use otto_kit::components::menu_item::{MenuItem as KitMenuItem, MenuItemIcon};
use otto_kit::dbus::settings::SettingsProxy;
use otto_kit::prelude::*;
use otto_kit::skia::{Canvas, Color, Paint, Rect};
use otto_network::{DeviceKind, DeviceState, Glyph, LinkState, Security, Snapshot, WifiNetwork};
use zbus::Connection;

use crate::battery::with_alpha;
use crate::settings::get_bool;

/// The setting that shows or hides the indicator.
const SHOW_ID: &str = "topbar.show_network";

/// What NetworkManager last reported.
static SNAPSHOT: LazyLock<Mutex<Snapshot>> = LazyLock::new(|| Mutex::new(Snapshot::default()));
/// Bumped whenever the snapshot or the setting changes.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Whether the user wants the indicator. On until the compositor says
/// otherwise, so a bar without Otto still shows the network.
static SHOWN: AtomicBool = AtomicBool::new(true);
/// System bus connection the actions run on.
static SYSTEM_BUS: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));
/// When the open menu last asked for a scan.
static LAST_SCAN_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

/// How often opening the menu may ask for a scan. NetworkManager refuses
/// more anyway; this saves asking.
const SCAN_INTERVAL: Duration = Duration::from_secs(10);

/// The snapshot as last read.
pub fn snapshot() -> Snapshot {
    SNAPSHOT.lock().unwrap().clone()
}

/// Changes whenever anything the indicator or its menu shows changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn bump() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    AppContext::request_wakeup();
}

/// What the indicator draws, or `Hidden`.
pub fn glyph() -> Glyph {
    if !SHOWN.load(Ordering::Relaxed) {
        return Glyph::Hidden;
    }
    SNAPSHOT.lock().unwrap().glyph()
}

/// Whether the indicator is drawn.
pub fn visible() -> bool {
    glyph() != Glyph::Hidden
}

// ---------------------------------------------------------------------------
// Watchers
// ---------------------------------------------------------------------------

/// Follow NetworkManager and the setting. Idempotent.
pub fn spawn_watcher() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async {
        if let Err(e) = follow_network().await {
            tracing::warn!("network indicator: {e}");
        }
    });
    tokio::spawn(async {
        if let Err(e) = follow_setting().await {
            tracing::warn!("network indicator setting: {e}");
        }
    });
}

async fn follow_network() -> zbus::Result<()> {
    let conn = Connection::system().await?;
    *SYSTEM_BUS.lock().unwrap() = Some(conn.clone());
    reread(&conn).await;
    // `watch` hands back a plain callback; the re-read it asks for runs on
    // the runtime, one at a time, the latest request winning.
    let (tx, mut rx) = tokio::sync::watch::channel(());
    let reader = conn.clone();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            reread(&reader).await;
        }
    });
    otto_network::watch(&conn, move || {
        let _ = tx.send(());
    })
    .await
}

async fn reread(conn: &Connection) {
    let next = match otto_network::read(conn).await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            tracing::debug!("network read failed: {e}");
            return;
        }
    };
    let mut slot = SNAPSHOT.lock().unwrap();
    if *slot != next {
        *slot = next;
        drop(slot);
        bump();
    }
}

async fn follow_setting() -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let proxy = SettingsProxy::new(&conn).await?;
    let mut changes = proxy.receive_changed().await?;
    let mut owners = proxy.inner().receive_owner_changed().await?;
    read_setting(&proxy).await;
    loop {
        tokio::select! {
            Some(signal) = changes.next() => {
                if signal.args().is_ok_and(|args| args.values.contains_key(SHOW_ID)) {
                    read_setting(&proxy).await;
                }
            }
            Some(owner) = owners.next() => {
                if owner.is_some() {
                    read_setting(&proxy).await;
                }
            }
            else => break,
        }
    }
    Ok(())
}

async fn read_setting(proxy: &SettingsProxy<'_>) {
    if let Some(show) = get_bool(proxy, SHOW_ID).await {
        if SHOWN.swap(show, Ordering::Relaxed) != show {
            bump();
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Run `work` on the system bus connection, logging a failure. The indicator
/// follows whatever NetworkManager does next through the watcher.
fn act<F, Fut>(what: &'static str, work: F)
where
    F: FnOnce(Connection) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = zbus::Result<()>> + Send,
{
    let Some(conn) = SYSTEM_BUS.lock().unwrap().clone() else {
        tracing::warn!("{what}: no system bus");
        return;
    };
    tokio::runtime::Handle::current().spawn(async move {
        if let Err(e) = work(conn).await {
            tracing::warn!("{what}: {e}");
        }
    });
}

/// Ask for a fresh scan, at most every [`SCAN_INTERVAL`]: the menu is
/// opening, and the list should be current by the time it is read.
pub fn scan_soon() {
    let Some(device) = snapshot().wifi.map(|w| w.device.path) else {
        return;
    };
    {
        let mut last = LAST_SCAN_REQUEST.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < SCAN_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
    }
    act("wifi scan", move |conn| async move {
        otto_network::request_scan(&conn, &device).await
    });
}

/// A menu entry was chosen.
pub fn activate(action_id: &str) {
    let snapshot = snapshot();
    if action_id == "wifi" {
        let on = !snapshot.wifi_enabled;
        act("wifi switch", move |conn| async move {
            otto_network::set_wifi_enabled(&conn, on).await
        });
    } else if let Some(hex) = action_id.strip_prefix("net:") {
        let Some(wifi) = snapshot.wifi else { return };
        let Some(network) = wifi
            .networks
            .into_iter()
            .find(|n| encode_ssid(&n.ssid) == hex)
        else {
            return;
        };
        if network.state != LinkState::Idle {
            return;
        }
        let device = wifi.device.path;
        act("wifi connect", move |conn| async move {
            otto_network::connect_wifi(&conn, &device, &network).await
        });
    } else if action_id == "disconnect" {
        let Some(device) = snapshot.wifi.map(|w| w.device.path) else {
            return;
        };
        act("wifi disconnect", move |conn| async move {
            otto_network::disconnect_device(&conn, &device).await
        });
    } else if let Some(path) = action_id.strip_prefix("vpn:") {
        let Some(vpn) = snapshot.vpns.into_iter().find(|v| v.path == path) else {
            return;
        };
        act("vpn", move |conn| async move {
            match vpn.active {
                Some(active) => otto_network::deactivate(&conn, &active).await,
                None => otto_network::activate(&conn, &vpn.path).await,
            }
        });
    }
}

/// A network name as an action id: hex, since a name can hold any byte,
/// a colon included.
fn encode_ssid(ssid: &[u8]) -> String {
    ssid.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

/// The freedesktop icon for a signal.
fn signal_icon(bars: u8) -> MenuItemIcon {
    let name = match bars {
        0 => "network-wireless-signal-weak-symbolic",
        1 => "network-wireless-signal-ok-symbolic",
        2 => "network-wireless-signal-good-symbolic",
        _ => "network-wireless-signal-excellent-symbolic",
    };
    MenuItemIcon::Named(name.into())
}

fn network_item(network: &WifiNetwork) -> KitMenuItem {
    let name = network.name();
    let label = match network.state {
        LinkState::Connecting => otto_kit::t_owned!("bar-network-connecting", name = name),
        _ => name,
    };
    let mut item = KitMenuItem::action(label)
        .with_action_id(format!("net:{}", encode_ssid(&network.ssid)))
        .with_icon(signal_icon(network.bars()));
    if network.security.is_secured() {
        // The lock, where a menu has room for one: beside the name, dimmed,
        // in the shortcut column.
        item = item.with_shortcut(otto_kit::t!("bar-network-secured"));
    }
    item = match network.state {
        LinkState::Connected => item.with_checked(true),
        _ => item.with_check_gutter(),
    };
    if network.security == Security::Enterprise && network.saved.is_none() {
        // 802.1X needs an identity and a method the menu cannot ask for.
        item = item.disabled();
    }
    item
}

/// The menu's items for `snapshot`. `settings` is whether there is a
/// settings app to open.
pub fn menu_items(snapshot: &Snapshot, settings: bool) -> Vec<KitMenuItem> {
    let mut items = Vec::new();

    if let Some(wifi) = &snapshot.wifi {
        let mut toggle = KitMenuItem::action(otto_kit::t!("bar-network-wifi"))
            .with_action_id("wifi".to_string())
            .with_checked(snapshot.wifi_enabled);
        if !snapshot.wifi_hardware_enabled {
            toggle = toggle.disabled();
        }
        items.push(toggle);

        if snapshot.wifi_enabled {
            items.push(KitMenuItem::separator());
            let (known, other): (Vec<_>, Vec<_>) = wifi
                .networks
                .iter()
                .partition(|n| n.saved.is_some() || n.state != LinkState::Idle);
            if wifi.networks.is_empty() {
                items.push(KitMenuItem::action(otto_kit::t!("bar-network-none-found")).disabled());
            }
            items.extend(known.iter().map(|n| network_item(n)));
            if !other.is_empty() {
                let others: Vec<_> = other.iter().map(|n| network_item(n)).collect();
                if known.is_empty() {
                    // Nothing saved in range: no reason to fold the list.
                    items.extend(others);
                } else {
                    items.push(KitMenuItem::submenu(
                        otto_kit::t!("bar-network-other"),
                        others,
                    ));
                }
            }
        }
    }

    let wired: Vec<_> = snapshot
        .devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Ethernet)
        .collect();
    if !wired.is_empty() {
        if !items.is_empty() {
            items.push(KitMenuItem::separator());
        }
        for device in wired {
            let label = match device.state {
                DeviceState::Connected => otto_kit::t!("bar-network-ethernet-connected"),
                DeviceState::Connecting => otto_kit::t!("bar-network-ethernet-connecting"),
                DeviceState::Unavailable => otto_kit::t!("bar-network-ethernet-unplugged"),
                _ => otto_kit::t!("bar-network-ethernet-disconnected"),
            };
            items.push(
                KitMenuItem::action(label)
                    .with_icon(MenuItemIcon::Named("network-wired-symbolic".into()))
                    .disabled(),
            );
        }
    }

    if !snapshot.vpns.is_empty() {
        items.push(KitMenuItem::separator());
        for vpn in &snapshot.vpns {
            let label = match vpn.state {
                LinkState::Connecting => {
                    otto_kit::t_owned!("bar-network-connecting", name = vpn.id.clone())
                }
                _ => vpn.id.clone(),
            };
            items.push(
                KitMenuItem::action(label)
                    .with_action_id(format!("vpn:{}", vpn.path))
                    .with_checked(vpn.state == LinkState::Connected),
            );
        }
    }

    let connected = snapshot.connected_network().map(|n| n.name());
    if connected.is_some() || settings {
        items.push(KitMenuItem::separator());
    }
    if let Some(name) = connected {
        items.push(
            KitMenuItem::action(otto_kit::t_owned!("bar-network-disconnect", name = name))
                .with_action_id("disconnect".to_string()),
        );
    }
    if settings {
        items.push(
            KitMenuItem::action(otto_kit::t!("bar-network-settings"))
                .with_action_id("settings".to_string()),
        );
    }
    items
}

/// The indicator's state in words, for a screen reader.
pub fn label() -> String {
    let snapshot = snapshot();
    let state = match snapshot.glyph() {
        Glyph::Hidden | Glyph::Disconnected => {
            otto_kit::t!("bar-network-label-disconnected").to_string()
        }
        Glyph::WifiOff => otto_kit::t!("bar-network-label-off").to_string(),
        Glyph::Connecting => otto_kit::t!("bar-network-label-connecting").to_string(),
        Glyph::Wired => otto_kit::t!("bar-network-label-wired").to_string(),
        Glyph::Wifi(bars) => {
            let signal = match bars {
                0 => otto_kit::t!("bar-network-signal-weak"),
                1 => otto_kit::t!("bar-network-signal-fair"),
                2 => otto_kit::t!("bar-network-signal-good"),
                _ => otto_kit::t!("bar-network-signal-excellent"),
            };
            let name = snapshot
                .connected_network()
                .map(|n| n.name())
                .unwrap_or_default();
            otto_kit::t_owned!("bar-network-label-wifi", name = name, signal = signal)
        }
    };
    if snapshot.limited() {
        otto_kit::t_owned!("bar-network-label-limited", state = state)
    } else {
        state
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The fan's width, and the wired glyph's.
const GLYPH_WIDTH: f32 = 19.0;
/// Room for the exclamation mark of a connection with no internet.
const LIMITED_WIDTH: f32 = 4.0;
/// Radii of the fan's three arcs, inside out.
const ARC_RADII: [f32; 3] = [4.6, 8.2, 11.8];
const ARC_STROKE: f32 = 2.1;
/// The dot at the fan's point.
const DOT_RADIUS: f32 = 1.7;
/// Alpha of a bar that is not lit.
const UNLIT: f32 = 0.28;
/// How long each step of the connecting animation shows.
const STEP: Duration = Duration::from_millis(350);

/// When the animation started; the steps are counted from here.
static ANIMATION_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
/// The step last drawn, so a wake that changes nothing draws nothing.
static LAST_STEP: AtomicU64 = AtomicU64::new(u64::MAX);

/// Whether the indicator is animating.
pub fn animating() -> bool {
    glyph() == Glyph::Connecting
}

/// How long until the animation's next step, while it runs.
pub fn until_next_step() -> Option<Duration> {
    if !animating() {
        return None;
    }
    let elapsed = ANIMATION_EPOCH.elapsed().as_millis() as u64;
    let step = STEP.as_millis() as u64;
    Some(Duration::from_millis(step - elapsed % step))
}

fn current_step() -> u64 {
    ANIMATION_EPOCH.elapsed().as_millis() as u64 / STEP.as_millis() as u64
}

/// Whether the animation moved on since it was last drawn.
pub fn step_changed() -> bool {
    animating() && LAST_STEP.load(Ordering::Relaxed) != current_step()
}

/// How wide the indicator is, or zero when it is not drawn.
pub fn width() -> f32 {
    if !visible() {
        return 0.0;
    }
    if SNAPSHOT.lock().unwrap().limited() {
        GLYPH_WIDTH + LIMITED_WIDTH
    } else {
        GLYPH_WIDTH
    }
}

/// Draw the indicator at `x`, vertically centred in a bar `height` tall.
pub fn draw(canvas: &Canvas, x: f32, height: f32, color: Color) {
    let glyph = glyph();
    if glyph == Glyph::Hidden {
        return;
    }
    let limited = SNAPSHOT.lock().unwrap().limited();
    // A connection with no internet is drawn fainter, so the exclamation
    // mark beside it reads as the point.
    let lit = if limited {
        with_alpha(color, 0.6)
    } else {
        color
    };
    let unlit = with_alpha(color, UNLIT);

    match glyph {
        Glyph::Wired => draw_wired(canvas, x, height, lit),
        Glyph::Wifi(bars) => draw_fan(canvas, x, height, bars as usize + 1, lit, unlit),
        Glyph::Connecting => {
            let step = current_step();
            LAST_STEP.store(step, Ordering::Relaxed);
            // The dot alone, then one arc more each step.
            draw_fan(canvas, x, height, (step % 4) as usize + 1, lit, unlit);
        }
        Glyph::Disconnected => draw_fan(canvas, x, height, 0, lit, unlit),
        Glyph::WifiOff => {
            draw_fan(canvas, x, height, 0, lit, unlit);
            let mut paint = stroke(color, 1.6);
            paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
            let (top, bottom) = fan_extent(height);
            canvas.draw_line((x + 3.0, top), (x + GLYPH_WIDTH - 3.0, bottom), &paint);
        }
        Glyph::Hidden => {}
    }

    if limited {
        let mark_x = x + GLYPH_WIDTH + LIMITED_WIDTH / 2.0;
        let (top, bottom) = fan_extent(height);
        let mut paint = stroke(color, 1.8);
        paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
        canvas.draw_line((mark_x, top + 2.0), (mark_x, bottom - 4.0), &paint);
        let mut dot = Paint::default();
        dot.set_anti_alias(true);
        dot.set_color(color);
        canvas.draw_circle((mark_x, bottom - 0.9), 1.1, &dot);
    }
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(otto_kit::skia::paint::Style::Stroke);
    paint.set_stroke_width(width);
    paint.set_color(color);
    paint
}

/// The top and bottom of the fan, centred in the bar.
fn fan_extent(height: f32) -> (f32, f32) {
    let fan_height = ARC_RADII[2] + ARC_STROKE / 2.0 + DOT_RADIUS;
    let top = ((height - fan_height) / 2.0).round();
    (top, top + fan_height)
}

/// The Wi-Fi fan: a dot and three arcs above it. `lit` counts the parts
/// drawn full strength, the dot first.
fn draw_fan(canvas: &Canvas, x: f32, height: f32, lit: usize, on: Color, off: Color) {
    let (_, bottom) = fan_extent(height);
    let cx = x + GLYPH_WIDTH / 2.0;
    let cy = bottom - DOT_RADIUS;

    let mut dot = Paint::default();
    dot.set_anti_alias(true);
    dot.set_color(if lit >= 1 { on } else { off });
    canvas.draw_circle((cx, cy), DOT_RADIUS, &dot);

    for (i, radius) in ARC_RADII.iter().enumerate() {
        let mut paint = stroke(if lit >= i + 2 { on } else { off }, ARC_STROKE);
        paint.set_stroke_cap(otto_kit::skia::paint::Cap::Round);
        let oval = Rect::from_xywh(cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        // Skia's angles run clockwise from the right: 225° to 315° is the
        // quarter above the centre.
        canvas.draw_arc(oval, 225.0, 90.0, false, &paint);
    }
}

/// The wired glyph: three linked boxes, a network in its simplest drawing.
fn draw_wired(canvas: &Canvas, x: f32, height: f32, color: Color) {
    let (top, bottom) = fan_extent(height);
    let box_size = 5.0;
    let cx = x + GLYPH_WIDTH / 2.0;
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(color);
    let upper = Rect::from_xywh(cx - box_size / 2.0, top, box_size, box_size);
    let left = Rect::from_xywh(x + 2.0, bottom - box_size, box_size, box_size);
    let right = Rect::from_xywh(
        x + GLYPH_WIDTH - 2.0 - box_size,
        bottom - box_size,
        box_size,
        box_size,
    );
    for r in [upper, left, right] {
        canvas.draw_round_rect(r, 1.2, 1.2, &fill);
    }
    let line = stroke(color, 1.4);
    let mid = (top + box_size + bottom - box_size) / 2.0;
    canvas.draw_line((cx, top + box_size), (cx, mid), &line);
    canvas.draw_line((left.center_x(), mid), (right.center_x(), mid), &line);
    canvas.draw_line((left.center_x(), mid), (left.center_x(), left.top), &line);
    canvas.draw_line(
        (right.center_x(), mid),
        (right.center_x(), right.top),
        &line,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_network::{Device, WifiDevice};

    fn snapshot_with(networks: Vec<WifiNetwork>) -> Snapshot {
        let device = Device {
            path: "/d/1".into(),
            kind: DeviceKind::Wifi,
            interface: "wlan0".into(),
            state: DeviceState::Connected,
            hw_address: String::new(),
        };
        Snapshot {
            running: true,
            wifi_enabled: true,
            wifi_hardware_enabled: true,
            devices: vec![device.clone()],
            wifi: Some(WifiDevice {
                device,
                networks,
                last_scan_ms: None,
            }),
            ..Snapshot::default()
        }
    }

    fn network(name: &str, saved: bool, state: LinkState) -> WifiNetwork {
        WifiNetwork {
            ssid: name.as_bytes().to_vec(),
            strength: 70,
            security: Security::Personal,
            access_point: format!("/ap/{name}"),
            saved: saved.then(|| format!("/c/{name}")),
            state,
        }
    }

    fn labels(items: &[KitMenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| i.label().unwrap_or("—").to_string())
            .collect()
    }

    #[test]
    fn other_networks_fold_under_saved_ones() {
        let snapshot = snapshot_with(vec![
            network("Home", true, LinkState::Connected),
            network("Cafe", false, LinkState::Idle),
        ]);
        let items = menu_items(&snapshot, true);
        let labels = labels(&items);
        assert_eq!(labels[0], otto_kit::t!("bar-network-wifi"));
        assert!(labels.contains(&"Home".to_string()));
        assert!(!labels.contains(&"Cafe".to_string()));
        let other = items
            .iter()
            .find(|i| i.has_submenu())
            .expect("an Other Networks submenu");
        assert_eq!(other.submenu_items().unwrap()[0].label(), Some("Cafe"));
    }

    #[test]
    fn with_nothing_saved_the_list_is_flat() {
        let snapshot = snapshot_with(vec![network("Cafe", false, LinkState::Idle)]);
        let items = menu_items(&snapshot, false);
        assert!(items.iter().all(|i| !i.has_submenu()));
        assert!(labels(&items).contains(&"Cafe".to_string()));
    }

    #[test]
    fn wifi_off_hides_the_list() {
        let mut snapshot = snapshot_with(vec![network("Home", true, LinkState::Idle)]);
        snapshot.wifi_enabled = false;
        let labels = labels(&menu_items(&snapshot, false));
        assert!(!labels.contains(&"Home".to_string()));
    }

    #[test]
    fn a_name_survives_the_action_id() {
        let ssid = b"caf\xc3\xa9:1";
        assert_eq!(encode_ssid(ssid), "636166c3a93a31");
    }
}
