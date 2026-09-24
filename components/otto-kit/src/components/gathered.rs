//! What otto-gather has gathered, followed from another app.
//!
//! otto-gather owns the gathering: it keeps it until it is sent or its items
//! are removed. An app that shows it, as Ask does, mirrors it through
//! [`Gathered`] and asks otto-gather for every change, so the card and the
//! app never disagree. While an app follows it holding it, the card steps
//! aside; it comes back when that app goes without sending.
//!
//! Items travel as files: text as `selection-N.txt` and screen regions as
//! `region-N.png` in otto-gather's directory, which
//! [`Attachment::for_file`](super::attachments::Attachment::for_file) reads
//! back as what they are.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc as async_mpsc;
use zbus::fdo::DBusProxy;
use zbus::message::Type;
use zbus::names::BusName;
use zbus::{MatchRule, MessageStream};

pub const NAME: &str = "org.otto.Gather1";
pub const PATH: &str = "/org/otto/Gather1";

/// Each gathered file, with whether it is struck out: kept, but not sent.
pub type Items = Vec<(PathBuf, bool)>;

/// How long otto-gather is given to add the selection.
const ADD_TIMEOUT: Duration = Duration::from_millis(800);

/// A change asked of otto-gather.
#[derive(Debug)]
enum Request {
    Toggle(u32),
    Remove(u32),
    Sent,
}

/// otto-gather's gathering, as last reported, kept up to date on a thread
/// of its own.
///
/// The thread reports through a socket [`Self::poll_fd`] for the app's poll
/// loop; [`Self::pump`] takes the news in.
pub struct Gathered {
    requests: async_mpsc::UnboundedSender<Request>,
    updates: mpsc::Receiver<Items>,
    wake: UnixStream,
    items: Items,
}

impl Gathered {
    /// Follow the gathering, holding it — the card steps aside — when
    /// `hold` is set. Nothing is gathered while otto-gather isn't running;
    /// once it starts, it is followed from then on.
    ///
    /// With `add_selection`, what is selected in the focused app is added
    /// first, once the gathering is held, so the card never shows for it.
    /// That waits for otto-gather, briefly: it is called before the app's
    /// own window takes the keyboard, while otto-gather can still read the
    /// selection from whatever has it.
    ///
    /// # Panics
    ///
    /// When the wake-up socket or the thread cannot be made.
    pub fn follow(hold: bool, add_selection: bool) -> Self {
        let (added_tx, added) = mpsc::channel::<()>();
        let (requests, request_rx) = async_mpsc::unbounded_channel();
        let (update_tx, updates) = mpsc::channel();
        let (wake, wake_tx) = UnixStream::pair().expect("cannot create a wake-up socket");
        let _ = wake.set_nonblocking(true);
        let _ = wake_tx.set_nonblocking(true);
        let report = move |items: Items| {
            if update_tx.send(items).is_ok() {
                // A full socket already has a wake-up in it.
                let _ = (&wake_tx).write(&[1]);
            }
        };
        std::thread::Builder::new()
            .name("otto-gather".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        tracing::warn!(%err, "cannot follow otto-gather");
                        return;
                    }
                };
                let added = add_selection.then_some(added_tx);
                if let Err(err) = runtime.block_on(follow(hold, added, request_rx, report)) {
                    tracing::debug!(%err, "stopped following otto-gather");
                }
            })
            .expect("cannot start following otto-gather");
        if add_selection && added.recv_timeout(ADD_TIMEOUT).is_err() {
            tracing::warn!("otto-gather did not add the selection in time");
        }
        Self {
            requests,
            updates,
            wake,
            items: Items::new(),
        }
    }

    /// The socket that becomes readable when the gathering changed.
    pub fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Take the latest gathering in. Never blocks. Returns whether it
    /// changed.
    pub fn pump(&mut self) -> bool {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let Some(items) = self.updates.try_iter().last() else {
            return false;
        };
        let changed = items != self.items;
        self.items = items;
        changed
    }

    /// What is gathered, oldest first.
    pub fn items(&self) -> &[(PathBuf, bool)] {
        &self.items
    }

    /// Strike the item at `index` out, or bring it back.
    pub fn toggle(&self, index: usize) {
        self.request(index, Request::Toggle);
    }

    /// Take the item at `index` out of the gathering.
    pub fn remove(&self, index: usize) {
        self.request(index, Request::Remove);
    }

    /// What is gathered was sent: the gathering is over. It is gone from
    /// here at once rather than when otto-gather says so.
    pub fn sent(&mut self) {
        if !self.items.is_empty() {
            self.items.clear();
            let _ = self.requests.send(Request::Sent);
        }
    }

    fn request(&self, index: usize, request: fn(u32) -> Request) {
        if let Ok(index) = u32::try_from(index) {
            let _ = self.requests.send(request(index));
        }
    }
}

/// Follow the gathering until the app goes: hold it, report it now and on
/// every change, pass requests on, and start over whenever otto-gather does.
async fn follow(
    hold: bool,
    added: Option<mpsc::Sender<()>>,
    mut requests: async_mpsc::UnboundedReceiver<Request>,
    report: impl Fn(Items),
) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(NAME)?
        .member("Changed")?
        .path(PATH)?
        .build();
    let mut changes = MessageStream::for_match_rule(rule, &bus, None).await?;
    let dbus = DBusProxy::new(&bus).await?;
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, NAME)])
        .await?;

    if dbus.name_has_owner(BusName::try_from(NAME)?).await? {
        if hold {
            hold_gathering(&bus).await;
        }
        if let Some(added) = added {
            if let Err(err) = bus
                .call_method(Some(NAME), PATH, Some(NAME), "Add", &())
                .await
            {
                tracing::debug!(%err, "cannot add the selection to the gathering");
            }
            let _ = added.send(());
        }
        report_items(&bus, &report).await;
    }
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else {
                    return Ok(());
                };
                let (method, index) = match request {
                    Request::Toggle(index) => ("Toggle", Some(index)),
                    Request::Remove(index) => ("Remove", Some(index)),
                    Request::Sent => ("Sent", None),
                };
                let called = match index {
                    Some(index) => bus.call_method(Some(NAME), PATH, Some(NAME), method, &(index,)).await,
                    None => bus.call_method(Some(NAME), PATH, Some(NAME), method, &()).await,
                };
                if let Err(err) = called {
                    tracing::warn!(%err, method, "otto-gather refused");
                }
            }
            Some(Ok(message)) = changes.next() => {
                if let Ok((items,)) = message.body().deserialize::<(Vec<(String, bool)>,)>() {
                    report(to_paths(items));
                }
            }
            Some(change) = owners.next() => {
                let started = change.args().is_ok_and(|args| args.new_owner().is_some());
                if started {
                    if hold {
                        hold_gathering(&bus).await;
                    }
                    report_items(&bus, &report).await;
                } else {
                    report(Items::new());
                }
            }
        }
    }
}

/// Hold the gathering: its card steps aside while this app is on the bus.
async fn hold_gathering(bus: &zbus::Connection) {
    if let Err(err) = bus
        .call_method(Some(NAME), PATH, Some(NAME), "Hold", &())
        .await
    {
        tracing::debug!(%err, "cannot hold the gathering");
    }
}

/// Report what is in the gathering now.
async fn report_items(bus: &zbus::Connection, report: &impl Fn(Items)) {
    let items = async {
        let reply = bus
            .call_method(Some(NAME), PATH, Some(NAME), "Items", &())
            .await?;
        reply.body().deserialize::<Vec<(String, bool)>>()
    };
    match items.await {
        Ok(items) => report(to_paths(items)),
        Err(err) => tracing::debug!(%err, "cannot list the gathering"),
    }
}

fn to_paths(items: Vec<(String, bool)>) -> Items {
    items
        .into_iter()
        .map(|(path, struck)| (PathBuf::from(path), struck))
        .collect()
}
