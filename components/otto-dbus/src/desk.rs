//! `org.otto.Desk1`: the desktop folder panel.
//!
//! Served by otto-files (`components/otto-files/src/desk_service.rs`).

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Desk1";
/// The object path.
pub const PATH: &str = "/org/otto/Desk1";

#[zbus::proxy(
    interface = "org.otto.Desk1",
    default_service = "org.otto.Desk1",
    default_path = "/org/otto/Desk1"
)]
pub trait Desk {
    /// Show the panel's outline and handles until Done or Cancel. Returns at
    /// once.
    fn edit_layout(&self) -> zbus::Result<()>;
}
