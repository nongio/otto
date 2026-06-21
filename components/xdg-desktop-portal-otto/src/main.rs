//! XDG Desktop Portal backend for Otto.
//!
//! This binary implements the `org.freedesktop.impl.portal.ScreenCast` D-Bus
//! interface, enabling screen sharing through the standard portal API.

use anyhow::Result;
use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::ConnectionBuilder;

use xdg_desktop_portal_otto::otto_client::OttoClient;
use xdg_desktop_portal_otto::portal::{desktop_path, ScreenCastPortal, SettingsPortal};

/// Well-known D-Bus name for the Otto portal backend.
const DBUS_NAME: &str = "org.freedesktop.impl.portal.desktop.otto";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let connection = ConnectionBuilder::session()?
        .name(DBUS_NAME)?
        .build()
        .await?;

    let sc_client = OttoClient::new(connection.clone()).await?;
    info!("Connected to D-Bus session bus");

    let screencast_portal = ScreenCastPortal::new(sc_client.clone());
    connection
        .object_server()
        .at(desktop_path(), screencast_portal)
        .await?;

    let settings_portal = SettingsPortal::new(sc_client);
    connection
        .object_server()
        .at(desktop_path(), settings_portal)
        .await?;

    info!(
        name = DBUS_NAME,
        "ScreenCast and Settings portal backends running"
    );

    // Wait for shutdown signal
    signal::ctrl_c().await?;
    info!("Shutdown requested");

    Ok(())
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
