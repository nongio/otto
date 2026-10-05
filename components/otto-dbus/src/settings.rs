//! `org.otto.Settings`: the compositor's settings.
//!
//! Served by the compositor (`src/settings_service.rs`); the contract is
//! `docs/developer/settings-dbus-api.md`.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Settings";
/// The object path.
pub const PATH: &str = "/org/otto/Settings";

#[zbus::proxy(
    interface = "org.otto.Settings",
    default_service = "org.otto.Settings",
    default_path = "/org/otto/Settings"
)]
pub trait Settings {
    /// `0` no preference, `1` prefer dark, `2` prefer light.
    fn get_color_scheme(&self) -> zbus::Result<u32>;

    /// The accent as sRGB components in `0.0..=1.0`.
    fn get_accent_color(&self) -> zbus::Result<(f64, f64, f64)>;

    /// The icon theme name; empty when none is configured.
    fn get_icon_theme(&self) -> zbus::Result<String>;

    /// The XDG sound theme name.
    fn get_sound_theme(&self) -> zbus::Result<String>;

    /// The preferred locales, most preferred first.
    fn get_locales(&self) -> zbus::Result<Vec<String>>;

    /// The schema: one dictionary per setting.
    fn describe(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// Every setting's effective value.
    fn get_all(&self) -> zbus::Result<HashMap<String, OwnedValue>>;

    /// One setting's effective value, by its identifier.
    fn get(&self, id: &str) -> zbus::Result<OwnedValue>;

    /// The identifiers the user's configuration sets.
    fn get_overridden(&self) -> zbus::Result<Vec<String>>;

    /// Set a setting; answers `applied` or `pending-restart`.
    fn set(&self, id: &str, value: &Value<'_>) -> zbus::Result<String>;

    /// Drop the user's value for a setting; answers like [`set`](Self::set).
    fn reset(&self, id: &str) -> zbus::Result<String>;

    /// Every keyboard shortcut in force, as `(trigger, action)`.
    fn list_shortcuts(&self) -> zbus::Result<Vec<(String, String)>>;

    /// The outputs, one dictionary each.
    fn list_outputs(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    /// The file a changed setting is written to.
    fn config_path(&self) -> zbus::Result<String>;

    /// Store a display profile for `connector`.
    #[allow(clippy::too_many_arguments)]
    fn set_output_profile(
        &self,
        connector: &str,
        width: u32,
        height: u32,
        refresh_hz: f64,
        x: i32,
        y: i32,
        primary: bool,
    ) -> zbus::Result<String>;

    /// Create a virtual output; answers its PipeWire node id.
    fn add_virtual_output(
        &self,
        name: &str,
        width: u32,
        height: u32,
        refresh_hz: f64,
        interactive: bool,
        persist: bool,
    ) -> zbus::Result<u32>;

    /// Remove a virtual output, and drop it from the configuration.
    fn remove_virtual_output(&self, name: &str) -> zbus::Result<()>;

    /// Emitted after any effective value changes, from any source, with the
    /// new values by identifier.
    #[zbus(signal)]
    fn changed(&self, values: HashMap<String, OwnedValue>) -> zbus::Result<()>;
}
