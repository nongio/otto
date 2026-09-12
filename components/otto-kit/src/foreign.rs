//! Naming another process's window as this one's parent.
//!
//! A portal dialog — a file chooser, an access prompt — is served by a
//! different process from the application that asked for it. The request
//! carries a handle the application exported with `zxdg_exporter_v2`, written
//! `wayland:<handle>`; importing it here gives a surface this process can pass
//! to `xdg_toplevel.set_parent`, so the compositor knows the dialog belongs to
//! that window. A window with a parent is a dialog: it stacks above its
//! parent, and Otto floats it rather than tiling it (`specs/tiling.md`,
//! *Floating within a tiling workspace*).
//!
//! The imported object has to outlive the call — destroying it unsets the
//! parent again — so it is kept here for the life of the process. There is at
//! most one parent per window, and a portal process opens few windows.

use std::sync::{Mutex, OnceLock};

use wayland_client::{protocol::wl_surface::WlSurface, Proxy};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_imported_v2::ZxdgImportedV2;

use crate::app_runner::AppContext;

/// The prefix the portal protocols put in front of a Wayland handle. An X11
/// parent is written `x11:<xid>` and is not ours to import.
const PREFIX: &str = "wayland:";

fn imported() -> &'static Mutex<Vec<ZxdgImportedV2>> {
    static IMPORTED: OnceLock<Mutex<Vec<ZxdgImportedV2>>> = OnceLock::new();
    IMPORTED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Make `surface`'s toplevel a child of the window `handle` was exported
/// from.
///
/// `handle` is the portal's `parent_window` string. Returns false — and
/// changes nothing — when it is empty, is not a Wayland handle, or the
/// compositor offers no `zxdg_importer_v2`; the caller then has to fall back
/// to saying "I am a dialog" some other way.
pub fn set_parent_from_handle(handle: &str, surface: &WlSurface) -> bool {
    let Some(handle) = handle.strip_prefix(PREFIX).filter(|h| !h.is_empty()) else {
        return false;
    };
    let Some(importer) = AppContext::xdg_importer() else {
        tracing::debug!("no zxdg_importer_v2; cannot adopt a foreign parent");
        return false;
    };
    let qh = AppContext::queue_handle();
    let object = importer.import_toplevel(handle.to_string(), qh, ());
    object.set_parent_of(surface);
    imported().lock().unwrap().push(object);
    true
}

/// The exporting window has gone; drop the imported object with it.
pub(crate) fn forget_imported(object: &ZxdgImportedV2) {
    if let Ok(mut list) = imported().lock() {
        list.retain(|held| held.id() != object.id());
    }
}
