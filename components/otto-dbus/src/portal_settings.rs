//! `org.freedesktop.portal.Settings`: the frontend interface every toolkit
//! reads desktop settings from.
//!
//! Served by xdg-desktop-portal, which asks the desktop's backend (for Otto,
//! `components/xdg-desktop-portal-otto/src/portal/settings.rs`). otto-kit
//! reads it through one shared client, `portal_settings.rs`.

use zbus::zvariant::{OwnedValue, Value};

/// The well-known bus name.
pub const SERVICE: &str = "org.freedesktop.portal.Desktop";
/// The object path.
pub const PATH: &str = "/org/freedesktop/portal/desktop";

#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
pub trait PortalSettings {
    /// One setting, wrapped in whatever variants the portal puts round it.
    fn read(&self, namespace: &str, key: &str) -> zbus::Result<OwnedValue>;

    /// A setting changed.
    #[zbus(signal)]
    fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;
}
