//! One Preview per session: the first to start owns `org.otto.Preview1`, and
//! every later start hands its file to that one and leaves.
//!
//! The owner opens the file in a window of its own, or brings forward the
//! window already showing it. Requests land in an [`Inbox`] the UI loop
//! drains, woken through otto-kit like every other piece of work that
//! arrives off the UI thread.
//!
//! The same interface serves the document tools an agent reaches through
//! `otto-preview --mcp` (see [`crate::mcp`]): what a window shows, its marks,
//! drawing back on it, reloading it, and the view as a picture. Each takes
//! the file's path and acts on the window showing it, through the viewer the
//! window shares with its draw; the window repaints on its next turn.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use otto_kit::prelude::AppContext;
use zbus::fdo;

use crate::viewer::Viewer;

pub const DBUS_NAME: &str = "org.otto.Preview1";
pub const DBUS_PATH: &str = "/org/otto/Preview1";

/// A file asked for, with the activation token its launcher handed over.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub path: PathBuf,
    pub token: Option<String>,
    /// Whether the chat shows beside the file from the start.
    pub chat: bool,
    /// The agent session to carry on in that chat, rather than a new one:
    /// one this app started about the file, picked from a list.
    pub session: Option<String>,
}

/// Requests from other starts, waiting for the UI loop.
pub type Inbox = Arc<Mutex<Vec<Request>>>;

/// The open windows' viewers, by the file's resolved path.
pub type Documents = Arc<Mutex<HashMap<PathBuf, Arc<Mutex<Viewer>>>>>;

struct Service {
    inbox: Inbox,
    documents: Documents,
}

impl Service {
    /// The viewer of the window showing `path`.
    fn viewer(&self, path: &str) -> fdo::Result<Arc<Mutex<Viewer>>> {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        self.documents
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| fdo::Error::FileNotFound(format!("no Preview window shows {path}")))
    }

    /// Run `change` on the viewer of `path`, then have the window repaint.
    fn change<T>(&self, path: &str, change: impl FnOnce(&mut Viewer) -> T) -> fdo::Result<T> {
        let viewer = self.viewer(path)?;
        let out = {
            let mut viewer = viewer.lock().unwrap();
            let out = change(&mut viewer);
            viewer.dirty = true;
            out
        };
        AppContext::request_wakeup();
        Ok(out)
    }
}

#[zbus::interface(name = "org.otto.Preview1")]
impl Service {
    /// Open `path`, an absolute path, or bring forward the window showing
    /// it. `token` is an xdg-activation token, empty when there is none.
    fn open(&self, path: String, token: String) {
        self.push(path, token, false);
    }

    /// As `Open`, with the chat showing beside the file.
    fn open_chat(&self, path: String, token: String) {
        self.push(path, token, true);
    }

    /// As `OpenChat`, carrying on the agent session `session` in the chat.
    fn open_session(&self, path: String, session: String, token: String) {
        let token = (!token.is_empty()).then_some(token);
        self.inbox.lock().unwrap().push(Request {
            path: PathBuf::from(path),
            token,
            chat: true,
            session: Some(session),
        });
        AppContext::request_wakeup();
    }

    /// What the window showing `path` shows, as JSON.
    fn info(&self, path: String) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let info = viewer.lock().unwrap().info();
        Ok(info.to_string())
    }

    /// Every mark on the document, the person's and the agent's, as JSON.
    fn marks(&self, path: String) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let viewer = viewer.lock().unwrap();
        let size = crate::marks::picture_size(&viewer.session.preview);
        Ok(viewer.marks.to_json(&viewer.path, size, false).to_string())
    }

    /// Replace the agent's layer `layer` with `marks`, a JSON list. Returns
    /// how many were drawn.
    fn draw(&self, path: String, layer: String, marks: String) -> fdo::Result<u32> {
        let marks: serde_json::Value = serde_json::from_str(&marks)
            .map_err(|err| fdo::Error::InvalidArgs(format!("marks are not JSON: {err}")))?;
        let layer = if layer.is_empty() {
            "agent".to_owned()
        } else {
            layer
        };
        self.change(&path, |viewer| {
            // What the agent points at is shown, even with the marks hidden.
            viewer.marks_hidden = false;
            viewer.marks.draw_agent(&layer, &marks)
        })?
        .map(|count| count as u32)
        .map_err(fdo::Error::InvalidArgs)
    }

    /// Take away the agent's layer `layer`, every agent mark for an empty
    /// one, or the person's marks for `person`. Returns how many went.
    fn clear(&self, path: String, layer: String) -> fdo::Result<u32> {
        self.change(&path, |viewer| {
            let gone = if layer == "person" {
                viewer.marks.clear_person()
            } else {
                viewer.marks.clear_layer(&layer)
            };
            gone as u32
        })
    }

    /// Read the file again and show it, as a change on disk would.
    fn reload(&self, path: String) -> fdo::Result<()> {
        self.change(&path, Viewer::reload)
    }

    /// The picture as the window shows it, with the marks drawn when `marks`,
    /// written as PNG. Returns the PNG's path.
    fn render(&self, path: String, marks: bool) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let png = {
            let viewer = viewer.lock().unwrap();
            let list: &[crate::marks::Mark] = if marks { &viewer.marks.list } else { &[] };
            crate::marks::render(&viewer.session.preview, list)
        }
        .ok_or_else(|| fdo::Error::NotSupported("only a picture can be rendered so far".into()))?;
        write_render(&png).map_err(|err| fdo::Error::IOError(err.to_string()))
    }
}

/// Write a rendered view where the agent can read it.
fn write_render(png: &[u8]) -> std::io::Result<String> {
    let dir = crate::marks::dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or_default();
    let path = dir.join(format!("view-{stamp}.png"));
    std::fs::write(&path, png)?;
    Ok(path_string(&path))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Service {
    fn push(&self, path: String, token: String, chat: bool) {
        let token = (!token.is_empty()).then_some(token);
        self.inbox.lock().unwrap().push(Request {
            path: PathBuf::from(path),
            token,
            chat,
            session: None,
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
pub async fn claim_or_forward(
    request: &Request,
    inbox: Inbox,
    documents: Documents,
) -> zbus::Result<Role> {
    use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

    let connection = zbus::connection::Builder::session()?.build().await?;
    // Served before the name is claimed, so no request can reach the name
    // ahead of the object.
    connection
        .object_server()
        .at(DBUS_PATH, Service { inbox, documents })
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
    match &request.session {
        Some(session) => {
            connection
                .call_method(
                    Some(DBUS_NAME),
                    DBUS_PATH,
                    Some(DBUS_NAME),
                    "OpenSession",
                    &(path, session.as_str(), token),
                )
                .await?
        }
        None => {
            connection
                .call_method(
                    Some(DBUS_NAME),
                    DBUS_PATH,
                    Some(DBUS_NAME),
                    if request.chat { "OpenChat" } else { "Open" },
                    &(path, token),
                )
                .await?
        }
    };
    Ok(Role::Forwarded)
}
