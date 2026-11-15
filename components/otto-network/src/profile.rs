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
            // IPv6's "ignore" leaves the family to the kernel; to a person
            // it reads as off, as "disabled" does.
            "disabled" | "ignore" => Self::Disabled,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn to_nm(&self, family: Family) -> String {
        match self {
            Self::Automatic => "auto",
            Self::DhcpOnly => "dhcp",
            Self::Manual => "manual",
            Self::LinkLocal => "link-local",
            Self::Shared => "shared",
            Self::Disabled => match family {
                Family::V4 => "disabled",
                // "disabled" needs NetworkManager 1.20; "ignore" works
                // everywhere and is what the setting has always meant.
                Family::V6 => "ignore",
            },
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
        if let Some(gateway) = &self.gateway {
            parse_ip(gateway, family).ok_or_else(|| IpError::BadGateway(gateway.clone()))?;
        }
        for server in &self.dns {
            dns_address(server, family).ok_or_else(|| IpError::BadDns(server.clone()))?;
        }
        for domain in &self.dns_search {
            let fine = domain.trim_start_matches('~').split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
            if !fine {
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
}

impl Profile {
    /// Read a profile out of `settings`; `None` for one without a
    /// `connection` setting, which NetworkManager never sends.
    pub fn from_settings(path: &str, settings: &Settings) -> Option<Self> {
        let connection = settings.get("connection")?;
        let kind_raw = string(connection, "type").unwrap_or_default();
        let wireless = settings.get("802-11-wireless");
        Some(Self {
            path: path.to_string(),
            id: string(connection, "id").unwrap_or_default(),
            uuid: string(connection, "uuid").unwrap_or_default(),
            kind: ProfileKind::from_type(&kind_raw),
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
    fn lists_split_on_commas_and_spaces() {
        assert_eq!(
            split_list("1.1.1.1, 9.9.9.9  8.8.8.8;"),
            ["1.1.1.1", "9.9.9.9", "8.8.8.8"]
        );
        assert!(split_list("  ").is_empty());
    }

    #[test]
    fn methods_round_trip() {
        for family in [Family::V4, Family::V6] {
            for method in IpMethod::choices(family) {
                assert_eq!(&IpMethod::from_nm(&method.to_nm(family)), method);
            }
        }
        assert_eq!(IpMethod::from_nm("ignore"), IpMethod::Disabled);
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
