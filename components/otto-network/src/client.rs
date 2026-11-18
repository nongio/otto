//! Reading NetworkManager into a [`Snapshot`], and asking it to change.
//!
//! Every function takes a system bus connection and runs on whatever tokio
//! runtime the caller has; [`crate::blocking`] wraps them for a plain thread.
//! The proxies are built without a property cache: a cache subscribes to
//! every object's changes, and a snapshot is read once and thrown away, so
//! the subscriptions would only pile up. [`watch`] follows changes instead.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::future::join_all;
use futures_util::StreamExt;
use otto_dbus::networkmanager::{
    AccessPointProxy, ActiveConnectionProxy, DeviceProxy, DeviceWiredProxy, DeviceWirelessProxy,
    Ip4ConfigProxy, Ip6ConfigProxy, NetworkManagerProxy, NetworkSettingsProxy,
    SettingsConnectionProxy, PATH, SERVICE,
};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::model::*;
use crate::profile::{
    Family, IpConfig, KeyKind, MacPolicy, MeteredSetting, Profile, ProfileKind, WifiShare,
};
use crate::reveal::ShareError;
use crate::rfkill::Radios;

/// `NMDeviceType` values Otto shows.
const TYPE_ETHERNET: u32 = 1;
const TYPE_WIFI: u32 = 2;

/// How long a burst of changes is gathered before the watcher reports it.
/// Signal strengths move on every scan, many objects at once.
const SETTLE: Duration = Duration::from_millis(150);

/// How long [`follow`] waits before reading again after a read failed, at
/// first. Each failure in a row doubles it, up to [`RETRY_MAX`]; a change
/// NetworkManager signals reads again at once.
const RETRY_FIRST: Duration = Duration::from_secs(1);
/// The longest [`follow`] waits between two failed reads.
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Held across the read and the write of a profile's settings. NetworkManager
/// takes a profile's settings whole (`Update`), so two edits made at once
/// from this process, each reading the profile before the other wrote it,
/// would leave only the last one. One lock for every profile: edits are a
/// person's clicks, never many at once.
static PROFILE_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Build a proxy of type `P` for the object at `path`, uncached.
macro_rules! at {
    ($proxy:ident, $conn:expr, $path:expr) => {
        async {
            $proxy::builder($conn)
                .path($path)?
                .cache_properties(CacheProperties::No)
                .build()
                .await
        }
    };
}

/// Whether NetworkManager is on the bus.
pub(crate) async fn is_running(conn: &Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(SERVICE) else {
        return false;
    };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

/// `/` is NetworkManager's "none".
fn some_path(path: OwnedObjectPath) -> Option<String> {
    let path = path.as_str();
    (path != "/").then(|| path.to_string())
}

/// Read everything a network UI shows. A NetworkManager that is not running
/// is an empty snapshot with `running` false, not an error. The radios'
/// kill switches are the machine's ([`Radios::system`]).
///
/// # Errors
///
/// A bus error from NetworkManager while it is running.
pub async fn read(conn: &Connection) -> zbus::Result<Snapshot> {
    read_with(conn, &Radios::system()).await
}

/// [`read`], with the kill switches read from `radios`.
///
/// # Errors
///
/// A bus error from NetworkManager while it is running.
pub async fn read_with(conn: &Connection, radios: &Radios) -> zbus::Result<Snapshot> {
    read_cached(conn, radios, None).await
}

/// [`read_with`], with the saved connections from an earlier read when none
/// of them changed since: a `GetSettings` per profile is the dearest part.
async fn read_cached(
    conn: &Connection,
    radios: &Radios,
    profiles: Option<Vec<Profile>>,
) -> zbus::Result<Snapshot> {
    if !is_running(conn).await {
        return Ok(Snapshot::default());
    }
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;

    let mut snapshot = Snapshot {
        running: true,
        wifi_enabled: nm.wireless_enabled().await?,
        wifi_hardware_enabled: nm.wireless_hardware_enabled().await.unwrap_or(true),
        wifi_block: radios.wifi_block().unwrap_or_default(),
        connectivity: Connectivity::from_raw(nm.connectivity().await.unwrap_or(0)),
        ..Snapshot::default()
    };

    // Devices, with the Wi-Fi device's active connection kept aside to tell
    // which network is in use. Of several Wi-Fi devices, the first with a
    // connection is the one shown, or the first when none has one.
    let mut wifi_device: Option<(Device, Option<String>)> = None;
    for path in nm.get_devices().await? {
        let Some((device, active)) = read_device(conn, path.as_str()).await else {
            continue;
        };
        let better = match &wifi_device {
            None => true,
            Some((_, shown)) => shown.is_none() && active.is_some(),
        };
        if device.kind == DeviceKind::Wifi && better {
            wifi_device = Some((device.clone(), active));
        }
        snapshot.devices.push(device);
    }

    // Saved connections.
    let profiles = match profiles {
        Some(profiles) => profiles,
        None => read_profiles(conn).await?,
    };
    let mut saved: Vec<SavedNetwork> = profiles
        .iter()
        .filter(|p| p.kind == ProfileKind::Wifi && !p.hotspot)
        .map(|p| SavedNetwork {
            path: p.path.clone(),
            id: p.id.clone(),
            ssid: p.ssid.clone(),
            autoconnect: p.autoconnect,
            last_used: p.last_used,
            in_use: false,
        })
        .collect();
    let mut vpns: Vec<Vpn> = profiles
        .iter()
        .filter(|p| p.kind == ProfileKind::Vpn)
        .map(|p| Vpn {
            path: p.path.clone(),
            id: p.id.clone(),
            active: None,
            state: LinkState::Idle,
        })
        .collect();
    vpns.sort_by_key(|v| v.id.to_lowercase());

    // Active connections, by the saved connection they came from.
    let mut active_by_connection: HashMap<String, (String, LinkState)> = HashMap::new();
    for path in nm.active_connections().await.unwrap_or_default() {
        let Ok(active) = at!(ActiveConnectionProxy, conn, path.as_str()).await else {
            continue;
        };
        let (Ok(connection), Ok(state)) = (active.connection().await, active.state().await) else {
            continue;
        };
        active_by_connection.insert(
            connection.as_str().to_string(),
            (path.as_str().to_string(), link_state(state)),
        );
    }

    snapshot.vpns = vpns
        .into_iter()
        .map(|mut vpn| {
            if let Some((active, state)) = active_by_connection.get(&vpn.path) {
                vpn.active = Some(active.clone());
                vpn.state = *state;
            }
            vpn
        })
        .collect();

    // What each device's active connection was activated from.
    for device in &mut snapshot.devices {
        device.active_profile = active_by_connection
            .iter()
            .find(|(_, (active, _))| Some(active.as_str()) == device.active.as_deref())
            .map(|(profile, _)| profile.clone());
    }

    if let Some((device, active)) = wifi_device {
        // The saved connection on the Wi-Fi device: which name is in use.
        let mut in_use: Option<(Vec<u8>, LinkState)> = None;
        if let Some(active) = active {
            if let Ok(proxy) = at!(ActiveConnectionProxy, conn, active.as_str()).await {
                if let (Ok(connection), Ok(state)) = (proxy.connection().await, proxy.state().await)
                {
                    if let Some(saved) = saved.iter_mut().find(|s| s.path == connection.as_str()) {
                        saved.in_use = true;
                        in_use = Some((saved.ssid.clone(), link_state(state)));
                    }
                }
            }
        }

        let wireless = at!(DeviceWirelessProxy, conn, device.path.as_str()).await?;
        let mut access_points = read_access_points(conn, &wireless).await;
        let last_scan_ms = wireless.last_scan().await.ok().filter(|ms| *ms >= 0);
        let active_ap_path = wireless
            .active_access_point()
            .await
            .ok()
            .and_then(some_path);
        name_hidden_access_point(
            &mut access_points,
            active_ap_path.as_deref(),
            in_use.as_ref().map(|(ssid, _)| ssid.as_slice()),
        );
        let active_ap = active_ap_path
            .and_then(|path| access_points.iter().find(|ap| ap.path == path).cloned());
        let bitrate_kbps = wireless
            .bitrate()
            .await
            .ok()
            .filter(|rate| *rate > 0 && active_ap.is_some());
        let networks = group_networks(
            &access_points,
            &saved,
            in_use
                .as_ref()
                .map(|(ssid, state)| (ssid.as_slice(), *state)),
        );
        let device = snapshot
            .devices
            .iter()
            .find(|d| d.path == device.path)
            .cloned()
            .unwrap_or(device);
        snapshot.wifi = Some(WifiDevice {
            device,
            networks,
            last_scan_ms,
            active_ap,
            access_points,
            in_use,
            bitrate_kbps,
        });
    }

    sort_saved(&mut saved);
    snapshot.saved = saved;
    snapshot.profiles = profiles;

    if let Some(primary) = some_path(nm.primary_connection().await.unwrap_or_default()) {
        snapshot.primary = read_link(conn, &primary).await;
    }
    // Every device's own addresses, for a page that shows a device that is
    // not the primary one.
    for device in &mut snapshot.devices {
        if let Some(active) = device.active.clone() {
            device.link = read_link(conn, &active).await;
        }
    }
    if let Some(wifi) = &mut snapshot.wifi {
        if let Some(device) = snapshot.devices.iter().find(|d| d.path == wifi.device.path) {
            wifi.device = device.clone();
        }
    }

    Ok(snapshot)
}

fn link_state(raw: u32) -> LinkState {
    match raw {
        1 => LinkState::Connecting,
        2 => LinkState::Connected,
        _ => LinkState::Idle,
    }
}

/// A managed Wi-Fi or Ethernet device, and its active connection.
async fn read_device(conn: &Connection, path: &str) -> Option<(Device, Option<String>)> {
    let proxy = at!(DeviceProxy, conn, path).await.ok()?;
    let kind = match proxy.device_type().await.ok()? {
        TYPE_ETHERNET => DeviceKind::Ethernet,
        TYPE_WIFI => DeviceKind::Wifi,
        _ => return None,
    };
    if !proxy.managed().await.unwrap_or(false) {
        return None;
    }
    let speed_mbps = match kind {
        DeviceKind::Ethernet => match at!(DeviceWiredProxy, conn, path).await {
            Ok(wired) => wired.speed().await.ok().filter(|s| *s > 0),
            Err(_) => None,
        },
        DeviceKind::Wifi => None,
    };
    let active = proxy.active_connection().await.ok().and_then(some_path);
    let device = Device {
        path: path.to_string(),
        kind,
        interface: proxy.interface().await.unwrap_or_default(),
        state: DeviceState::from_raw(proxy.state().await.unwrap_or(0)),
        hw_address: proxy.hw_address().await.unwrap_or_default(),
        active_profile: None,
        speed_mbps,
        active: active.clone(),
        link: None,
        metered: Metered::from_raw(proxy.metered().await.unwrap_or(0)),
    };
    Some((device, active))
}

/// Every access point the device sees, read side by side: there can be
/// dozens, and one at a time is a round trip each.
async fn read_access_points(
    conn: &Connection,
    wireless: &DeviceWirelessProxy<'_>,
) -> Vec<AccessPoint> {
    let paths = wireless.access_points().await.unwrap_or_default();
    let reads = paths.iter().map(|path| async move {
        let proxy = at!(AccessPointProxy, conn, path.as_str()).await.ok()?;
        let flags = proxy.flags().await.unwrap_or(0);
        let wpa = proxy.wpa_flags().await.unwrap_or(0);
        let rsn = proxy.rsn_flags().await.unwrap_or(0);
        Some(AccessPoint {
            path: path.as_str().to_string(),
            ssid: proxy.ssid().await.ok()?,
            strength: proxy.strength().await.unwrap_or(0),
            security: Security::from_flags(flags, wpa, rsn),
            wpa3_only: Security::wpa3_only(wpa, rsn),
            frequency_mhz: proxy.frequency().await.unwrap_or(0),
            bssid: proxy.hw_address().await.unwrap_or_default(),
        })
    });
    join_all(reads).await.into_iter().flatten().collect()
}

fn string_in(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    settings
        .get(key)
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

/// Every saved connection, with its settings read into a [`Profile`].
async fn read_profiles(conn: &Connection) -> zbus::Result<Vec<Profile>> {
    let settings = NetworkSettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let paths = settings.list_connections().await?;
    let reads = paths.iter().map(|path| async move {
        let proxy = at!(SettingsConnectionProxy, conn, path.as_str())
            .await
            .ok()?;
        let settings = proxy.get_settings().await.ok()?;
        Profile::from_settings(path.as_str(), &settings)
    });
    Ok(join_all(reads).await.into_iter().flatten().collect())
}

/// An address list as `address/prefix`.
fn addresses(data: Vec<HashMap<String, OwnedValue>>) -> Vec<String> {
    data.into_iter()
        .filter_map(|entry| {
            let address = string_in(&entry, "address")?;
            let prefix = entry.get("prefix").and_then(|v| u32::try_from(v).ok());
            Some(match prefix {
                Some(prefix) => format!("{address}/{prefix}"),
                None => address,
            })
        })
        .collect()
}

/// The primary connection and its addresses.
async fn read_link(conn: &Connection, path: &str) -> Option<ActiveLink> {
    let active = at!(ActiveConnectionProxy, conn, path).await.ok()?;
    let mut link = ActiveLink {
        path: path.to_string(),
        id: active.id().await.unwrap_or_default(),
        kind: LinkKind::from_type(&active.connection_type().await.unwrap_or_default()),
        state: link_state(active.state().await.unwrap_or(0)),
        interface: String::new(),
        hw_address: String::new(),
        ipv4: Vec::new(),
        gateway: None,
        dns: Vec::new(),
        ipv6: Vec::new(),
        gateway6: None,
    };
    if let Some(device) = active
        .devices()
        .await
        .ok()
        .and_then(|d| d.into_iter().next())
    {
        if let Ok(proxy) = at!(DeviceProxy, conn, device.as_str()).await {
            link.interface = proxy.interface().await.unwrap_or_default();
            link.hw_address = proxy.hw_address().await.unwrap_or_default();
        }
    }
    if let Some(ip4) = active.ip4_config().await.ok().and_then(some_path) {
        if let Ok(proxy) = at!(Ip4ConfigProxy, conn, ip4.as_str()).await {
            link.ipv4 = addresses(proxy.address_data().await.unwrap_or_default());
            link.gateway = proxy.gateway().await.ok().filter(|g| !g.is_empty());
            link.dns = proxy
                .nameserver_data()
                .await
                .unwrap_or_default()
                .into_iter()
                .filter_map(|entry| string_in(&entry, "address"))
                .collect();
        }
    }
    if let Some(ip6) = active.ip6_config().await.ok().and_then(some_path) {
        if let Ok(proxy) = at!(Ip6ConfigProxy, conn, ip6.as_str()).await {
            link.ipv6 = addresses(proxy.address_data().await.unwrap_or_default());
        }
        link.gateway6 = ip6_gateway(conn, &ip6).await;
    }
    Some(link)
}

/// The IPv6 router of the `IP6Config` object at `path`, through the generic
/// properties interface: the `otto-dbus` proxy does not list `Gateway`.
async fn ip6_gateway(conn: &Connection, path: &str) -> Option<String> {
    let properties = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(SERVICE)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;
    let interface =
        zbus::names::InterfaceName::try_from("org.freedesktop.NetworkManager.IP6Config").ok()?;
    let value = properties.get(interface, "Gateway").await.ok()?;
    String::try_from(value).ok().filter(|g| !g.is_empty())
}

fn object_path(path: &str) -> zbus::Result<ObjectPath<'_>> {
    Ok(ObjectPath::try_from(path)?)
}

/// Turn the Wi-Fi radios on or off. Turning them on also lifts a soft block
/// (the airplane key, `rfkill block wifi`): the person asked for Wi-Fi, and
/// NetworkManager's switch alone does not lift a block set elsewhere. A
/// hard block stays; only its switch lifts that. The kill switches are the
/// machine's ([`Radios::system`]).
///
/// # Errors
///
/// NetworkManager refusing the switch, or not being on the bus.
pub async fn set_wifi_enabled(conn: &Connection, enabled: bool) -> zbus::Result<()> {
    set_wifi_enabled_with(conn, &Radios::system(), enabled).await
}

/// [`set_wifi_enabled`], lifting a soft block on `radios`.
///
/// # Errors
///
/// NetworkManager refusing the switch, or not being on the bus.
pub async fn set_wifi_enabled_with(
    conn: &Connection,
    radios: &Radios,
    enabled: bool,
) -> zbus::Result<()> {
    if enabled && radios.wifi_block().is_some_and(|block| block.soft) {
        // Without write access to the device (no logind seat), the switch
        // below is all there is, and NetworkManager may lift the block
        // itself.
        let radios = radios.clone();
        let _ = tokio::task::spawn_blocking(move || radios.unblock_wifi()).await;
    }
    NetworkManagerProxy::new(conn)
        .await?
        .set_wireless_enabled(enabled)
        .await
}

/// Join `network` on the Wi-Fi device at `device`: its saved connection when
/// it has one, otherwise a new one NetworkManager completes from the access
/// point. A secured network's password comes from whichever secret agent is
/// registered.
pub async fn connect_wifi(
    conn: &Connection,
    device: &str,
    network: &WifiNetwork,
) -> zbus::Result<()> {
    let nm = NetworkManagerProxy::new(conn).await?;
    let device = object_path(device)?;
    let access_point = object_path(&network.access_point)?;
    match &network.saved {
        Some(saved) => {
            nm.activate_connection(&object_path(saved)?, &device, &access_point)
                .await?;
        }
        None => {
            let mut wireless = HashMap::new();
            wireless.insert("ssid", Value::from(network.ssid.clone()));
            let mut settings = HashMap::new();
            settings.insert("802-11-wireless", wireless);
            nm.add_and_activate_connection(settings, &device, &access_point)
                .await?;
        }
    }
    Ok(())
}

/// Join the secured `network` with `password` typed for it: a new saved
/// connection carrying the key, so NetworkManager keeps it and asks no
/// agent. For a network with a saved connection, or one with no key a
/// person types, this is [`connect_wifi`].
pub async fn connect_wifi_with_password(
    conn: &Connection,
    device: &str,
    network: &WifiNetwork,
    password: &str,
) -> zbus::Result<()> {
    let Some(key) = network.key_kind().filter(|_| network.saved.is_none()) else {
        return connect_wifi(conn, device, network).await;
    };
    let nm = NetworkManagerProxy::new(conn).await?;
    let security = key.security_setting(password);
    let mut wireless = HashMap::new();
    wireless.insert("ssid", Value::from(network.ssid.clone()));
    let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
    settings.insert("802-11-wireless", wireless);
    settings.insert(
        "802-11-wireless-security",
        security
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str(),
                    Value::from(v.try_clone().expect("plain values")),
                )
            })
            .collect(),
    );
    nm.add_and_activate_connection(
        settings,
        &object_path(device)?,
        &object_path(&network.access_point)?,
    )
    .await?;
    Ok(())
}

/// Join a network that does not broadcast its name. `key` is `None` for an
/// open one; `password` is ignored then.
pub async fn connect_hidden(
    conn: &Connection,
    device: &str,
    ssid: &[u8],
    key: Option<KeyKind>,
    password: &str,
) -> zbus::Result<()> {
    let nm = NetworkManagerProxy::new(conn).await?;
    let mut wireless = HashMap::new();
    wireless.insert("ssid", Value::from(ssid.to_vec()));
    wireless.insert("hidden", Value::from(true));
    let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
    settings.insert("802-11-wireless", wireless);
    let security = key.map(|key| key.security_setting(password));
    if let Some(security) = &security {
        settings.insert(
            "802-11-wireless-security",
            security
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str(),
                        Value::from(v.try_clone().expect("plain values")),
                    )
                })
                .collect(),
        );
    }
    nm.add_and_activate_connection(settings, &object_path(device)?, &object_path("/")?)
        .await?;
    Ok(())
}

/// Save `v4` and `v6` as the IP configuration of the profile at `profile`
/// in one update, then apply it to the connection if it is up: in place
/// where NetworkManager can (`Reapply`), by reconnecting where it cannot.
///
/// Refused before anything is sent when either does not validate, so a
/// half-typed address never reaches the profile, and one family is never
/// saved without the other.
pub async fn set_ip_config(
    conn: &Connection,
    profile: &str,
    v4: &IpConfig,
    v6: &IpConfig,
) -> zbus::Result<()> {
    for (family, config) in [(Family::V4, v4), (Family::V6, v6)] {
        config
            .validate(family)
            .map_err(|e| zbus::Error::Failure(format!("{e:?}")))?;
    }
    edit_profile(conn, profile, |settings| {
        v4.write_into(settings, Family::V4);
        v6.write_into(settings, Family::V6);
    })
    .await?;
    reapply(conn, profile, Reapply::OrReconnect).await
}

/// How far [`reapply`] goes to apply a saved profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reapply {
    /// In place where NetworkManager can, by reconnecting where it cannot.
    OrReconnect,
    /// In place only; what cannot be applied waits for the next connect.
    InPlace,
}

/// Apply a saved profile's settings to the device it is active on, if any.
async fn reapply(conn: &Connection, profile: &str, how: Reapply) -> zbus::Result<()> {
    let nm = NetworkManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    for active in nm.active_connections().await.unwrap_or_default() {
        let Ok(proxy) = at!(ActiveConnectionProxy, conn, active.as_str()).await else {
            continue;
        };
        if proxy.connection().await.ok().as_ref().map(|c| c.as_str()) != Some(profile) {
            continue;
        }
        let devices = proxy.devices().await.unwrap_or_default();
        let Some(device) = devices.first() else {
            if how == Reapply::InPlace {
                return Ok(());
            }
            // A VPN runs on no device of its own: start it again.
            nm.deactivate_connection(&active).await?;
            nm.activate_connection(
                &object_path(profile)?,
                &object_path("/")?,
                &object_path("/")?,
            )
            .await?;
            return Ok(());
        };
        let device_proxy = at!(DeviceProxy, conn, device.as_str()).await?;
        if device_proxy.reapply(HashMap::new(), 0, 0).await.is_err() && how == Reapply::OrReconnect
        {
            // Some changes cannot be applied in place (NetworkManager says
            // which); reconnecting applies anything.
            nm.activate_connection(&object_path(profile)?, device, &object_path("/")?)
                .await?;
        }
        return Ok(());
    }
    Ok(())
}

/// Bring down whatever the device at `device` is connected to.
pub async fn disconnect_device(conn: &Connection, device: &str) -> zbus::Result<()> {
    at!(DeviceProxy, conn, device).await?.disconnect().await
}

/// Ask the Wi-Fi device at `device` to scan. A refusal for scanning too soon
/// is not reported.
pub async fn request_scan(conn: &Connection, device: &str) -> zbus::Result<()> {
    let wireless = at!(DeviceWirelessProxy, conn, device).await?;
    match wireless.request_scan(HashMap::new()).await {
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with("NotAllowed") => {
            Ok(())
        }
        other => other,
    }
}

/// Start the saved connection at `connection` (a VPN) wherever
/// NetworkManager chooses.
pub async fn activate(conn: &Connection, connection: &str) -> zbus::Result<()> {
    let none = object_path("/")?;
    NetworkManagerProxy::new(conn)
        .await?
        .activate_connection(&object_path(connection)?, &none, &none)
        .await?;
    Ok(())
}

/// Stop the active connection at `active`.
pub async fn deactivate(conn: &Connection, active: &str) -> zbus::Result<()> {
    NetworkManagerProxy::new(conn)
        .await?
        .deactivate_connection(&object_path(active)?)
        .await
}

/// Forget the saved connection at `connection`.
pub async fn forget(conn: &Connection, connection: &str) -> zbus::Result<()> {
    at!(SettingsConnectionProxy, conn, connection)
        .await?
        .delete()
        .await
}

/// Read the saved connection at `connection` fresh, change it with `edit`
/// and save it, under [`PROFILE_WRITES`]. The settings come without secrets,
/// and an update without secrets keeps the ones already stored.
async fn edit_profile(
    conn: &Connection,
    connection: &str,
    edit: impl FnOnce(&mut crate::profile::Settings),
) -> zbus::Result<()> {
    let proxy = at!(SettingsConnectionProxy, conn, connection).await?;
    let _writing = PROFILE_WRITES.lock().await;
    let mut settings = proxy.get_settings().await?;
    edit(&mut settings);
    proxy.update(settings).await
}

/// Let NetworkManager join the saved connection at `connection` by itself,
/// or not.
pub async fn set_autoconnect(conn: &Connection, connection: &str, on: bool) -> zbus::Result<()> {
    edit_profile(conn, connection, |settings| {
        settings
            .entry("connection".to_string())
            .or_default()
            .insert("autoconnect".to_string(), OwnedValue::from(on));
    })
    .await
}

/// Mark the Wi-Fi or Ethernet connection at `connection` metered, not
/// metered, or leave it to NetworkManager to guess, and apply that to the
/// connection in place if it is up. Where NetworkManager cannot apply it in
/// place, it takes effect on the next connect: being metered is no reason to
/// drop a link.
pub async fn set_metered(
    conn: &Connection,
    connection: &str,
    metered: MeteredSetting,
) -> zbus::Result<()> {
    edit_profile(conn, connection, |settings| metered.write_into(settings)).await?;
    reapply(conn, connection, Reapply::InPlace).await
}

/// The name, security and password of the saved Wi-Fi network at
/// `connection`, for showing or sharing it.
///
/// The password comes from `GetSecrets`, asked only where NetworkManager
/// stores one: a password an agent keeps, or one asked for on every
/// connect, is `None` with no call. Before it, polkit confirms it's the
/// person through [`crate::reveal::ACTION`], on the same bus as `conn`; the
/// session's authentication agent asks for their password, and this waits
/// for the answer. NetworkManager then authorises `GetSecrets` itself and
/// may still refuse.
///
/// # Errors
///
/// [`ShareError::NotConfirmed`] or [`ShareError::Unavailable`] when polkit
/// did not authorize the read, and [`ShareError::Bus`] for a connection that
/// is not Wi-Fi, NetworkManager's refusal (`PermissionDenied`), or any other
/// bus error.
pub async fn wifi_share(conn: &Connection, connection: &str) -> Result<WifiShare, ShareError> {
    let proxy = at!(SettingsConnectionProxy, conn, connection).await?;
    let settings = proxy.get_settings().await?;
    if !settings.contains_key("802-11-wireless") {
        return Err(zbus::Error::Failure("not a Wi-Fi connection".into()).into());
    }
    let secrets = if WifiShare::stores_password(&settings) {
        crate::reveal::confirm(conn).await?;
        proxy.get_secrets("802-11-wireless-security").await?
    } else {
        HashMap::new()
    };
    Ok(WifiShare::from_settings(&settings, &secrets))
}

/// Save `policy` as the hardware address the Wi-Fi or Ethernet connection
/// at `connection` shows its network. It takes effect the next time the
/// connection comes up: NetworkManager cannot change the address of a link
/// in use without taking it down.
pub async fn set_mac_policy(
    conn: &Connection,
    connection: &str,
    policy: &MacPolicy,
) -> zbus::Result<()> {
    edit_profile(conn, connection, |settings| {
        let kind = settings
            .get("connection")
            .and_then(|c| c.get("type"))
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .map(|kind| ProfileKind::from_type(&kind))
            .unwrap_or(ProfileKind::Other);
        policy.write_into(settings, kind);
    })
    .await
}

/// What one of NetworkManager's signals says changed.
fn classify(message: &zbus::Message) -> Changes {
    let header = message.header();
    let path = header.path().map(|p| p.as_str()).unwrap_or_default();
    let member = header.member().map(|m| m.as_str()).unwrap_or_default();
    let interface = header.interface().map(|i| i.as_str()).unwrap_or_default();

    if path.starts_with("/org/freedesktop/NetworkManager/Settings") {
        return Changes {
            profiles: true,
            ..Changes::default()
        };
    }
    if member == "PropertiesChanged"
        && path.starts_with("/org/freedesktop/NetworkManager/AccessPoint/")
    {
        // The standard signal names the interface first; NetworkManager's
        // own, older one carries only the properties.
        let changed: Option<HashMap<String, OwnedValue>> =
            if interface == "org.freedesktop.DBus.Properties" {
                message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    .ok()
                    .map(|(_, changed, _)| changed)
            } else {
                message
                    .body()
                    .deserialize::<(HashMap<String, OwnedValue>,)>()
                    .ok()
                    .map(|(changed,)| changed)
            };
        if let Some(changed) = changed {
            let only_signal = changed
                .keys()
                .all(|key| key == "Strength" || key == "LastSeen");
            if only_signal {
                let strength = changed.get("Strength").and_then(|v| u8::try_from(v).ok());
                return Changes {
                    strengths: strength
                        .map(|s| vec![(path.to_string(), s)])
                        .unwrap_or_default(),
                    ..Changes::default()
                };
            }
        }
    }
    Changes {
        other: true,
        ..Changes::default()
    }
}

/// Call `changed` with what changed whenever NetworkManager signals: any of
/// its signals, it joining or leaving the bus, a kill switch moving and the
/// machine waking. A burst is reported once, [`SETTLE`] after it starts.
/// The kill switches followed are the machine's ([`Radios::system`]).
///
/// Returns when the connection closes, once the burst under way is reported.
///
/// # Errors
///
/// The bus refusing the subscriptions at the start.
pub async fn watch(conn: &Connection, changed: impl FnMut(Changes)) -> zbus::Result<()> {
    watch_with(conn, &Radios::system(), changed).await
}

/// [`watch`], following the kill switches of `radios`.
///
/// # Errors
///
/// The bus refusing the subscriptions at the start.
pub async fn watch_with(
    conn: &Connection,
    radios: &Radios,
    mut changed: impl FnMut(Changes),
) -> zbus::Result<()> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)?
        .build();
    // The rule names NetworkManager by its well-known name, which the bus
    // routes by but this end cannot check a message's unique sender
    // against: another service's signal delivered to this connection for
    // another rule (logind's, the bus's own) would pass it. Every one of
    // NetworkManager's signals comes from under its own path, so that is
    // checked too.
    let signals = zbus::MessageStream::for_match_rule(rule, conn, Some(256))
        .await?
        .filter_map(|message| async move {
            let message = message.ok()?;
            let ours = message
                .header()
                .path()
                .is_some_and(|path| path.as_str().starts_with(PATH));
            ours.then(|| classify(&message))
        });
    // A kill switch moving: the airplane key, a physical switch. Not every
    // one of these reaches NetworkManager's own properties.
    let switches = radios.changes().map(|()| Changes {
        other: true,
        ..Changes::default()
    });
    // Waking from sleep: NetworkManager brings the devices back, and the
    // world may have changed while the lid was shut. Everything is read
    // again, once it is awake.
    let wakes = match otto_dbus::login1::Login1ManagerProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await
    {
        Ok(login1) => match login1.receive_prepare_for_sleep().await {
            Ok(signals) => signals
                .filter_map(|signal| async move {
                    let waking = signal.args().is_ok_and(|args| !*args.start());
                    waking.then(|| Changes {
                        profiles: true,
                        other: true,
                        ..Changes::default()
                    })
                })
                .boxed(),
            Err(_) => futures_util::stream::pending().boxed(),
        },
        Err(_) => futures_util::stream::pending().boxed(),
    };
    // NetworkManager coming or going changes everything, its saved
    // connections included.
    let owners = zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, SERVICE)])
        .await?
        .map(|_| Changes {
            profiles: true,
            other: true,
            ..Changes::default()
        });
    // NetworkManager's signals and its owner changes end when the connection
    // closes; each ends with a `None` that ends the watch. The kill switches
    // and logind may never end, or never start, so they only add changes.
    let closing = || futures_util::stream::once(async { None });
    let mut events = futures_util::stream::select_all([
        signals.map(Some).chain(closing()).boxed(),
        owners.map(Some).chain(closing()).boxed(),
        switches.map(Some).boxed(),
        wakes.map(Some).boxed(),
    ]);

    while let Some(Some(first)) = events.next().await {
        let mut burst = first;
        let mut closed = false;
        let settle = tokio::time::sleep(SETTLE);
        tokio::pin!(settle);
        loop {
            tokio::select! {
                _ = &mut settle => break,
                more = events.next() => match more {
                    Some(Some(more)) => burst.merge(more),
                    _ => {
                        closed = true;
                        break;
                    }
                },
            }
        }
        changed(burst);
        if closed {
            break;
        }
    }
    Ok(())
}

/// Read NetworkManager, then follow it for as long as the connection lasts,
/// handing every new snapshot to `on_snapshot`. A burst of changes costs
/// only what it needs: strengths alone are applied without a bus call, and
/// saved connections are read again only when one of them changed. The kill
/// switches are the machine's ([`Radios::system`]).
///
/// A read that fails, the first one included, is tried again on the next
/// change NetworkManager signals, or after a pause when none comes: a second
/// at first, doubling with each failure in a row up to half a minute. The
/// first snapshot is handed over once a read succeeds.
///
/// The reads run on the caller's runtime, never inside a callback, so this
/// is what a plain thread runs through [`crate::blocking::follow`].
///
/// # Errors
///
/// The bus refusing the subscriptions at the start. Otherwise it returns
/// `Ok` when the connection closes.
pub async fn follow(conn: &Connection, on_snapshot: impl FnMut(&Snapshot)) -> zbus::Result<()> {
    follow_with(conn, &Radios::system(), on_snapshot).await
}

/// [`follow`], with the kill switches of `radios`.
///
/// # Errors
///
/// The bus refusing the subscriptions at the start. Otherwise it returns
/// `Ok` when the connection closes.
pub async fn follow_with(
    conn: &Connection,
    radios: &Radios,
    mut on_snapshot: impl FnMut(&Snapshot),
) -> zbus::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Changes>();
    let watching = watch_with(conn, radios, move |changes| {
        let _ = tx.send(changes);
    });
    let everything = || Changes {
        profiles: true,
        other: true,
        ..Changes::default()
    };
    let reading = async {
        let mut current: Option<Snapshot> = None;
        // What the next read is for; a first read is for everything.
        let mut pending = everything();
        let mut retry = RETRY_FIRST;
        loop {
            let next = match &current {
                Some(current) if pending.only_strengths() => {
                    let mut next = current.clone();
                    next.apply_strengths(&pending.strengths);
                    Ok(next)
                }
                Some(current) => {
                    let profiles = (!pending.profiles).then(|| current.profiles.clone());
                    read_cached(conn, radios, profiles).await
                }
                None => read_cached(conn, radios, None).await,
            };
            match next {
                Ok(next) => {
                    retry = RETRY_FIRST;
                    if current.as_ref() != Some(&next) {
                        on_snapshot(&next);
                        current = Some(next);
                    }
                    match rx.recv().await {
                        Some(changes) => pending = changes,
                        None => break,
                    }
                }
                // NetworkManager failing a call, or going away mid-read: what
                // the read was for stays pending, and is read on the next
                // change or once the pause is over.
                Err(_) => {
                    tokio::select! {
                        changes = rx.recv() => match changes {
                            Some(changes) => pending.merge(changes),
                            None => break,
                        },
                        _ = tokio::time::sleep(retry) => {
                            retry = (retry * 2).min(RETRY_MAX);
                        }
                    }
                }
            }
            while let Ok(more) = rx.try_recv() {
                pending.merge(more);
            }
        }
        Ok::<(), zbus::Error>(())
    };
    tokio::try_join!(watching, reading)?;
    Ok(())
}
