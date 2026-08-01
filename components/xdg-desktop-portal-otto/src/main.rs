//! XDG Desktop Portal backend for Otto.
//!
//! This binary implements the `org.freedesktop.impl.portal.ScreenCast` D-Bus
//! interface, enabling screen sharing through the standard portal API.

use anyhow::Result;
use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::export::futures_util::StreamExt;
use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::ConnectionBuilder;

use xdg_desktop_portal_otto::otto_client::OttoClient;
use xdg_desktop_portal_otto::portal::{
    desktop_path, AccessPortal, ScreenCastPortal, SettingsPortal,
};

/// Well-known D-Bus name for the Otto portal backend.
const DBUS_NAME: &str = "org.freedesktop.impl.portal.desktop.otto";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let connection = ConnectionBuilder::session()?.build().await?;

    let sc_client = OttoClient::new(connection.clone()).await?;
    info!("Connected to D-Bus session bus");

    let screencast_portal = ScreenCastPortal::new(sc_client.clone());
    connection
        .object_server()
        .at(desktop_path(), screencast_portal)
        .await?;

    let settings_portal = SettingsPortal::new(sc_client.clone());
    connection
        .object_server()
        .at(desktop_path(), settings_portal)
        .await?;

    let access_portal = AccessPortal::new(sc_client);
    connection
        .object_server()
        .at(desktop_path(), access_portal)
        .await?;

    // Claim the name only once every interface is exported, and claim it even
    // when someone already holds it. The session bus outlives the graphical
    // session, so a backend left over from an earlier login — or from before an
    // upgrade — otherwise keeps the name forever and every later instance dies
    // on startup while the desktop silently talks to the stale one.
    let dbus = DBusProxy::new(&connection).await?;
    let mut name_lost = dbus.receive_name_lost().await?;
    let reply = dbus
        .request_name(
            DBUS_NAME.try_into()?,
            RequestNameFlags::AllowReplacement
                | RequestNameFlags::ReplaceExisting
                | RequestNameFlags::DoNotQueue,
        )
        .await?;
    if reply != RequestNameReply::PrimaryOwner {
        // Only reachable against an owner that refuses replacement — a backend
        // predating this flag. Say so plainly: the symptom on the desktop is
        // this build's behaviour simply not showing up.
        anyhow::bail!(
            "{DBUS_NAME} is held by another portal backend that refuses replacement \
             ({reply:?}); kill it and start this one again"
        );
    }

    info!(
        name = DBUS_NAME,
        "ScreenCast, Settings and Access portal backends running"
    );

    // Wait for a shutdown signal, or for a newer instance to take the name —
    // staying alive without it would leave a portal nothing can reach.
    loop {
        tokio::select! {
            result = signal::ctrl_c() => {
                result?;
                info!("Shutdown requested");
                return Ok(());
            }
            Some(signal) = name_lost.next() => {
                if signal.args()?.name.as_str() == DBUS_NAME {
                    info!(name = DBUS_NAME, "Name taken over by another instance; exiting");
                    return Ok(());
                }
            }
        }
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
