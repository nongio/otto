//! `org.otto.Compositor`: small requests to the compositor itself.
//!
//! Served by the compositor (`src/screenshare/dbus_service.rs`).

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Compositor";
/// The object path.
pub const PATH: &str = "/org/otto/Compositor";

#[zbus::proxy(
    interface = "org.otto.Compositor",
    default_service = "org.otto.Compositor",
    default_path = "/org/otto/Compositor"
)]
pub trait Compositor {
    /// Answers while the compositor is up.
    fn ping(&self) -> zbus::Result<String>;

    /// Bring `app_id`'s window forward; whether there was one.
    fn focus_app(&self, app_id: &str) -> zbus::Result<bool>;

    /// Version, backend, renderer and outputs for a bug report, as a JSON
    /// object (see `docs/developer/compositor-dbus-api.md`).
    fn debug_info(&self) -> zbus::Result<String>;
}
