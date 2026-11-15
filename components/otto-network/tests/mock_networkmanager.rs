//! `read` and the actions against a stand-in NetworkManager on a private
//! bus: a Wi-Fi device seeing three access points (two broadcasting the same
//! name), a saved network in use, a saved VPN, and the address configuration
//! of the connection in use.
//!
//! Needs `dbus-daemon`; skipped without it.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use otto_network::{Connectivity, DeviceKind, DeviceState, LinkKind, LinkState, Security};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

const NM: &str = "org.freedesktop.NetworkManager";

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p).unwrap()
}

/// What the stand-in was asked to do.
#[derive(Default)]
struct Calls {
    wifi_enabled: Vec<bool>,
    activated: Vec<(String, String, String)>,
    added: Vec<Vec<u8>>,
    deleted: Vec<String>,
    autoconnect: Vec<bool>,
}

struct Manager {
    calls: Arc<Mutex<Calls>>,
    wifi: bool,
}

#[interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        vec![
            path("/org/freedesktop/NetworkManager/Devices/1"),
            path("/org/freedesktop/NetworkManager/Devices/2"),
        ]
    }

    fn activate_connection(
        &self,
        connection: ObjectPath<'_>,
        device: ObjectPath<'_>,
        specific_object: ObjectPath<'_>,
    ) -> OwnedObjectPath {
        self.calls.lock().unwrap().activated.push((
            connection.to_string(),
            device.to_string(),
            specific_object.to_string(),
        ));
        path("/org/freedesktop/NetworkManager/ActiveConnection/9")
    }

    fn add_and_activate_connection(
        &self,
        connection: HashMap<String, HashMap<String, OwnedValue>>,
        _device: ObjectPath<'_>,
        _specific_object: ObjectPath<'_>,
    ) -> (OwnedObjectPath, OwnedObjectPath) {
        let ssid = connection["802-11-wireless"]["ssid"].try_clone().unwrap();
        self.calls
            .lock()
            .unwrap()
            .added
            .push(Vec::<u8>::try_from(ssid).unwrap());
        (
            path("/org/freedesktop/NetworkManager/Settings/9"),
            path("/org/freedesktop/NetworkManager/ActiveConnection/9"),
        )
    }

    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.wifi
    }
    #[zbus(property)]
    fn set_wireless_enabled(&mut self, on: bool) {
        self.wifi = on;
        self.calls.lock().unwrap().wifi_enabled.push(on);
    }
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        vec![
            path("/org/freedesktop/NetworkManager/ActiveConnection/1"),
            path("/org/freedesktop/NetworkManager/ActiveConnection/2"),
        ]
    }
    #[zbus(property)]
    fn primary_connection(&self) -> OwnedObjectPath {
        path("/org/freedesktop/NetworkManager/ActiveConnection/1")
    }
    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        4
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        70
    }
}

struct Device {
    kind: u32,
    interface: &'static str,
    state: u32,
    active: &'static str,
}

#[interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    fn disconnect(&self) {}
    #[zbus(property)]
    fn interface(&self) -> String {
        self.interface.into()
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        self.kind
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
    #[zbus(property)]
    fn managed(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn active_connection(&self) -> OwnedObjectPath {
        path(self.active)
    }
    #[zbus(property)]
    fn hw_address(&self) -> String {
        "AA:BB:CC:DD:EE:FF".into()
    }
}

struct Wireless;

/// NetworkManager's refusal of a scan asked for too soon.
#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop.NetworkManager.Device")]
enum DeviceError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotAllowed(String),
}

#[interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    fn request_scan(&self, _options: HashMap<String, OwnedValue>) -> Result<(), DeviceError> {
        Err(DeviceError::NotAllowed(
            "scanning not allowed so soon".into(),
        ))
    }
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        (1..=3)
            .map(|i| path(&format!("/org/freedesktop/NetworkManager/AccessPoint/{i}")))
            .collect()
    }
    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        path("/org/freedesktop/NetworkManager/AccessPoint/2")
    }
    #[zbus(property)]
    fn last_scan(&self) -> i64 {
        1234
    }
}

struct Ap {
    ssid: &'static str,
    strength: u8,
    rsn: u32,
}

#[interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl Ap {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.ssid.as_bytes().to_vec()
    }
    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.strength
    }
    #[zbus(property)]
    fn flags(&self) -> u32 {
        u32::from(self.rsn != 0)
    }
    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.rsn
    }
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        5180
    }
    #[zbus(property)]
    fn hw_address(&self) -> String {
        "00:11:22:33:44:55".into()
    }
}

struct SettingsRoot;

#[interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsRoot {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        vec![
            path("/org/freedesktop/NetworkManager/Settings/1"),
            path("/org/freedesktop/NetworkManager/Settings/2"),
        ]
    }
}

struct Saved {
    calls: Arc<Mutex<Calls>>,
    path: &'static str,
    kind: &'static str,
    id: &'static str,
    ssid: Option<&'static str>,
}

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Saved {
    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut connection = HashMap::new();
        connection.insert(
            "type".into(),
            OwnedValue::try_from(Value::from(self.kind)).unwrap(),
        );
        connection.insert(
            "id".into(),
            OwnedValue::try_from(Value::from(self.id)).unwrap(),
        );
        connection.insert("timestamp".into(), OwnedValue::from(42u64));
        let mut settings = HashMap::new();
        settings.insert("connection".into(), connection);
        if let Some(ssid) = self.ssid {
            let mut wireless = HashMap::new();
            wireless.insert(
                "ssid".into(),
                OwnedValue::try_from(Value::from(ssid.as_bytes().to_vec())).unwrap(),
            );
            settings.insert("802-11-wireless".into(), wireless);
        }
        settings
    }

    fn update(&self, properties: HashMap<String, HashMap<String, OwnedValue>>) {
        let on = bool::try_from(&properties["connection"]["autoconnect"]).unwrap();
        // The rest of the settings came back as they were sent.
        assert!(properties["connection"].contains_key("id"));
        self.calls.lock().unwrap().autoconnect.push(on);
    }

    fn delete(&self) {
        self.calls.lock().unwrap().deleted.push(self.path.into());
    }
}

struct Active {
    connection: &'static str,
    id: &'static str,
    kind: &'static str,
    devices: Vec<OwnedObjectPath>,
    ip4: &'static str,
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        path(self.connection)
    }
    #[zbus(property)]
    fn id(&self) -> String {
        self.id.into()
    }
    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        self.kind.into()
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        self.devices.clone()
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        path(self.ip4)
    }
    #[zbus(property)]
    fn ip6_config(&self) -> OwnedObjectPath {
        path("/")
    }
}

struct Ip4;

fn entry(address: &str, prefix: Option<u32>) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    map.insert(
        "address".to_string(),
        OwnedValue::try_from(Value::from(address)).unwrap(),
    );
    if let Some(prefix) = prefix {
        map.insert("prefix".to_string(), OwnedValue::from(prefix));
    }
    map
}

#[interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![entry("192.168.1.20", Some(24))]
    }
    #[zbus(property)]
    fn gateway(&self) -> String {
        "192.168.1.1".into()
    }
    #[zbus(property)]
    fn nameserver_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![entry("1.1.1.1", None)]
    }
}

/// A private bus, killed when dropped.
struct Bus(Child, String);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn private_bus() -> Option<Bus> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut address = String::new();
    let mut byte = [0u8];
    use std::io::Read;
    while out.read(&mut byte).ok()? == 1 && byte[0] != b'\n' {
        address.push(byte[0] as char);
    }
    Some(Bus(child, address))
}

async fn serve(address: &str, calls: Arc<Mutex<Calls>>) -> Connection {
    let wifi_dev = "/org/freedesktop/NetworkManager/Devices/1";
    let builder = zbus::connection::Builder::address(address)
        .unwrap()
        .name(NM)
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager",
            Manager {
                calls: calls.clone(),
                wifi: true,
            },
        )
        .unwrap()
        .serve_at(
            wifi_dev,
            Device {
                kind: 2,
                interface: "wlan0",
                state: 100,
                active: "/org/freedesktop/NetworkManager/ActiveConnection/1",
            },
        )
        .unwrap()
        .serve_at(wifi_dev, Wireless)
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/Devices/2",
            Device {
                kind: 1,
                interface: "eth0",
                state: 20,
                active: "/",
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/AccessPoint/1",
            Ap {
                ssid: "Home",
                strength: 40,
                rsn: 0x188,
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/AccessPoint/2",
            Ap {
                ssid: "Home",
                strength: 81,
                rsn: 0x188,
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/AccessPoint/3",
            Ap {
                ssid: "Cafe",
                strength: 90,
                rsn: 0,
            },
        )
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/Settings", SettingsRoot)
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/Settings/1",
            Saved {
                calls: calls.clone(),
                path: "/org/freedesktop/NetworkManager/Settings/1",
                kind: "802-11-wireless",
                id: "Home",
                ssid: Some("Home"),
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/Settings/2",
            Saved {
                calls: calls.clone(),
                path: "/org/freedesktop/NetworkManager/Settings/2",
                kind: "wireguard",
                id: "Office",
                ssid: None,
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/ActiveConnection/1",
            Active {
                connection: "/org/freedesktop/NetworkManager/Settings/1",
                id: "Home",
                kind: "802-11-wireless",
                devices: vec![path(wifi_dev)],
                ip4: "/org/freedesktop/NetworkManager/IP4Config/1",
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/ActiveConnection/2",
            Active {
                connection: "/org/freedesktop/NetworkManager/Settings/2",
                id: "Office",
                kind: "wireguard",
                devices: Vec::new(),
                ip4: "/",
            },
        )
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/IP4Config/1", Ip4)
        .unwrap();
    builder.build().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_and_the_actions_against_a_stand_in() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls.clone()).await;
    let conn = zbus::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();

    let snapshot = otto_network::read(&conn).await.expect("a snapshot");
    assert!(snapshot.running);
    assert!(snapshot.wifi_enabled);
    assert_eq!(snapshot.connectivity, Connectivity::Full);
    assert_eq!(snapshot.devices.len(), 2);
    let eth = snapshot
        .devices
        .iter()
        .find(|d| d.kind == DeviceKind::Ethernet)
        .unwrap();
    assert_eq!(eth.state, DeviceState::Unavailable);

    let wifi = snapshot.wifi.as_ref().expect("a Wi-Fi device");
    assert_eq!(wifi.device.interface, "wlan0");
    assert_eq!(wifi.last_scan_ms, Some(1234));
    let names: Vec<_> = wifi.networks.iter().map(|n| n.name()).collect();
    assert_eq!(names, ["Home", "Cafe"], "in use first, one row per name");
    let home = &wifi.networks[0];
    assert_eq!(home.state, LinkState::Connected);
    assert_eq!(home.strength, 81);
    assert_eq!(home.security, Security::Personal);
    assert_eq!(
        home.saved.as_deref(),
        Some("/org/freedesktop/NetworkManager/Settings/1")
    );
    assert_eq!(wifi.networks[1].security, Security::Open);

    assert_eq!(snapshot.saved.len(), 1);
    assert!(snapshot.saved[0].in_use);
    assert_eq!(snapshot.vpns.len(), 1);
    assert_eq!(snapshot.vpns[0].id, "Office");
    assert_eq!(snapshot.vpns[0].state, LinkState::Connected);

    let primary = snapshot.primary.as_ref().expect("a primary connection");
    assert_eq!(primary.kind, LinkKind::Wifi);
    assert_eq!(primary.interface, "wlan0");
    assert_eq!(primary.ipv4, ["192.168.1.20/24"]);
    assert_eq!(primary.gateway.as_deref(), Some("192.168.1.1"));
    assert_eq!(primary.dns, ["1.1.1.1"]);
    assert_eq!(snapshot.glyph(), otto_network::Glyph::Wifi(3));

    // The actions.
    let device = wifi.device.path.clone();
    otto_network::set_wifi_enabled(&conn, false).await.unwrap();
    otto_network::connect_wifi(&conn, &device, home)
        .await
        .unwrap();
    otto_network::connect_wifi(&conn, &device, &wifi.networks[1])
        .await
        .unwrap();
    otto_network::set_autoconnect(&conn, &snapshot.saved[0].path, false)
        .await
        .unwrap();
    otto_network::forget(&conn, &snapshot.saved[0].path)
        .await
        .unwrap();
    // A scan refused for coming too soon is not an error.
    otto_network::request_scan(&conn, &device).await.unwrap();

    let calls = calls.lock().unwrap();
    assert_eq!(calls.wifi_enabled, [false]);
    assert_eq!(
        calls.activated,
        [(
            "/org/freedesktop/NetworkManager/Settings/1".to_string(),
            device.clone(),
            "/org/freedesktop/NetworkManager/AccessPoint/2".to_string()
        )]
    );
    assert_eq!(calls.added, [b"Cafe".to_vec()]);
    assert_eq!(calls.autoconnect, [false]);
    assert_eq!(
        calls.deleted,
        ["/org/freedesktop/NetworkManager/Settings/1"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_networkmanager_is_an_empty_snapshot() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let conn = zbus::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let snapshot = otto_network::read(&conn).await.unwrap();
    assert!(!snapshot.running);
    assert_eq!(snapshot.glyph(), otto_network::Glyph::Hidden);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_wakes_the_watcher_once() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls).await;
    let conn = zbus::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = conn.clone();
    tokio::spawn(async move {
        let _ = otto_network::watch(&watcher, move || {
            let _ = tx.send(());
        })
        .await;
    });
    // Let the match rule land before anything changes.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A burst: three changes in a row, each a PropertiesChanged signal.
    for on in [false, true, false] {
        otto_network::set_wifi_enabled(&conn, on).await.unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("the watcher reported the change")
        .unwrap();
    // The burst was reported once.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(rx.try_recv().is_err());
}
