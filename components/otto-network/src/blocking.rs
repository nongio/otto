//! The same calls for a plain thread with no runtime of its own.
//!
//! Each call starts a small single-threaded tokio runtime, connects to the
//! system bus on it, does its work and drops both. That is a few milliseconds
//! a call, which is nothing next to a person clicking. Never call these from
//! inside a tokio runtime, a callback of [`follow`] included: tokio refuses
//! to start a runtime on a thread that already runs one.

use std::future::Future;

use zbus::Connection;

use crate::model::{Snapshot, WifiNetwork};
use crate::profile::{IpConfig, MacPolicy, MeteredSetting, WifiShare};
use crate::reveal::ShareError;
use crate::rfkill::Radios;

/// Run `work` with a fresh system bus connection on a runtime of its own.
fn run<T, F, Fut>(work: F) -> zbus::Result<T>
where
    F: FnOnce(Connection) -> Fut,
    Fut: Future<Output = zbus::Result<T>>,
{
    run_on(None, work)
}

/// Run `work` with a fresh connection to the bus at `address`, or to the
/// system bus when it is `None`, on a runtime of its own.
fn run_on<T, F, Fut>(address: Option<&str>, work: F) -> zbus::Result<T>
where
    F: FnOnce(Connection) -> Fut,
    Fut: Future<Output = zbus::Result<T>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    runtime.block_on(async move {
        let conn = match address {
            Some(address) => zbus::connection::Builder::address(address)?.build().await?,
            None => Connection::system().await?,
        };
        work(conn).await
    })
}

/// [`crate::read`].
pub fn read() -> zbus::Result<Snapshot> {
    run(|conn| async move { crate::read(&conn).await })
}

/// [`crate::set_wifi_enabled`].
pub fn set_wifi_enabled(enabled: bool) -> zbus::Result<()> {
    run(|conn| async move { crate::set_wifi_enabled(&conn, enabled).await })
}

/// [`crate::connect_wifi`].
pub fn connect_wifi(device: &str, network: &WifiNetwork) -> zbus::Result<()> {
    run(|conn| async move { crate::connect_wifi(&conn, device, network).await })
}

/// [`crate::connect_wifi_with_password`].
pub fn connect_wifi_with_password(
    device: &str,
    network: &WifiNetwork,
    password: &str,
) -> zbus::Result<()> {
    run(
        |conn| async move { crate::connect_wifi_with_password(&conn, device, network, password).await },
    )
}

/// [`crate::set_ip_config`].
pub fn set_ip_config(profile: &str, v4: &IpConfig, v6: &IpConfig) -> zbus::Result<()> {
    run(|conn| async move { crate::set_ip_config(&conn, profile, v4, v6).await })
}

/// [`crate::activate`].
pub fn activate(connection: &str) -> zbus::Result<()> {
    run(|conn| async move { crate::activate(&conn, connection).await })
}

/// [`crate::deactivate`].
pub fn deactivate(active: &str) -> zbus::Result<()> {
    run(|conn| async move { crate::deactivate(&conn, active).await })
}

/// [`crate::disconnect_device`].
pub fn disconnect_device(device: &str) -> zbus::Result<()> {
    run(|conn| async move { crate::disconnect_device(&conn, device).await })
}

/// [`crate::forget`].
pub fn forget(connection: &str) -> zbus::Result<()> {
    run(|conn| async move { crate::forget(&conn, connection).await })
}

/// [`crate::set_autoconnect`].
pub fn set_autoconnect(connection: &str, on: bool) -> zbus::Result<()> {
    run(|conn| async move { crate::set_autoconnect(&conn, connection, on).await })
}

/// [`crate::set_mac_policy`].
pub fn set_mac_policy(connection: &str, policy: &MacPolicy) -> zbus::Result<()> {
    run(|conn| async move { crate::set_mac_policy(&conn, connection, policy).await })
}

/// [`crate::set_metered`].
pub fn set_metered(connection: &str, metered: MeteredSetting) -> zbus::Result<()> {
    run(|conn| async move { crate::set_metered(&conn, connection, metered).await })
}

/// [`crate::wifi_share`]. Blocks while a polkit agent asks.
///
/// # Errors
///
/// As [`crate::wifi_share`], and [`ShareError::Bus`] with no system bus.
pub fn wifi_share(connection: &str) -> Result<WifiShare, ShareError> {
    run(|conn| async move { Ok(crate::wifi_share(&conn, connection).await) })?
}

/// [`crate::follow`], blocking the thread until the bus connection closes.
/// `on_snapshot` runs on this thread between reads; it must not call the
/// other functions here (see the module's note).
///
/// # Errors
///
/// No system bus, or the bus refusing the subscriptions at the start.
pub fn follow(on_snapshot: impl FnMut(&Snapshot)) -> zbus::Result<()> {
    run(|conn| async move { crate::follow(&conn, on_snapshot).await })
}

/// [`follow`], on the bus at `address` (a D-Bus address such as
/// `unix:path=/run/dbus/system_bus_socket`) and with the kill switches of
/// `radios`: what a test points at a stand-in NetworkManager.
///
/// # Errors
///
/// No bus at `address`, or the bus refusing the subscriptions at the start.
pub fn follow_with(
    address: &str,
    radios: Radios,
    on_snapshot: impl FnMut(&Snapshot),
) -> zbus::Result<()> {
    run_on(Some(address), |conn| async move {
        crate::follow_with(&conn, &radios, on_snapshot).await
    })
}
