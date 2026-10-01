//! Which apps may notify, in xdg-permission-store's `notifications` table.
//!
//! xdg-desktop-portal keeps that table for apps that notify through its
//! Notification interface (Flatpak apps), and drops their notifications while
//! it says `no`. Every other app calls this daemon directly, so the daemon
//! keeps the same table for them: an app is recorded as allowed the first
//! time it notifies, and its notifications are dropped while Settings ›
//! Privacy has it switched off.
//!
//! The app is named by the process that made the call, not by what the
//! notification says: a Flatpak app by its sandbox's app id, any other app by
//! its `desktop-entry` hint or the desktop entry that runs its executable.
//! Notifications forwarded by a portal backend were checked by the portal and
//! are let through.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::zvariant::OwnedValue;

const STORE_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const STORE_INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const TABLE: &str = "notifications";
const ID: &str = "notification";

/// Who sent a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sender {
    /// An app, by the id its row in the table has.
    App(String),
    /// A portal backend passing on a notification the portal already let
    /// through.
    Portal,
    /// Nothing says which app this is.
    Unknown,
}

/// Name the app behind a `Notify` call.
pub async fn identify(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
    desktop_entry: Option<&str>,
) -> Sender {
    let hinted = desktop_entry
        .map(|entry| entry.trim_end_matches(".desktop"))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string);
    let Some(pid) = sender_pid(connection, header).await else {
        return hinted.map_or(Sender::Unknown, Sender::App);
    };
    if let Some(app) = otto_kit::process_app::flatpak_app(pid) {
        return Sender::App(app);
    }
    let program = otto_kit::process_app::program(pid);
    if program
        .as_deref()
        .is_some_and(|program| program.starts_with("xdg-desktop-portal"))
    {
        return Sender::Portal;
    }
    if let Some(app) = hinted {
        return Sender::App(app);
    }
    match program {
        Some(program) => Sender::App(
            otto_kit::desktop_entry::lookup_app_by_binary(&program)
                .and_then(|info| info.desktop_file_id)
                .unwrap_or(program),
        ),
        None => Sender::Unknown,
    }
}

async fn sender_pid(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<u32> {
    let sender = header.sender()?.to_owned();
    zbus::fdo::DBusProxy::new(connection)
        .await
        .ok()?
        .get_connection_unix_process_id(sender.into())
        .await
        .ok()
}

/// Whether `app` may notify. An app the table has never seen is recorded as
/// allowed, so it can be switched off. When the store cannot be read the
/// answer is yes: a broken store must not silence every app.
pub async fn allows(connection: &zbus::Connection, app: &str) -> bool {
    let reply = connection
        .call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "Lookup",
            &(TABLE, ID),
        )
        .await;
    let apps: HashMap<String, Vec<String>> = match reply {
        Ok(reply) => match reply
            .body()
            .deserialize::<(HashMap<String, Vec<String>>, OwnedValue)>()
        {
            Ok((apps, _)) => apps,
            Err(err) => {
                tracing::warn!(%err, "unreadable notifications entry; letting it through");
                return true;
            }
        },
        // No entry yet: nobody has been recorded.
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with(".NotFound") => {
            HashMap::new()
        }
        Err(err) => {
            tracing::warn!(%err, "cannot read the permission store; letting it through");
            return true;
        }
    };
    if let Some(permissions) = apps.get(app) {
        return !permissions.iter().any(|permission| permission == "no");
    }
    let recorded = connection
        .call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "SetPermission",
            &(TABLE, true, ID, app, vec!["yes"]),
        )
        .await;
    if let Err(err) = recorded {
        tracing::warn!(%err, app, "cannot record the app as allowed to notify");
    }
    true
}

/// An id for a notification that was dropped. The caller gets one as it
/// would for any notification; it names nothing on screen, and the range is
/// far from the daemon's own.
pub fn dropped_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0x8000_0000);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
