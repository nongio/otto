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

use crate::versions::Versions;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use otto_kit::prelude::AppContext;
use zbus::fdo;

use crate::edit_queue::Failure;
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

    /// Read the file again, as a change on disk would, and keep it as a
    /// version named by `note`. Refused while the window holds changes not
    /// yet saved, which reading the file again would lose.
    fn reload(&self, path: String, note: String) -> fdo::Result<()> {
        self.change(&path, |viewer| {
            refuse_unsaved(viewer, "read the file again")?;
            viewer.reload();
            viewer.keep_version(Some(&note));
            Ok(())
        })?
    }

    /// The file's versions, as JSON.
    fn versions(&self, path: String) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let viewer = viewer.lock().unwrap();
        Ok(viewer
            .versions
            .as_ref()
            .map(Versions::to_json)
            .unwrap_or_else(|| serde_json::json!({ "versions": [] }))
            .to_string())
    }

    /// Put version `version` back in place of the file. The window follows.
    /// Refused while the window holds changes not yet saved.
    fn revert(&self, path: String, version: u32) -> fdo::Result<()> {
        self.change(&path, |viewer| {
            refuse_unsaved(viewer, "put a version back")?;
            let versions = viewer
                .versions
                .as_mut()
                .ok_or_else(|| fdo::Error::Failed("no versions are kept for this file".into()))?;
            versions
                .revert(version)
                .map_err(|err| fdo::Error::Failed(err.to_string()))
        })?
    }

    /// The PDF's form fields, as JSON, with the `revision` their ids were
    /// read at.
    fn fields(&self, path: String) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let viewer = viewer.lock().unwrap();
        let editing = editing_of(&viewer)?;
        let mut json = crate::pdf_tools::fields_json(&editing.snapshot);
        json["revision"] = editing.numbering.into();
        Ok(json.to_string())
    }

    /// The PDF's annotations, form widgets aside, as JSON, with the
    /// `revision` their ids were read at.
    fn annotations(&self, path: String) -> fdo::Result<String> {
        let viewer = self.viewer(&path)?;
        let viewer = viewer.lock().unwrap();
        let editing = editing_of(&viewer)?;
        let mut json = crate::pdf_tools::annotations_json(&editing.snapshot);
        json["revision"] = editing.numbering.into();
        Ok(json.to_string())
    }

    /// Fill the PDF's form and change its annotations as `request` (JSON)
    /// says, and show it; the changes stay in memory until the person saves.
    /// They wait their turn behind the person's own edits. Returns the
    /// fields and annotations as they are after, with their revision.
    ///
    /// The request's `revision`, from `Fields`, `Annotations` or an earlier
    /// `Edit`, says which document its ids were read from, so each is
    /// followed to the same field or annotation if the person has deleted
    /// or undone something since; ids that can't be followed, or a revision
    /// too old to be remembered, are refused as stale. Without one, the ids
    /// are taken as naming the document as it is when the call arrives.
    async fn edit(&self, path: String, request: String) -> fdo::Result<String> {
        let request: serde_json::Value = serde_json::from_str(&request)
            .map_err(|err| fdo::Error::InvalidArgs(format!("not JSON: {err}")))?;
        let revision = match request.get("revision") {
            None | Some(serde_json::Value::Null) => None,
            Some(revision) => Some(revision.as_u64().ok_or_else(|| {
                fdo::Error::InvalidArgs(
                    "revision is the number preview_fields or preview_annotations gave".into(),
                )
            })?),
        };
        let viewer = self.viewer(&path)?;
        let (reply, answer) = tokio::sync::oneshot::channel();
        {
            let mut viewer = viewer.lock().unwrap();
            let editing = editing_of(&viewer)?;
            let base = match revision {
                None => editing.base(),
                Some(revision) => editing
                    .base_at(revision)
                    .ok_or_else(|| fdo::Error::Failed(Failure::Stale.to_string()))?,
            };
            let edits = crate::pdf_tools::edits_from_json(&base.snapshot, &request)
                .map_err(fdo::Error::InvalidArgs)?;
            if !viewer.submit_on(edits, base, Some(reply)) {
                return Err(fdo::Error::Failed(
                    "the document's queue has stopped".into(),
                ));
            }
        }
        AppContext::request_wakeup();
        let outcome = answer
            .await
            .map_err(|_| fdo::Error::Failed("the window closed before the edit was made".into()))?;
        outcome
            .result
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        let snapshot = outcome.snapshot;
        Ok(serde_json::json!({
            "saved": false,
            "note": "Shown in the window, not yet saved: the person saves the file.",
            "revision": outcome.numbering,
            "fields": crate::pdf_tools::fields_json(&snapshot)["fields"],
            "annotations": crate::pdf_tools::annotations_json(&snapshot)["annotations"],
        })
        .to_string())
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

/// Refuse to `what` while `viewer` holds changes not yet saved: the agent
/// asks the person to save (or undo) them first.
fn refuse_unsaved(viewer: &Viewer, what: &str) -> fdo::Result<()> {
    if viewer.edits_pending() {
        return Err(fdo::Error::Failed(format!(
            "can't {what}: the window holds changes not yet saved, which this would \
             throw away; ask the person to save them (or undo them) first"
        )));
    }
    Ok(())
}

/// The window's document opened for editing, or why it isn't.
fn editing_of(viewer: &Viewer) -> fdo::Result<&crate::editing::Editing> {
    viewer.editing.as_ref().ok_or_else(|| {
        let why = if otto_pdf::available() {
            "the window doesn't show a PDF that can be edited (or it is still opening)"
        } else {
            "PDF editing isn't installed: libpdfium.so is missing"
        };
        fdo::Error::NotSupported(why.into())
    })
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
