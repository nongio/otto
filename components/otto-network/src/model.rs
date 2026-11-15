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
    /// Whether a hardware switch lets them be on.
    pub wifi_hardware_enabled: bool,
    pub connectivity: Connectivity,
    /// The first managed Wi-Fi device, with the networks it sees.
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
}

/// The Wi-Fi device and the networks it sees.
#[derive(Debug, Clone, PartialEq)]
pub struct WifiDevice {
    pub device: Device,
    /// One per network name, as [`group_networks`] makes them.
    pub networks: Vec<WifiNetwork>,
    /// Milliseconds of `CLOCK_BOOTTIME` at the last scan; `None` for never.
    pub last_scan_ms: Option<i64>,
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
    pub frequency_mhz: u32,
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
    /// The strongest access point, which a new connection is made for.
    pub access_point: String,
    /// The saved connection for this network, if there is one.
    pub saved: Option<String>,
    pub state: LinkState,
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
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    /// `address/prefix`.
    pub ipv6: Vec<String>,
}

/// Fold access points into networks: one per name, hidden ones left out,
/// with the strongest access point's signal and path. `saved` matches names
/// to saved connections; `active` is the name in use or coming up.
///
/// Sorted with the network in use first, then saved networks, then the rest,
/// each by signal, then by name.
pub fn group_networks(
    access_points: &[AccessPoint],
    saved: &[SavedNetwork],
    active: Option<(&[u8], LinkState)>,
) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for ap in access_points {
        if ap.ssid.is_empty() || ap.ssid.iter().all(|b| *b == 0) {
            continue;
        }
        match networks.iter_mut().find(|n| n.ssid == ap.ssid) {
            Some(network) => {
                if ap.strength > network.strength {
                    network.strength = ap.strength;
                    network.access_point = ap.path.clone();
                }
                // A name shared by an open and a secured access point is
                // treated as the secured one: a password asked for and not
                // needed beats one needed and not asked for.
                network.security = network.security.max(ap.security);
            }
            None => networks.push(WifiNetwork {
                ssid: ap.ssid.clone(),
                strength: ap.strength,
                security: ap.security,
                access_point: ap.path.clone(),
                saved: None,
                state: LinkState::Idle,
            }),
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
        if self.connecting() {
            return Glyph::Connecting;
        }
        // The kind of the primary connection decides, so a laptop on a
        // cable with Wi-Fi also up shows what its traffic goes over.
        let primary = self.primary.as_ref().map(|p| p.kind);
        if primary == Some(LinkKind::Ethernet) {
            return Glyph::Wired;
        }
        if let Some(network) = self.connected_network() {
            return Glyph::Wifi(network.bars());
        }
        if self.wired_connected() {
            return Glyph::Wired;
        }
        if !self.wifi_enabled && self.wifi.is_some() {
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
            frequency_mhz: 2412,
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
        }
    }

    fn wifi_snapshot(networks: Vec<WifiNetwork>, state: DeviceState) -> Snapshot {
        let dev = device(DeviceKind::Wifi, state);
        Snapshot {
            running: true,
            wifi_enabled: true,
            wifi_hardware_enabled: true,
            devices: vec![dev.clone()],
            wifi: Some(WifiDevice {
                device: dev,
                networks,
                last_scan_ms: None,
            }),
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
