//! `read` and the actions against a stand-in NetworkManager on a private
//! bus: a Wi-Fi device seeing three access points (two broadcasting the same
//! name), a saved network in use, a saved VPN, and the address configuration
//! of the connection in use.
//!
//! Needs `dbus-daemon`; skipped without it. Every call is given
//! [`Radios::none`], so nothing here reads the host's radios or turns its
//! Wi-Fi on, and every connection names its bus, so no test touches the
//! system bus or the environment.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_network::rfkill::Radios;
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
    /// The security setting of each connection added.
    added_security: Vec<Option<(String, String)>>,
    added_hidden: Vec<bool>,
    /// Settings each Update carried.
    updated: Vec<HashMap<String, HashMap<String, OwnedValue>>>,
    reapplied: Vec<String>,
    /// How many `GetDevices` calls to fail before answering.
    fail_get_devices: u32,
}

struct Manager {
    calls: Arc<Mutex<Calls>>,
    wifi: bool,
}

#[interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    fn get_devices(&self) -> zbus::fdo::Result<Vec<OwnedObjectPath>> {
        let mut calls = self.calls.lock().unwrap();
        if calls.fail_get_devices > 0 {
            calls.fail_get_devices -= 1;
            return Err(zbus::fdo::Error::Failed("starting up".into()));
        }
        Ok(vec![
            path("/org/freedesktop/NetworkManager/Devices/1"),
            path("/org/freedesktop/NetworkManager/Devices/2"),
        ])
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
        let mut calls = self.calls.lock().unwrap();
        calls.added.push(Vec::<u8>::try_from(ssid).unwrap());
        calls.added_hidden.push(
            connection["802-11-wireless"]
                .get("hidden")
                .is_some_and(|v| bool::try_from(v).unwrap()),
        );
        calls
            .added_security
            .push(connection.get("802-11-wireless-security").map(|sec| {
                let get = |k: &str| String::try_from(sec[k].try_clone().unwrap()).unwrap();
                (get("key-mgmt"), get("psk"))
            }));
        drop(calls);
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
    calls: Arc<Mutex<Calls>>,
    path: &'static str,
    kind: u32,
    interface: &'static str,
    state: u32,
    active: &'static str,
    metered: u32,
}

#[interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    fn disconnect(&self) {}
    fn reapply(&self, _connection: HashMap<String, HashMap<String, OwnedValue>>, _v: u64, _f: u32) {
        self.calls.lock().unwrap().reapplied.push(self.path.into());
    }
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
    #[zbus(property)]
    fn metered(&self) -> u32 {
        self.metered
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
    #[zbus(property)]
    fn bitrate(&self) -> u32 {
        866_000
    }
}

struct Wired;

#[interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl Wired {
    #[zbus(property)]
    fn speed(&self) -> u32 {
        1000
    }
    #[zbus(property)]
    fn carrier(&self) -> bool {
        false
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
    /// Writable here only, so a test can move it the way a scan does.
    #[zbus(property)]
    fn set_strength(&mut self, strength: u8) {
        self.strength = strength;
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

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

fn copy(settings: &Settings) -> Settings {
    settings
        .iter()
        .map(|(group, values)| {
            let values = values
                .iter()
                .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                .collect();
            (group.clone(), values)
        })
        .collect()
}

/// A saved connection. It keeps what an `Update` sends, and takes a moment
/// to answer `GetSettings`, as NetworkManager reading a profile from disk
/// can: two edits made at once both read before either writes.
struct Saved {
    calls: Arc<Mutex<Calls>>,
    path: &'static str,
    kind: &'static str,
    id: &'static str,
    ssid: Option<&'static str>,
    stored: Mutex<Option<Settings>>,
}

impl Saved {
    fn new(
        calls: Arc<Mutex<Calls>>,
        path: &'static str,
        kind: &'static str,
        id: &'static str,
        ssid: Option<&'static str>,
    ) -> Self {
        Self {
            calls,
            path,
            kind,
            id,
            ssid,
            stored: Mutex::new(None),
        }
    }

    fn initial(&self) -> Settings {
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
}

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Saved {
    async fn get_settings(&self) -> Settings {
        let settings = match &*self.stored.lock().unwrap() {
            Some(stored) => copy(stored),
            None => self.initial(),
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        settings
    }

    fn update(&self, properties: Settings) {
        // The rest of the settings came back as they were sent.
        assert!(properties["connection"].contains_key("id"));
        *self.stored.lock().unwrap() = Some(copy(&properties));
        let mut calls = self.calls.lock().unwrap();
        if let Some(on) = properties["connection"].get("autoconnect") {
            calls.autoconnect.push(bool::try_from(on).unwrap());
        }
        calls.updated.push(properties);
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
    ip6: &'static str,
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
        path(self.ip6)
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

struct Ip6;

#[interface(name = "org.freedesktop.NetworkManager.IP6Config")]
impl Ip6 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![entry("2001:db8::20", Some(64))]
    }
    #[zbus(property)]
    fn gateway(&self) -> String {
        "fe80::1".into()
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

/// A client connection to the bus at `address`.
async fn connect(address: &str) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .build()
        .await
        .unwrap()
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
                calls: calls.clone(),
                path: wifi_dev,
                kind: 2,
                interface: "wlan0",
                state: 100,
                active: "/org/freedesktop/NetworkManager/ActiveConnection/1",
                metered: 3,
            },
        )
        .unwrap()
        .serve_at(wifi_dev, Wireless)
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/Devices/2", Wired)
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/Devices/2",
            Device {
                calls: calls.clone(),
                path: "/org/freedesktop/NetworkManager/Devices/2",
                kind: 1,
                interface: "eth0",
                state: 20,
                active: "/",
                metered: 0,
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
            Saved::new(
                calls.clone(),
                "/org/freedesktop/NetworkManager/Settings/1",
                "802-11-wireless",
                "Home",
                Some("Home"),
            ),
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/NetworkManager/Settings/2",
            Saved::new(
                calls.clone(),
                "/org/freedesktop/NetworkManager/Settings/2",
                "wireguard",
                "Office",
                None,
            ),
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
                ip6: "/org/freedesktop/NetworkManager/IP6Config/1",
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
                ip6: "/",
            },
        )
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/IP4Config/1", Ip4)
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/IP6Config/1", Ip6)
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
    let conn = connect(&bus.1).await;

    let snapshot = otto_network::read_with(&conn, &Radios::none())
        .await
        .expect("a snapshot");
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
    assert_eq!(eth.speed_mbps, Some(1000));

    let wifi = snapshot.wifi.as_ref().expect("a Wi-Fi device");
    assert_eq!(wifi.device.interface, "wlan0");
    assert_eq!(wifi.device.metered, otto_network::Metered::GuessYes);
    let link = wifi.active_ap.as_ref().expect("the access point in use");
    assert_eq!(link.path, "/org/freedesktop/NetworkManager/AccessPoint/2");
    assert_eq!(link.band(), otto_network::Band::Ghz5);
    assert_eq!(link.channel(), 36);
    assert_eq!(wifi.bitrate_kbps, Some(866_000));
    assert_eq!(
        wifi.device.active_profile.as_deref(),
        Some("/org/freedesktop/NetworkManager/Settings/1")
    );
    assert_eq!(snapshot.profiles.len(), 2);
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
    assert_eq!(primary.ipv6, ["2001:db8::20/64"]);
    assert_eq!(primary.gateway6.as_deref(), Some("fe80::1"));
    assert_eq!(snapshot.glyph(), otto_network::Glyph::Wifi(3));

    // The actions.
    let device = wifi.device.path.clone();
    otto_network::set_wifi_enabled_with(&conn, &Radios::none(), false)
        .await
        .unwrap();
    otto_network::connect_wifi(&conn, &device, home)
        .await
        .unwrap();
    otto_network::connect_wifi(&conn, &device, &wifi.networks[1])
        .await
        .unwrap();
    otto_network::set_autoconnect(&conn, &snapshot.saved[0].path, false)
        .await
        .unwrap();
    // A new secured network joined with a password: the key goes with it.
    let mut secured = wifi.networks[1].clone();
    secured.security = Security::Personal;
    otto_network::connect_wifi_with_password(&conn, &device, &secured, "hunter22")
        .await
        .unwrap();
    otto_network::connect_hidden(
        &conn,
        &device,
        b"Secret",
        Some(otto_network::KeyKind::Sae),
        "pw",
    )
    .await
    .unwrap();

    // A manual address, saved and applied in place.
    let manual = otto_network::IpConfig {
        method: otto_network::IpMethod::Manual,
        addresses: vec![
            otto_network::parse_address("10.0.0.9/8", otto_network::Family::V4).unwrap(),
        ],
        gateway: Some("10.0.0.1".into()),
        dns: vec!["1.1.1.1".into()],
        ignore_auto_dns: true,
        dns_search: Vec::new(),
    };
    let profile = snapshot.saved[0].path.clone();
    let v6 = otto_network::IpConfig::default();
    otto_network::set_ip_config(&conn, &profile, &manual, &v6)
        .await
        .unwrap();
    // One that does not validate is refused before anything is sent.
    let broken = otto_network::IpConfig {
        addresses: Vec::new(),
        ..manual.clone()
    };
    assert!(otto_network::set_ip_config(&conn, &profile, &broken, &v6)
        .await
        .is_err());

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
    assert_eq!(
        calls.added,
        [b"Cafe".to_vec(), b"Cafe".to_vec(), b"Secret".to_vec()]
    );
    assert_eq!(
        calls.added_security,
        [
            None,
            Some(("wpa-psk".to_string(), "hunter22".to_string())),
            Some(("sae".to_string(), "pw".to_string())),
        ]
    );
    assert_eq!(calls.added_hidden, [false, false, true]);
    // autoconnect, then the address.
    assert_eq!(calls.updated.len(), 2);
    let written =
        otto_network::IpConfig::from_settings(&calls.updated[1], otto_network::Family::V4);
    assert_eq!(written, manual);
    assert_eq!(calls.reapplied, std::slice::from_ref(&device));
    // The address update kept the autoconnect change made before it.
    assert_eq!(calls.autoconnect, [false, false]);
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
    let conn = connect(&bus.1).await;
    let snapshot = otto_network::read_with(&conn, &Radios::none())
        .await
        .unwrap();
    assert!(!snapshot.running);
    assert_eq!(snapshot.glyph(), otto_network::Glyph::Hidden);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_wakes_the_watcher_once_and_says_what_changed() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls).await;
    let conn = connect(&bus.1).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = conn.clone();
    tokio::spawn(async move {
        let _ = otto_network::watch_with(&watcher, &Radios::none(), move |changes| {
            let _ = tx.send(changes);
        })
        .await;
    });
    // Let the match rule land before anything changes.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A burst: three changes in a row, each a PropertiesChanged signal.
    for on in [false, true, false] {
        otto_network::set_wifi_enabled_with(&conn, &Radios::none(), on)
            .await
            .unwrap();
    }
    let changes = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("the watcher reported")
        .unwrap();
    assert!(changes.other && !changes.profiles);
    // The burst was reported once.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(rx.try_recv().is_err());

    // A scan moving one access point's signal is a strength, and nothing
    // more: no read is needed for it.
    let ap = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination(NM)
        .unwrap()
        .path("/org/freedesktop/NetworkManager/AccessPoint/3")
        .unwrap()
        .build()
        .await
        .unwrap();
    ap.set(
        zbus::names::InterfaceName::try_from("org.freedesktop.NetworkManager.AccessPoint").unwrap(),
        "Strength",
        Value::from(12u8),
    )
    .await
    .unwrap();
    let changes = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("the watcher reported")
        .unwrap();
    assert_eq!(
        changes.strengths,
        [(
            "/org/freedesktop/NetworkManager/AccessPoint/3".to_string(),
            12
        )]
    );
    assert!(changes.only_strengths());
}

/// What the Network pane runs: following on a plain thread with no runtime
/// of its own. A callback that read from inside the watch's runtime used to
/// panic here on the first change.
#[test]
fn following_on_a_plain_thread_reads_again_on_every_change() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _server = runtime.block_on(serve(&bus.1, Arc::new(Mutex::new(Calls::default()))));
    let conn = runtime.block_on(connect(&bus.1));

    let (tx, rx) = std::sync::mpsc::channel();
    let address = bus.1.clone();
    std::thread::spawn(move || {
        let _ = otto_network::blocking::follow_with(&address, Radios::none(), move |snapshot| {
            let _ = tx.send(snapshot.wifi_enabled);
        });
    });
    let wait = std::time::Duration::from_secs(5);
    assert_eq!(rx.recv_timeout(wait), Ok(true), "the first read");
    runtime
        .block_on(otto_network::set_wifi_enabled_with(
            &conn,
            &Radios::none(),
            false,
        ))
        .unwrap();
    assert_eq!(
        rx.recv_timeout(wait),
        Ok(false),
        "read again after the change"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn waking_from_sleep_reads_everything_again() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let conn = zbus::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    // logind's stand-in: only its bus name and its sleep signal.
    let logind = zbus::connection::Builder::address(bus.1.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .build()
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = conn.clone();
    tokio::spawn(async move {
        let _ = otto_network::watch_with(&watcher, &Radios::none(), move |changes| {
            let _ = tx.send(changes);
        })
        .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sleep_signal = |start: bool| {
        let logind = logind.clone();
        async move {
            logind
                .emit_signal(
                    None::<()>,
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "PrepareForSleep",
                    &(start,),
                )
                .await
        }
    };
    // Going to sleep is nothing to read for: the screen is about to go dark.
    sleep_signal(true).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let got = rx.try_recv();
    assert!(got.is_err(), "{got:?}");

    // Waking is everything.
    sleep_signal(false).await.unwrap();
    let changes = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("the watcher reported the wake")
        .unwrap();
    assert!(changes.other && changes.profiles);
}

/// The address the race test writes, valid for the profile.
fn manual_v4() -> otto_network::IpConfig {
    otto_network::IpConfig {
        method: otto_network::IpMethod::Manual,
        addresses: vec![
            otto_network::parse_address("10.0.0.9/8", otto_network::Family::V4).unwrap(),
        ],
        gateway: Some("10.0.0.1".into()),
        dns: vec!["1.1.1.1".into()],
        ignore_auto_dns: true,
        dns_search: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_edits_at_once_both_stay_on_the_profile() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls.clone()).await;
    let conn = connect(&bus.1).await;
    let profile = "/org/freedesktop/NetworkManager/Settings/1";
    let manual = manual_v4();
    let v6 = otto_network::IpConfig::default();

    // Settings saves an address while the bar turns auto-join off.
    let (ip, autoconnect) = tokio::join!(
        otto_network::set_ip_config(&conn, profile, &manual, &v6),
        otto_network::set_autoconnect(&conn, profile, false),
    );
    ip.unwrap();
    autoconnect.unwrap();

    let calls = calls.lock().unwrap();
    assert_eq!(calls.updated.len(), 2);
    let last = calls.updated.last().unwrap();
    assert_eq!(
        last["connection"]
            .get("autoconnect")
            .map(|v| bool::try_from(v).unwrap()),
        Some(false),
        "the auto-join change survived"
    );
    assert_eq!(
        otto_network::IpConfig::from_settings(last, otto_network::Family::V4),
        manual,
        "the address survived"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_read_that_fails_is_tried_again() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls {
        fail_get_devices: 2,
        ..Calls::default()
    }));
    let _server = serve(&bus.1, calls.clone()).await;
    let conn = connect(&bus.1).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let _ = otto_network::follow_with(&conn, &Radios::none(), move |snapshot| {
            let _ = tx.send(snapshot.clone());
        })
        .await;
    });
    // Two failures: a second's pause, then two.
    let snapshot = tokio::time::timeout(Duration::from_secs(6), rx.recv())
        .await
        .expect("a snapshot once NetworkManager answers")
        .unwrap();
    assert!(snapshot.running);
    assert_eq!(snapshot.devices.len(), 2);
    assert_eq!(calls.lock().unwrap().fail_get_devices, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn following_ends_when_the_bus_goes_away() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let _server = serve(&bus.1, Arc::new(Mutex::new(Calls::default()))).await;
    let conn = connect(&bus.1).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let following = tokio::spawn(async move {
        otto_network::follow_with(&conn, &Radios::none(), move |_| {
            let _ = tx.send(());
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("the first snapshot")
        .unwrap();

    drop(bus);
    let ended = tokio::time::timeout(Duration::from_secs(5), following)
        .await
        .expect("follow returned once the bus closed")
        .unwrap();
    assert!(ended.is_ok(), "{ended:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_private_address_is_saved_on_the_profile() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls.clone()).await;
    let conn = connect(&bus.1).await;
    let profile = "/org/freedesktop/NetworkManager/Settings/1";

    otto_network::set_mac_policy(&conn, profile, &otto_network::MacPolicy::Stable)
        .await
        .unwrap();

    let calls = calls.lock().unwrap();
    let sent = calls.updated.last().expect("an update");
    let wireless = &sent["802-11-wireless"];
    assert_eq!(
        String::try_from(wireless["assigned-mac-address"].try_clone().unwrap()).unwrap(),
        "stable"
    );
    // The rest of the profile went back as it was.
    assert!(wireless.contains_key("ssid"));
    // Saving it does not reconnect: it takes effect on the next join.
    assert!(calls.reapplied.is_empty());
    assert!(calls.activated.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn metered_is_saved_and_applied_in_place() {
    let Some(bus) = private_bus() else {
        eprintln!("no dbus-daemon: skipped");
        return;
    };
    let calls = Arc::new(Mutex::new(Calls::default()));
    let _server = serve(&bus.1, calls.clone()).await;
    let conn = connect(&bus.1).await;
    let profile = "/org/freedesktop/NetworkManager/Settings/1";

    otto_network::set_metered(&conn, profile, otto_network::MeteredSetting::Yes)
        .await
        .unwrap();

    let calls = calls.lock().unwrap();
    let sent = calls.updated.last().expect("an update");
    assert_eq!(i32::try_from(&sent["connection"]["metered"]).unwrap(), 1);
    assert!(sent.contains_key("802-11-wireless"));
    // Applied to the link in use, without reconnecting it.
    assert_eq!(
        calls.reapplied,
        ["/org/freedesktop/NetworkManager/Devices/1"]
    );
    assert!(calls.activated.is_empty());
}
