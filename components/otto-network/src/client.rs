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
    AccessPointProxy, ActiveConnectionProxy, DeviceProxy, DeviceWirelessProxy, Ip4ConfigProxy,
    Ip6ConfigProxy, NetworkManagerProxy, NetworkSettingsProxy, SettingsConnectionProxy, SERVICE,
};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::model::*;

/// `NMDeviceType` values Otto shows.
const TYPE_ETHERNET: u32 = 1;
const TYPE_WIFI: u32 = 2;

/// How long a burst of changes is gathered before the watcher reports it.
/// Signal strengths move on every scan, many objects at once.
const SETTLE: Duration = Duration::from_millis(150);

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
pub async fn is_running(conn: &Connection) -> bool {
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
/// is an empty snapshot with `running` false, not an error.
pub async fn read(conn: &Connection) -> zbus::Result<Snapshot> {
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
        connectivity: Connectivity::from_raw(nm.connectivity().await.unwrap_or(0)),
        ..Snapshot::default()
    };

    // Devices, with the Wi-Fi device's active connection kept aside to tell
    // which network is in use.
    let mut wifi_device: Option<(Device, Option<String>)> = None;
    for path in nm.get_devices().await? {
        let Some((device, active)) = read_device(conn, path.as_str()).await else {
            continue;
        };
        if device.kind == DeviceKind::Wifi && wifi_device.is_none() {
            wifi_device = Some((device.clone(), active));
        }
        snapshot.devices.push(device);
    }

    // Saved connections.
    let (mut saved, vpns) = read_saved(conn).await?;

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
        let access_points = read_access_points(conn, &wireless).await;
        let last_scan_ms = wireless.last_scan().await.ok().filter(|ms| *ms >= 0);
        let networks = group_networks(
            &access_points,
            &saved,
            in_use
                .as_ref()
                .map(|(ssid, state)| (ssid.as_slice(), *state)),
        );
        snapshot.wifi = Some(WifiDevice {
            device,
            networks,
            last_scan_ms,
        });
    }

    sort_saved(&mut saved);
    snapshot.saved = saved;

    if let Some(primary) = some_path(nm.primary_connection().await.unwrap_or_default()) {
        snapshot.primary = read_link(conn, &primary).await;
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
    let device = Device {
        path: path.to_string(),
        kind,
        interface: proxy.interface().await.unwrap_or_default(),
        state: DeviceState::from_raw(proxy.state().await.unwrap_or(0)),
        hw_address: proxy.hw_address().await.unwrap_or_default(),
    };
    let active = proxy.active_connection().await.ok().and_then(some_path);
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
        Some(AccessPoint {
            path: path.as_str().to_string(),
            ssid: proxy.ssid().await.ok()?,
            strength: proxy.strength().await.unwrap_or(0),
            security: Security::from_flags(
                proxy.flags().await.unwrap_or(0),
                proxy.wpa_flags().await.unwrap_or(0),
                proxy.rsn_flags().await.unwrap_or(0),
            ),
            frequency_mhz: proxy.frequency().await.unwrap_or(0),
        })
    });
    join_all(reads).await.into_iter().flatten().collect()
}

fn string_in(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    settings
        .get(key)
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

/// The saved Wi-Fi networks and VPNs.
async fn read_saved(conn: &Connection) -> zbus::Result<(Vec<SavedNetwork>, Vec<Vpn>)> {
    let settings = NetworkSettingsProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let paths = settings.list_connections().await?;
    let reads = paths.iter().map(|path| async move {
        let proxy = at!(SettingsConnectionProxy, conn, path.as_str())
            .await
            .ok()?;
        Some((path.as_str().to_string(), proxy.get_settings().await.ok()?))
    });

    let mut wifi = Vec::new();
    let mut vpns = Vec::new();
    for (path, settings) in join_all(reads).await.into_iter().flatten() {
        let Some(connection) = settings.get("connection") else {
            continue;
        };
        let kind = string_in(connection, "type").unwrap_or_default();
        let id = string_in(connection, "id").unwrap_or_default();
        match kind.as_str() {
            "802-11-wireless" => {
                let ssid = settings
                    .get("802-11-wireless")
                    .and_then(|w| w.get("ssid"))
                    .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_default();
                // A hotspot this machine serves is not a network to join.
                let mode = settings
                    .get("802-11-wireless")
                    .and_then(|w| string_in(w, "mode"))
                    .unwrap_or_default();
                if mode == "ap" {
                    continue;
                }
                wifi.push(SavedNetwork {
                    path,
                    id,
                    ssid,
                    // Absent means the default, which is on.
                    autoconnect: connection
                        .get("autoconnect")
                        .and_then(|v| bool::try_from(v).ok())
                        .unwrap_or(true),
                    last_used: connection
                        .get("timestamp")
                        .and_then(|v| u64::try_from(v).ok())
                        .unwrap_or(0),
                    in_use: false,
                });
            }
            "vpn" | "wireguard" => vpns.push(Vpn {
                path,
                id,
                active: None,
                state: LinkState::Idle,
            }),
            _ => {}
        }
    }
    vpns.sort_by_key(|v| v.id.to_lowercase());
    Ok((wifi, vpns))
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
    }
    Some(link)
}

fn object_path(path: &str) -> zbus::Result<ObjectPath<'_>> {
    Ok(ObjectPath::try_from(path)?)
}

/// Turn the Wi-Fi radios on or off.
pub async fn set_wifi_enabled(conn: &Connection, enabled: bool) -> zbus::Result<()> {
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

/// Let NetworkManager join the saved connection at `connection` by itself,
/// or not.
pub async fn set_autoconnect(conn: &Connection, connection: &str, on: bool) -> zbus::Result<()> {
    let proxy = at!(SettingsConnectionProxy, conn, connection).await?;
    let mut settings = proxy.get_settings().await?;
    settings
        .entry("connection".to_string())
        .or_default()
        .insert("autoconnect".to_string(), OwnedValue::from(on));
    // The settings come without secrets, and an update without secrets keeps
    // the ones already stored.
    proxy.update(settings).await
}

/// Call `changed` whenever NetworkManager's state may have changed: any of
/// its signals, and it joining or leaving the bus. A burst is reported once,
/// [`SETTLE`] after it starts. Returns when the connection closes.
pub async fn watch(conn: &Connection, mut changed: impl FnMut()) -> zbus::Result<()> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)?
        .build();
    let signals = zbus::MessageStream::for_match_rule(rule, conn, Some(256))
        .await?
        .map(|_| ());
    let owners = zbus::fdo::DBusProxy::new(conn)
        .await?
        .receive_name_owner_changed_with_args(&[(0, SERVICE)])
        .await?
        .map(|_| ());
    let mut events = futures_util::stream::select(signals, owners);

    while events.next().await.is_some() {
        let settle = tokio::time::sleep(SETTLE);
        tokio::pin!(settle);
        loop {
            tokio::select! {
                _ = &mut settle => break,
                more = events.next() => if more.is_none() { break },
            }
        }
        changed();
    }
    Ok(())
}
