//! Saved connection profiles: what a connection's settings say, the IP
//! configuration a person can edit, and turning an edit back into settings.
//!
//! NetworkManager passes a connection as nested dictionaries
//! (`a{sa{sv}}`): setting name to property to value. Everything here reads
//! and writes that shape without the bus, so all of it is tested without
//! one. The property names are NetworkManager's (`man nm-settings-dbus`).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use zbus::zvariant::{OwnedValue, Value};

/// A connection's settings, as NetworkManager passes them.
pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

/// Which address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    /// The setting's name in a connection.
    pub fn setting(self) -> &'static str {
        match self {
            Self::V4 => "ipv4",
            Self::V6 => "ipv6",
        }
    }

    /// The prefix an address typed without one gets.
    pub fn default_prefix(self) -> u32 {
        match self {
            Self::V4 => 24,
            Self::V6 => 64,
        }
    }

    fn max_prefix(self) -> u32 {
        match self {
            Self::V4 => 32,
            Self::V6 => 128,
        }
    }
}

/// How a family gets its addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpMethod {
    /// DHCP, and for IPv6 router advertisements (SLAAC) first.
    Automatic,
    /// IPv6 only: DHCPv6 without router advertisements.
    DhcpOnly,
    Manual,
    /// Only a link-local address, no server asked.
    LinkLocal,
    /// Share this machine's connection with the network on this device.
    Shared,
    /// The family is off on this connection.
    Disabled,
    /// IPv6 only: NetworkManager leaves the family to the kernel. Read as
    /// off, and written back as it was; choosing Off writes `disabled`.
    Ignore,
    /// A method this code has no name for, kept as it is.
    Other(String),
}

impl IpMethod {
    pub fn from_nm(raw: &str) -> Self {
        match raw {
            "auto" => Self::Automatic,
            "dhcp" => Self::DhcpOnly,
            "manual" => Self::Manual,
            "link-local" => Self::LinkLocal,
            "shared" => Self::Shared,
            "disabled" => Self::Disabled,
            "ignore" => Self::Ignore,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn to_nm(&self, _family: Family) -> String {
        match self {
            Self::Automatic => "auto",
            Self::DhcpOnly => "dhcp",
            Self::Manual => "manual",
            Self::LinkLocal => "link-local",
            Self::Shared => "shared",
            Self::Disabled => "disabled",
            Self::Ignore => "ignore",
            Self::Other(raw) => raw,
        }
        .to_string()
    }

    /// The methods a person picks from, for `family`.
    pub fn choices(family: Family) -> &'static [IpMethod] {
        match family {
            Family::V4 => &[
                IpMethod::Automatic,
                IpMethod::Manual,
                IpMethod::LinkLocal,
                IpMethod::Shared,
                IpMethod::Disabled,
            ],
            Family::V6 => &[
                IpMethod::Automatic,
                IpMethod::DhcpOnly,
                IpMethod::Manual,
                IpMethod::LinkLocal,
                IpMethod::Shared,
                IpMethod::Disabled,
            ],
        }
    }
}

/// One address with its prefix length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpAddress {
    pub address: String,
    pub prefix: u32,
}

impl std::fmt::Display for IpAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

/// One family's configuration on a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpConfig {
    pub method: IpMethod,
    /// Static addresses; what Manual uses.
    pub addresses: Vec<IpAddress>,
    pub gateway: Option<String>,
    /// Name servers set on the connection, used before (or, with
    /// `ignore_auto_dns`, instead of) the ones DHCP hands out.
    pub dns: Vec<String>,
    /// Use only `dns`, not the servers DHCP or the router offers.
    pub ignore_auto_dns: bool,
    pub dns_search: Vec<String>,
}

impl Default for IpConfig {
    fn default() -> Self {
        Self {
            method: IpMethod::Automatic,
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            ignore_auto_dns: false,
            dns_search: Vec::new(),
        }
    }
}

/// What is wrong with an edited configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpError {
    /// Manual with no address.
    NoAddress,
    /// Not an address of this family; carries what was typed.
    BadAddress(String),
    BadPrefix(String),
    BadGateway(String),
    BadDns(String),
    BadDomain(String),
}

fn parse_ip(text: &str, family: Family) -> Option<IpAddr> {
    let ip: IpAddr = text.trim().parse().ok()?;
    match (family, ip) {
        (Family::V4, IpAddr::V4(_)) | (Family::V6, IpAddr::V6(_)) => Some(ip),
        _ => None,
    }
}

/// The server address in a DNS entry as `dns-data` writes one: an address,
/// optionally with a port (`1.1.1.1:853`, `[fd00::1]:853`), an IPv6 scope
/// (`fe80::1%eth0`) and a DNS-over-TLS server name (`1.1.1.1#one.one.one.one`).
pub fn dns_address(entry: &str, family: Family) -> Option<IpAddr> {
    let entry = entry.trim();
    let address = entry.split('#').next()?;
    if let Some(ip) = parse_ip(address, family) {
        return Some(ip);
    }
    let address = match family {
        Family::V4 => address.split(':').next()?,
        Family::V6 => {
            let unbracketed = match address.strip_prefix('[') {
                Some(rest) => rest.split(']').next()?,
                None => address,
            };
            unbracketed.split('%').next()?
        }
    };
    parse_ip(address, family)
}

/// Read an address as a person types it: `192.168.1.20/24`, or without a
/// prefix, which then gets the family's usual one.
pub fn parse_address(text: &str, family: Family) -> Result<IpAddress, IpError> {
    let text = text.trim();
    let (address, prefix) = match text.split_once('/') {
        Some((address, prefix)) => {
            let prefix: u32 = prefix
                .trim()
                .parse()
                .map_err(|_| IpError::BadPrefix(text.to_string()))?;
            (address, prefix)
        }
        None => (text, family.default_prefix()),
    };
    let ip = parse_ip(address, family).ok_or_else(|| IpError::BadAddress(text.to_string()))?;
    if prefix == 0 || prefix > family.max_prefix() {
        return Err(IpError::BadPrefix(text.to_string()));
    }
    Ok(IpAddress {
        address: ip.to_string(),
        prefix,
    })
}

/// Whether `domain` is a search domain NetworkManager takes: a name, with
/// an optional trailing dot, or prefixed `~` to route its lookups without
/// searching it. `~.` sends every lookup to this connection's servers.
/// Labels may hold `_` (service names) and letters outside ASCII (an
/// internationalised name as typed).
pub fn search_domain(domain: &str) -> bool {
    let name = domain.strip_prefix('~').unwrap_or(domain);
    if name == "." && domain.starts_with('~') {
        return true;
    }
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
}

/// Split a typed list: commas, spaces or both between entries.
pub fn split_list(text: &str) -> Vec<String> {
    text.split([',', ' ', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

impl IpConfig {
    /// Whether the configuration can be saved, as a person filled it in.
    pub fn validate(&self, family: Family) -> Result<(), IpError> {
        if self.method == IpMethod::Manual && self.addresses.is_empty() {
            return Err(IpError::NoAddress);
        }
        for address in &self.addresses {
            parse_address(&address.to_string(), family)?;
        }
        // NetworkManager takes a gateway only with manual addresses, and one
        // left over from Manual is not saved otherwise.
        if let Some(gateway) = self
            .gateway
            .as_ref()
            .filter(|_| self.method == IpMethod::Manual)
        {
            parse_ip(gateway, family).ok_or_else(|| IpError::BadGateway(gateway.clone()))?;
        }
        for server in &self.dns {
            dns_address(server, family).ok_or_else(|| IpError::BadDns(server.clone()))?;
        }
        for domain in &self.dns_search {
            if !search_domain(domain) {
                return Err(IpError::BadDomain(domain.clone()));
            }
        }
        Ok(())
    }

    /// Read `family`'s setting out of a connection. A connection without one
    /// is automatic, which is what NetworkManager assumes.
    pub fn from_settings(settings: &Settings, family: Family) -> Self {
        let Some(ip) = settings.get(family.setting()) else {
            return Self::default();
        };
        let method = string(ip, "method")
            .map(|m| IpMethod::from_nm(&m))
            .unwrap_or(IpMethod::Automatic);

        let mut addresses: Vec<IpAddress> = ip
            .get("address-data")
            .and_then(|v| Vec::<HashMap<String, OwnedValue>>::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| {
                Some(IpAddress {
                    address: string(&entry, "address")?,
                    prefix: entry
                        .get("prefix")
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(family.default_prefix()),
                })
            })
            .collect();
        // Before `address-data` (NetworkManager 1.0), IPv4 addresses were
        // `[address, prefix, gateway]` in network byte order.
        if addresses.is_empty() && family == Family::V4 {
            addresses = ip
                .get("addresses")
                .and_then(|v| Vec::<Vec<u32>>::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|triple| {
                    Some(IpAddress {
                        address: Ipv4Addr::from(triple.first()?.to_ne_bytes()).to_string(),
                        prefix: *triple.get(1)?,
                    })
                })
                .collect();
        }

        // `dns-data` (1.42) holds strings; `dns` holds raw addresses, IPv4
        // as network-order u32 and IPv6 as 16 bytes.
        let dns_data: Option<Vec<String>> = ip
            .get("dns-data")
            .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok());
        let dns = match dns_data {
            Some(list) if !list.is_empty() => list,
            _ => match family {
                Family::V4 => ip
                    .get("dns")
                    .and_then(|v| Vec::<u32>::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|raw| Ipv4Addr::from(raw.to_ne_bytes()).to_string())
                    .collect(),
                Family::V6 => ip
                    .get("dns")
                    .and_then(|v| Vec::<Vec<u8>>::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|raw| {
                        let bytes: [u8; 16] = raw.try_into().ok()?;
                        Some(Ipv6Addr::from(bytes).to_string())
                    })
                    .collect(),
            },
        };

        Self {
            method,
            addresses,
            gateway: string(ip, "gateway").filter(|g| !g.is_empty()),
            dns,
            ignore_auto_dns: ip
                .get("ignore-auto-dns")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false),
            dns_search: ip
                .get("dns-search")
                .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default(),
        }
    }

    /// Write this configuration into `settings`, over `family`'s setting,
    /// keeping every property of it this code does not edit. Each value is
    /// written in one spelling only (`address-data`, not the older
    /// `addresses`; `dns-data` or the older `dns`, see below), so
    /// NetworkManager cannot prefer a stale copy over the new one.
    pub fn write_into(&self, settings: &mut Settings, family: Family) {
        let ip = settings.entry(family.setting().to_string()).or_default();
        let owned = |value: Value<'_>| OwnedValue::try_from(value).expect("plain values");

        ip.insert(
            "method".into(),
            owned(Value::from(self.method.to_nm(family))),
        );
        ip.remove("addresses");
        ip.remove("address-data");
        ip.remove("gateway");
        // Static addresses only mean something to Manual; an automatic
        // connection keeps none, as a person switching back expects.
        if self.method == IpMethod::Manual {
            let data: Vec<HashMap<String, Value<'_>>> = self
                .addresses
                .iter()
                .map(|a| {
                    HashMap::from([
                        ("address".to_string(), Value::from(a.address.clone())),
                        ("prefix".to_string(), Value::from(a.prefix)),
                    ])
                })
                .collect();
            ip.insert("address-data".into(), owned(Value::from(data)));
            if let Some(gateway) = &self.gateway {
                ip.insert("gateway".into(), owned(Value::from(gateway.clone())));
            }
        }

        // `dns-data` (NetworkManager 1.42) is the newer spelling and the
        // only one that holds a port, a scope or a DNS-over-TLS name. It is
        // written where the connection already uses it, or where an entry
        // needs it; otherwise the older `dns`, which every NetworkManager
        // reads. Only one is written, so neither can shadow the other.
        let plain = |entry: &String| {
            dns_address(entry, family).is_some_and(|ip| ip.to_string() == entry.trim())
        };
        let wants_data = ip.contains_key("dns-data") || !self.dns.iter().all(plain);
        ip.remove("dns-data");
        ip.remove("dns");
        if wants_data {
            let entries: Vec<String> = self.dns.iter().map(|e| e.trim().to_string()).collect();
            ip.insert("dns-data".into(), owned(Value::from(entries)));
        } else {
            match family {
                Family::V4 => {
                    let raw: Vec<u32> = self
                        .dns
                        .iter()
                        .filter_map(|s| s.trim().parse::<Ipv4Addr>().ok())
                        .map(|ip| u32::from_ne_bytes(ip.octets()))
                        .collect();
                    ip.insert("dns".into(), owned(Value::from(raw)));
                }
                Family::V6 => {
                    let raw: Vec<Vec<u8>> = self
                        .dns
                        .iter()
                        .filter_map(|s| s.trim().parse::<Ipv6Addr>().ok())
                        .map(|ip| ip.octets().to_vec())
                        .collect();
                    ip.insert("dns".into(), owned(Value::from(raw)));
                }
            }
        }
        ip.insert(
            "ignore-auto-dns".into(),
            owned(Value::from(self.ignore_auto_dns)),
        );
        ip.insert(
            "dns-search".into(),
            owned(Value::from(self.dns_search.clone())),
        );
    }
}

fn string(map: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

/// What kind of connection a profile is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    Wifi,
    Ethernet,
    Vpn,
    Other,
}

impl ProfileKind {
    pub fn from_type(kind: &str) -> Self {
        match kind {
            "802-11-wireless" => Self::Wifi,
            "802-3-ethernet" => Self::Ethernet,
            "vpn" | "wireguard" => Self::Vpn,
            _ => Self::Other,
        }
    }
}

impl ProfileKind {
    /// The setting of this kind's link: where its hardware address lives.
    /// `None` for a kind with no hardware address of its own.
    pub fn link_setting(self) -> Option<&'static str> {
        match self {
            Self::Wifi => Some("802-11-wireless"),
            Self::Ethernet => Some("802-3-ethernet"),
            Self::Vpn | Self::Other => None,
        }
    }
}

/// The hardware address a connection shows the network it joins.
///
/// NetworkManager keeps it as `cloned-mac-address` in the link's setting
/// (`802-11-wireless` or `802-3-ethernet`). On D-Bus that property holds only
/// an explicit address, as bytes; the named policies travel as the string
/// `assigned-mac-address`, which is read first and the only one written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacPolicy {
    /// Nothing set: NetworkManager's own default applies.
    Unset,
    /// The device's own address (`permanent`): no private address.
    Permanent,
    /// A private address generated once for this connection (`stable`).
    Stable,
    /// A new private address each time it connects (`random`).
    Random,
    /// Any other value (`preserve`, an explicit address), kept as it is.
    Kept(String),
}

impl MacPolicy {
    /// The three a person picks from.
    pub fn choices() -> &'static [MacPolicy] {
        &[MacPolicy::Permanent, MacPolicy::Stable, MacPolicy::Random]
    }

    /// Read NetworkManager's value; an empty one is unset.
    pub fn from_nm(raw: &str) -> Self {
        match raw {
            "" => Self::Unset,
            "permanent" => Self::Permanent,
            "stable" => Self::Stable,
            "random" => Self::Random,
            other => Self::Kept(other.to_string()),
        }
    }

    /// NetworkManager's value; empty for unset.
    pub fn to_nm(&self) -> &str {
        match self {
            Self::Unset => "",
            Self::Permanent => "permanent",
            Self::Stable => "stable",
            Self::Random => "random",
            Self::Kept(raw) => raw,
        }
    }

    /// Read the policy of a connection of `kind` out of `settings`.
    pub fn from_settings(settings: &Settings, kind: ProfileKind) -> Self {
        let Some(link) = kind.link_setting().and_then(|name| settings.get(name)) else {
            return Self::Unset;
        };
        if let Some(assigned) = string(link, "assigned-mac-address") {
            return Self::from_nm(&assigned);
        }
        // An explicit address, from a NetworkManager that sends only the
        // older property.
        link.get("cloned-mac-address")
            .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| {
                let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                Self::Kept(hex.join(":"))
            })
            .unwrap_or(Self::Unset)
    }

    /// Write this policy into a connection of `kind`, keeping every other
    /// property. Written as `assigned-mac-address` only, so the older
    /// `cloned-mac-address` cannot shadow it. Does nothing for a kind with
    /// no link setting.
    pub fn write_into(&self, settings: &mut Settings, kind: ProfileKind) {
        let Some(name) = kind.link_setting() else {
            return;
        };
        let link = settings.entry(name.to_string()).or_default();
        link.remove("cloned-mac-address");
        link.remove("assigned-mac-address");
        if *self != Self::Unset {
            link.insert(
                "assigned-mac-address".into(),
                OwnedValue::try_from(Value::from(self.to_nm())).expect("plain values"),
            );
        }
    }
}

/// Whether a profile is marked metered: `connection.metered`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MeteredSetting {
    /// Unset (`0`): NetworkManager guesses from the link.
    #[default]
    Automatic,
    /// `1`.
    Yes,
    /// `2`.
    No,
}

impl MeteredSetting {
    /// The three a person picks from, in the order a pop-up lists them.
    pub fn choices() -> &'static [MeteredSetting] {
        &[Self::Automatic, Self::Yes, Self::No]
    }

    /// NetworkManager's value. Anything but `1` and `2` is automatic, as
    /// NetworkManager reads it.
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::Yes,
            2 => Self::No,
            _ => Self::Automatic,
        }
    }

    pub fn to_raw(self) -> i32 {
        match self {
            Self::Automatic => 0,
            Self::Yes => 1,
            Self::No => 2,
        }
    }

    /// A short name, for a pop-up's value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Yes => "yes",
            Self::No => "no",
        }
    }

    /// The setting a [`Self::name`] names, `None` for no name of these.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::choices().iter().copied().find(|m| m.name() == name)
    }

    /// Read a profile's setting; unset is automatic.
    pub fn from_settings(settings: &Settings) -> Self {
        settings
            .get("connection")
            .and_then(|c| c.get("metered"))
            .and_then(|v| i32::try_from(v).ok())
            .map(Self::from_raw)
            .unwrap_or_default()
    }

    /// Write this setting into `settings`, keeping every other property.
    pub fn write_into(self, settings: &mut Settings) {
        settings
            .entry("connection".to_string())
            .or_default()
            .insert("metered".into(), OwnedValue::from(self.to_raw()));
    }
}

/// A saved connection, read out of its settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Its object path, which the actions take.
    pub path: String,
    pub id: String,
    pub uuid: String,
    pub kind: ProfileKind,
    /// The Wi-Fi network name, for a Wi-Fi profile.
    pub ssid: Vec<u8>,
    /// A hotspot this machine serves, rather than a network to join.
    pub hotspot: bool,
    pub autoconnect: bool,
    /// Seconds since the epoch it was last used, `0` for never.
    pub last_used: u64,
    pub ipv4: IpConfig,
    pub ipv6: IpConfig,
    /// The hardware address it shows the network.
    pub mac: MacPolicy,
    /// Whether it is marked metered.
    pub metered: MeteredSetting,
}

impl Profile {
    /// Read a profile out of `settings`; `None` for one without a
    /// `connection` setting, which NetworkManager never sends.
    pub fn from_settings(path: &str, settings: &Settings) -> Option<Self> {
        let connection = settings.get("connection")?;
        let kind_raw = string(connection, "type").unwrap_or_default();
        let wireless = settings.get("802-11-wireless");
        let kind = ProfileKind::from_type(&kind_raw);
        Some(Self {
            path: path.to_string(),
            id: string(connection, "id").unwrap_or_default(),
            uuid: string(connection, "uuid").unwrap_or_default(),
            kind,
            ssid: wireless
                .and_then(|w| w.get("ssid"))
                .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default(),
            hotspot: wireless
                .and_then(|w| string(w, "mode"))
                .is_some_and(|mode| mode == "ap"),
            // Absent means the default, which is on.
            autoconnect: connection
                .get("autoconnect")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(true),
            last_used: connection
                .get("timestamp")
                .and_then(|v| u64::try_from(v).ok())
                .unwrap_or(0),
            ipv4: IpConfig::from_settings(settings, Family::V4),
            ipv6: IpConfig::from_settings(settings, Family::V6),
            mac: MacPolicy::from_settings(settings, kind),
            metered: MeteredSetting::from_settings(settings),
        })
    }
}

/// How a secured Wi-Fi network takes its password, as a request for one
/// names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// WPA/WPA2 Personal: `psk`.
    WpaPsk,
    /// WPA3 Personal: `psk`, through SAE.
    Sae,
    /// WEP: `wep-key0`.
    Wep,
}

impl KeyKind {
    /// The `key-mgmt` value.
    pub fn key_mgmt(self) -> &'static str {
        match self {
            Self::WpaPsk => "wpa-psk",
            Self::Sae => "sae",
            Self::Wep => "none",
        }
    }

    /// Whether `password` can be a key of this kind, so a dialog can keep
    /// Join dimmed until it is: 8 to 63 characters, or 64 hex digits, for
    /// WPA; 5 or 13 characters, or 10 or 26 hex digits, for WEP. SAE takes
    /// any length but none.
    pub fn accepts(self, password: &str) -> bool {
        let len = password.chars().count();
        let hex = password.chars().all(|c| c.is_ascii_hexdigit());
        match self {
            Self::WpaPsk => (8..=63).contains(&len) || (len == 64 && hex),
            Self::Sae => len > 0,
            Self::Wep => len == 5 || len == 13 || (hex && (len == 10 || len == 26)),
        }
    }

    /// The security setting for joining with `password`.
    pub fn security_setting(self, password: &str) -> HashMap<String, OwnedValue> {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).expect("plain values");
        let mut security = HashMap::new();
        security.insert("key-mgmt".to_string(), owned(Value::from(self.key_mgmt())));
        match self {
            Self::WpaPsk | Self::Sae => {
                security.insert("psk".to_string(), owned(Value::from(password)));
            }
            Self::Wep => {
                security.insert("wep-key0".to_string(), owned(Value::from(password)));
                // 1: the key itself, hex or ASCII, which is all `accepts`
                // lets through. (2 would be a passphrase hashed into one.)
                security.insert("wep-key-type".to_string(), owned(Value::from(1u32)));
            }
        }
        security
    }
}

/// A request for a Wi-Fi password, read out of what NetworkManager sends a
/// secret agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRequest {
    pub ssid: Vec<u8>,
    pub key: KeyKind,
}

impl SecretRequest {
    /// The request for `setting_name` on a connection with `settings`, when
    /// it is one a password answers: `802-11-wireless-security` with a
    /// personal or WEP key. Enterprise and VPN secrets are not.
    pub fn from_settings(setting_name: &str, settings: &Settings) -> Option<Self> {
        if setting_name != "802-11-wireless-security" {
            return None;
        }
        let security = settings.get("802-11-wireless-security")?;
        let key = match string(security, "key-mgmt")?.as_str() {
            "wpa-psk" => KeyKind::WpaPsk,
            "sae" => KeyKind::Sae,
            "none" => KeyKind::Wep,
            _ => return None,
        };
        let ssid = settings
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        Some(Self { ssid, key })
    }

    /// The reply carrying `password`.
    pub fn reply(&self, password: &str) -> Settings {
        let mut security = HashMap::new();
        let full = self.key.security_setting(password);
        for key in ["psk", "wep-key0", "wep-key-type"] {
            if let Some(value) = full.get(key) {
                security.insert(key.to_string(), value.try_clone().expect("plain values"));
            }
        }
        HashMap::from([("802-11-wireless-security".to_string(), security)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(value: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(value).unwrap()
    }

    fn manual() -> IpConfig {
        IpConfig {
            method: IpMethod::Manual,
            addresses: vec![IpAddress {
                address: "192.168.1.20".into(),
                prefix: 24,
            }],
            gateway: Some("192.168.1.1".into()),
            dns: vec!["1.1.1.1".into(), "9.9.9.9".into()],
            ignore_auto_dns: true,
            dns_search: vec!["home.arpa".into()],
        }
    }

    #[test]
    fn an_edit_round_trips_through_the_settings() {
        let mut settings = Settings::new();
        let v4 = manual();
        v4.write_into(&mut settings, Family::V4);
        assert_eq!(IpConfig::from_settings(&settings, Family::V4), v4);

        let v6 = IpConfig {
            method: IpMethod::Manual,
            addresses: vec![IpAddress {
                address: "2001:db8::5".into(),
                prefix: 64,
            }],
            gateway: Some("2001:db8::1".into()),
            dns: vec!["2606:4700:4700::1111".into()],
            ignore_auto_dns: false,
            dns_search: Vec::new(),
        };
        v6.write_into(&mut settings, Family::V6);
        assert_eq!(IpConfig::from_settings(&settings, Family::V6), v6);
        assert_eq!(IpConfig::from_settings(&settings, Family::V4), v4);
    }

    #[test]
    fn going_automatic_drops_the_static_addresses() {
        let mut settings = Settings::new();
        manual().write_into(&mut settings, Family::V4);
        let auto = IpConfig {
            method: IpMethod::Automatic,
            ..manual()
        };
        auto.write_into(&mut settings, Family::V4);
        let read = IpConfig::from_settings(&settings, Family::V4);
        assert_eq!(read.method, IpMethod::Automatic);
        assert!(read.addresses.is_empty());
        assert_eq!(read.gateway, None);
        // DNS stays: automatic addresses with chosen servers is common.
        assert_eq!(read.dns, ["1.1.1.1", "9.9.9.9"]);
    }

    #[test]
    fn properties_it_does_not_edit_are_kept() {
        let mut ip = HashMap::new();
        ip.insert("route-metric".to_string(), owned(Value::from(100i64)));
        ip.insert(
            "dns-data".to_string(),
            owned(Value::from(vec!["8.8.8.8".to_string()])),
        );
        let mut settings = Settings::from([("ipv4".to_string(), ip)]);
        assert_eq!(
            IpConfig::from_settings(&settings, Family::V4).dns,
            ["8.8.8.8"]
        );
        manual().write_into(&mut settings, Family::V4);
        assert!(settings["ipv4"].contains_key("route-metric"));
        // The connection was on dns-data, so it stays on it.
        assert!(settings["ipv4"].contains_key("dns-data"));
        assert!(!settings["ipv4"].contains_key("dns"));
    }

    #[test]
    fn dns_entries_with_a_port_a_scope_or_a_tls_name() {
        assert!(dns_address("1.1.1.1#one.one.one.one", Family::V4).is_some());
        assert!(dns_address("1.1.1.1:853#one.one.one.one", Family::V4).is_some());
        assert!(dns_address("fe80::1%eth0", Family::V6).is_some());
        assert!(dns_address("[2606:4700::1111]:853#cloudflare-dns.com", Family::V6).is_some());
        assert!(dns_address("one.one", Family::V4).is_none());
        let tls = IpConfig {
            dns: vec!["1.1.1.1#one.one.one.one".into()],
            ..manual()
        };
        assert_eq!(tls.validate(Family::V4), Ok(()));
        // Written as dns-data, since the old field cannot hold the name,
        // and read back as typed.
        let mut settings = Settings::new();
        tls.write_into(&mut settings, Family::V4);
        assert!(settings["ipv4"].contains_key("dns-data"));
        assert!(!settings["ipv4"].contains_key("dns"));
        assert_eq!(
            IpConfig::from_settings(&settings, Family::V4).dns,
            ["1.1.1.1#one.one.one.one"]
        );
    }

    #[test]
    fn a_connection_on_dns_data_stays_on_it() {
        let mut ip = HashMap::new();
        ip.insert(
            "dns-data".to_string(),
            owned(Value::from(vec!["8.8.8.8".to_string()])),
        );
        let mut settings = Settings::from([("ipv4".to_string(), ip)]);
        manual().write_into(&mut settings, Family::V4);
        assert!(settings["ipv4"].contains_key("dns-data"));
        assert!(!settings["ipv4"].contains_key("dns"));
        // A plain one on no dns-data keeps the old field every
        // NetworkManager reads.
        let mut fresh = Settings::new();
        manual().write_into(&mut fresh, Family::V4);
        assert!(fresh["ipv4"].contains_key("dns"));
        assert!(!fresh["ipv4"].contains_key("dns-data"));
    }

    #[test]
    fn the_old_address_list_is_read() {
        let raw = u32::from_ne_bytes([10, 0, 0, 7]);
        let mut ip = HashMap::new();
        ip.insert("method".to_string(), owned(Value::from("manual")));
        ip.insert(
            "addresses".to_string(),
            owned(Value::from(vec![vec![raw, 8u32, 0u32]])),
        );
        let settings = Settings::from([("ipv4".to_string(), ip)]);
        let read = IpConfig::from_settings(&settings, Family::V4);
        assert_eq!(read.addresses[0].to_string(), "10.0.0.7/8");
    }

    #[test]
    fn no_setting_reads_as_automatic() {
        let read = IpConfig::from_settings(&Settings::new(), Family::V6);
        assert_eq!(read, IpConfig::default());
    }

    #[test]
    fn addresses_as_typed() {
        assert_eq!(
            parse_address("192.168.1.9/16", Family::V4).unwrap().prefix,
            16
        );
        assert_eq!(
            parse_address(" 192.168.1.9 ", Family::V4).unwrap().prefix,
            24
        );
        assert_eq!(parse_address("fd00::2", Family::V6).unwrap().prefix, 64);
        assert!(parse_address("192.168.1.300", Family::V4).is_err());
        assert!(parse_address("192.168.1.9/33", Family::V4).is_err());
        assert!(parse_address("fd00::2", Family::V4).is_err());
        assert!(parse_address("10.0.0.1/x", Family::V4).is_err());
    }

    #[test]
    fn validation() {
        assert_eq!(manual().validate(Family::V4), Ok(()));
        let empty = IpConfig {
            addresses: Vec::new(),
            ..manual()
        };
        assert_eq!(empty.validate(Family::V4), Err(IpError::NoAddress));
        let bad_dns = IpConfig {
            dns: vec!["one.one".into()],
            ..manual()
        };
        assert_eq!(
            bad_dns.validate(Family::V4),
            Err(IpError::BadDns("one.one".into()))
        );
        let bad_gateway = IpConfig {
            gateway: Some("fe80::1".into()),
            ..manual()
        };
        assert!(matches!(
            bad_gateway.validate(Family::V4),
            Err(IpError::BadGateway(_))
        ));
        let bad_domain = IpConfig {
            dns_search: vec!["no spaces.example".into()],
            ..manual()
        };
        assert!(matches!(
            bad_domain.validate(Family::V4),
            Err(IpError::BadDomain(_))
        ));
        // A routing-only domain.
        let routing = IpConfig {
            dns_search: vec!["~corp.example".into()],
            ..manual()
        };
        assert_eq!(routing.validate(Family::V4), Ok(()));
    }

    #[test]
    fn a_router_left_over_from_manual_does_not_block_automatic() {
        let leftover = IpConfig {
            method: IpMethod::Automatic,
            addresses: Vec::new(),
            gateway: Some("not a router".into()),
            ..manual()
        };
        assert_eq!(leftover.validate(Family::V4), Ok(()));
        // Nor is it saved.
        let mut settings = HashMap::new();
        leftover.write_into(&mut settings, Family::V4);
        assert!(!settings["ipv4"].contains_key("gateway"));
        // Back on Manual it is checked again.
        let manual_again = IpConfig {
            method: IpMethod::Manual,
            ..manual()
        };
        let bad = IpConfig {
            gateway: Some("not a router".into()),
            ..manual_again
        };
        assert!(matches!(
            bad.validate(Family::V4),
            Err(IpError::BadGateway(_))
        ));
    }

    #[test]
    fn lists_split_on_commas_and_spaces() {
        assert_eq!(
            split_list("1.1.1.1, 9.9.9.9  8.8.8.8;"),
            ["1.1.1.1", "9.9.9.9", "8.8.8.8"]
        );
        assert!(split_list("  ").is_empty());
    }

    #[test]
    fn search_domains_as_networkmanager_takes_them() {
        for good in [
            "example.com",
            "example.com.",
            "~example.com",
            "~.",
            "_ldap.corp",
            "bücher.de",
        ] {
            assert!(search_domain(good), "{good}");
        }
        for bad in ["", ".", "~", "a..b", "has space.com", "a/b"] {
            assert!(!search_domain(bad), "{bad}");
        }
    }

    #[test]
    fn methods_round_trip() {
        for family in [Family::V4, Family::V6] {
            for method in IpMethod::choices(family) {
                assert_eq!(&IpMethod::from_nm(&method.to_nm(family)), method);
            }
        }
        // Read back as it was written, so an unrelated edit keeps it.
        assert_eq!(IpMethod::from_nm("ignore").to_nm(Family::V6), "ignore");
        assert_eq!(IpMethod::Disabled.to_nm(Family::V6), "disabled");
        assert_eq!(
            IpMethod::from_nm("weird").to_nm(Family::V4),
            "weird".to_string()
        );
    }

    #[test]
    fn a_profile_from_its_settings() {
        let connection = HashMap::from([
            ("id".to_string(), owned(Value::from("Home"))),
            ("uuid".to_string(), owned(Value::from("u-1"))),
            ("type".to_string(), owned(Value::from("802-11-wireless"))),
            ("autoconnect".to_string(), owned(Value::from(false))),
            ("timestamp".to_string(), owned(Value::from(77u64))),
        ]);
        let wireless = HashMap::from([("ssid".to_string(), owned(Value::from(b"Home".to_vec())))]);
        let mut settings = Settings::from([
            ("connection".to_string(), connection),
            ("802-11-wireless".to_string(), wireless),
        ]);
        manual().write_into(&mut settings, Family::V4);
        let profile = Profile::from_settings("/c/1", &settings).unwrap();
        assert_eq!(profile.kind, ProfileKind::Wifi);
        assert_eq!(profile.ssid, b"Home");
        assert!(!profile.autoconnect);
        assert_eq!(profile.last_used, 77);
        assert_eq!(profile.ipv4, manual());
        assert_eq!(profile.ipv6, IpConfig::default());
        assert!(!profile.hotspot);
    }

    fn link(kind: &str, entries: Vec<(&str, OwnedValue)>) -> Settings {
        let link = entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Settings::from([
            (
                "connection".to_string(),
                HashMap::from([("id".to_string(), owned(Value::from("Home")))]),
            ),
            (kind.to_string(), link),
        ])
    }

    #[test]
    fn the_private_address_is_read_by_its_name() {
        for (raw, policy) in [
            ("permanent", MacPolicy::Permanent),
            ("stable", MacPolicy::Stable),
            ("random", MacPolicy::Random),
            ("preserve", MacPolicy::Kept("preserve".into())),
            (
                "02:11:22:33:44:55",
                MacPolicy::Kept("02:11:22:33:44:55".into()),
            ),
        ] {
            let settings = link(
                "802-11-wireless",
                vec![("assigned-mac-address", owned(Value::from(raw)))],
            );
            assert_eq!(
                MacPolicy::from_settings(&settings, ProfileKind::Wifi),
                policy
            );
            assert_eq!(policy.to_nm(), raw);
        }
        // Nothing set, and nothing to set on a VPN.
        let none = link("802-11-wireless", Vec::new());
        assert_eq!(
            MacPolicy::from_settings(&none, ProfileKind::Wifi),
            MacPolicy::Unset
        );
        assert_eq!(
            MacPolicy::from_settings(&none, ProfileKind::Vpn),
            MacPolicy::Unset
        );
    }

    #[test]
    fn an_explicit_address_in_the_older_property_is_kept() {
        let settings = link(
            "802-3-ethernet",
            vec![(
                "cloned-mac-address",
                owned(Value::from(vec![0x02u8, 0x0a, 0, 0, 0, 0xff])),
            )],
        );
        assert_eq!(
            MacPolicy::from_settings(&settings, ProfileKind::Ethernet),
            MacPolicy::Kept("02:0A:00:00:00:FF".into())
        );
    }

    #[test]
    fn writing_the_private_address_keeps_the_rest() {
        let mut settings = link(
            "802-11-wireless",
            vec![
                ("ssid", owned(Value::from(b"Home".to_vec()))),
                (
                    "cloned-mac-address",
                    owned(Value::from(vec![2u8, 0, 0, 0, 0, 1])),
                ),
            ],
        );
        MacPolicy::Stable.write_into(&mut settings, ProfileKind::Wifi);
        let wireless = &settings["802-11-wireless"];
        assert!(wireless.contains_key("ssid"));
        // One spelling only.
        assert!(!wireless.contains_key("cloned-mac-address"));
        assert_eq!(
            MacPolicy::from_settings(&settings, ProfileKind::Wifi),
            MacPolicy::Stable
        );
        assert!(settings["connection"].contains_key("id"));

        let mut wired = link("802-3-ethernet", Vec::new());
        wired
            .get_mut("connection")
            .unwrap()
            .insert("type".into(), owned(Value::from("802-3-ethernet")));
        MacPolicy::Random.write_into(&mut wired, ProfileKind::Ethernet);
        let profile = Profile::from_settings("/c/2", &wired).unwrap();
        assert_eq!(profile.mac, MacPolicy::Random);

        MacPolicy::Unset.write_into(&mut wired, ProfileKind::Ethernet);
        assert!(!wired["802-3-ethernet"].contains_key("assigned-mac-address"));

        let mut vpn = Settings::new();
        MacPolicy::Random.write_into(&mut vpn, ProfileKind::Vpn);
        assert!(vpn.is_empty());
    }

    #[test]
    fn metered_is_read_and_written_on_the_connection() {
        let mut settings = link("802-11-wireless", Vec::new());
        assert_eq!(
            MeteredSetting::from_settings(&settings),
            MeteredSetting::Automatic
        );
        for setting in MeteredSetting::choices() {
            setting.write_into(&mut settings);
            assert_eq!(MeteredSetting::from_settings(&settings), *setting);
            assert_eq!(MeteredSetting::from_name(setting.name()), Some(*setting));
            // The rest of the connection stays.
            assert!(settings["connection"].contains_key("id"));
        }
        // NetworkManager sends an `i`.
        let raw = i32::try_from(&settings["connection"]["metered"]).unwrap();
        assert_eq!(raw, 2);
        // A value NetworkManager reads as unknown is automatic.
        settings
            .get_mut("connection")
            .unwrap()
            .insert("metered".into(), OwnedValue::from(7i32));
        assert_eq!(
            MeteredSetting::from_settings(&settings),
            MeteredSetting::Automatic
        );
        assert_eq!(MeteredSetting::from_name("sometimes"), None);
    }

    #[test]
    fn keys_a_dialog_accepts() {
        assert!(!KeyKind::WpaPsk.accepts("short"));
        assert!(KeyKind::WpaPsk.accepts("long enough"));
        assert!(KeyKind::WpaPsk.accepts(&"a".repeat(64)));
        assert!(!KeyKind::WpaPsk.accepts(&"z".repeat(64)));
        assert!(KeyKind::Sae.accepts("x"));
        assert!(KeyKind::Wep.accepts("abcde"));
        assert!(KeyKind::Wep.accepts("0123456789"));
        assert!(!KeyKind::Wep.accepts("abcdef"));
    }

    #[test]
    fn a_secret_request_and_its_reply() {
        let security = HashMap::from([("key-mgmt".to_string(), owned(Value::from("wpa-psk")))]);
        let wireless = HashMap::from([("ssid".to_string(), owned(Value::from(b"Cafe".to_vec())))]);
        let settings = Settings::from([
            ("802-11-wireless-security".to_string(), security),
            ("802-11-wireless".to_string(), wireless),
        ]);
        let request = SecretRequest::from_settings("802-11-wireless-security", &settings).unwrap();
        assert_eq!(request.ssid, b"Cafe");
        assert_eq!(request.key, KeyKind::WpaPsk);
        let reply = request.reply("hunter22");
        let psk = String::try_from(
            reply["802-11-wireless-security"]["psk"]
                .try_clone()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(psk, "hunter22");
        // Only the secret goes back, not the key management.
        assert!(!reply["802-11-wireless-security"].contains_key("key-mgmt"));

        assert!(SecretRequest::from_settings("vpn", &settings).is_none());
        let enterprise = Settings::from([(
            "802-11-wireless-security".to_string(),
            HashMap::from([("key-mgmt".to_string(), owned(Value::from("wpa-eap")))]),
        )]);
        assert!(SecretRequest::from_settings("802-11-wireless-security", &enterprise).is_none());
    }
}
