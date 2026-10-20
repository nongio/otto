//! The one client of the desktop portal's `org.freedesktop.portal.FileChooser`
//! in otto-kit: open, folder and save dialogs.
//!
//! This goes through the **frontend** (`org.freedesktop.portal.Desktop`), the
//! same door any other application uses, not straight to Otto's backend. That
//! exercises the whole chain, including the `portals.conf` routing that
//! decides Otto's picker serves `FileChooser` at all; calling the backend
//! directly would hide a misrouted frontend.
//!
//! A [`Request`] names the dialog: its title, the window it belongs to, its
//! filters and, for a save, the name and folder it starts on. [`Request::ask`]
//! blocks for as long as the dialog is up, minutes if the user browses, so an
//! app calls [`Request::spawn`] instead: the dialog runs on a thread of its
//! own, the answer is handed to a closure there, and the run loop is woken
//! afterwards whatever the answer was. `spawn` hands back a [`Cancel`], which
//! takes the dialog down again when the window that asked goes first.
//!
//! Every request carries a handle token of its own. The frontend derives the
//! request's object path from the caller's unique bus name and that token, so
//! the answer is listened for before the call and cannot be missed, and two
//! dialogs asked one after the other never share an answer. A frontend that
//! returns some other handle is followed to it. The wait also ends when the
//! frontend leaves the bus, which would otherwise leave it waiting forever.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

/// One rule of a [`Request::filter`]: what a file must match to be offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// A shell glob on the name, such as `*.png`.
    Glob(String),
    /// A MIME type, such as `image/png`.
    Mime(String),
}

impl Rule {
    /// The rule as the portal takes it: `(kind, pattern)`, `kind` `0` a glob
    /// and `1` a MIME type.
    fn wire(&self) -> (u32, String) {
        match self {
            Rule::Glob(glob) => (0, glob.clone()),
            Rule::Mime(mime) => (1, mime.clone()),
        }
    }
}

/// What a finished request came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The user chose something: absolute local paths, in the order the
    /// portal gave them. Never empty.
    Chosen(Vec<PathBuf>),
    /// The user cancelled, or the app withdrew the dialog through [`Cancel`].
    Dismissed,
    /// The request never got as far as a dialog, the portal ended it without
    /// an answer, or its answer was unusable.
    Failed(String),
}

/// Takes down a dialog [`Request::spawn`] put up: the portal is asked to
/// close the request, and the answer handed over is [`Outcome::Dismissed`].
///
/// Dropping it does nothing; the dialog stays until it is answered. Cancelling
/// a dialog that has already been answered does nothing either.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<CancelState>);

#[derive(Debug, Default)]
struct CancelState {
    cancelled: AtomicBool,
    /// Wakes the dialog's thread. Keeps its permit when nobody waits yet, so
    /// a cancel that lands before the wait begins is not lost.
    wake: Notify,
}

impl Cancel {
    /// Take the dialog down. Returns at once: the dialog's thread asks the
    /// portal to close the request.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
        self.0.wake.notify_one();
    }

    fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }

    async fn cancelled(&self) {
        self.0.wake.notified().await;
    }
}

/// Which of the portal's methods a request calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    OpenFile,
    SaveFile,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::OpenFile => "OpenFile",
            Method::SaveFile => "SaveFile",
        }
    }
}

/// A file dialog to put up through the portal.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    title: String,
    parent: String,
    filters: Vec<(String, Vec<(u32, String)>)>,
    directory: bool,
    current_name: Option<String>,
    current_folder: Option<PathBuf>,
}

impl Request {
    /// A dialog asking which file to open, titled `title`.
    pub fn open_file(title: impl Into<String>) -> Self {
        Self::new(Method::OpenFile, title.into())
    }

    /// A dialog asking for a folder rather than a file, titled `title`.
    pub fn open_folder(title: impl Into<String>) -> Self {
        let mut request = Self::new(Method::OpenFile, title.into());
        request.directory = true;
        request
    }

    /// A dialog asking where to save a file, titled `title`.
    pub fn save_file(title: impl Into<String>) -> Self {
        Self::new(Method::SaveFile, title.into())
    }

    fn new(method: Method, title: String) -> Self {
        Self {
            method,
            title,
            parent: String::new(),
            filters: Vec::new(),
            directory: false,
            current_name: None,
            current_folder: None,
        }
    }

    /// Make the dialog one of the window `parent` names (`wayland:<handle>`,
    /// as xdg-foreign exports it). Without one the dialog has no parent.
    pub fn parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = parent.into();
        self
    }

    /// Offer a filter labelled `label`, matching a file that matches any of
    /// `rules`. The first filter added is the one selected.
    pub fn filter(mut self, label: impl Into<String>, rules: &[Rule]) -> Self {
        self.filters
            .push((label.into(), rules.iter().map(Rule::wire).collect()));
        self
    }

    /// The name a save dialog suggests.
    pub fn current_name(mut self, name: impl Into<String>) -> Self {
        self.current_name = Some(name.into());
        self
    }

    /// The folder a save dialog starts in.
    pub fn current_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        self.current_folder = Some(folder.into());
        self
    }

    /// Put the dialog up on a thread of its own, hand its answer to
    /// `on_answer` there, then wake the run loop: for every outcome, so a
    /// cancelled or failed dialog is noticed as promptly as a chosen file.
    ///
    /// The [`Cancel`] it returns takes the dialog down, for when the window
    /// that asked closes first.
    ///
    /// # Errors
    ///
    /// When the thread cannot be started; `on_answer` is then never called.
    pub fn spawn(
        self,
        on_answer: impl FnOnce(Outcome) + Send + 'static,
    ) -> std::io::Result<Cancel> {
        let cancel = Cancel::default();
        let watched = cancel.clone();
        std::thread::Builder::new()
            .name("file-chooser".into())
            .spawn(move || {
                on_answer(self.ask_until(&watched));
                crate::app_runner::AppContext::request_wakeup();
            })
            .map(|_| cancel)
    }

    /// Put the dialog up and block until it is answered.
    ///
    /// There is no timeout on purpose: the user may reasonably take minutes,
    /// and a timeout would leave the dialog on screen answering to nobody.
    /// Never call this on the thread that draws, nor inside an async
    /// runtime; see [`Request::spawn`].
    pub fn ask(self) -> Outcome {
        self.ask_until(&Cancel::default())
    }

    /// [`Request::ask`], given up when `cancel` fires.
    fn ask_until(self, cancel: &Cancel) -> Outcome {
        if cancel.is_cancelled() {
            return Outcome::Dismissed;
        }
        // A runtime of the dialog's own, alive for exactly as long as the
        // dialog: the connection's tasks run on it while it waits.
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => return Outcome::Failed(format!("no runtime for the dialog: {err}")),
        };
        runtime.block_on(self.ask_async(cancel))
    }

    async fn ask_async(self, cancel: &Cancel) -> Outcome {
        let connection = match Connection::session().await {
            Ok(connection) => connection,
            Err(err) => return Outcome::Failed(format!("no session bus: {err}")),
        };
        let Some(unique) = connection.unique_name().map(|name| name.to_string()) else {
            return Outcome::Failed("no unique bus name".into());
        };
        // Watched from before the call, so a portal that dies while it is
        // being asked is not missed either.
        let mut owner_changes = match portal_owner_changes(&connection).await {
            Ok(changes) => changes,
            Err(err) => return Outcome::Failed(format!("cannot watch the portal: {err}")),
        };
        let token = next_token();
        let mut path = request_path(&unique, &token);
        let mut responses = match responses_at(&connection, &path).await {
            Ok(responses) => responses,
            Err(err) => return Outcome::Failed(format!("cannot watch the request: {err}")),
        };
        let portal =
            match zbus::Proxy::new(&connection, PORTAL_NAME, PORTAL_PATH, FILE_CHOOSER).await {
                Ok(proxy) => proxy,
                Err(err) => return Outcome::Failed(format!("no portal: {err}")),
            };

        let method = self.method.name();
        let options = self.options(token);
        // Owned, because a borrowed `ObjectPath` cannot outlive the reply it
        // was deserialised from.
        let handle: zbus::Result<OwnedObjectPath> = portal
            .call(
                method,
                &(self.parent.as_str(), self.title.as_str(), options),
            )
            .await;
        let handle = match handle {
            Ok(handle) => handle,
            Err(err) => return Outcome::Failed(format!("{method} failed: {err}")),
        };
        // A frontend that ignores the handle token picks the path itself, and
        // answers there. An answer sent before this second subscription is
        // lost, which nothing on this side can prevent.
        if handle.as_str() != path {
            path = handle.as_str().to_owned();
            responses = match responses_at(&connection, &path).await {
                Ok(responses) => responses,
                Err(err) => return Outcome::Failed(format!("cannot watch the request: {err}")),
            };
        }

        loop {
            tokio::select! {
                message = responses.next() => {
                    let Some(message) = message else {
                        return Outcome::Failed("the portal closed without answering".into());
                    };
                    return match message.body().deserialize() {
                        Ok((response, results)) => outcome(response, &results),
                        Err(err) => Outcome::Failed(format!("malformed Response: {err}")),
                    };
                }
                change = owner_changes.next() => {
                    let Some(change) = change else {
                        return Outcome::Failed("lost track of the portal".into());
                    };
                    // The portal gaining an owner (activated by the call) is
                    // no news; losing it is the end of the request.
                    let lost = change
                        .args()
                        .map(|args| args.new_owner().is_none())
                        .unwrap_or(false);
                    if lost {
                        return Outcome::Failed(
                            "the portal left the bus without answering".into(),
                        );
                    }
                }
                () = cancel.cancelled() => {
                    close_request(&connection, &path).await;
                    return Outcome::Dismissed;
                }
            }
        }
    }

    /// The call's `a{sv}` options, carrying `token` as the handle token.
    fn options(&self, token: String) -> HashMap<&'static str, Value<'static>> {
        let mut options: HashMap<&'static str, Value<'static>> = HashMap::new();
        options.insert("handle_token", Value::from(token));
        options.insert("modal", Value::from(true));
        if self.directory {
            options.insert("directory", Value::from(true));
        }
        if !self.filters.is_empty() {
            options.insert("filters", Value::from(self.filters.clone()));
        }
        if let Some(name) = &self.current_name {
            options.insert("current_name", Value::from(name.clone()));
        }
        if let Some(folder) = &self.current_folder {
            options.insert("current_folder", Value::from(folder_bytes(folder)));
        }
        options
    }
}

/// The `Response` signals of the request at `path`.
async fn responses_at(
    connection: &Connection,
    path: &str,
) -> zbus::Result<zbus::proxy::SignalStream<'static>> {
    zbus::Proxy::new_owned(connection.clone(), PORTAL_NAME, path.to_owned(), REQUEST)
        .await?
        .receive_signal("Response")
        .await
}

/// Changes of who owns the portal's bus name.
async fn portal_owner_changes(
    connection: &Connection,
) -> zbus::Result<zbus::fdo::NameOwnerChangedStream> {
    zbus::fdo::DBusProxy::new(connection)
        .await?
        .receive_name_owner_changed_with_args(&[(0, PORTAL_NAME)])
        .await
}

/// Ask the portal to close the request at `path`, which takes its dialog
/// down. The portal sends no `Response` for a closed request.
async fn close_request(connection: &Connection, path: &str) {
    let closed = connection
        .call_method(Some(PORTAL_NAME), path, Some(REQUEST), "Close", &())
        .await;
    if let Err(err) = closed {
        tracing::warn!(%err, path, "could not close the file dialog's request");
    }
}

/// A handle token no other request from this process has used.
fn next_token() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    token(std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The handle token for request number `serial` of process `pid`. Tokens may
/// hold only `[A-Za-z0-9_]`: they become an object path element.
fn token(pid: u32, serial: u64) -> String {
    format!("otto_kit_{pid}_{serial}")
}

/// The object path the frontend gives a request, from the caller's unique
/// bus name (`:1.42`) and the request's handle token.
fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

/// A folder as the portal takes it: its bytes, ending in a NUL.
fn folder_bytes(folder: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = folder.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// What a `Response` signal's `(response, results)` amount to.
fn outcome(response: u32, results: &HashMap<String, OwnedValue>) -> Outcome {
    match response {
        0 => {}
        1 => return Outcome::Dismissed,
        // `2` is the request ending some other way: the picker unreachable
        // or dead, the request refused. Not the user's choice.
        2 => return Outcome::Failed("the portal ended the request".into()),
        other => return Outcome::Failed(format!("unknown portal response {other}")),
    }
    let uris = results
        .get("uris")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .unwrap_or_default();
    let paths: Vec<PathBuf> = uris
        .iter()
        .filter_map(|uri| crate::uri::uri_to_path(uri))
        .collect();
    if paths.is_empty() {
        // Accepted with nothing usable in it. Saying "dismissed" would be a
        // lie, and an empty selection would look like success.
        return Outcome::Failed(format!("no local file in the answer ({uris:?})"));
    }
    Outcome::Chosen(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn results(uris: &[&str]) -> HashMap<String, OwnedValue> {
        let uris: Vec<String> = uris.iter().map(|uri| (*uri).to_owned()).collect();
        let value = OwnedValue::try_from(Value::from(uris)).unwrap();
        HashMap::from([("uris".to_owned(), value)])
    }

    #[test]
    fn a_dialog_cancelled_before_it_is_asked_is_never_put_up() {
        let cancel = Cancel::default();
        cancel.cancel();
        assert_eq!(
            Request::open_file("Open").ask_until(&cancel),
            Outcome::Dismissed
        );
    }

    #[tokio::test]
    async fn a_cancel_before_the_wait_is_not_lost() {
        let cancel = Cancel::default();
        cancel.clone().cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), cancel.cancelled())
            .await
            .expect("the stored cancel wakes the wait");
    }

    #[test]
    fn every_request_gets_a_token_of_its_own() {
        let first = next_token();
        let second = next_token();
        assert_ne!(first, second);
        assert_eq!(token(42, 7), "otto_kit_42_7");
        assert!(first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn the_request_path_follows_the_unique_name_and_the_token() {
        assert_eq!(
            request_path(":1.42", "otto_kit_9_0"),
            "/org/freedesktop/portal/desktop/request/1_42/otto_kit_9_0"
        );
    }

    #[test]
    fn a_chosen_uri_becomes_a_local_path() {
        assert_eq!(
            outcome(
                0,
                &results(&["file:///home/me/My%20Photos/a.png", "file:///b"])
            ),
            Outcome::Chosen(vec![
                PathBuf::from("/home/me/My Photos/a.png"),
                PathBuf::from("/b")
            ])
        );
    }

    #[test]
    fn a_cancelled_or_unusable_answer_is_not_a_choice() {
        assert_eq!(outcome(1, &results(&["file:///a"])), Outcome::Dismissed);
        assert!(matches!(outcome(2, &HashMap::new()), Outcome::Failed(_)));
        assert!(matches!(outcome(3, &HashMap::new()), Outcome::Failed(_)));
        assert!(matches!(
            outcome(0, &results(&["https://example.com/a"])),
            Outcome::Failed(_)
        ));
        assert!(matches!(outcome(0, &HashMap::new()), Outcome::Failed(_)));
    }

    #[test]
    fn a_save_starts_on_the_name_and_folder_it_was_given() {
        let options = Request::save_file("Save")
            .current_name("form.pdf")
            .current_folder("/home/me")
            .filter(
                "PDF",
                &[
                    Rule::Glob("*.pdf".into()),
                    Rule::Mime("application/pdf".into()),
                ],
            )
            .options("t".into());
        assert_eq!(options["handle_token"], Value::from("t"));
        assert_eq!(options["modal"], Value::from(true));
        assert_eq!(options["current_name"], Value::from("form.pdf"));
        assert_eq!(
            options["current_folder"],
            Value::from(b"/home/me\0".to_vec())
        );
        assert_eq!(
            options["filters"],
            Value::from(vec![(
                "PDF".to_owned(),
                vec![
                    (0u32, "*.pdf".to_owned()),
                    (1, "application/pdf".to_owned())
                ]
            )])
        );
        assert!(!options.contains_key("directory"));
    }

    #[test]
    fn a_folder_request_asks_for_a_directory_and_offers_no_filters_unless_given() {
        let options = Request::open_folder("Choose").options("t".into());
        assert_eq!(options["directory"], Value::from(true));
        assert!(!options.contains_key("filters"));
        assert!(!options.contains_key("current_name"));
    }
}
