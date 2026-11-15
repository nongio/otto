//! The Network pane: Wi-Fi, Ethernet and VPN, each a tab, with the link's
//! details and an editor for its TCP/IP and DNS settings.
//!
//! None of it is a setting but the top bar switch: the network belongs to
//! NetworkManager, which keeps it, and is read and changed through
//! `otto-network` on threads of their own, the way the Sound pane treats the
//! sound server. The pane reads NetworkManager when it comes on screen and
//! follows its changes while it is. Its own identifiers live under
//! `network.`, which `org.otto.Settings` serves nothing under, and `main.rs`
//! hands them here before the bus sees them. See `specs/network.md`.
//!
//! The TCP/IP and DNS rows edit a draft of the shown connection's settings.
//! Nothing reaches NetworkManager until Apply, which checks the draft first:
//! a half-typed address is never saved, and switching to Manual does not
//! drop the connection before an address is in.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use otto_network::{
    parse_address, split_list, Band, DeviceKind, DeviceState, Family, IpConfig, IpError, IpMethod,
    LinkState, Profile, ProfileKind, Security, Snapshot,
};

use crate::discovery::Choice;
use crate::model::{group, untitled, Control, Group, Pane, Row};
use crate::settings_client::{self, Value};

/// The tab bar.
const TABS_ID: &str = "network.tabs";
/// The Wi-Fi switch.
const WIFI_ID: &str = "network.wifi";
/// Which saved Wi-Fi network the TCP/IP rows edit.
const PROFILE_ID: &str = "network.profile";
/// A saved network's row: this, then its connection's object path.
const SAVED_ROW: &str = "network.saved.";
/// A VPN's row: this, then its connection's object path.
const VPN_ROW: &str = "network.vpn.";
const V4_METHOD_ID: &str = "network.ipv4.method";
const V4_ADDRESS_ID: &str = "network.ipv4.address";
const V4_GATEWAY_ID: &str = "network.ipv4.gateway";
const V6_METHOD_ID: &str = "network.ipv6.method";
const V6_ADDRESS_ID: &str = "network.ipv6.address";
const V6_GATEWAY_ID: &str = "network.ipv6.gateway";
const DNS_AUTO_ID: &str = "network.dns.auto";
const DNS_SERVERS_ID: &str = "network.dns.servers";
const DNS_SEARCH_ID: &str = "network.dns.search";
const APPLY_ID: &str = "network.apply";
/// The VPN tab's Import… button.
const IMPORT_ID: &str = "network.vpn-import";

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

/// The pane's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Wifi,
    Ethernet,
    Vpn,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Wifi => otto_kit::t!("settings-network-kind-wifi"),
            Tab::Ethernet => otto_kit::t!("settings-network-kind-ethernet"),
            Tab::Vpn => otto_kit::t!("settings-network-kind-vpn"),
        }
    }
}

/// What the person is doing on the pane, kept across rebuilds.
struct Ui {
    tab: Option<Tab>,
    /// The saved Wi-Fi network picked in "Settings for"; `None` follows the
    /// one in use.
    wifi_profile: Option<String>,
    draft: Option<Draft>,
    /// Why the last Apply was refused, or how it failed.
    error: Option<String>,
    /// An Apply is on its way.
    applying: bool,
}

/// Edits to one connection's TCP/IP and DNS, not yet applied.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    profile: String,
    v4: IpConfig,
    v6: IpConfig,
    /// What a field holds as typed, kept so text that does not parse yet is
    /// not thrown away under the person's fingers.
    typed: HashMap<&'static str, String>,
}

static UI: Mutex<Ui> = Mutex::new(Ui {
    tab: None,
    wifi_profile: None,
    draft: None,
    error: None,
    applying: false,
});

fn ui() -> std::sync::MutexGuard<'static, Ui> {
    UI.lock().unwrap()
}

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

// ---------------------------------------------------------------------------
// Which tab, which connection
// ---------------------------------------------------------------------------

/// The tabs this machine has: Wi-Fi and Ethernet where there is such a
/// device, VPN always, since a VPN needs no device of its own.
fn tabs(snapshot: &Snapshot) -> Vec<Tab> {
    let mut tabs = Vec::new();
    if snapshot.wifi.is_some() {
        tabs.push(Tab::Wifi);
    }
    if snapshot
        .devices
        .iter()
        .any(|d| d.kind == DeviceKind::Ethernet)
    {
        tabs.push(Tab::Ethernet);
    }
    tabs.push(Tab::Vpn);
    tabs
}

/// The tab shown: the one picked, or else the kind of the connection in use,
/// so the pane opens on what the machine is actually using.
fn current_tab(snapshot: &Snapshot) -> Tab {
    let tabs = tabs(snapshot);
    if let Some(tab) = ui().tab.filter(|t| tabs.contains(t)) {
        return tab;
    }
    let primary = snapshot.primary.as_ref().map(|p| p.kind);
    if primary == Some(otto_network::LinkKind::Ethernet) && tabs.contains(&Tab::Ethernet) {
        return Tab::Ethernet;
    }
    tabs[0]
}

/// The labels for a set of tabs, as the `'static` slice a tab row wants. A
/// machine has at most a handful of combinations; each is made once.
fn tab_labels(tabs: &[Tab]) -> &'static [&'static str] {
    static MADE: OnceLock<Mutex<HashMap<Vec<&'static str>, &'static [&'static str]>>> =
        OnceLock::new();
    let labels: Vec<&'static str> = tabs.iter().map(|t| t.label()).collect();
    let mut made = MADE.get_or_init(Default::default).lock().unwrap();
    made.entry(labels.clone())
        .or_insert_with(|| Vec::leak(labels))
}

/// A tab was picked in the row `id`. Whether the row is this module's.
pub fn select_tab(id: &str, index: usize) -> bool {
    if id != TABS_ID {
        return false;
    }
    if let Read::Ready(snapshot) = &*SNAPSHOT.read().unwrap() {
        if let Some(tab) = tabs(snapshot).get(index) {
            ui().tab = Some(*tab);
        }
    }
    true
}

/// The saved connection the TCP/IP rows edit on `tab`.
fn edited_profile(snapshot: &Snapshot, tab: Tab) -> Option<&Profile> {
    let find = |path: &str| snapshot.profiles.iter().find(|p| p.path == path);
    match tab {
        Tab::Wifi => {
            let picked = ui().wifi_profile.clone();
            picked
                .as_deref()
                .and_then(find)
                .or_else(|| {
                    snapshot
                        .wifi
                        .as_ref()?
                        .device
                        .active_profile
                        .as_deref()
                        .and_then(find)
                })
                .or_else(|| {
                    let first = snapshot.saved.first()?;
                    find(&first.path)
                })
        }
        Tab::Ethernet => snapshot
            .devices
            .iter()
            .filter(|d| d.kind == DeviceKind::Ethernet)
            .find_map(|d| d.active_profile.as_deref().and_then(find))
            .or_else(|| {
                snapshot
                    .profiles
                    .iter()
                    .filter(|p| p.kind == ProfileKind::Ethernet)
                    .max_by_key(|p| p.last_used)
            }),
        Tab::Vpn => None,
    }
}

/// The configuration the rows show for `profile`: the draft where there is
/// one for it, otherwise what is saved.
fn shown_config(profile: &Profile) -> Draft {
    match ui().draft.as_ref().filter(|d| d.profile == profile.path) {
        Some(draft) => draft.clone(),
        None => Draft {
            profile: profile.path.clone(),
            v4: profile.ipv4.clone(),
            v6: profile.ipv6.clone(),
            typed: HashMap::new(),
        },
    }
}

/// Change the draft for the connection the shown tab edits.
fn edit(change: impl FnOnce(&mut Draft)) {
    let Read::Ready(snapshot) = &*SNAPSHOT.read().unwrap() else {
        return;
    };
    let tab = current_tab(snapshot);
    let Some(profile) = edited_profile(snapshot, tab) else {
        return;
    };
    let mut draft = shown_config(profile);
    change(&mut draft);
    let mut ui = ui();
    ui.error = None;
    // An edit back to what is saved is no edit.
    let unchanged = draft.v4 == profile.ipv4 && draft.v6 == profile.ipv6 && draft.typed.is_empty();
    ui.draft = (!unchanged).then_some(draft);
}

// ---------------------------------------------------------------------------
// Hooks main.rs calls
// ---------------------------------------------------------------------------

/// Every pop-up this module owns, for the menu pool built at startup: the
/// pane has no rows until NetworkManager answers, so the walk over the
/// panes would find none of them.
pub fn slot_ids() -> &'static [&'static str] {
    &[PROFILE_ID, V4_METHOD_ID, V6_METHOD_ID]
}

fn method_label(method: &IpMethod, family: Family) -> String {
    match method {
        IpMethod::Automatic => match family {
            Family::V4 => otto_kit::t!("settings-network-method-dhcp").to_string(),
            Family::V6 => otto_kit::t!("settings-network-method-auto").to_string(),
        },
        IpMethod::DhcpOnly => otto_kit::t!("settings-network-method-dhcp-only").to_string(),
        IpMethod::Manual => otto_kit::t!("settings-network-method-manual").to_string(),
        IpMethod::LinkLocal => otto_kit::t!("settings-network-method-link-local").to_string(),
        IpMethod::Shared => otto_kit::t!("settings-network-method-shared").to_string(),
        IpMethod::Disabled => otto_kit::t!("settings-network-method-off").to_string(),
        IpMethod::Other(raw) => raw.clone(),
    }
}

fn family_of(id: &str) -> Option<Family> {
    match id {
        V4_METHOD_ID | V4_ADDRESS_ID | V4_GATEWAY_ID => Some(Family::V4),
        V6_METHOD_ID | V6_ADDRESS_ID | V6_GATEWAY_ID => Some(Family::V6),
        _ => None,
    }
}

/// The choices of one of this module's pop-ups. `None` for any other.
pub fn menu_choices(id: &str) -> Option<Vec<Choice>> {
    match id {
        V4_METHOD_ID | V6_METHOD_ID => {
            let family = family_of(id)?;
            Some(
                IpMethod::choices(family)
                    .iter()
                    .map(|m| Choice {
                        label: method_label(m, family),
                        value: m.to_nm(family),
                    })
                    .collect(),
            )
        }
        PROFILE_ID => {
            let Read::Ready(snapshot) = &*SNAPSHOT.read().unwrap() else {
                return Some(Vec::new());
            };
            Some(
                snapshot
                    .saved
                    .iter()
                    .map(|s| Choice {
                        label: s.name(),
                        value: s.path.clone(),
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

/// What one of this module's pop-ups shows for `value`.
pub fn display(id: &str, value: &str) -> Option<String> {
    match id {
        V4_METHOD_ID => Some(method_label(&IpMethod::from_nm(value), Family::V4)),
        V6_METHOD_ID => Some(method_label(&IpMethod::from_nm(value), Family::V6)),
        PROFILE_ID => {
            let Read::Ready(snapshot) = &*SNAPSHOT.read().unwrap() else {
                return Some(String::new());
            };
            Some(
                snapshot
                    .saved
                    .iter()
                    .find(|s| s.path == value)
                    .map(|s| s.name())
                    .unwrap_or_default(),
            )
        }
        _ => None,
    }
}

/// A pop-up choice. Whether `id` is one of this module's.
pub fn choose(id: &str, value: &str) -> bool {
    match id {
        V4_METHOD_ID | V6_METHOD_ID => {
            let Some(family) = family_of(id) else {
                return true;
            };
            let method = IpMethod::from_nm(value);
            edit(|draft| {
                let config = match family {
                    Family::V4 => &mut draft.v4,
                    Family::V6 => &mut draft.v6,
                };
                config.method = method;
            });
            settings_client::request_redraw();
            true
        }
        PROFILE_ID => {
            let mut ui = ui();
            ui.wifi_profile = Some(value.to_string());
            ui.draft = None;
            ui.error = None;
            drop(ui);
            settings_client::request_redraw();
            true
        }
        _ => false,
    }
}

/// A switch was flipped or a field committed. Whether `id` is one of this
/// pane's, so `main.rs` knows not to send it to the compositor.
///
/// A switch shows the new state straight away; the read after the action
/// puts it back if NetworkManager refused.
pub fn apply(id: &str, value: &Value) -> bool {
    match (id, value) {
        (WIFI_ID, Value::Bool(on)) => {
            let on = *on;
            if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
                snapshot.wifi_enabled = on;
            }
            act("network-wifi", move || {
                otto_network::blocking::set_wifi_enabled(on)
            });
            return true;
        }
        (DNS_AUTO_ID, Value::Bool(on)) => {
            let ignore = !*on;
            edit(|draft| {
                draft.v4.ignore_auto_dns = ignore;
                draft.v6.ignore_auto_dns = ignore;
            });
            return true;
        }
        (V4_ADDRESS_ID | V6_ADDRESS_ID | V4_GATEWAY_ID | V6_GATEWAY_ID, Value::Text(text)) => {
            let text = text.trim().to_string();
            let family = family_of(id).expect("an address row");
            let is_address = matches!(id, V4_ADDRESS_ID | V6_ADDRESS_ID);
            let static_id: &'static str = match id {
                V4_ADDRESS_ID => V4_ADDRESS_ID,
                V6_ADDRESS_ID => V6_ADDRESS_ID,
                V4_GATEWAY_ID => V4_GATEWAY_ID,
                _ => V6_GATEWAY_ID,
            };
            edit(|draft| {
                let config = match family {
                    Family::V4 => &mut draft.v4,
                    Family::V6 => &mut draft.v6,
                };
                if is_address {
                    let parsed: Result<Vec<_>, _> = split_list(&text)
                        .iter()
                        .map(|a| parse_address(a, family))
                        .collect();
                    match parsed {
                        Ok(addresses) => {
                            config.addresses = addresses;
                            draft.typed.remove(static_id);
                        }
                        Err(_) => {
                            draft.typed.insert(static_id, text.clone());
                        }
                    }
                } else {
                    config.gateway = (!text.is_empty()).then(|| text.clone());
                }
            });
            return true;
        }
        (DNS_SERVERS_ID, Value::Text(text)) => {
            let servers = split_list(text);
            edit(|draft| {
                draft.v4.dns = servers
                    .iter()
                    .filter(|s| !s.contains(':'))
                    .cloned()
                    .collect();
                draft.v6.dns = servers
                    .iter()
                    .filter(|s| s.contains(':'))
                    .cloned()
                    .collect();
            });
            return true;
        }
        (DNS_SEARCH_ID, Value::Text(text)) => {
            let domains = split_list(text);
            edit(|draft| {
                draft.v4.dns_search = domains.clone();
                draft.v6.dns_search = Vec::new();
            });
            return true;
        }
        _ => {}
    }
    if let Some(path) = id.strip_prefix(SAVED_ROW) {
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
        return true;
    }
    if let Some(path) = id.strip_prefix(VPN_ROW) {
        if let Value::Bool(on) = *value {
            let active = match &*SNAPSHOT.read().unwrap() {
                Read::Ready(snapshot) => snapshot
                    .vpns
                    .iter()
                    .find(|v| v.path == path)
                    .and_then(|v| v.active.clone()),
                _ => None,
            };
            let path = path.to_string();
            act("network-vpn", move || match (on, active) {
                (true, _) => otto_network::blocking::activate(&path),
                (false, Some(active)) => otto_network::blocking::deactivate(&active),
                (false, None) => Ok(()),
            });
        }
        return true;
    }
    id.starts_with("network.")
}

/// A push button let go on: Apply or Revert, or a VPN's Import….
pub fn press(row: &str, button: &str) {
    if row == IMPORT_ID {
        import_vpn();
        return;
    }
    if row != APPLY_ID {
        return;
    }
    if button == otto_kit::t!("settings-network-revert") {
        let mut ui = ui();
        ui.draft = None;
        ui.error = None;
        drop(ui);
        settings_client::request_redraw();
        return;
    }
    let Some(draft) = ui().draft.clone() else {
        return;
    };
    if let Some(field) = draft.typed.keys().next() {
        ui().error = Some(field_error(field));
        settings_client::request_redraw();
        return;
    }
    let saved = match &*SNAPSHOT.read().unwrap() {
        Read::Ready(snapshot) => snapshot
            .profiles
            .iter()
            .find(|p| p.path == draft.profile)
            .cloned(),
        _ => None,
    };
    let Some(saved) = saved else {
        return;
    };
    for (family, config) in [(Family::V4, &draft.v4), (Family::V6, &draft.v6)] {
        if let Err(error) = config.validate(family) {
            ui().error = Some(error_text(&error, family));
            settings_client::request_redraw();
            return;
        }
    }
    ui().applying = true;
    settings_client::request_redraw();
    in_background("network-apply", move || {
        let mut result = Ok(());
        if draft.v4 != saved.ipv4 {
            result = otto_network::blocking::set_ip_config(&draft.profile, Family::V4, &draft.v4);
        }
        if result.is_ok() && draft.v6 != saved.ipv6 {
            result = otto_network::blocking::set_ip_config(&draft.profile, Family::V6, &draft.v6);
        }
        {
            let mut ui = ui();
            ui.applying = false;
            match result {
                Ok(()) => ui.draft = None,
                Err(why) => {
                    eprintln!("network: {why}");
                    ui.error = Some(otto_kit::t_owned!(
                        "settings-network-apply-failed",
                        reason = why.to_string()
                    ));
                }
            }
        }
        reload();
    });
}

/// What `nmcli connection import` calls the kind of VPN in `path`, by its
/// extension: a WireGuard `.conf` or an OpenVPN `.ovpn`.
fn vpn_kind(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "conf" => Some("wireguard"),
        "ovpn" => Some("openvpn"),
        _ => None,
    }
}

/// Ask for a VPN file through the desktop portal and import it.
///
/// NetworkManager imports through its VPN plug-ins, which only `nmcli` and
/// libnm reach; there is no D-Bus call for it. The import runs `nmcli` on a
/// thread, and the watcher shows the new connection when it lands.
fn import_vpn() {
    use otto_kit::portal_file_chooser::{Outcome, Request, Rule};
    let spawned = Request::open_file(otto_kit::t!("settings-network-vpn-import-title"))
        .filter(
            otto_kit::t!("settings-network-vpn-import-filter"),
            &[Rule::Glob("*.conf".into()), Rule::Glob("*.ovpn".into())],
        )
        .spawn(|outcome| {
            let path = match outcome {
                Outcome::Chosen(paths) => paths.into_iter().next(),
                Outcome::Dismissed => None,
                Outcome::Failed(why) => {
                    eprintln!("network: {why}");
                    None
                }
            };
            let Some(path) = path else {
                return;
            };
            let Some(kind) = vpn_kind(&path) else {
                eprintln!("network: {} is not a VPN file", path.display());
                return;
            };
            let status = std::process::Command::new("nmcli")
                .args(["connection", "import", "type", kind, "file"])
                .arg(&path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .status();
            match status {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("network: importing {} failed ({status})", path.display()),
                Err(why) => eprintln!("network: cannot run nmcli: {why}"),
            }
            reload();
        });
    if let Err(err) = spawned {
        eprintln!("network: could not open the file picker: {err}");
    }
}

/// A press on a row's Forget.
pub fn remove(id: &str) {
    let (path, vpn) = match (id.strip_prefix(SAVED_ROW), id.strip_prefix(VPN_ROW)) {
        (Some(path), _) => (path, false),
        (_, Some(path)) => (path, true),
        _ => return,
    };
    if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
        if vpn {
            snapshot.vpns.retain(|v| v.path != path);
        } else {
            snapshot.saved.retain(|s| s.path != path);
        }
    }
    let path = path.to_string();
    act("network-forget", move || {
        otto_network::blocking::forget(&path)
    });
}

fn field_error(field: &str) -> String {
    match field {
        V6_ADDRESS_ID => otto_kit::t!("settings-network-error-address-v6").to_string(),
        _ => otto_kit::t!("settings-network-error-address-v4").to_string(),
    }
}

fn error_text(error: &IpError, family: Family) -> String {
    let v = match family {
        Family::V4 => "IPv4",
        Family::V6 => "IPv6",
    };
    match error {
        IpError::NoAddress => otto_kit::t_owned!("settings-network-error-no-address", family = v),
        IpError::BadAddress(text) | IpError::BadPrefix(text) => otto_kit::t_owned!(
            "settings-network-error-bad-address",
            family = v,
            value = text.clone()
        ),
        IpError::BadGateway(text) => otto_kit::t_owned!(
            "settings-network-error-bad-router",
            family = v,
            value = text.clone()
        ),
        IpError::BadDns(text) => {
            otto_kit::t_owned!("settings-network-error-bad-dns", value = text.clone())
        }
        IpError::BadDomain(text) => {
            otto_kit::t_owned!("settings-network-error-bad-domain", value = text.clone())
        }
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

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

fn bound(mut row: Row, id: &'static str) -> Row {
    row.id = Some(id);
    row
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

/// A rate in kbit/s, as the bar writes it.
fn rate(kbps: u64) -> String {
    let mbps = kbps / 1000;
    if mbps >= 1000 {
        let gbps = mbps as f64 / 1000.0;
        if (gbps - gbps.round()).abs() < 0.05 {
            format!("{} Gb/s", gbps.round())
        } else {
            format!("{gbps:.1} Gb/s")
        }
    } else {
        format!("{mbps} Mb/s")
    }
}

/// The signal in words and as a percentage, with bars to read at a glance.
fn signal_text(strength: u8) -> String {
    let bars = otto_network::bars(strength);
    let word = match bars {
        0 => otto_kit::t!("settings-network-signal-weak"),
        1 => otto_kit::t!("settings-network-signal-fair"),
        2 => otto_kit::t!("settings-network-signal-good"),
        _ => otto_kit::t!("settings-network-signal-excellent"),
    };
    let meter: String = (0..4).map(|i| if i <= bars { '▮' } else { '▯' }).collect();
    format!("{meter}  {word} · {strength}%")
}

fn security_text(security: Security, wpa3_only: bool) -> &'static str {
    match (security, wpa3_only) {
        (Security::Open, _) => otto_kit::t!("settings-network-security-open"),
        (Security::Wep, _) => "WEP",
        (Security::Personal, true) => otto_kit::t!("settings-network-security-wpa3"),
        (Security::Personal, false) => otto_kit::t!("settings-network-security-wpa2"),
        (Security::Enterprise, _) => otto_kit::t!("settings-network-security-enterprise"),
    }
}

fn wifi_groups(snapshot: &Snapshot) -> Vec<Group> {
    let Some(wifi) = snapshot.wifi.as_ref() else {
        return Vec::new();
    };
    let mut groups = Vec::new();

    let mut switch = bound(
        Row::new(
            otto_kit::t!("settings-network-wifi"),
            Control::Toggle(snapshot.wifi_enabled),
        ),
        WIFI_ID,
    );
    if !snapshot.wifi_hardware_enabled {
        switch = switch
            .detail(otto_kit::t!("settings-network-wifi-blocked"))
            .inactive(true);
    }
    let connected = snapshot.connected_network();
    let current = match connected {
        Some(network) => network.name(),
        None => otto_kit::t!("settings-network-not-connected").to_string(),
    };
    groups.push(untitled(vec![
        switch,
        value(otto_kit::t!("settings-network-current"), current),
        Row::new(otto_kit::t!("settings-show-network"), Control::Toggle(true))
            .id("topbar.show_network"),
    ]));

    // The link, in the detail macOS keeps behind Option-click.
    if let (Some(network), Some(ap)) = (connected, wifi.active_ap.as_ref()) {
        let mut rows = vec![value(
            otto_kit::t!("settings-network-signal"),
            signal_text(ap.strength),
        )];
        let band = ap.band();
        if band != Band::Unknown {
            rows.push(value(
                otto_kit::t!("settings-network-band"),
                otto_kit::t_owned!(
                    "settings-network-band-value",
                    band = band.label(),
                    channel = ap.channel().to_string()
                ),
            ));
        }
        if let Some(kbps) = wifi.bitrate_kbps {
            rows.push(value(
                otto_kit::t!("settings-network-link-rate"),
                rate(u64::from(kbps)),
            ));
        }
        rows.push(value(
            otto_kit::t!("settings-network-security"),
            security_text(network.security, network.wpa3_only),
        ));
        if !ap.bssid.is_empty() {
            rows.push(value(
                otto_kit::t!("settings-network-bssid"),
                ap.bssid.clone(),
            ));
        }
        if !wifi.device.hw_address.is_empty() {
            rows.push(value(
                otto_kit::t!("settings-network-hardware-address"),
                wifi.device.hw_address.clone(),
            ));
        }
        if snapshot.limited() {
            rows.push(note(otto_kit::t!("settings-network-limited")));
        }
        groups.push(group(
            otto_kit::t!("settings-group-network-connection"),
            rows,
        ));
    }

    if let Some(profile) = edited_profile(snapshot, Tab::Wifi) {
        let live = wifi
            .device
            .link
            .as_ref()
            .filter(|_| wifi.device.active_profile.as_deref() == Some(profile.path.as_str()));
        let picker = bound(
            Row::new(
                otto_kit::t!("settings-network-settings-for"),
                Control::Select(profile.path.clone()),
            ),
            PROFILE_ID,
        );
        groups.extend(editor(profile, live, Some(picker)));
    }

    groups.push(known_group(snapshot));
    groups
}

fn known_group(snapshot: &Snapshot) -> Group {
    if snapshot.saved.is_empty() {
        return group(
            otto_kit::t!("settings-group-network-known"),
            vec![note(otto_kit::t!("settings-network-known-none"))],
        );
    }
    let rows = snapshot
        .saved
        .iter()
        .map(|saved| {
            let detail = if saved.in_use {
                otto_kit::t!("settings-network-known-in-use")
            } else {
                otto_kit::t!("settings-network-known-detail")
            };
            bound(
                Row::new(intern(saved.name()), Control::Toggle(saved.autoconnect))
                    .removable(true)
                    .remove_label(otto_kit::t!("settings-network-forget"))
                    .detail(detail),
                intern(format!("{SAVED_ROW}{}", saved.path)),
            )
        })
        .collect();
    group(otto_kit::t!("settings-group-network-known"), rows)
}

fn ethernet_groups(snapshot: &Snapshot) -> Vec<Group> {
    let mut groups = Vec::new();
    let rows: Vec<Row> = snapshot
        .devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Ethernet)
        .flat_map(|device| {
            let mut state = device_state(device.kind, device.state).to_string();
            if let (DeviceState::Connected, Some(speed)) = (device.state, device.speed_mbps) {
                state = format!("{state} · {}", rate(u64::from(speed) * 1000));
            }
            let mut rows = vec![Row::new(
                intern(device.interface.clone()),
                Control::Value(state),
            )];
            if !device.hw_address.is_empty() {
                rows.push(value(
                    otto_kit::t!("settings-network-hardware-address"),
                    device.hw_address.clone(),
                ));
            }
            rows
        })
        .collect();
    groups.push(group(otto_kit::t!("settings-group-network-status"), rows));

    match edited_profile(snapshot, Tab::Ethernet) {
        Some(profile) => {
            let live = snapshot
                .devices
                .iter()
                .find(|d| d.active_profile.as_deref() == Some(profile.path.as_str()))
                .and_then(|d| d.link.as_ref());
            groups.extend(editor(profile, live, None));
        }
        None => groups.push(untitled(vec![note(otto_kit::t!(
            "settings-network-ethernet-no-profile"
        ))])),
    }
    groups
}

fn vpn_groups(snapshot: &Snapshot) -> Vec<Group> {
    static IMPORT: OnceLock<[&'static str; 1]> = OnceLock::new();
    let import = bound(
        Row::new(
            otto_kit::t!("settings-network-vpn-add"),
            Control::Button(IMPORT.get_or_init(|| [otto_kit::t!("settings-network-vpn-import")])),
        )
        .detail(otto_kit::t!("settings-network-vpn-add-detail")),
        IMPORT_ID,
    );
    if snapshot.vpns.is_empty() {
        return vec![group(
            otto_kit::t!("settings-group-network-vpn"),
            vec![note(otto_kit::t!("settings-network-vpn-none")), import],
        )];
    }
    let rows = snapshot
        .vpns
        .iter()
        .map(|vpn| {
            let detail = match vpn.state {
                LinkState::Connected => otto_kit::t!("settings-network-state-connected"),
                LinkState::Connecting => otto_kit::t!("settings-network-state-connecting"),
                LinkState::Idle => otto_kit::t!("settings-network-state-disconnected"),
            };
            bound(
                Row::new(
                    intern(vpn.id.clone()),
                    Control::Toggle(vpn.state != LinkState::Idle),
                )
                .detail(detail)
                .removable(true)
                .remove_label(otto_kit::t!("settings-network-forget")),
                intern(format!("{VPN_ROW}{}", vpn.path)),
            )
        })
        .collect();
    vec![
        group(otto_kit::t!("settings-group-network-vpn"), rows),
        untitled(vec![import]),
    ]
}

/// The TCP/IP and DNS groups for `profile`. `live` is the connection's
/// addresses as NetworkManager reports them now, when it is up: what an
/// automatic family shows instead of fields.
fn editor(
    profile: &Profile,
    live: Option<&otto_network::ActiveLink>,
    picker: Option<Row>,
) -> Vec<Group> {
    let draft = shown_config(profile);
    let (error, applying, dirty) = {
        let ui = ui();
        (
            ui.error.clone(),
            ui.applying,
            ui.draft.as_ref().is_some_and(|d| d.profile == profile.path),
        )
    };

    let mut ip = Vec::new();
    if let Some(picker) = picker {
        ip.push(picker);
    }
    for (family, config) in [(Family::V4, &draft.v4), (Family::V6, &draft.v6)] {
        let (method_id, address_id, gateway_id, method_label_text) = match family {
            Family::V4 => (
                V4_METHOD_ID,
                V4_ADDRESS_ID,
                V4_GATEWAY_ID,
                otto_kit::t!("settings-network-configure-ipv4"),
            ),
            Family::V6 => (
                V6_METHOD_ID,
                V6_ADDRESS_ID,
                V6_GATEWAY_ID,
                otto_kit::t!("settings-network-configure-ipv6"),
            ),
        };
        ip.push(bound(
            Row::new(
                method_label_text,
                Control::Select(config.method.to_nm(family)),
            ),
            method_id,
        ));
        let (address_label, router_label) = match family {
            Family::V4 => (
                otto_kit::t!("settings-network-ipv4"),
                otto_kit::t!("settings-network-router"),
            ),
            Family::V6 => (
                otto_kit::t!("settings-network-ipv6"),
                otto_kit::t!("settings-network-router-v6"),
            ),
        };
        if config.method == IpMethod::Manual {
            let typed = draft.typed.get(address_id).cloned().unwrap_or_else(|| {
                config
                    .addresses
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            ip.push(bound(
                Row::new(address_label, Control::Text(typed)).detail(match family {
                    Family::V4 => otto_kit::t!("settings-network-address-detail-v4"),
                    Family::V6 => otto_kit::t!("settings-network-address-detail-v6"),
                }),
                address_id,
            ));
            ip.push(bound(
                Row::new(
                    router_label,
                    Control::Text(config.gateway.clone().unwrap_or_default()),
                ),
                gateway_id,
            ));
        } else if let Some(live) = live {
            let (addresses, gateway) = match family {
                Family::V4 => (&live.ipv4, live.gateway.clone()),
                Family::V6 => (&live.ipv6, None),
            };
            if !addresses.is_empty() {
                ip.push(value(address_label, addresses.join(", ")));
            }
            if let Some(gateway) = gateway {
                ip.push(value(router_label, gateway));
            }
        }
    }

    let servers: Vec<String> = draft
        .v4
        .dns
        .iter()
        .chain(draft.v6.dns.iter())
        .cloned()
        .collect();
    let mut dns = vec![
        bound(
            Row::new(
                otto_kit::t!("settings-network-dns-auto"),
                Control::Toggle(!draft.v4.ignore_auto_dns),
            )
            .detail(otto_kit::t!("settings-network-dns-auto-detail")),
            DNS_AUTO_ID,
        ),
        bound(
            Row::new(
                otto_kit::t!("settings-network-dns"),
                Control::Text(servers.join(", ")),
            )
            .detail(otto_kit::t!("settings-network-dns-detail")),
            DNS_SERVERS_ID,
        ),
        bound(
            Row::new(
                otto_kit::t!("settings-network-dns-search"),
                Control::Text(
                    draft
                        .v4
                        .dns_search
                        .iter()
                        .chain(draft.v6.dns_search.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            )
            .detail(otto_kit::t!("settings-network-dns-search-detail")),
            DNS_SEARCH_ID,
        ),
    ];
    if let Some(live) = live.filter(|l| !l.dns.is_empty()) {
        dns.push(value(
            otto_kit::t!("settings-network-dns-in-use"),
            live.dns.join(", "),
        ));
    }

    static BUTTONS: OnceLock<[&'static str; 2]> = OnceLock::new();
    let buttons = BUTTONS.get_or_init(|| {
        [
            otto_kit::t!("settings-network-revert"),
            otto_kit::t!("settings-network-apply"),
        ]
    });
    let detail = match (&error, applying) {
        (_, true) => Some(otto_kit::t!("settings-network-applying").to_string()),
        (Some(error), false) => Some(error.clone()),
        (None, false) if dirty => Some(otto_kit::t!("settings-network-unsaved").to_string()),
        _ => None,
    };
    let mut apply = bound(
        Row::new(
            otto_kit::t!("settings-network-changes"),
            Control::Button(buttons),
        )
        .inactive(!dirty || applying),
        APPLY_ID,
    );
    if let Some(detail) = detail {
        apply = apply.detail(detail);
    }
    dns.push(apply);

    vec![
        group(
            otto_kit::t_owned!("settings-group-network-tcpip", name = profile.id.clone()),
            ip,
        ),
        group(otto_kit::t!("settings-group-network-dns"), dns),
    ]
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
            let tabs = tabs(snapshot);
            let tab = current_tab(snapshot);
            let selected = tabs.iter().position(|t| *t == tab).unwrap_or(0);
            let mut bar = Row::new(
                "",
                Control::Tabs {
                    labels: tab_labels(&tabs),
                    selected,
                },
            );
            bar.id = Some(TABS_ID);
            let mut groups = vec![untitled(vec![bar])];
            groups.extend(match tab {
                Tab::Wifi => wifi_groups(snapshot),
                Tab::Ethernet => ethernet_groups(snapshot),
                Tab::Vpn => vpn_groups(snapshot),
            });
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

    fn profile(path: &str, kind: ProfileKind) -> Profile {
        Profile {
            path: path.into(),
            id: "Home".into(),
            uuid: "u".into(),
            kind,
            ssid: b"Home".to_vec(),
            hotspot: false,
            autoconnect: true,
            last_used: 1,
            ipv4: IpConfig::default(),
            ipv6: IpConfig::default(),
        }
    }

    fn snapshot() -> Snapshot {
        let mut device = Device::new("/d/1", DeviceKind::Wifi, "wlan0", DeviceState::Connected);
        device.active_profile = Some("/c/7".into());
        Snapshot {
            running: true,
            wifi_enabled: true,
            wifi_hardware_enabled: true,
            devices: vec![device.clone()],
            wifi: Some(WifiDevice::new(device, Vec::new())),
            saved: vec![SavedNetwork {
                path: "/c/7".into(),
                id: "Home".into(),
                ssid: b"Home".to_vec(),
                autoconnect: false,
                last_used: 0,
                in_use: true,
            }],
            profiles: vec![profile("/c/7", ProfileKind::Wifi)],
            ..Snapshot::default()
        }
    }

    #[test]
    fn a_saved_network_is_a_forgettable_switch() {
        let group = known_group(&snapshot());
        let row = &group.rows[0];
        assert_eq!(row.label, "Home");
        assert_eq!(row.id, Some("network.saved./c/7"));
        assert!(row.removable);
        assert!(matches!(row.control, Control::Toggle(false)));
    }

    #[test]
    fn the_tabs_follow_the_devices() {
        let mut snapshot = snapshot();
        assert_eq!(tabs(&snapshot), [Tab::Wifi, Tab::Vpn]);
        snapshot.devices.push(Device::new(
            "/d/2",
            DeviceKind::Ethernet,
            "eth0",
            DeviceState::Unavailable,
        ));
        assert_eq!(tabs(&snapshot), [Tab::Wifi, Tab::Ethernet, Tab::Vpn]);
        snapshot.wifi = None;
        assert_eq!(tabs(&snapshot), [Tab::Ethernet, Tab::Vpn]);
    }

    #[test]
    fn the_editor_edits_the_connection_in_use() {
        let snapshot = snapshot();
        assert_eq!(
            edited_profile(&snapshot, Tab::Wifi).map(|p| p.path.as_str()),
            Some("/c/7")
        );
        let groups = editor(&snapshot.profiles[0], None, None);
        let ids: Vec<_> = groups
            .iter()
            .flat_map(|g| g.rows.iter().filter_map(|r| r.id))
            .collect();
        // Automatic: a method pop-up per family, no address fields.
        assert!(ids.contains(&V4_METHOD_ID) && ids.contains(&V6_METHOD_ID));
        assert!(!ids.contains(&V4_ADDRESS_ID));
        assert!(ids.contains(&DNS_SERVERS_ID) && ids.contains(&APPLY_ID));
    }

    #[test]
    fn manual_shows_the_address_fields() {
        let mut profile = profile("/c/9", ProfileKind::Ethernet);
        profile.ipv4.method = IpMethod::Manual;
        let groups = editor(&profile, None, None);
        let ids: Vec<_> = groups
            .iter()
            .flat_map(|g| g.rows.iter().filter_map(|r| r.id))
            .collect();
        assert!(ids.contains(&V4_ADDRESS_ID) && ids.contains(&V4_GATEWAY_ID));
        assert!(!ids.contains(&V6_ADDRESS_ID));
    }

    #[test]
    fn method_choices_show_their_names() {
        let choices = menu_choices(V4_METHOD_ID).unwrap();
        assert_eq!(choices[0].value, "auto");
        assert_eq!(
            display(V4_METHOD_ID, "manual").as_deref(),
            Some(otto_kit::t!("settings-network-method-manual"))
        );
    }

    #[test]
    fn the_signal_reads_in_words_and_bars() {
        assert!(signal_text(90).starts_with("▮▮▮▮"));
        assert!(signal_text(10).starts_with("▮▯▯▯"));
        assert!(signal_text(60).ends_with("60%"));
    }

    #[test]
    fn vpn_files_by_extension() {
        use std::path::Path;
        assert_eq!(vpn_kind(Path::new("/x/wg0.conf")), Some("wireguard"));
        assert_eq!(vpn_kind(Path::new("/x/Work.OVPN")), Some("openvpn"));
        assert_eq!(vpn_kind(Path::new("/x/notes.txt")), None);
    }

    #[test]
    fn the_pane_owns_its_identifiers_only() {
        assert!(!apply("dock.size", &Value::Bool(true)));
        assert!(!choose("dock.position", "left"));
    }
}
