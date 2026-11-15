//! The Network pane.
//!
//! None of it is a setting but the top bar switch: the network belongs to
//! NetworkManager, which keeps it, and is read and changed through
//! `otto-network` on threads of their own, the way the Sound pane treats the
//! sound server. The pane reads NetworkManager when it comes on screen and
//! follows its changes while it is. Its own identifiers live under
//! `network.`, which `org.otto.Settings` serves nothing under, and `main.rs`
//! hands them here before the bus sees them. See `specs/network.md`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use otto_network::{DeviceKind, DeviceState, LinkKind, Snapshot};

use crate::model::{group, untitled, Control, Group, Pane, Row};
use crate::settings_client::{self, Value};

/// The Wi-Fi switch.
const WIFI_ID: &str = "network.wifi";
/// A saved network's row: this, then its connection's object path.
const SAVED_ROW: &str = "network.saved.";

/// What the pane last learned from NetworkManager.
enum Read {
    /// Not asked yet.
    Pending,
    /// The system bus could not be reached.
    Failed,
    Ready(Box<Snapshot>),
}

static SNAPSHOT: RwLock<Read> = RwLock::new(Read::Pending);
static SHOWN: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Read NetworkManager and keep what it says, then have the pane redrawn.
fn reload() {
    let read = match otto_network::blocking::read() {
        Ok(snapshot) => Read::Ready(Box::new(snapshot)),
        Err(why) => {
            eprintln!("network: {why}");
            Read::Failed
        }
    };
    *SNAPSHOT.write().unwrap() = read;
    settings_client::request_redraw();
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("network: could not start {name}: {err}");
    }
}

/// Follow NetworkManager for as long as the app runs. Off screen a change
/// waits for the pane to be shown, which reads anyway.
fn watch() {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    in_background("network-watch", || {
        let result = otto_network::blocking::watch(|| {
            if SHOWN.load(Ordering::Relaxed) {
                reload();
            }
        });
        if let Err(why) = result {
            eprintln!("network: stopped following NetworkManager: {why}");
        }
        WATCHING.store(false, Ordering::Relaxed);
    });
}

/// Tell the pane whether it is on screen. Coming on screen reads afresh,
/// showing what was read last until the answer is in.
pub fn set_shown(shown: bool) {
    let was = SHOWN.swap(shown, Ordering::Relaxed);
    if shown && !was {
        watch();
        in_background("network-read", reload);
    }
}

/// Read NetworkManager now, on this thread. For the offscreen preview, which
/// draws once and cannot wait for the background read to come back.
pub fn load_now() {
    reload();
}

/// Run an action on a thread, then read again so the pane shows what took.
fn act(name: &str, work: impl FnOnce() -> zbus::Result<()> + Send + 'static) {
    in_background(name, move || {
        if let Err(why) = work() {
            eprintln!("network: {why}");
        }
        reload();
    });
}

/// A switch was flipped. Whether `id` is one of this pane's, so `main.rs`
/// knows not to send it to the compositor.
///
/// The switch shows the new state straight away; the read after the action
/// puts it back if NetworkManager refused.
pub fn apply(id: &str, value: &Value) -> bool {
    if id == WIFI_ID {
        if let Value::Bool(on) = *value {
            if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
                snapshot.wifi_enabled = on;
            }
            act("network-wifi", move || {
                otto_network::blocking::set_wifi_enabled(on)
            });
        }
        return true;
    }
    let Some(path) = id.strip_prefix(SAVED_ROW) else {
        return false;
    };
    if let Value::Bool(on) = *value {
        if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
            if let Some(saved) = snapshot.saved.iter_mut().find(|s| s.path == path) {
                saved.autoconnect = on;
            }
        }
        let path = path.to_string();
        act("network-autoconnect", move || {
            otto_network::blocking::set_autoconnect(&path, on)
        });
    }
    true
}

/// A press on a saved network's Forget.
pub fn remove(id: &str) {
    let Some(path) = id.strip_prefix(SAVED_ROW) else {
        return;
    };
    if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
        snapshot.saved.retain(|s| s.path != path);
    }
    let path = path.to_string();
    act("network-forget", move || {
        otto_network::blocking::forget(&path)
    });
}

/// `text` as a `&'static str`, kept once however often it is asked for: a
/// row's label and identifier are static, and the model is rebuilt every
/// frame.
fn intern(text: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interned = INTERNED.get_or_init(Default::default).lock().unwrap();
    interned
        .entry(text)
        .or_insert_with_key(|text| text.clone().leak())
}

/// A row that says something rather than holding a value.
fn note(text: &'static str) -> Row {
    Row::new(text, Control::Value(String::new()))
}

fn value(label: &'static str, value: impl Into<String>) -> Row {
    Row::new(label, Control::Value(value.into()))
}

fn device_state(kind: DeviceKind, state: DeviceState) -> &'static str {
    match (kind, state) {
        (_, DeviceState::Connected) => otto_kit::t!("settings-network-state-connected"),
        (_, DeviceState::Connecting) => otto_kit::t!("settings-network-state-connecting"),
        (_, DeviceState::Disconnecting) => otto_kit::t!("settings-network-state-disconnecting"),
        (DeviceKind::Ethernet, DeviceState::Unavailable) => {
            otto_kit::t!("settings-network-state-unplugged")
        }
        (DeviceKind::Wifi, DeviceState::Unavailable) => otto_kit::t!("settings-network-state-off"),
        (_, DeviceState::Failed) => otto_kit::t!("settings-network-state-failed"),
        _ => otto_kit::t!("settings-network-state-disconnected"),
    }
}

fn wifi_group(snapshot: &Snapshot) -> Group {
    let mut rows = Vec::new();
    if snapshot.wifi.is_some() {
        let mut switch = Row::new(
            otto_kit::t!("settings-network-wifi"),
            Control::Toggle(snapshot.wifi_enabled),
        );
        switch.id = Some(WIFI_ID);
        if !snapshot.wifi_hardware_enabled {
            switch = switch.detail(otto_kit::t!("settings-network-wifi-blocked"));
            switch.inactive = true;
        }
        rows.push(switch);
        let current = snapshot
            .connected_network()
            .map(|n| n.name())
            .unwrap_or_else(|| otto_kit::t!("settings-network-not-connected").to_string());
        rows.push(value(otto_kit::t!("settings-network-current"), current));
    }
    rows.push(
        Row::new(otto_kit::t!("settings-show-network"), Control::Toggle(true))
            .id("topbar.show_network"),
    );
    untitled(rows)
}

fn status_group(snapshot: &Snapshot) -> Option<Group> {
    if snapshot.devices.is_empty() {
        return None;
    }
    let rows = snapshot
        .devices
        .iter()
        .map(|device| {
            let kind = match device.kind {
                DeviceKind::Wifi => otto_kit::t!("settings-network-kind-wifi"),
                DeviceKind::Ethernet => otto_kit::t!("settings-network-kind-ethernet"),
            };
            Row::new(
                intern(format!("{kind} ({})", device.interface)),
                Control::Value(device_state(device.kind, device.state).to_string()),
            )
        })
        .collect();
    Some(group(otto_kit::t!("settings-group-network-status"), rows))
}

fn details_group(snapshot: &Snapshot) -> Option<Group> {
    let link = snapshot.primary.as_ref()?;
    let kind = match link.kind {
        LinkKind::Wifi => otto_kit::t!("settings-network-kind-wifi"),
        LinkKind::Ethernet => otto_kit::t!("settings-network-kind-ethernet"),
        LinkKind::Vpn => otto_kit::t!("settings-network-kind-vpn"),
        LinkKind::Other => otto_kit::t!("settings-network-kind-other"),
    };
    let mut rows = vec![
        value(otto_kit::t!("settings-network-connection"), link.id.clone()),
        value(otto_kit::t!("settings-network-kind"), kind),
    ];
    let joined = |list: &[String]| list.join(", ");
    if !link.ipv4.is_empty() {
        rows.push(value(
            otto_kit::t!("settings-network-ipv4"),
            joined(&link.ipv4),
        ));
    }
    if let Some(gateway) = &link.gateway {
        rows.push(value(
            otto_kit::t!("settings-network-router"),
            gateway.clone(),
        ));
    }
    if !link.dns.is_empty() {
        rows.push(value(
            otto_kit::t!("settings-network-dns"),
            joined(&link.dns),
        ));
    }
    if !link.ipv6.is_empty() {
        rows.push(value(
            otto_kit::t!("settings-network-ipv6"),
            joined(&link.ipv6),
        ));
    }
    if !link.hw_address.is_empty() {
        rows.push(value(
            otto_kit::t!("settings-network-hardware-address"),
            link.hw_address.clone(),
        ));
    }
    if snapshot.limited() {
        rows.push(note(otto_kit::t!("settings-network-limited")));
    }
    Some(group(otto_kit::t!("settings-group-network-details"), rows))
}

fn saved_group(snapshot: &Snapshot) -> Option<Group> {
    snapshot.wifi.as_ref()?;
    if snapshot.saved.is_empty() {
        return Some(group(
            otto_kit::t!("settings-group-network-known"),
            vec![note(otto_kit::t!("settings-network-known-none"))],
        ));
    }
    let rows = snapshot
        .saved
        .iter()
        .map(|saved| {
            let mut row = Row::new(intern(saved.name()), Control::Toggle(saved.autoconnect))
                .removable(true)
                .remove_label(otto_kit::t!("settings-network-forget"));
            row.id = Some(intern(format!("{SAVED_ROW}{}", saved.path)));
            let detail = if saved.in_use {
                otto_kit::t!("settings-network-known-in-use")
            } else {
                otto_kit::t!("settings-network-known-detail")
            };
            row.detail(detail)
        })
        .collect();
    Some(group(otto_kit::t!("settings-group-network-known"), rows))
}

pub fn build() -> Pane {
    let groups = match &*SNAPSHOT.read().unwrap() {
        Read::Pending => vec![untitled(vec![note(otto_kit::t!(
            "settings-network-reading"
        ))])],
        Read::Failed => vec![untitled(vec![note(otto_kit::t!(
            "settings-network-unavailable"
        ))])],
        Read::Ready(snapshot) if !snapshot.running => {
            vec![untitled(vec![note(otto_kit::t!(
                "settings-network-unavailable"
            ))
            .detail(otto_kit::t!("settings-network-unavailable-detail"))])]
        }
        Read::Ready(snapshot) => {
            let mut groups = vec![wifi_group(snapshot)];
            groups.extend(status_group(snapshot));
            groups.extend(details_group(snapshot));
            groups.extend(saved_group(snapshot));
            groups
        }
    };
    Pane {
        name: otto_kit::t!("settings-pane-network"),
        icon: "network",
        intro: None,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_network::{Device, SavedNetwork, WifiDevice};

    fn snapshot() -> Snapshot {
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
                networks: Vec::new(),
                last_scan_ms: None,
            }),
            saved: vec![SavedNetwork {
                path: "/c/7".into(),
                id: "Home".into(),
                ssid: b"Home".to_vec(),
                autoconnect: false,
                last_used: 0,
                in_use: false,
            }],
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_saved_network_is_a_forgettable_switch() {
        let group = saved_group(&snapshot()).expect("a known networks group");
        let row = &group.rows[0];
        assert_eq!(row.label, "Home");
        assert_eq!(row.id, Some("network.saved./c/7"));
        assert!(row.removable);
        assert!(matches!(row.control, Control::Toggle(false)));
    }

    #[test]
    fn no_wifi_means_no_known_networks() {
        let mut snapshot = snapshot();
        snapshot.wifi = None;
        assert!(saved_group(&snapshot).is_none());
    }

    #[test]
    fn the_pane_owns_its_identifiers_only() {
        assert!(!apply("dock.size", &Value::Bool(true)));
    }
}
