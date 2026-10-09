//! One Preview per session: the first to start owns `org.otto.Preview1`, and
//! every later start hands its file to that one and leaves.
//!
//! The owner opens the file in a window of its own, or brings forward the
//! window already showing it. Requests land in an [`Inbox`] the UI loop
//! drains, woken through otto-kit like every other piece of work that
//! arrives off the UI thread.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_kit::prelude::AppContext;

const DBUS_NAME: &str = "org.otto.Preview1";
const DBUS_PATH: &str = "/org/otto/Preview1";

/// A file asked for, with the activation token its launcher handed over.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub path: PathBuf,
    pub token: Option<String>,
}

/// Requests from other starts, waiting for the UI loop.
pub type Inbox = Arc<Mutex<Vec<Request>>>;

struct Service {
    inbox: Inbox,
}

#[zbus::interface(name = "org.otto.Preview1")]
impl Service {
    /// Open `path`, an absolute path, or bring forward the window showing
    /// it. `token` is an xdg-activation token, empty when there is none.
    fn open(&self, path: String, token: String) {
        let token = (!token.is_empty()).then_some(token);
        self.inbox.lock().unwrap().push(Request {
            path: PathBuf::from(path),
            token,
        });
        AppContext::request_wakeup();
    }
}

/// Who this start turned out to be.
pub enum Role {
    /// The owner: serves requests for as long as the connection is held.
    Owner(zbus::Connection),
    /// Another Preview owns the name and has been handed the file.
    Forwarded,
}

/// Claim the bus name, or hand `request` to the Preview that holds it.
///
/// The name is claimed without replacement: a Preview still running keeps
/// its windows, and one that has exited no longer holds it. An error means
/// no session bus, and the caller carries on alone.
pub async fn claim_or_forward(request: &Request, inbox: Inbox) -> zbus::Result<Role> {
    use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

    let connection = zbus::connection::Builder::session()?.build().await?;
    // Served before the name is claimed, so no request can reach the name
    // ahead of the object.
    connection
        .object_server()
        .at(DBUS_PATH, Service { inbox })
        .await?;
    let reply = DBusProxy::new(&connection)
        .await?
        .request_name(DBUS_NAME.try_into()?, RequestNameFlags::DoNotQueue.into())
        .await?;
    if reply == RequestNameReply::PrimaryOwner {
        tracing::info!(name = DBUS_NAME, "preview service running");
        return Ok(Role::Owner(connection));
    }

    let path = request.path.to_string_lossy().into_owned();
    let token = request.token.clone().unwrap_or_default();
    connection
        .call_method(
            Some(DBUS_NAME),
            DBUS_PATH,
            Some(DBUS_NAME),
            "Open",
            &(path, token),
        )
        .await?;
    Ok(Role::Forwarded)
}
