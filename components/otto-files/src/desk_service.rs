//! `org.otto.Desk1`: what the rest of the desktop can ask the desk.
//!
//! One thing so far: `EditLayout`, which Settings calls from its *Size and
//! position* row to put the desk into edit mode, where the panel is moved and
//! resized with the pointer (see `specs/desk.md`). Only the desk serves this
//! name, and only one desk runs per session, so there is no queue to manage.
//!
//! The call only raises a flag and wakes the UI loop, which enters edit mode
//! on its next pass. Nothing in the UI thread awaits, and the caller is not
//! kept waiting for the person to finish dragging.

// Rust guideline compliant 2026-02-21

use std::sync::atomic::{AtomicBool, Ordering};

use otto_kit::prelude::AppContext;
use zbus::interface;

pub const DBUS_NAME: &str = "org.otto.Desk1";
pub const DBUS_PATH: &str = "/org/otto/Desk1";

/// Set by a call to `EditLayout`, taken by the UI loop.
static EDIT_ASKED: AtomicBool = AtomicBool::new(false);

/// Whether edit mode was asked for since the last call.
pub fn take_edit_request() -> bool {
    EDIT_ASKED.swap(false, Ordering::Relaxed)
}

struct DeskService;

#[interface(name = "org.otto.Desk1")]
impl DeskService {
    /// Show the panel's outline with handles, so it can be moved and resized
    /// with the pointer until Done or Cancel. Returns at once.
    async fn edit_layout(&self) {
        EDIT_ASKED.store(true, Ordering::Relaxed);
        AppContext::request_wakeup();
    }
}

/// Serve the interface and hold the name until the connection dies.
///
/// # Errors
///
/// Fails when there is no session bus, or the object or the name cannot be
/// registered on it.
pub async fn serve() -> zbus::Result<()> {
    let connection = zbus::connection::Builder::session()?
        .serve_at(DBUS_PATH, DeskService)?
        .allow_name_replacements(false)
        .name(DBUS_NAME)?
        .build()
        .await?;
    // Held for the life of the process: dropping the connection gives the
    // name up.
    let _connection = connection;
    std::future::pending::<()>().await;
    Ok(())
}
