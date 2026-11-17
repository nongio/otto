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
use std::time::{Duration, Instant};

use otto_network::{
    dns_address, parse_address, split_list, Band, DeviceKind, DeviceState, Family, IpConfig,
    IpError, IpMethod, LinkState, Profile, ProfileKind, Security, Snapshot,
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
    /// Why the last VPN import did not take.
    import_error: Option<String>,
    /// Why the last switch, Forget or VPN connect did not take.
    action_error: Option<ActionError>,
}

/// An action NetworkManager refused, shown on the row it came from.
struct ActionError {
    /// The connection whose row it was; `None` for the Wi-Fi switch.
    path: Option<String>,
    text: String,
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

impl Draft {
    fn config(&self, family: Family) -> &IpConfig {
        match family {
            Family::V4 => &self.v4,
            Family::V6 => &self.v6,
        }
    }

    fn config_mut(&mut self, family: Family) -> &mut IpConfig {
        match family {
            Family::V4 => &mut self.v4,
            Family::V6 => &mut self.v6,
        }
    }

    /// Set `family`'s method. Text typed into its address field goes when
    /// the method hides the field, so Apply never stops on what is not shown.
    fn set_method(&mut self, family: Family, method: IpMethod) {
        if method != IpMethod::Manual {
            self.typed.remove(address_id(family));
        }
        self.config_mut(family).method = method;
    }

    /// The first shown address field whose text does not parse yet.
    fn unfinished_field(&self) -> Option<&'static str> {
        [Family::V4, Family::V6]
            .into_iter()
            .filter(|family| self.config(*family).method == IpMethod::Manual)
            .map(address_id)
            .find(|id| self.typed.contains_key(id))
    }

    /// Whether the Automatic DNS switch shows on: either family taking its
    /// servers from the network, since the switch writes both.
    fn dns_auto(&self) -> bool {
        !self.v4.ignore_auto_dns || !self.v6.ignore_auto_dns
    }
}

/// The address field of `family`.
fn address_id(family: Family) -> &'static str {
    match family {
        Family::V4 => V4_ADDRESS_ID,
        Family::V6 => V6_ADDRESS_ID,
    }
}

static UI: Mutex<Ui> = Mutex::new(Ui {
    tab: None,
    wifi_profile: None,
    draft: None,
    error: None,
    applying: false,
    import_error: None,
    action_error: None,
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

/// The shortest wait before following NetworkManager again.
const WATCH_BACKOFF_MIN: Duration = Duration::from_secs(1);
/// The longest wait: a bus that keeps refusing costs a retry every half
/// minute, not a busy loop.
const WATCH_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// The wait before following again, given the last wait and how long the
/// last `follow` ran: a run as long as the longest wait starts over from the
/// shortest, a shorter one doubles the last wait.
fn watch_backoff(last: Duration, ran: Duration) -> Duration {
    if ran >= WATCH_BACKOFF_MAX {
        WATCH_BACKOFF_MIN
    } else {
        (last * 2).clamp(WATCH_BACKOFF_MIN, WATCH_BACKOFF_MAX)
    }
}

/// Follow NetworkManager for as long as the app runs, once the pane has
/// been shown. Changes cost little (see `otto_network::follow`), and off
/// screen they only update what the pane shows next. `follow` returns when
/// NetworkManager's bus stream ends, as when the service restarts; the
/// watcher then follows again, after a wait that grows while it keeps
/// returning straight away.
fn watch() {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    in_background("network-watch", || {
        let mut wait = Duration::ZERO;
        loop {
            let started = Instant::now();
            // The snapshot comes with the change: reading again from in here
            // would start a runtime inside the one `follow` runs on.
            let result = otto_network::blocking::follow(|snapshot| {
                *SNAPSHOT.write().unwrap() = Read::Ready(Box::new(snapshot.clone()));
                if SHOWN.load(Ordering::Relaxed) {
                    settings_client::request_redraw();
                }
            });
            match result {
                Ok(()) => eprintln!("network: NetworkManager's changes stopped coming"),
                Err(why) => eprintln!("network: stopped following NetworkManager: {why}"),
            }
            wait = watch_backoff(wait, started.elapsed());
            std::thread::sleep(wait);
        }
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
/// A refusal shows on the row it came from: the connection at `path`, or
/// the Wi-Fi switch for `None`.
fn act(name: &str, path: Option<String>, work: impl FnOnce() -> zbus::Result<()> + Send + 'static) {
    ui().action_error = None;
    in_background(name, move || {
        if let Err(why) = work() {
            eprintln!("network: {why}");
            ui().action_error = Some(ActionError {
                path,
                text: otto_kit::t_owned!("settings-network-apply-failed", reason = why.to_string()),
            });
        }
        reload();
    });
}

/// Why the last action on the row for `path` failed, if it did; `None` asks
/// for the Wi-Fi switch.
fn action_error(path: Option<&str>) -> Option<String> {
    ui().action_error
        .as_ref()
        .filter(|e| e.path.as_deref() == path)
        .map(|e| e.text.clone())
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
        IpMethod::Disabled | IpMethod::Ignore => {
            otto_kit::t!("settings-network-method-off").to_string()
        }
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
            edit(|draft| draft.set_method(family, method));
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
                if on {
                    // Turning on lifts a soft block too.
                    snapshot.wifi_block.soft = false;
                }
            }
            act("network-wifi", None, move || {
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
                // By what the entry's address is: a port (`1.1.1.1:853`)
                // has a colon too. What is neither stays on IPv6, where
                // validation names it.
                let (v4, v6) = servers
                    .iter()
                    .cloned()
                    .partition(|s| dns_address(s, Family::V4).is_some());
                draft.v4.dns = v4;
                draft.v6.dns = v6;
            });
            return true;
        }
        (DNS_SEARCH_ID, Value::Text(text)) => {
            let domains = split_list(text);
            // Both families: a family that is off ignores its own, and an
            // IPv6-only connection needs them on IPv6.
            edit(|draft| {
                draft.v4.dns_search = domains.clone();
                draft.v6.dns_search = domains.clone();
            });
            return true;
        }
        _ => {}
    }
    if id.starts_with(SAVED_ROW) {
        let Some(path) = slot_path(id) else {
            return true;
        };
        let path = path.as_str();
        if let Value::Bool(on) = *value {
            if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
                if let Some(saved) = snapshot.saved.iter_mut().find(|s| s.path == path) {
                    saved.autoconnect = on;
                }
            }
            let path = path.to_string();
            act("network-autoconnect", Some(path.clone()), move || {
                otto_network::blocking::set_autoconnect(&path, on)
            });
        }
        return true;
    }
    if id.starts_with(VPN_ROW) {
        let Some(path) = slot_path(id) else {
            return true;
        };
        let path = path.as_str();
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
            act("network-vpn", Some(path.clone()), move || {
                match (on, active) {
                    (true, _) => otto_network::blocking::activate(&path),
                    (false, Some(active)) => otto_network::blocking::deactivate(&active),
                    (false, None) => Ok(()),
                }
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
    if let Some(field) = draft.unfinished_field() {
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
        // Both families in one update: neither is saved without the other.
        let result = if draft.v4 != saved.ipv4 || draft.v6 != saved.ipv6 {
            otto_network::blocking::set_ip_config(&draft.profile, &draft.v4, &draft.v6)
        } else {
            Ok(())
        };
        settle_apply(&mut ui(), &draft, result);
        reload();
    });
}

/// Take in how the Apply of `sent` went. A draft edited while Apply was on
/// its way is kept: only the draft that was sent is done with.
fn settle_apply(ui: &mut Ui, sent: &Draft, result: zbus::Result<()>) {
    ui.applying = false;
    match result {
        Ok(()) => {
            if ui.draft.as_ref() == Some(sent) {
                ui.draft = None;
            }
        }
        Err(why) => {
            eprintln!("network: {why}");
            ui.error = Some(otto_kit::t_owned!(
                "settings-network-apply-failed",
                reason = why.to_string()
            ));
        }
    }
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
            let error = match vpn_kind(&path) {
                None => Some(otto_kit::t!("settings-network-vpn-import-unknown").to_string()),
                Some(kind) => {
                    let output = std::process::Command::new("nmcli")
                        .args(["connection", "import", "type", kind, "file"])
                        .arg(&path)
                        .stdin(std::process::Stdio::null())
                        .output();
                    match output {
                        Ok(output) if output.status.success() => None,
                        // nmcli says why on its error stream, in the
                        // person's language: its first line is the reason.
                        Ok(output) => Some(otto_kit::t_owned!(
                            "settings-network-vpn-import-failed",
                            reason = String::from_utf8_lossy(&output.stderr)
                                .lines()
                                .find(|line| !line.trim().is_empty())
                                .unwrap_or_default()
                                .trim()
                                .to_string()
                        )),
                        Err(_) => {
                            Some(otto_kit::t!("settings-network-vpn-import-no-nmcli").to_string())
                        }
                    }
                }
            };
            if let Some(error) = &error {
                eprintln!("network: importing {}: {error}", path.display());
            }
            ui().import_error = error;
            reload();
        });
    if let Err(err) = spawned {
        eprintln!("network: could not open the file picker: {err}");
    }
}

/// A press on a row's Forget.
pub fn remove(id: &str) {
    let vpn = id.starts_with(VPN_ROW);
    if !vpn && !id.starts_with(SAVED_ROW) {
        return;
    }
    let Some(path) = slot_path(id) else {
        return;
    };
    let path = path.as_str();
    if let Read::Ready(snapshot) = &mut *SNAPSHOT.write().unwrap() {
        if vpn {
            snapshot.vpns.retain(|v| v.path != path);
        } else {
            snapshot.saved.retain(|s| s.path != path);
        }
    }
    let path = path.to_string();
    act("network-forget", Some(path.clone()), move || {
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

/// How many saved networks, and how many VPNs, get rows that can be
/// switched and forgotten. Rows are named by place (`network.saved.3`), not
/// by object path: NetworkManager's paths keep counting up as connections
/// are made again, and a name per path would be a new string each time.
const SLOTS: usize = 128;

/// The row identifiers for `prefix`, made once.
fn row_ids(prefix: &'static str) -> &'static [&'static str] {
    static MADE: OnceLock<Mutex<HashMap<&'static str, &'static [&'static str]>>> = OnceLock::new();
    let mut made = MADE.get_or_init(Default::default).lock().unwrap();
    made.entry(prefix).or_insert_with(|| {
        Vec::leak(
            (0..SLOTS)
                .map(|i| &*format!("{prefix}{i}").leak())
                .collect(),
        )
    })
}

/// Which connection each row identifier names, as the pane was last built.
static SLOT_PATHS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());

/// The identifier for the `index`th row under `prefix`, naming `path`.
/// `None` past the last slot: that row shows, but cannot be changed.
fn slot(prefix: &'static str, index: usize, path: &str) -> Option<&'static str> {
    let id = *row_ids(prefix).get(index)?;
    let mut paths = SLOT_PATHS.lock().unwrap();
    paths.retain(|(slot, _)| *slot != id);
    paths.push((id, path.to_string()));
    Some(id)
}

/// The connection a row identifier names.
fn slot_path(id: &str) -> Option<String> {
    SLOT_PATHS
        .lock()
        .unwrap()
        .iter()
        .find(|(slot, _)| *slot == id)
        .map(|(_, path)| path.clone())
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
            Control::Toggle(snapshot.wifi_on()),
        ),
        WIFI_ID,
    );
    if snapshot.wifi_hard_blocked() {
        switch = switch
            .detail(otto_kit::t!("settings-network-wifi-hard-blocked"))
            .inactive(true);
    } else if snapshot.wifi_soft_blocked() {
        switch = switch.detail(otto_kit::t!("settings-network-wifi-soft-blocked"));
    }
    if let Some(error) = action_error(None) {
        switch = switch.detail(error);
    }
    let connected = snapshot.connected_network();
    let current = match connected {
        Some(network) => network.name(),
        None => otto_kit::t!("settings-network-not-connected").to_string(),
    };
    groups.push(untitled(vec![
        switch,
        value(otto_kit::t!("settings-network-current"), current),
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
                otto_network::format_rate(u64::from(kbps)),
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
        .enumerate()
        .map(|(index, saved)| {
            let detail = match action_error(Some(&saved.path)) {
                Some(error) => std::borrow::Cow::Owned(error),
                None if saved.in_use => otto_kit::t!("settings-network-known-in-use").into(),
                None => otto_kit::t!("settings-network-known-detail").into(),
            };
            let mut row = Row::new(intern(saved.name()), Control::Toggle(saved.autoconnect))
                .remove_label(otto_kit::t!("settings-network-forget"))
                .detail(detail);
            row.id = slot(SAVED_ROW, index, &saved.path);
            let removable = row.id.is_some();
            row.removable(removable)
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
                state = format!(
                    "{state} · {}",
                    otto_network::format_rate(u64::from(speed) * 1000)
                );
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
        .detail(match ui().import_error.clone() {
            Some(error) => std::borrow::Cow::Owned(error),
            None => otto_kit::t!("settings-network-vpn-add-detail").into(),
        }),
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
        .enumerate()
        .map(|(index, vpn)| {
            let detail = match (action_error(Some(&vpn.path)), vpn.state) {
                (Some(error), _) => std::borrow::Cow::Owned(error),
                (None, LinkState::Connected) => {
                    otto_kit::t!("settings-network-state-connected").into()
                }
                (None, LinkState::Connecting) => {
                    otto_kit::t!("settings-network-state-connecting").into()
                }
                (None, LinkState::Idle) => {
                    otto_kit::t!("settings-network-state-disconnected").into()
                }
            };
            let mut row = Row::new(
                intern(vpn.id.clone()),
                Control::Toggle(vpn.state != LinkState::Idle),
            )
            .detail(detail)
            .remove_label(otto_kit::t!("settings-network-forget"));
            row.id = slot(VPN_ROW, index, &vpn.path);
            let removable = row.id.is_some();
            row.removable(removable)
        })
        .collect();
    vec![
        group(otto_kit::t!("settings-group-network-vpn"), rows),
        untitled(vec![import]),
    ]
}

/// The search domains of both families, each once, IPv4's first: an edit
/// writes the same list to both.
fn search_domains(draft: &Draft) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for domain in draft.v4.dns_search.iter().chain(&draft.v6.dns_search) {
        if !domains.contains(domain) {
            domains.push(domain.clone());
        }
    }
    domains
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
                Family::V6 => (&live.ipv6, live.gateway6.clone()),
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
                Control::Toggle(draft.dns_auto()),
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
                Control::Text(search_domains(&draft).join(", ")),
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
        // Named by place, and resolved back to the connection it names.
        assert_eq!(row.id, Some("network.saved.0"));
        assert_eq!(slot_path("network.saved.0").as_deref(), Some("/c/7"));
        assert!(row.removable);
        assert!(matches!(row.control, Control::Toggle(false)));
    }

    #[test]
    fn the_wifi_switch_follows_the_radio() {
        let mut snapshot = snapshot();
        snapshot.wifi_block.soft = true;
        let groups = wifi_groups(&snapshot);
        let switch = &groups[0].rows[0];
        assert!(matches!(switch.control, Control::Toggle(false)));
        assert!(!switch.inactive);
        assert_eq!(
            switch.detail.as_deref(),
            Some(otto_kit::t!("settings-network-wifi-soft-blocked"))
        );

        snapshot.wifi_hardware_enabled = false;
        let groups = wifi_groups(&snapshot);
        let switch = &groups[0].rows[0];
        assert!(switch.inactive, "only the switch on the machine can");
        assert_eq!(
            switch.detail.as_deref(),
            Some(otto_kit::t!("settings-network-wifi-hard-blocked"))
        );
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

    fn draft() -> Draft {
        Draft {
            profile: "/c/7".into(),
            v4: IpConfig::default(),
            v6: IpConfig::default(),
            typed: HashMap::new(),
        }
    }

    #[test]
    fn text_in_a_hidden_field_does_not_stop_apply() {
        let mut draft = draft();
        draft.set_method(Family::V4, IpMethod::Manual);
        draft.typed.insert(V4_ADDRESS_ID, "192.168.1".into());
        assert_eq!(draft.unfinished_field(), Some(V4_ADDRESS_ID));

        draft.set_method(Family::V4, IpMethod::Automatic);
        assert_eq!(draft.unfinished_field(), None);
        assert!(draft.typed.is_empty(), "the hidden field's text is dropped");

        // Text left under a method that hides it is not asked about either.
        draft.typed.insert(V6_ADDRESS_ID, "2001:db8::".into());
        assert_eq!(draft.unfinished_field(), None);
    }

    #[test]
    fn an_edit_made_while_applying_is_kept() {
        let sent = draft();
        let mut edited = sent.clone();
        edited.v4.dns = vec!["1.1.1.1".into()];
        let mut ui = Ui {
            tab: None,
            wifi_profile: None,
            draft: Some(edited.clone()),
            error: None,
            applying: true,
            import_error: None,
            action_error: None,
        };

        settle_apply(&mut ui, &sent, Ok(()));
        assert!(!ui.applying);
        assert_eq!(ui.draft, Some(edited));

        ui.draft = Some(sent.clone());
        settle_apply(&mut ui, &sent, Ok(()));
        assert_eq!(ui.draft, None, "the draft that was sent is done with");

        ui.draft = Some(sent.clone());
        settle_apply(&mut ui, &sent, Err(zbus::Error::Failure("no".into())));
        assert_eq!(ui.draft, Some(sent), "a refused draft stays to fix");
        assert!(ui.error.is_some());
    }

    #[test]
    fn automatic_dns_is_on_while_either_family_is() {
        let mut draft = draft();
        draft.v4.ignore_auto_dns = true;
        assert!(draft.dns_auto(), "IPv6 still takes the network's servers");
        draft.v6.ignore_auto_dns = true;
        assert!(!draft.dns_auto());

        let mut profile = profile("/c/dns", ProfileKind::Ethernet);
        profile.ipv4.method = IpMethod::Disabled;
        profile.ipv4.ignore_auto_dns = true;
        let groups = editor(&profile, None, None);
        let switch = groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.id == Some(DNS_AUTO_ID))
            .expect("the Automatic DNS row");
        assert!(matches!(switch.control, Control::Toggle(true)));
    }

    #[test]
    fn the_watcher_backs_off_while_follow_returns_at_once() {
        let quick = Duration::from_millis(5);
        let first = watch_backoff(Duration::ZERO, quick);
        assert_eq!(first, WATCH_BACKOFF_MIN);
        assert_eq!(watch_backoff(first, quick), first * 2);
        assert_eq!(watch_backoff(WATCH_BACKOFF_MAX, quick), WATCH_BACKOFF_MAX);
        // A long run was a working watch: the next wait is short again.
        assert_eq!(
            watch_backoff(WATCH_BACKOFF_MAX, WATCH_BACKOFF_MAX),
            WATCH_BACKOFF_MIN
        );
    }

    #[test]
    fn the_pane_owns_its_identifiers_only() {
        assert!(!apply("dock.size", &Value::Bool(true)));
        assert!(!choose("dock.position", "left"));
    }
}
