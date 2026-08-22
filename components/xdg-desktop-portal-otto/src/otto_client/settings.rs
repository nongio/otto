//! Client proxy for `org.otto.Settings`.
//!
//! This module speaks to `org.otto.Settings` (the backend interface
//! exposed by the Otto compositor).

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;
use zbus::{proxy, Result};

/// D-Bus proxy for `org.otto.Settings` service.
#[proxy(
    interface = "org.otto.Settings",
    default_service = "org.otto.Settings",
    default_path = "/org/otto/Settings"
)]
trait OttoSettings {
    /// Get the color scheme preference from the compositor.
    ///
    /// Returns:
    /// - 0: No preference
    /// - 1: Prefer dark appearance
    /// - 2: Prefer light appearance
    async fn get_color_scheme(&self) -> Result<u32>;

    /// Get the icon theme name from the compositor.
    ///
    /// Returns an empty string if no theme is configured.
    async fn get_icon_theme(&self) -> Result<String>;

    /// The XDG sound theme name apps play their event sounds from.
    async fn get_sound_theme(&self) -> Result<String>;

    /// Get the accent colour as sRGB components in `0.0..=1.0`, already in the
    /// shape `org.freedesktop.appearance accent-color` calls for.
    async fn get_accent_color(&self) -> Result<(f64, f64, f64)>;

    /// Emitted whenever any setting's effective value changes.
    ///
    /// Carries the changed identifiers; the values are re-read rather than
    /// taken from the signal, so one code path serves both.
    #[zbus(signal)]
    async fn changed(&self, values: HashMap<String, OwnedValue>) -> Result<()>;
}
