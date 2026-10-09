//! `org.otto.Files1`: what a Files window has selected.
//!
//! Served by otto-files (`components/otto-files/src/files_service.rs`). Each
//! window is a process of its own queued for [`SERVICE`], so a caller lists
//! the queued owners and asks each by its unique name.

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.Files1";
/// The object path.
pub const PATH: &str = "/org/otto/Files1";

#[zbus::proxy(
    interface = "org.otto.Files1",
    default_service = "org.otto.Files1",
    default_path = "/org/otto/Files1"
)]
pub trait Files {
    /// The selected paths of the window that has the keyboard, in order.
    /// Every other window answers with `org.freedesktop.DBus.Error.Failed`
    /// rather than an empty list, so a caller can tell "nothing selected"
    /// from "not the focused window" (otto-stash relies on that).
    fn focused_selection(&self) -> zbus::Result<Vec<String>>;
}
