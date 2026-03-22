//! D-Bus service implementation for `org.otto.Settings`.
//!
//! Exposes compositor settings like theme color scheme to the portal backend.

use tracing::info;
use zbus::{interface, Connection};

use crate::config::Config;
use crate::theme::ThemeScheme;

/// The main Settings D-Bus interface.
///
/// Implements `org.otto.Settings` at `/org/otto/Settings`.
pub struct SettingsInterface;

#[interface(name = "org.otto.Settings")]
impl SettingsInterface {
    /// Returns the color scheme preference.
    ///
    /// Returns:
    /// - 0: No preference
    /// - 1: Prefer dark appearance
    /// - 2: Prefer light appearance
    async fn get_color_scheme(&self) -> u32 {
        Config::with(|config| match config.theme_scheme {
            ThemeScheme::Dark => 1,
            ThemeScheme::Light => 2,
        })
    }

    /// Returns the configured icon theme name (e.g. "Adwaita", "Papirus").
    ///
    /// Returns an empty string if no theme is configured (auto-detect).
    async fn get_icon_theme(&self) -> String {
        Config::with(|config| {
            config.icon_theme.clone().unwrap_or_default()
        })
    }
}

/// Registers the Settings interface on the existing D-Bus connection.
pub async fn register_settings_interface(connection: &Connection) -> zbus::Result<()> {
    let settings = SettingsInterface;

    connection
        .object_server()
        .at("/org/otto/Settings", settings)
        .await?;

    connection.request_name("org.otto.Settings").await?;

    info!("Settings D-Bus interface registered at org.otto.Settings");

    Ok(())
}
