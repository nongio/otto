//! `org.freedesktop.impl.portal.Access`: the standard permission dialog a
//! portal backend presents.
//!
//! Served by other desktops' portal backends (GTK, GNOME, KDE), which
//! xdg-desktop-portal-otto falls back to when otto-islands is not running.
//! Otto's own backend serves it too
//! (`components/xdg-desktop-portal-otto/src/portal/access.rs`).
//!
//! There is no default service: each backend has a bus name of its own, so
//! the caller picks the destination per call.

use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedValue, Value};

/// The object path every backend serves it at.
pub const PATH: &str = "/org/freedesktop/portal/desktop";

#[zbus::proxy(
    interface = "org.freedesktop.impl.portal.Access",
    default_path = "/org/freedesktop/portal/desktop"
)]
pub trait PortalAccess {
    /// Present a dialog and block until the user answers.
    ///
    /// `options` carries `modal`, `grant_label`, `deny_label`, `icon` and
    /// `choices`; `results` carries the selected `choices` back.
    #[allow(clippy::too_many_arguments)]
    fn access_dialog(
        &self,
        handle: &ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<(u32, HashMap<String, OwnedValue>)>;
}
