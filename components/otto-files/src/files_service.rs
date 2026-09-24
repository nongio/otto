//! `org.otto.Files1` — what the rest of the desktop can ask a Files window.
//!
//! Today that is one question: which files are selected in the window that
//! has the keyboard. otto-gather's shortcut is a compositor keybinding, so
//! Files never sees the key press; gather asks here instead.
//!
//! Every browser window is its own process, so every one of them serves this
//! interface. Each asks for the well-known name *without* `DoNotQueue`: one
//! owns it and the rest wait in its queue, which makes
//! `org.freedesktop.DBus.ListQueuedOwners("org.otto.Files1")` the list of
//! every Files window on the bus. A caller asks each unique name in turn; at
//! most one of them holds the keyboard, and only that one answers with
//! anything. A name nobody may replace would hide every window but the
//! first, and a signal broadcast would need a reply channel of its own.
//!
//! The bridge is the picker's (see `dbus.rs`): the zbus task parks a one-shot
//! sender, wakes the UI loop, and awaits the reply. Nothing in the UI thread
//! awaits.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_kit::prelude::AppContext;
use tokio::sync::oneshot;
use zbus::interface;

pub const DBUS_NAME: &str = "org.otto.Files1";
pub const DBUS_PATH: &str = "/org/otto/Files1";

/// Where an answer goes: the call waiting on it.
pub type Reply = oneshot::Sender<Vec<String>>;

/// Questions the UI thread has not answered yet.
#[derive(Default)]
pub struct Queue {
    asks: Mutex<Vec<Reply>>,
}

pub type SharedQueue = Arc<Queue>;

impl Queue {
    fn push(&self, reply: Reply) {
        self.asks.lock().unwrap().push(reply);
        AppContext::request_wakeup();
    }

    /// Take every waiting question. The UI thread answers them all with the
    /// same selection, since nothing changed between them.
    pub fn take(&self) -> Vec<Reply> {
        std::mem::take(&mut *self.asks.lock().unwrap())
    }
}

struct FilesService {
    queue: SharedQueue,
}

#[interface(name = "org.otto.Files1")]
impl FilesService {
    /// Absolute paths of the items selected in this process's window, while
    /// it holds the keyboard. Empty when it does not, or nothing is selected.
    async fn focused_selection(&self) -> Vec<String> {
        let (tx, rx) = oneshot::channel();
        self.queue.push(tx);
        // A dropped sender means the window closed before answering: nothing
        // of ours has the keyboard any more.
        rx.await.unwrap_or_default()
    }
}

/// Serve the interface and queue for the name until the connection dies.
pub async fn serve(queue: SharedQueue) -> zbus::Result<()> {
    use zbus::fdo::DBusProxy;

    let connection = zbus::ConnectionBuilder::session()?.build().await?;
    connection
        .object_server()
        .at(DBUS_PATH, FilesService { queue })
        .await?;

    // No flags: queued, never replacing — see the module docs. Owning the
    // name and waiting for it are equally fine, as callers reach us by
    // unique name.
    DBusProxy::new(&connection)
        .await?
        .request_name(DBUS_NAME.try_into()?, Default::default())
        .await?;

    std::future::pending::<()>().await;
    Ok(())
}

/// The selection as it goes on the wire: `focused` gates it, and a path that
/// is not absolute UTF-8 is left out, as `s` cannot carry it faithfully.
pub fn wire_paths(focused: bool, selected: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    if !focused {
        return Vec::new();
    }
    selected
        .into_iter()
        .filter(|path| path.is_absolute())
        .filter_map(|path| path.into_os_string().into_string().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfocused_window_answers_with_nothing() {
        assert!(wire_paths(false, [PathBuf::from("/tmp/a.txt")]).is_empty());
    }

    #[test]
    fn a_focused_window_answers_with_its_selection_in_order() {
        assert_eq!(
            wire_paths(true, [PathBuf::from("/tmp/b"), PathBuf::from("/tmp/a")]),
            vec!["/tmp/b".to_string(), "/tmp/a".to_string()]
        );
    }

    #[test]
    fn paths_the_wire_cannot_carry_are_left_out() {
        use std::os::unix::ffi::OsStringExt;
        let not_utf8 = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff".to_vec()));
        assert_eq!(
            wire_paths(
                true,
                [not_utf8, PathBuf::from("relative"), PathBuf::from("/ok")]
            ),
            vec!["/ok".to_string()]
        );
    }

    #[test]
    fn every_waiting_question_is_taken_at_once() {
        let queue = Queue::default();
        let (a, _) = oneshot::channel();
        let (b, _) = oneshot::channel();
        queue.asks.lock().unwrap().extend([a, b]);
        assert_eq!(queue.take().len(), 2);
        assert!(queue.take().is_empty());
    }
}
