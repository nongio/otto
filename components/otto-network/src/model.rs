//! What NetworkManager reported, as plain data, and the decisions every
//! caller makes the same way: which networks a list shows and in what order,
//! how a signal reads in bars, what the indicator draws.
//!
//! Nothing here touches the bus, so all of it is tested without one.

/// Everything one read of NetworkManager found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// Whether NetworkManager is on the system bus at all. Everything else is
    /// empty when it is not.
    pub running: bool,
    /// Whether the Wi-Fi radios are on.
    pub wifi_enabled: bool,
    /// Whether a hardware switch lets them be on, as NetworkManager says.
    pub wifi_hardware_enabled: bool,
    /// How the kernel's kill switches block the Wi-Fi radios.
    pub wifi_block: crate::rfkill::Block,
    pub connectivity: Connectivity,
    /// The managed Wi-Fi device shown, with the networks it sees: the first
    /// with a connection, or the first when none has one.
    pub wifi: Option<WifiDevice>,
    /// Every managed Wi-Fi and Ethernet device, for a status list.
    pub devices: Vec<Device>,
    /// The connection holding the default route.
    pub primary: Option<ActiveLink>,
    /// Saved Wi-Fi networks, the one in use first, then the most recently
    /// used.
    pub saved: Vec<SavedNetwork>,
    /// Saved VPN and WireGuard connections.
    pub vpns: Vec<Vpn>,
    /// Every saved connection, with its IP configuration.
    pub profiles: Vec<crate::profile::Profile>,
}

/// NetworkManager's `NMConnectivityState`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Connectivity {
    #[default]
    Unknown,
    None,
    /// Behind a captive portal: a login page stands between the machine and
    /// the internet.
    Portal,
    /// Connected to a network that does not reach the internet.
    Limited,
    Full,
}

impl Connectivity {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::None,
            2 => Self::Portal,
            3 => Self::Limited,
            4 => Self::Full,
            _ => Self::Unknown,
        }
    }
}

/// What kind of device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Ethernet,
    Wifi,
}

/// A device's state, narrowed from `NMDeviceState` to what a person is told.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeviceState {
    /// Unmanaged, or unknown to NetworkManager.
    #[default]
    Unknown,
    /// No carrier (a cable unplugged) or the radio off.
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Failed,
}

impl DeviceState {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            20 => Self::Unavailable,
            30 => Self::Disconnected,
            40..=90 => Self::Connecting,
            100 => Self::Connected,
            110 => Self::Disconnecting,
            120 => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

/// One network device.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Its object path, which the actions take.
    pub path: String,
    pub kind: DeviceKind,
    /// The kernel name ("wlan0").
    pub interface: String,
    pub state: DeviceState,
    pub hw_address: String,
    /// The saved connection active on it, if any.
    pub active_profile: Option<String>,
    /// A wired device's negotiated speed in Mb/s, when it is known.
    pub speed_mbps: Option<u32>,
    /// Its active connection's object path.
    pub active: Option<String>,
    /// Its active connection's addresses.
    pub link: Option<ActiveLink>,
}

impl Device {
    /// A device with nothing known about it beyond what it is.
    pub fn new(path: &str, kind: DeviceKind, interface: &str, state: DeviceState) -> Self {
        Self {
            path: path.to_string(),
            kind,
            interface: interface.to_string(),
            state,
            hw_address: String::new(),
            active_profile: None,
            speed_mbps: None,
            active: None,
            link: None,
        }
    }
}

/// The Wi-Fi device and the networks it sees.
#[derive(Debug, Clone, PartialEq)]
pub struct WifiDevice {
    pub device: Device,
    /// One per network name, in the order a list shows them: the one in use,
    /// then saved ones, then the rest, each by signal.
    pub networks: Vec<WifiNetwork>,
    /// Milliseconds of `CLOCK_BOOTTIME` at the last scan; `None` for never.
    pub(crate) last_scan_ms: Option<i64>,
    /// The access point the device is associated with.
    pub active_ap: Option<AccessPoint>,
    /// Every access point the device sees, before they are folded into
    /// `networks`: what a signal-strength change is applied to.
    pub access_points: Vec<AccessPoint>,
    /// The network name in use or coming up, as `networks` was grouped with.
    pub in_use: Option<(Vec<u8>, LinkState)>,
    /// The link rate in kbit/s, while associated.
    pub bitrate_kbps: Option<u32>,
}

impl WifiDevice {
    /// A Wi-Fi device seeing `networks`, not associated with any.
    pub fn new(device: Device, networks: Vec<WifiNetwork>) -> Self {
        Self {
            device,
            networks,
            last_scan_ms: None,
            active_ap: None,
            access_points: Vec::new(),
            in_use: None,
            bitrate_kbps: None,
        }
    }
}

/// How a network is secured, as far as joining it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Security {
    /// No password (Enhanced Open included: it encrypts with no password).
    Open,
    /// WEP.
    Wep,
    /// WPA, WPA2 or WPA3 with a shared password.
    Personal,
    /// 802.1X: a user name and credentials a menu cannot ask for.
    Enterprise,
}

impl Security {
    /// Whether joining asks for a password.
    pub fn is_secured(self) -> bool {
        self != Self::Open
    }

    /// Whether an access point offers only WPA3 Personal (SAE), so a new
    /// connection must use it rather than WPA2's PSK.
    pub fn wpa3_only(wpa_flags: u32, rsn_flags: u32) -> bool {
        const KEY_MGMT_PSK: u32 = 0x100;
        const KEY_MGMT_SAE: u32 = 0x400;
        let both = wpa_flags | rsn_flags;
        both & KEY_MGMT_SAE != 0 && both & KEY_MGMT_PSK == 0
    }

    /// Read an access point's flags: `NM80211ApFlags` and the WPA and RSN
    /// `NM80211ApSecurityFlags`.
    pub fn from_flags(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Self {
        const PRIVACY: u32 = 0x1;
        const KEY_MGMT_PSK: u32 = 0x100;
        const KEY_MGMT_802_1X: u32 = 0x200;
        const KEY_MGMT_SAE: u32 = 0x400;
        const KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;

        let both = wpa_flags | rsn_flags;
        if both & (KEY_MGMT_802_1X | KEY_MGMT_EAP_SUITE_B_192) != 0 {
            Self::Enterprise
        } else if both & (KEY_MGMT_PSK | KEY_MGMT_SAE) != 0 {
            Self::Personal
        } else if both == 0 && flags & PRIVACY != 0 {
            Self::Wep
        } else {
            // No flags, or OWE alone.
            Self::Open
        }
    }
}

/// One access point, as NetworkManager lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessPoint {
    pub path: String,
    pub ssid: Vec<u8>,
    pub strength: u8,
    pub security: Security,
    /// Only WPA3 (SAE) is offered: a new connection must use it.
    pub wpa3_only: bool,
    pub frequency_mhz: u32,
    pub bssid: String,
}

impl AccessPoint {
    /// The band, for display.
    pub fn band(&self) -> Band {
        Band::from_mhz(self.frequency_mhz)
    }

    /// The channel number, `0` when the frequency is not on a channel.
    pub fn channel(&self) -> u32 {
        channel(self.frequency_mhz)
    }
}

/// A Wi-Fi band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Ghz2,
    Ghz5,
    Ghz6,
    Unknown,
}

impl Band {
    pub fn from_mhz(mhz: u32) -> Self {
        match mhz {
            2400..=2500 => Self::Ghz2,
            // 5.925 GHz up is 6 GHz (Wi-Fi 6E); below it is 5 GHz.
            5150..=5924 => Self::Ghz5,
            5925..=7125 => Self::Ghz6,
            _ => Self::Unknown,
        }
    }

    /// How it is written: "2.4 GHz", "5 GHz", "6 GHz".
    pub fn label(self) -> &'static str {
        match self {
            Self::Ghz2 => "2.4 GHz",
            Self::Ghz5 => "5 GHz",
            Self::Ghz6 => "6 GHz",
            Self::Unknown => "",
        }
    }
}

/// The channel number of a frequency in MHz, `0` for none.
pub fn channel(mhz: u32) -> u32 {
    match mhz {
        2484 => 14,
        2412..=2472 => (mhz - 2407) / 5,
        5955..=7115 => (mhz - 5950) / 5,
        5000..=5900 => (mhz - 5000) / 5,
        _ => 0,
    }
}

/// Whether a network is the one in use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LinkState {
    #[default]
    Idle,
    Connecting,
    Connected,
}

/// One network, however many access points broadcast it.
#[derive(Debug, Clone, PartialEq)]
pub struct WifiNetwork {
    /// The name, as bytes: what NetworkManager matches on.
    pub ssid: Vec<u8>,
    /// The strongest access point's signal, `0..=100`.
    pub strength: u8,
    pub security: Security,
    /// The strongest access point that takes the key [`Self::key_kind`]
    /// names, which a connection is made for.
    pub access_point: String,
    /// The saved connection for this network, if there is one.
    pub saved: Option<String>,
    pub state: LinkState,
    /// Only WPA3 is offered: every access point secured as `security` says
    /// offers WPA3 alone.
    pub wpa3_only: bool,
}

impl WifiNetwork {
    /// The name to show. A name that is not UTF-8 is shown with replacement
    /// characters; it is still joined by its bytes.
    pub fn name(&self) -> String {
        ssid_name(&self.ssid)
    }

    /// Signal bars, `0..=3`.
    pub fn bars(&self) -> u8 {
        bars(self.strength)
    }

    /// The kind of key joining it takes, for a network with a password a
    /// person can type. `None` for an open or an enterprise network.
    pub fn key_kind(&self) -> Option<crate::profile::KeyKind> {
        use crate::profile::KeyKind;
        match self.security {
            Security::Personal if self.wpa3_only => Some(KeyKind::Sae),
            Security::Personal => Some(KeyKind::WpaPsk),
            Security::Wep => Some(KeyKind::Wep),
            Security::Open | Security::Enterprise => None,
        }
    }

    /// Whether joining it needs a password asked for first: secured and not
    /// saved. A saved network has its password already.
    pub fn needs_password(&self) -> bool {
        self.saved.is_none() && self.key_kind().is_some()
    }
}

/// A network name for display.
pub fn ssid_name(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).into_owned()
}

/// Signal bars, `0..=3`, for a strength of `0..=100`.
pub fn bars(strength: u8) -> u8 {
    match strength {
        0..=24 => 0,
        25..=49 => 1,
        50..=74 => 2,
        _ => 3,
    }
}

/// A saved Wi-Fi network.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedNetwork {
    /// The saved connection's object path.
    pub path: String,
    /// The connection's name, usually the network name.
    pub id: String,
    pub ssid: Vec<u8>,
    /// Whether NetworkManager joins it by itself.
    pub autoconnect: bool,
    /// Seconds since the epoch it was last used, `0` for never.
    pub last_used: u64,
    /// Whether it is the connection in use on the Wi-Fi device.
    pub in_use: bool,
}

impl SavedNetwork {
    pub fn name(&self) -> String {
        if self.ssid.is_empty() {
            self.id.clone()
        } else {
            ssid_name(&self.ssid)
        }
    }
}

/// A saved VPN or WireGuard connection.
#[derive(Debug, Clone, PartialEq)]
pub struct Vpn {
    /// The saved connection's object path.
    pub path: String,
    pub id: String,
    /// Its active connection, while it is up or coming up.
    pub active: Option<String>,
    pub state: LinkState,
}

/// What kind of link the primary connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Wifi,
    Ethernet,
    Vpn,
    Other,
}

impl LinkKind {
    /// From an active connection's `Type`.
    pub fn from_type(kind: &str) -> Self {
        match kind {
            "802-11-wireless" => Self::Wifi,
            "802-3-ethernet" => Self::Ethernet,
            "vpn" | "wireguard" => Self::Vpn,
            _ => Self::Other,
        }
    }
}

/// The connection holding the default route, with its addresses.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveLink {
    /// Its active connection's object path.
    pub path: String,
    /// The connection's name.
    pub id: String,
    pub kind: LinkKind,
    pub state: LinkState,
    pub interface: String,
    pub hw_address: String,
    /// `address/prefix`.
    pub ipv4: Vec<String>,
    /// The IPv4 router.
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    /// `address/prefix`.
    pub ipv6: Vec<String>,
    /// The IPv6 router, usually a link-local address.
    pub gateway6: Option<String>,
}

/// Whether a network name is hidden: empty, or all zero bytes.
fn is_hidden(ssid: &[u8]) -> bool {
    ssid.iter().all(|b| *b == 0)
}

/// Name the access point in use after the network in use, when it hides its
/// name. NetworkManager can report an empty name for the access point of a
/// hidden network it joined; the saved connection knows the name, and the
/// access point then folds into that network's row like any other.
pub(crate) fn name_hidden_access_point(
    access_points: &mut [AccessPoint],
    active_ap: Option<&str>,
    in_use: Option<&[u8]>,
) {
    let (Some(active_ap), Some(in_use)) = (active_ap, in_use) else {
        return;
    };
    if is_hidden(in_use) {
        return;
    }
    if let Some(ap) = access_points
        .iter_mut()
        .find(|ap| ap.path == active_ap && is_hidden(&ap.ssid))
    {
        ap.ssid = in_use.to_vec();
    }
}

/// Fold access points into networks: one per name, hidden ones left out,
/// with the strongest access point's signal. `saved` matches names to saved
/// connections; `active` is the name in use or coming up.
///
/// A name shared by access points secured differently takes the strongest
/// security among them, and is WPA3-only when every access point with that
/// security is. The access point a connection is made for is the strongest
/// that takes that key: on a mesh where one node offers WPA3 alone and
/// another WPA2 too, a WPA2 key goes to the one that takes it.
///
/// Sorted with the network in use first, then saved networks, then the rest,
/// each by signal, then by name.
pub(crate) fn group_networks(
    access_points: &[AccessPoint],
    saved: &[SavedNetwork],
    active: Option<(&[u8], LinkState)>,
) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for ap in access_points {
        if is_hidden(&ap.ssid) {
            continue;
        }
        match networks.iter_mut().find(|n| n.ssid == ap.ssid) {
            Some(network) => {
                network.strength = network.strength.max(ap.strength);
                // A name shared by an open and a secured access point is
                // treated as the secured one: a password asked for and not
                // needed beats one needed and not asked for.
                network.security = network.security.max(ap.security);
            }
            None => networks.push(WifiNetwork {
                ssid: ap.ssid.clone(),
                strength: ap.strength,
                security: ap.security,
                access_point: String::new(),
                saved: None,
                state: LinkState::Idle,
                wpa3_only: false,
            }),
        }
    }

    for network in &mut networks {
        let secured_alike: Vec<&AccessPoint> = access_points
            .iter()
            .filter(|ap| ap.ssid == network.ssid && ap.security == network.security)
            .collect();
        network.wpa3_only = secured_alike.iter().all(|ap| ap.wpa3_only);
        // The first of the strongest, among those that take the key.
        let mut chosen: Option<&AccessPoint> = None;
        for ap in secured_alike {
            if (network.wpa3_only || !ap.wpa3_only)
                && chosen.is_none_or(|best| ap.strength > best.strength)
            {
                chosen = Some(ap);
            }
        }
        if let Some(ap) = chosen {
            network.access_point = ap.path.clone();
        }
    }

    for network in &mut networks {
        // The most recently used of several profiles for one name.
        network.saved = saved
            .iter()
            .filter(|s| s.ssid == network.ssid)
            .max_by_key(|s| s.last_used)
            .map(|s| s.path.clone());
        if let Some((ssid, state)) = active {
            if ssid == network.ssid.as_slice() {
                network.state = state;
            }
        }
    }

    networks.sort_by(|a, b| {
        let rank = |n: &WifiNetwork| (n.state == LinkState::Idle, n.saved.is_none());
        rank(a)
            .cmp(&rank(b))
            .then(b.strength.cmp(&a.strength))
            .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    networks
}

/// Saved networks in the order a list shows them: the one in use, then the
/// most recently used, then by name.
pub fn sort_saved(saved: &mut [SavedNetwork]) {
    saved.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.last_used.cmp(&a.last_used))
            .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
}

/// A link rate in kbit/s as a person reads it: "866 Mb/s", "1 Gb/s",
/// "2.4 Gb/s".
pub fn format_rate(kbps: u64) -> String {
    let mbps = kbps / 1000;
    if mbps < 1000 {
        return format!("{mbps} Mb/s");
    }
    let gbps = mbps as f64 / 1000.0;
    if (gbps - gbps.round()).abs() < 0.05 {
        format!("{} Gb/s", gbps.round())
    } else {
        format!("{gbps:.1} Gb/s")
    }
}

/// What changed, as NetworkManager's signals tell it, gathered over one
/// burst. It decides how much has to be read again: a scan moving signal
/// strengths needs no bus call at all, and saved connections are read only
/// when one of them changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// Access points whose strength moved, and to what.
    pub strengths: Vec<(String, u8)>,
    /// A saved connection was added, removed or changed.
    pub profiles: bool,
    /// Anything else: devices, active connections, NetworkManager's own
    /// state, or NetworkManager joining or leaving the bus.
    pub other: bool,
}

impl Changes {
    /// Fold `more` into this one.
    pub fn merge(&mut self, more: Changes) {
        for (path, strength) in more.strengths {
            match self.strengths.iter_mut().find(|(p, _)| *p == path) {
                Some(entry) => entry.1 = strength,
                None => self.strengths.push((path, strength)),
            }
        }
        self.profiles |= more.profiles;
        self.other |= more.other;
    }

    /// Whether only signal strengths moved.
    pub fn only_strengths(&self) -> bool {
        !self.profiles && !self.other
    }
}

impl Snapshot {
    /// Apply signal-strength changes without reading anything: the access
    /// points move, the networks are folded again, and the access point in
    /// use follows.
    pub fn apply_strengths(&mut self, strengths: &[(String, u8)]) {
        let saved = self.saved.clone();
        let Some(wifi) = self.wifi.as_mut() else {
            return;
        };
        for (path, strength) in strengths {
            if let Some(ap) = wifi.access_points.iter_mut().find(|ap| ap.path == *path) {
                ap.strength = *strength;
            }
            if let Some(ap) = wifi.active_ap.as_mut().filter(|ap| ap.path == *path) {
                ap.strength = *strength;
            }
        }
        wifi.networks = group_networks(
            &wifi.access_points,
            &saved,
            wifi.in_use
                .as_ref()
                .map(|(ssid, state)| (ssid.as_slice(), *state)),
        );
    }
}

/// What the top bar's indicator draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// Nothing: NetworkManager is not running, or there is no device.
    Hidden,
    /// Wi-Fi off and nothing wired.
    WifiOff,
    /// A connection coming up.
    Connecting,
    Wired,
    /// Wi-Fi in use with `0..=3` bars.
    Wifi(u8),
    /// Nothing connected.
    Disconnected,
}

impl Snapshot {
    /// The network in use on the Wi-Fi device.
    pub fn connected_network(&self) -> Option<&WifiNetwork> {
        self.wifi
            .as_ref()?
            .networks
            .iter()
            .find(|n| n.state == LinkState::Connected)
    }

    /// Whether the Wi-Fi radio is actually on: NetworkManager has it on, and
    /// no switch blocks it. NetworkManager can report itself on while a
    /// switch holds the radio off, which is still off to a person.
    pub fn wifi_on(&self) -> bool {
        self.wifi_enabled && self.wifi_hardware_enabled && !self.wifi_block.any()
    }

    /// Whether only a physical switch (or the firmware) can turn the radio
    /// back on: nothing on screen can.
    pub fn wifi_hard_blocked(&self) -> bool {
        !self.wifi_hardware_enabled || self.wifi_block.hard
    }

    /// Whether a switch the person flipped elsewhere (the airplane key,
    /// another desktop) holds the radio off, which turning Wi-Fi on lifts.
    pub fn wifi_soft_blocked(&self) -> bool {
        self.wifi_block.soft && !self.wifi_hard_blocked()
    }

    /// The signal bars of the Wi-Fi link, `None` while there is none.
    ///
    /// The network in use when it has a row; otherwise the Wi-Fi device's
    /// own state, with the access point it is associated with: a link
    /// whose network has no row (no saved connection names a hidden
    /// access point) is still up.
    fn wifi_link_bars(&self) -> Option<u8> {
        if let Some(network) = self.connected_network() {
            return Some(network.bars());
        }
        let wifi = self.wifi.as_ref()?;
        (wifi.device.state == DeviceState::Connected)
            .then(|| wifi.active_ap.as_ref().map_or(0, |ap| bars(ap.strength)))
    }

    /// Whether a wired device has its connection up.
    pub fn wired_connected(&self) -> bool {
        self.devices
            .iter()
            .any(|d| d.kind == DeviceKind::Ethernet && d.state == DeviceState::Connected)
    }

    /// Whether any device is bringing a connection up.
    pub fn connecting(&self) -> bool {
        self.devices
            .iter()
            .any(|d| d.state == DeviceState::Connecting)
    }

    /// Whether the connection is up but the internet is not reachable
    /// through it.
    pub fn limited(&self) -> bool {
        matches!(
            self.connectivity,
            Connectivity::Portal | Connectivity::Limited
        ) && self.primary.is_some()
    }

    /// What the indicator draws.
    pub fn glyph(&self) -> Glyph {
        if !self.running || self.devices.is_empty() {
            return Glyph::Hidden;
        }
        // The kind of the primary connection decides, so a laptop on a
        // cable with Wi-Fi also up shows what its traffic goes over. A link
        // carrying traffic also wins over another coming up: a cable stuck
        // waiting for an address is not what the machine is using.
        let primary = self.primary.as_ref().map(|p| p.kind);
        if primary == Some(LinkKind::Ethernet) {
            return Glyph::Wired;
        }
        if primary.is_some() {
            if let Some(bars) = self.wifi_link_bars() {
                return Glyph::Wifi(bars);
            }
            if self.wired_connected() {
                return Glyph::Wired;
            }
        }
        if self.connecting() {
            return Glyph::Connecting;
        }
        if let Some(bars) = self.wifi_link_bars() {
            return Glyph::Wifi(bars);
        }
        if self.wired_connected() {
            return Glyph::Wired;
        }
        if !self.wifi_on() && self.wifi.is_some() {
            return Glyph::WifiOff;
        }
        Glyph::Disconnected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(path: &str, ssid: &str, strength: u8, security: Security) -> AccessPoint {
        AccessPoint {
            path: path.into(),
            ssid: ssid.as_bytes().to_vec(),
            strength,
            security,
            wpa3_only: false,
            frequency_mhz: 2412,
            bssid: String::new(),
        }
    }

    fn saved(path: &str, ssid: &str, last_used: u64) -> SavedNetwork {
        SavedNetwork {
            path: path.into(),
            id: ssid.into(),
            ssid: ssid.as_bytes().to_vec(),
            autoconnect: true,
            last_used,
            in_use: false,
        }
    }

    #[test]
    fn one_row_per_name_with_the_strongest_access_point() {
        let aps = [
            ap("/ap/1", "Home", 40, Security::Personal),
            ap("/ap/2", "Home", 80, Security::Personal),
            ap("/ap/3", "Cafe", 60, Security::Open),
        ];
        let networks = group_networks(&aps, &[], None);
        assert_eq!(networks.len(), 2);
        let home = networks.iter().find(|n| n.name() == "Home").unwrap();
        assert_eq!(home.strength, 80);
        assert_eq!(home.access_point, "/ap/2");
    }

    #[test]
    fn hidden_networks_have_no_row() {
        let aps = [
            ap("/ap/1", "", 90, Security::Personal),
            AccessPoint {
                ssid: vec![0, 0, 0],
                ..ap("/ap/2", "", 90, Security::Personal)
            },
        ];
        assert!(group_networks(&aps, &[], None).is_empty());
    }

    #[test]
    fn in_use_then_saved_then_the_rest_by_signal() {
        let aps = [
            ap("/ap/1", "Strong", 90, Security::Personal),
            ap("/ap/2", "Saved", 30, Security::Personal),
            ap("/ap/3", "Current", 20, Security::Personal),
            ap("/ap/4", "Weak", 10, Security::Open),
        ];
        let saved = [saved("/c/1", "Saved", 5), saved("/c/2", "Current", 9)];
        let networks = group_networks(
            &aps,
            &saved,
            Some((b"Current".as_slice(), LinkState::Connected)),
        );
        let names: Vec<_> = networks.iter().map(|n| n.name()).collect();
        assert_eq!(names, ["Current", "Saved", "Strong", "Weak"]);
        assert_eq!(networks[0].state, LinkState::Connected);
        assert_eq!(networks[1].saved.as_deref(), Some("/c/1"));
        assert_eq!(networks[2].saved, None);
    }

    #[test]
    fn the_most_recent_of_two_profiles_is_used() {
        let aps = [ap("/ap/1", "Home", 50, Security::Personal)];
        let saved = [saved("/c/old", "Home", 1), saved("/c/new", "Home", 2)];
        let networks = group_networks(&aps, &saved, None);
        assert_eq!(networks[0].saved.as_deref(), Some("/c/new"));
    }

    #[test]
    fn a_name_mixing_open_and_secured_counts_as_secured() {
        let aps = [
            ap("/ap/1", "Mixed", 80, Security::Open),
            ap("/ap/2", "Mixed", 20, Security::Personal),
        ];
        assert_eq!(
            group_networks(&aps, &[], None)[0].security,
            Security::Personal
        );
    }

    #[test]
    fn security_from_flags() {
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(1, 0, 0), Security::Wep);
        assert_eq!(Security::from_flags(1, 0, 0x188), Security::Personal);
        // WPA3-Personal.
        assert_eq!(Security::from_flags(1, 0, 0x400), Security::Personal);
        assert_eq!(Security::from_flags(1, 0x200, 0x200), Security::Enterprise);
        // Enhanced Open (OWE).
        assert_eq!(Security::from_flags(1, 0, 0x800), Security::Open);
    }

    #[test]
    fn link_rates() {
        assert_eq!(format_rate(866_000), "866 Mb/s");
        assert_eq!(format_rate(1_000_000), "1 Gb/s");
        assert_eq!(format_rate(2_402_000), "2.4 Gb/s");
    }

    #[test]
    fn changes_merge_and_strengths_apply_without_a_read() {
        let mut changes = Changes {
            strengths: vec![("/ap/1".into(), 10)],
            ..Changes::default()
        };
        changes.merge(Changes {
            strengths: vec![("/ap/1".into(), 90)],
            ..Changes::default()
        });
        assert_eq!(changes.strengths, [("/ap/1".to_string(), 90)]);
        assert!(changes.only_strengths());
        changes.merge(Changes {
            profiles: true,
            ..Changes::default()
        });
        assert!(!changes.only_strengths());

        let aps = vec![
            ap("/ap/1", "Home", 30, Security::Personal),
            ap("/ap/2", "Cafe", 60, Security::Open),
        ];
        let mut snapshot = wifi_snapshot(group_networks(&aps, &[], None), DeviceState::Connected);
        let wifi = snapshot.wifi.as_mut().unwrap();
        wifi.access_points = aps;
        assert_eq!(wifi.networks[0].name(), "Cafe");
        snapshot.apply_strengths(&[("/ap/1".into(), 95)]);
        let networks = &snapshot.wifi.as_ref().unwrap().networks;
        assert_eq!(networks[0].name(), "Home");
        assert_eq!(networks[0].strength, 95);
    }

    #[test]
    fn bands_and_channels() {
        assert_eq!(Band::from_mhz(2412), Band::Ghz2);
        assert_eq!(channel(2412), 1);
        assert_eq!(channel(2437), 6);
        assert_eq!(channel(2484), 14);
        assert_eq!(Band::from_mhz(5180), Band::Ghz5);
        assert_eq!(channel(5180), 36);
        assert_eq!(channel(5825), 165);
        assert_eq!(Band::from_mhz(5955), Band::Ghz6);
        assert_eq!(channel(5955), 1);
        assert_eq!(Band::from_mhz(60_000), Band::Unknown);
    }

    #[test]
    fn what_joining_asks_for() {
        use crate::profile::KeyKind;
        let mut network =
            group_networks(&[ap("/ap/1", "X", 50, Security::Personal)], &[], None).remove(0);
        assert_eq!(network.key_kind(), Some(KeyKind::WpaPsk));
        assert!(network.needs_password());
        network.wpa3_only = true;
        assert_eq!(network.key_kind(), Some(KeyKind::Sae));
        network.saved = Some("/c/1".into());
        assert!(!network.needs_password());
        network.security = Security::Enterprise;
        assert_eq!(network.key_kind(), None);
    }

    #[test]
    fn signal_bars() {
        assert_eq!(bars(0), 0);
        assert_eq!(bars(24), 0);
        assert_eq!(bars(25), 1);
        assert_eq!(bars(50), 2);
        assert_eq!(bars(74), 2);
        assert_eq!(bars(75), 3);
        assert_eq!(bars(100), 3);
    }

    #[test]
    fn device_states() {
        assert_eq!(DeviceState::from_raw(20), DeviceState::Unavailable);
        assert_eq!(DeviceState::from_raw(50), DeviceState::Connecting);
        assert_eq!(DeviceState::from_raw(100), DeviceState::Connected);
        assert_eq!(DeviceState::from_raw(10), DeviceState::Unknown);
    }

    fn device(kind: DeviceKind, state: DeviceState) -> Device {
        Device {
            path: "/d".into(),
            kind,
            interface: "x".into(),
            state,
            hw_address: String::new(),
            active_profile: None,
            speed_mbps: None,
            active: None,
            link: None,
        }
    }

    fn link(kind: LinkKind) -> ActiveLink {
        ActiveLink {
            path: "/a".into(),
            id: "x".into(),
            kind,
            state: LinkState::Connected,
            interface: String::new(),
            hw_address: String::new(),
            ipv4: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            ipv6: Vec::new(),
            gateway6: None,
        }
    }

    fn wifi_snapshot(networks: Vec<WifiNetwork>, state: DeviceState) -> Snapshot {
        let dev = device(DeviceKind::Wifi, state);
        Snapshot {
            running: true,
            wifi_enabled: true,
            wifi_hardware_enabled: true,
            devices: vec![dev.clone()],
            wifi: Some(WifiDevice::new(dev, networks)),
            ..Snapshot::default()
        }
    }

    #[test]
    fn the_glyph_follows_the_state() {
        assert_eq!(Snapshot::default().glyph(), Glyph::Hidden);

        let mut snapshot = wifi_snapshot(Vec::new(), DeviceState::Disconnected);
        assert_eq!(snapshot.glyph(), Glyph::Disconnected);

        snapshot.wifi_enabled = false;
        assert_eq!(snapshot.glyph(), Glyph::WifiOff);

        let connected = group_networks(
            &[ap("/ap/1", "Home", 60, Security::Personal)],
            &[],
            Some((b"Home".as_slice(), LinkState::Connected)),
        );
        let mut snapshot = wifi_snapshot(connected, DeviceState::Connected);
        snapshot.primary = Some(link(LinkKind::Wifi));
        assert_eq!(snapshot.glyph(), Glyph::Wifi(2));

        // A cable carrying the default route wins over Wi-Fi.
        snapshot
            .devices
            .push(device(DeviceKind::Ethernet, DeviceState::Connected));
        snapshot.primary = Some(link(LinkKind::Ethernet));
        assert_eq!(snapshot.glyph(), Glyph::Wired);

        let snapshot = wifi_snapshot(Vec::new(), DeviceState::Connecting);
        assert_eq!(snapshot.glyph(), Glyph::Connecting);
    }

    #[test]
    fn a_cable_coming_up_does_not_hide_the_wifi_in_use() {
        let connected = group_networks(
            &[ap("/ap/1", "Home", 80, Security::Personal)],
            &[],
            Some((b"Home".as_slice(), LinkState::Connected)),
        );
        let mut snapshot = wifi_snapshot(connected, DeviceState::Connected);
        snapshot
            .devices
            .push(device(DeviceKind::Ethernet, DeviceState::Connecting));
        // Nothing carries the default route yet: something is coming up.
        assert_eq!(snapshot.glyph(), Glyph::Connecting);
        snapshot.primary = Some(link(LinkKind::Wifi));
        assert_eq!(snapshot.glyph(), Glyph::Wifi(3));
    }

    #[test]
    fn a_hidden_network_in_use_takes_its_saved_name() {
        let mut aps = vec![
            ap("/ap/1", "", 80, Security::Personal),
            ap("/ap/2", "", 40, Security::Personal),
            ap("/ap/3", "Cafe", 60, Security::Open),
        ];
        name_hidden_access_point(&mut aps, Some("/ap/1"), Some(b"Secret".as_slice()));
        assert_eq!(aps[0].ssid, b"Secret");
        assert!(aps[1].ssid.is_empty(), "only the access point in use");
        // A named access point keeps its name.
        name_hidden_access_point(&mut aps, Some("/ap/3"), Some(b"Secret".as_slice()));
        assert_eq!(aps[2].ssid, b"Cafe");

        let networks = group_networks(
            &aps,
            &[],
            Some((b"Secret".as_slice(), LinkState::Connected)),
        );
        let mut snapshot = wifi_snapshot(networks, DeviceState::Connected);
        snapshot.primary = Some(link(LinkKind::Wifi));
        assert_eq!(
            snapshot.connected_network().map(|n| n.name()).as_deref(),
            Some("Secret")
        );
        assert_eq!(snapshot.glyph(), Glyph::Wifi(3));
    }

    #[test]
    fn a_wifi_link_without_a_row_still_reads_connected() {
        let mut snapshot = wifi_snapshot(Vec::new(), DeviceState::Connected);
        snapshot.primary = Some(link(LinkKind::Wifi));
        snapshot.wifi.as_mut().unwrap().active_ap = Some(ap("/ap/1", "", 55, Security::Personal));
        assert_eq!(snapshot.connected_network(), None);
        assert_eq!(snapshot.glyph(), Glyph::Wifi(2));
    }

    #[test]
    fn a_mesh_joins_the_node_that_takes_the_key() {
        let sae_only = |path: &str, strength: u8| AccessPoint {
            wpa3_only: true,
            ..ap(path, "Mesh", strength, Security::Personal)
        };
        // The strongest node takes WPA3 alone, another WPA2 too: the key is
        // WPA2's, so the connection goes to the node that takes it.
        let networks = group_networks(
            &[
                sae_only("/ap/1", 90),
                ap("/ap/2", "Mesh", 50, Security::Personal),
                ap("/ap/3", "Mesh", 30, Security::Personal),
            ],
            &[],
            None,
        );
        let mesh = &networks[0];
        assert_eq!(mesh.strength, 90);
        assert_eq!(mesh.key_kind(), Some(crate::profile::KeyKind::WpaPsk));
        assert_eq!(mesh.access_point, "/ap/2");

        // Every node WPA3 alone: WPA3's key, to the strongest.
        let networks = group_networks(&[sae_only("/ap/1", 40), sae_only("/ap/2", 70)], &[], None);
        assert_eq!(networks[0].key_kind(), Some(crate::profile::KeyKind::Sae));
        assert_eq!(networks[0].access_point, "/ap/2");

        // An open node beside a WPA3-only one: the name is secured, with
        // WPA3's key, and the open node is not where it goes.
        let networks = group_networks(
            &[
                ap("/ap/1", "Mesh", 90, Security::Open),
                sae_only("/ap/2", 20),
            ],
            &[],
            None,
        );
        assert_eq!(networks[0].key_kind(), Some(crate::profile::KeyKind::Sae));
        assert_eq!(networks[0].access_point, "/ap/2");
    }

    #[test]
    fn a_switch_holding_the_radio_off_is_off() {
        let mut snapshot = wifi_snapshot(Vec::new(), DeviceState::Unavailable);
        assert!(snapshot.wifi_on());
        // NetworkManager still says enabled, but the switch says no.
        snapshot.wifi_hardware_enabled = false;
        assert!(!snapshot.wifi_on());
        assert!(snapshot.wifi_hard_blocked() && !snapshot.wifi_soft_blocked());
        assert_eq!(snapshot.glyph(), Glyph::WifiOff);

        snapshot.wifi_hardware_enabled = true;
        snapshot.wifi_block.soft = true;
        assert!(!snapshot.wifi_on());
        assert!(snapshot.wifi_soft_blocked());
        assert_eq!(snapshot.glyph(), Glyph::WifiOff);

        // A hard block wins: the soft one under it is not what to lift.
        snapshot.wifi_block.hard = true;
        assert!(snapshot.wifi_hard_blocked() && !snapshot.wifi_soft_blocked());
    }

    #[test]
    fn limited_needs_a_connection() {
        let mut snapshot = wifi_snapshot(Vec::new(), DeviceState::Connected);
        snapshot.connectivity = Connectivity::Portal;
        assert!(!snapshot.limited());
        snapshot.primary = Some(link(LinkKind::Wifi));
        assert!(snapshot.limited());
        snapshot.connectivity = Connectivity::Full;
        assert!(!snapshot.limited());
    }

    #[test]
    fn saved_networks_in_use_first_then_recent() {
        let mut list = vec![
            saved("/c/1", "Old", 1),
            saved("/c/2", "Recent", 9),
            SavedNetwork {
                in_use: true,
                ..saved("/c/3", "Here", 0)
            },
        ];
        sort_saved(&mut list);
        let names: Vec<_> = list.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["Here", "Recent", "Old"]);
    }
}
