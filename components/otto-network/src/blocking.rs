//! The same calls for a plain thread with no runtime of its own.
//!
//! Each call starts a small single-threaded tokio runtime, connects to the
//! system bus on it, does its work and drops both. That is a few milliseconds
//! a call, which is nothing next to a person clicking. Never call these from
//! inside a tokio runtime: tokio refuses to start a runtime on a thread that
//! already runs one.

use std::future::Future;

use zbus::Connection;

use crate::model::{Snapshot, WifiNetwork};
use crate::profile::{Family, IpConfig};

/// Run `work` with a fresh system bus connection on a runtime of its own.
fn run<T, F, Fut>(work: F) -> zbus::Result<T>
where
    F: FnOnce(Connection) -> Fut,
    Fut: Future<Output = zbus::Result<T>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
    runtime.block_on(async move {
        let conn = Connection::system().await?;
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
pub fn set_ip_config(profile: &str, family: Family, config: &IpConfig) -> zbus::Result<()> {
    run(|conn| async move { crate::set_ip_config(&conn, profile, family, config).await })
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

/// [`crate::watch`], blocking the thread until the bus connection closes.
pub fn watch(changed: impl FnMut()) -> zbus::Result<()> {
    run(|conn| async move { crate::watch(&conn, changed).await })
}
