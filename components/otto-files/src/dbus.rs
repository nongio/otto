//! `org.otto.FilePicker1` — the private interface `otto-portal` brokers
//! `org.freedesktop.impl.portal.FileChooser` requests over.
//!
//! Both ends are Otto's, so the signature is typed rather than `a{sv}`: the
//! same decision, for the same reason, as `org.otto.Dialog1`. See
//! `specs/file-picker.md` for the contract, which is permanent from the
//! first release.
//!
//! The bridge is otto-islands' dialog bridge in miniature: the zbus task
//! parks a request plus a one-shot sender in shared state, wakes the UI loop,
//! and awaits the reply. Nothing here touches Wayland, and nothing in the UI
//! thread awaits.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_kit::prelude::AppContext;
use tokio::sync::oneshot;
use zbus::interface;

use crate::picker::{self, Filter, Outcome, Request, Session};

pub const DBUS_NAME: &str = "org.otto.FilePicker1";
pub const DBUS_PATH: &str = "/org/otto/FilePicker";

/// The request tuple as it arrives on the wire. Field for field the table in
/// `specs/file-picker.md`; unpacked into a [`Request`] by [`parse_request`].
#[allow(clippy::type_complexity)]
pub type WireRequest = (
    u32,                                                  // mode
    String,                                               // handle
    String,                                               // app_id
    String,                                               // parent_window
    String,                                               // title
    String,                                               // accept_label
    bool,                                                 // multiple
    bool,                                                 // directory
    bool,                                                 // modal
    String,                                               // current_name
    String,                                               // current_folder
    String,                                               // current_file
    Vec<String>,                                          // files
    Vec<(String, Vec<(u32, String)>)>,                    // filters
    String,                                               // current_filter
    Vec<(String, String, Vec<(String, String)>, String)>, // choices
);

/// What the UI thread has been asked to do, and has not done yet.
#[derive(Default)]
struct Pending {
    /// Requests waiting for a window. v1 shows one at a time — see
    /// `run_picker` — so a second request queues behind the first rather than
    /// opening a second window.
    requests: VecDeque<Session>,
    /// Handles the portal has withdrawn. Checked against both the queue and
    /// the window currently up.
    withdrawn: Vec<String>,
}

/// The queue as both halves see it.
///
/// Two wakeups, because there are two waiters and they wait differently: the
/// zbus side is async and parks on [`Notify`], the UI side is a `poll`-driven
/// event loop and parks on otto-kit's wakeup pipe. A request arriving pokes
/// both, and whichever is actually waiting picks it up.
#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    arrived: tokio::sync::Notify,
}

pub type SharedQueue = Arc<Queue>;

impl Queue {
    /// Park until there is something to serve, then take it.
    pub async fn next_session_async(&self) -> Session {
        loop {
            // Register interest *before* looking, so a request arriving
            // between the two is not missed.
            let notified = self.arrived.notified();
            if let Some(session) = self.next_session() {
                return session;
            }
            notified.await;
        }
    }

    fn push(&self, session: Session) {
        self.pending.lock().unwrap().requests.push_back(session);
        self.arrived.notify_one();
        AppContext::request_wakeup();
    }

    fn withdraw(&self, handle: String) {
        self.pending.lock().unwrap().withdrawn.push(handle);
        self.arrived.notify_one();
        AppContext::request_wakeup();
    }
    /// Take the next request that has not been withdrawn in the meantime.
    pub fn next_session(&self) -> Option<Session> {
        let mut pending = self.pending.lock().unwrap();
        while let Some(mut session) = pending.requests.pop_front() {
            if let Some(index) = pending
                .withdrawn
                .iter()
                .position(|h| *h == session.request.handle)
            {
                pending.withdrawn.remove(index);
                session.resolve(Outcome::ended());
                continue;
            }
            return Some(session);
        }
        None
    }

    /// Whether `handle` has been withdrawn, consuming the record if so. The
    /// window currently up asks this about its own request every update.
    pub fn take_withdrawn(&self, handle: &str) -> bool {
        let mut pending = self.pending.lock().unwrap();
        match pending.withdrawn.iter().position(|h| h == handle) {
            Some(index) => {
                pending.withdrawn.remove(index);
                true
            }
            None => false,
        }
    }
}

pub struct FilePickerService {
    queue: SharedQueue,
}

impl FilePickerService {
    pub fn new(queue: SharedQueue) -> Self {
        Self { queue }
    }
}

#[interface(name = "org.otto.FilePicker1")]
impl FilePickerService {
    /// Present a picker and block until the user answers or the request is
    /// withdrawn. See `specs/file-picker.md` for the tuple's fields.
    async fn present(
        &self,
        request: WireRequest,
    ) -> (u32, Vec<String>, String, Vec<(String, String)>) {
        let request = match parse_request(request) {
            Ok(request) => request,
            Err(reason) => {
                tracing::warn!(reason, "rejecting malformed picker request");
                let ended = Outcome::ended();
                return (
                    ended.response,
                    ended.uris,
                    ended.current_filter,
                    ended.choices,
                );
            }
        };

        // Save mode is specified but not built. Saying so is the contract's
        // `response = 2`: the application learns it got no file, rather than
        // being shown an Open dialog that returns a file it cannot write.
        if request.mode != picker::Mode::Open {
            tracing::warn!(?request.mode, "save modes are not implemented yet");
            let ended = Outcome::ended();
            return (
                ended.response,
                ended.uris,
                ended.current_filter,
                ended.choices,
            );
        }

        tracing::info!(
            app_id = %request.app_id,
            handle = %request.handle,
            "picker request received"
        );

        let (tx, rx) = oneshot::channel();
        self.queue.push(Session::new(request, tx));

        // A dropped sender means the window went away without deciding, which
        // the Session's own Drop has already turned into `response = 2`; the
        // `Err` arm only catches the case where even that could not run.
        let outcome = rx.await.unwrap_or_else(|_| Outcome::ended());
        (
            outcome.response,
            outcome.uris,
            outcome.current_filter,
            outcome.choices,
        )
    }

    /// Withdraw a pending request. Its `Present` resolves with `response = 2`.
    async fn close(&self, handle: String) {
        tracing::info!(%handle, "picker request withdrawn");
        self.queue.withdraw(handle);
    }
}

/// Claim the bus name and serve requests until the connection dies.
///
/// The name is claimed with replacement allowed, for the reason the portal
/// backend records: the session bus outlives the graphical session, so a
/// picker left over from an earlier login would otherwise hold the name
/// forever and every later instance would die on startup.
pub async fn serve(queue: SharedQueue) -> zbus::Result<()> {
    use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

    let connection = zbus::ConnectionBuilder::session()?.build().await?;
    connection
        .object_server()
        .at(DBUS_PATH, FilePickerService::new(queue))
        .await?;

    let dbus = DBusProxy::new(&connection).await?;
    let reply = dbus
        .request_name(
            DBUS_NAME.try_into()?,
            RequestNameFlags::AllowReplacement
                | RequestNameFlags::ReplaceExisting
                | RequestNameFlags::DoNotQueue,
        )
        .await?;
    if reply != RequestNameReply::PrimaryOwner {
        return Err(zbus::Error::Failure(format!(
            "{DBUS_NAME} is held by another picker that refuses replacement ({reply:?})"
        )));
    }

    tracing::info!(name = DBUS_NAME, "file picker service running");
    std::future::pending::<()>().await;
    Ok(())
}

/// Unpack the wire tuple, validating what must be valid and dropping what may
/// simply be absent.
fn parse_request(wire: WireRequest) -> Result<Request, &'static str> {
    let (
        mode,
        handle,
        app_id,
        parent_window,
        title,
        accept_label,
        multiple,
        directory,
        modal,
        current_name,
        current_folder,
        current_file,
        files,
        filters,
        current_filter,
        choices,
    ) = wire;

    let mode = picker::Mode::from_wire(mode).ok_or("unknown mode")?;
    if handle.is_empty() {
        return Err("empty request handle");
    }

    let filters: Vec<Filter> = filters.into_iter().map(Filter::from_wire).collect();
    let current_filter = filters
        .iter()
        .position(|f| f.label == current_filter)
        .unwrap_or(0);

    Ok(Request {
        mode,
        handle,
        app_id,
        parent_window,
        title,
        accept_label,
        multiple,
        directory,
        modal,
        current_name,
        current_folder: absolute_path(current_folder),
        current_file: absolute_path(current_file),
        files,
        filters,
        current_filter,
        choices,
    })
}

/// A path the request supplied, if it is usable.
///
/// A relative path is dropped rather than rejected — the spec's rule is that
/// the picker then falls back as if the field were absent, which is friendlier
/// than failing a whole request over a hint.
fn absolute_path(value: String) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire() -> WireRequest {
        (
            0,
            "req1".into(),
            "org.example.App".into(),
            String::new(),
            String::new(),
            String::new(),
            false,
            false,
            true,
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            Vec::new(),
            String::new(),
            Vec::new(),
        )
    }

    #[test]
    fn a_relative_folder_hint_is_dropped_rather_than_failing_the_request() {
        let mut w = wire();
        w.10 = "not/absolute".into();
        let request = parse_request(w).expect("relative hints are dropped, not fatal");
        assert!(request.current_folder.is_none());
    }

    #[test]
    fn an_absolute_folder_hint_is_kept() {
        let mut w = wire();
        w.10 = "/tmp".into();
        assert_eq!(
            parse_request(w).unwrap().current_folder,
            Some(PathBuf::from("/tmp"))
        );
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        let mut w = wire();
        w.0 = 9;
        assert!(parse_request(w).is_err());
    }

    #[test]
    fn a_request_with_no_handle_is_refused() {
        let mut w = wire();
        w.1 = String::new();
        assert!(parse_request(w).is_err());
    }

    #[test]
    fn current_filter_names_the_filter_by_label() {
        let mut w = wire();
        w.13 = vec![
            ("Text".into(), vec![(0, "*.txt".into())]),
            ("Images".into(), vec![(0, "*.png".into())]),
        ];
        w.14 = "Images".into();
        assert_eq!(parse_request(w).unwrap().current_filter, 1);
    }

    #[test]
    fn an_unknown_current_filter_falls_back_to_the_first() {
        let mut w = wire();
        w.13 = vec![("Text".into(), vec![(0, "*.txt".into())])];
        w.14 = "Nothing By That Name".into();
        assert_eq!(parse_request(w).unwrap().current_filter, 0);
    }

    #[test]
    fn a_withdrawn_request_is_skipped_and_answered() {
        let (tx, mut rx) = oneshot::channel();
        let queue = Queue::default();
        let request = parse_request(wire()).unwrap();
        queue
            .pending
            .lock()
            .unwrap()
            .requests
            .push_back(Session::new(request, tx));
        queue.pending.lock().unwrap().withdrawn.push("req1".into());

        assert!(
            queue.next_session().is_none(),
            "a withdrawn request must not open a window"
        );
        assert_eq!(rx.try_recv().map(|o| o.response), Ok(2));
    }

    #[test]
    fn a_withdrawal_is_reported_once_and_then_forgotten() {
        let queue = Queue::default();
        queue.pending.lock().unwrap().withdrawn.push("req1".into());
        assert!(queue.take_withdrawn("req1"));
        assert!(!queue.take_withdrawn("req1"));
    }
}
