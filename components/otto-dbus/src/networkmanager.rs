//! `org.freedesktop.NetworkManager`: the network connections NetworkManager
//! manages.
//!
//! Served by NetworkManager on the system bus. Only what Otto reads and does
//! is declared: the manager, a device and its Wi-Fi and wired halves, an
//! access point, the saved connections, an active connection and its address
//! configuration. Every object but the manager and the settings lives at a
//! path NetworkManager hands out, so those proxies have no default path and
//! are built with `builder(..).path(..)`. The reference is NetworkManager's
//! D-Bus API documentation (`man nm-dbus-api`, or networkmanager.dev).

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

/// The well-known bus name.
pub const SERVICE: &str = "org.freedesktop.NetworkManager";
/// The manager's object path.
pub const PATH: &str = "/org/freedesktop/NetworkManager";
/// The saved connections' object path.
pub const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";

/// A connection's settings as NetworkManager passes them: setting name
/// (`"connection"`, `"802-11-wireless"`, …) to its properties.
pub type ConnectionSettings = HashMap<String, HashMap<String, OwnedValue>>;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    /// The realized network devices.
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Activate a saved `connection` on `device` (`/` lets NetworkManager
    /// pick one, as for a VPN). `specific_object` is an access point for
    /// Wi-Fi, or `/`. Returns the active connection.
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Save a new connection from `connection`, completed from `device` and
    /// `specific_object` where it leaves things out, and activate it.
    /// Returns the saved connection and the active connection.
    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    /// Bring an active connection down.
    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;

    /// Whether the Wi-Fi radios are on. Writable.
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;

    /// Whether a hardware switch lets the Wi-Fi radios on at all.
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> zbus::Result<bool>;

    /// The active connections.
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// The active connection that holds the default route, or `/`.
    #[zbus(property)]
    fn primary_connection(&self) -> zbus::Result<OwnedObjectPath>;

    /// `NMConnectivityState`: `0` unknown, `1` none, `2` captive portal,
    /// `3` limited, `4` full.
    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;

    /// `NMState`, the overall state.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    /// Bring the device's connection down and keep it from auto-connecting
    /// until something activates a connection on it again.
    fn disconnect(&self) -> zbus::Result<()>;

    /// The kernel interface name ("wlan0").
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;

    /// `NMDeviceType`: `1` Ethernet, `2` Wi-Fi, and many more.
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;

    /// `NMDeviceState`: `20` unavailable, `30` disconnected, `40`–`90`
    /// activating, `100` activated, `110` deactivating, `120` failed.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Whether NetworkManager manages the device at all.
    #[zbus(property)]
    fn managed(&self) -> zbus::Result<bool>;

    /// The device's active connection, or `/`.
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;

    /// The hardware address.
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait DeviceWireless {
    /// Ask for a scan. NetworkManager refuses one asked for too soon after
    /// the last; the refusal is not worth reporting.
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    /// The access points in range.
    #[zbus(property)]
    fn access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// The access point the device is associated with, or `/`.
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;

    /// `CLOCK_BOOTTIME` milliseconds of the last scan, `-1` for never.
    #[zbus(property)]
    fn last_scan(&self) -> zbus::Result<i64>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait AccessPoint {
    /// The network name, as bytes: it need not be UTF-8.
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;

    /// Signal quality, `0..=100`.
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;

    /// `NM80211ApFlags`: `0x1` privacy (WEP or better).
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;

    /// `NM80211ApSecurityFlags` for WPA.
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;

    /// `NM80211ApSecurityFlags` for WPA2/RSN, WPA3 included.
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;

    /// The radio frequency in MHz.
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;

    /// The BSSID.
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait NetworkSettings {
    /// Every saved connection.
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait SettingsConnection {
    /// The connection's settings, secrets left out.
    fn get_settings(&self) -> zbus::Result<ConnectionSettings>;

    /// Replace the connection's settings. Settings with no secrets in them
    /// keep the secrets already stored.
    fn update(&self, properties: ConnectionSettings) -> zbus::Result<()>;

    /// Forget the connection.
    fn delete(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait ActiveConnection {
    /// The saved connection this one was activated from.
    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;

    /// The connection's name.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    /// The connection's type ("802-11-wireless", "802-3-ethernet", "vpn",
    /// "wireguard").
    #[zbus(property, name = "Type")]
    fn connection_type(&self) -> zbus::Result<String>;

    /// `NMActiveConnectionState`: `1` activating, `2` activated, `3`
    /// deactivating, `4` deactivated.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// The devices it runs on.
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// The IPv4 configuration, or `/`.
    #[zbus(property)]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;

    /// The IPv6 configuration, or `/`.
    #[zbus(property)]
    fn ip6_config(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Ip4Config {
    /// Each address as `{"address": s, "prefix": u}`.
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// The default gateway, empty for none.
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;

    /// Each name server as `{"address": s}`.
    #[zbus(property)]
    fn nameserver_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP6Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Ip6Config {
    /// Each address as `{"address": s, "prefix": u}`.
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}
