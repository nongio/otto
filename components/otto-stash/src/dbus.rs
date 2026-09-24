//! `org.otto.Stash1` on the session bus: how shortcuts, Files and the region
//! picker reach the stash, and how Ask follows it.
//!
//! otto-stash owns the stash. Ask shows it and changes it through
//! these methods, and `Changed` tells it about every change.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::sync::Mutex;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use tokio::sync::oneshot;
use zbus::export::futures_util::StreamExt;
use zbus::fdo;

pub use otto_kit::components::stashed::{Items, NAME, PATH};

/// Where Files says what is selected in its focused window.
const FILES_NAME: &str = "org.otto.Files1";
const FILES_PATH: &str = "/org/otto/Files1";
const FILES_METHOD: &str = "FocusedSelection";
/// How long Files is given to say.
const FILES_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

/// Signalled back when an add is done, whatever it added.
pub type Done = Option<oneshot::Sender<()>>;

/// A request from the bus, handled on the Wayland thread.
#[derive(Debug)]
pub enum Command {
    /// Start a stash, and add what is selected: the focused field's text,
    /// the files selected in Files, or the primary selection.
    Add(Done),
    /// Files answered: the files selected in its focused window, empty when
    /// none, or `None` when no Files window is in front. Sent by the bus
    /// task, not over the bus.
    AddFocusedFiles(Option<Vec<PathBuf>>, Done),
    /// Add a file; starts stash when it is not on.
    AddFile(PathBuf),
    /// Pick a screen region and add a capture of it.
    AddRegion,
    /// A region pick ended: the capture, or `None` when it was cancelled or
    /// failed. Sent by the picking thread, not over the bus.
    RegionCaptured(Option<PathBuf>),
    /// Open Ask, which takes everything stashed.
    Send,
    /// Everything stashed, as files.
    Items(oneshot::Sender<Items>),
    /// The bus client named here shows the stash, so the card steps
    /// aside until it is released.
    Hold(String),
    /// The client holding the stash left the bus.
    Release(String),
    /// Strike the item at the index out, or bring it back.
    Toggle(usize),
    /// Take the item at the index out.
    Remove(usize),
    /// What is stashed went to Ask: the stash is over.
    Sent,
    /// Throw the stash away.
    Cancel,
}

struct Service {
    commands: Mutex<Sender<Command>>,
}

impl Service {
    fn sender(&self) -> fdo::Result<Sender<Command>> {
        self.commands
            .lock()
            .map(|commands| commands.clone())
            .map_err(|_| fdo::Error::Failed("the stash stopped".into()))
    }

    fn forward(&self, command: Command) -> fdo::Result<()> {
        self.sender()?
            .send(command)
            .map_err(|_| fdo::Error::Failed("the stash stopped".into()))
    }

    fn index(index: u32) -> fdo::Result<usize> {
        usize::try_from(index).map_err(|_| fdo::Error::InvalidArgs("no such item".into()))
    }
}

#[zbus::interface(name = "org.otto.Stash1")]
impl Service {
    /// Returns once what was selected is in the stash, or it turned out
    /// nothing was.
    async fn add(&self) -> fdo::Result<()> {
        let (done, added) = oneshot::channel();
        self.forward(Command::Add(Some(done)))?;
        let _ = added.await;
        Ok(())
    }

    /// `path` must be absolute.
    fn add_file(&self, path: String) -> fdo::Result<()> {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(fdo::Error::InvalidArgs("the path must be absolute".into()));
        }
        self.forward(Command::AddFile(path))
    }

    fn add_region(&self) -> fdo::Result<()> {
        self.forward(Command::AddRegion)
    }

    fn send(&self) -> fdo::Result<()> {
        self.forward(Command::Send)
    }

    fn cancel(&self) -> fdo::Result<()> {
        self.forward(Command::Cancel)
    }

    /// Everything stashed, oldest first, as files: text as
    /// `selection-N.txt`, each with whether it is struck out.
    async fn items(&self) -> fdo::Result<Vec<(String, bool)>> {
        let (reply, items) = oneshot::channel();
        self.forward(Command::Items(reply))?;
        let items = items
            .await
            .map_err(|_| fdo::Error::Failed("the stash stopped".into()))?;
        Ok(to_wire(items))
    }

    /// The caller shows the stash: the card steps aside until the caller
    /// leaves the bus.
    async fn hold(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] bus: &zbus::Connection,
    ) -> fdo::Result<()> {
        let Some(caller) = header.sender().map(|sender| sender.to_string()) else {
            return Err(fdo::Error::Failed("no caller to hold for".into()));
        };
        let commands = self.sender()?;
        let dbus = fdo::DBusProxy::new(bus).await?;
        let mut owners = dbus
            .receive_name_owner_changed_with_args(&[(0, caller.as_str())])
            .await?;
        commands
            .send(Command::Hold(caller.clone()))
            .map_err(|_| fdo::Error::Failed("the stash stopped".into()))?;
        tokio::spawn(async move {
            while let Some(change) = owners.next().await {
                if change.args().is_ok_and(|args| args.new_owner().is_none()) {
                    break;
                }
            }
            let _ = commands.send(Command::Release(caller));
        });
        Ok(())
    }

    fn toggle(&self, index: u32) -> fdo::Result<()> {
        self.forward(Command::Toggle(Self::index(index)?))
    }

    fn remove(&self, index: u32) -> fdo::Result<()> {
        self.forward(Command::Remove(Self::index(index)?))
    }

    /// What is stashed went to Ask: the stash is over.
    fn sent(&self) -> fdo::Result<()> {
        self.forward(Command::Sent)
    }

    /// Everything stashed, after every change.
    #[zbus(signal)]
    async fn changed(
        emitter: &zbus::object_server::SignalContext<'_>,
        items: Vec<(String, bool)>,
    ) -> zbus::Result<()>;
}

/// Items as they go on the bus; a path that isn't UTF-8 is left out.
fn to_wire(items: Items) -> Vec<(String, bool)> {
    items
        .into_iter()
        .filter_map(|(path, struck)| Some((path.into_os_string().into_string().ok()?, struck)))
        .collect()
}

/// Tell whoever follows the stash what is in it now.
///
/// # Errors
///
/// When the signal cannot be sent.
pub async fn announce(bus: &zbus::Connection, items: Items) -> zbus::Result<()> {
    let emitter = zbus::object_server::SignalContext::new(bus, PATH)?;
    Service::changed(&emitter, to_wire(items)).await
}

/// The files selected in the Files window that has the keyboard, empty when
/// nothing is selected in it; `None` when no Files window has the keyboard,
/// or Files doesn't answer in time.
///
/// Each Files window is a process of its own, queued for [`FILES_NAME`], so
/// each is asked; at most one has the keyboard, and the others answer with
/// an error.
pub async fn focused_files() -> Option<Vec<PathBuf>> {
    let asked = tokio::time::timeout(FILES_TIMEOUT, async {
        let bus = zbus::Connection::session().await?;
        let dbus = fdo::DBusProxy::new(&bus).await?;
        let windows = dbus
            .list_queued_owners(zbus::names::WellKnownName::try_from(FILES_NAME)?)
            .await?;
        let asks = windows.iter().map(|window| {
            let bus = &bus;
            async move {
                let reply = bus
                    .call_method(
                        Some(window.as_str()),
                        FILES_PATH,
                        Some(FILES_NAME),
                        FILES_METHOD,
                        &(),
                    )
                    .await?;
                reply.body().deserialize::<Vec<String>>()
            }
        });
        let answers = zbus::export::futures_util::future::join_all(asks).await;
        Ok::<_, zbus::Error>(answers.into_iter().find_map(Result::ok))
    })
    .await;
    match asked {
        Ok(Ok(paths)) => paths.map(|paths| paths.into_iter().map(PathBuf::from).collect()),
        Ok(Err(error)) => {
            tracing::debug!(%error, "Files didn't say what is selected");
            None
        }
        Err(_) => {
            tracing::warn!("Files didn't say what is selected in time");
            None
        }
    }
}

/// Own [`NAME`] and serve the interface, forwarding requests to `commands`.
/// The returned connection must be kept alive.
///
/// # Errors
///
/// When there is no session bus, or another otto-stash owns the name.
pub async fn serve(commands: Sender<Command>) -> zbus::Result<zbus::Connection> {
    let service = Service {
        commands: Mutex::new(commands),
    };
    zbus::connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, service)?
        .build()
        .await
}

/// Call `method` with `body` on the running otto-stash.
///
/// # Errors
///
/// When otto-stash isn't running or refuses the request.
pub async fn call<B>(method: &str, body: &B) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let bus = zbus::Connection::session().await?;
    bus.call_method(Some(NAME), PATH, Some(NAME), method, body)
        .await?;
    Ok(())
}

/// The shortcut that opens Ask, as "Ctrl+Alt+A", or failing that the one
/// bound to `otto-stash send`; `None` when there is neither or Otto's
/// settings can't be asked.
pub async fn send_shortcut() -> Option<String> {
    let bus = zbus::Connection::session().await.ok()?;
    let reply = bus
        .call_method(
            Some("org.otto.Settings"),
            "/org/otto/Settings",
            Some("org.otto.Settings"),
            "ListShortcuts",
            &(),
        )
        .await
        .ok()?;
    let shortcuts: Vec<(String, String)> = reply.body().deserialize().ok()?;
    let find = |wanted: fn(&str) -> bool| {
        shortcuts
            .iter()
            .find(|(_, action)| wanted(action))
            .map(|(keys, _)| key_label(keys))
    };
    find(opens_ask).or_else(|| find(sends))
}

/// Whether a shortcut's action opens Ask: `otto-launcher --ask`, or
/// `otto-ask`.
fn opens_ask(action: &str) -> bool {
    let mut words = action.split_whitespace();
    if words.next() != Some("run") {
        return false;
    }
    let program = words.next().map(std::path::Path::new);
    let args: Vec<&str> = words.collect();
    match program.and_then(|program| program.file_name()?.to_str()) {
        Some("otto-ask") => args.is_empty(),
        Some("otto-launcher") => args == ["--ask"],
        _ => false,
    }
}

/// Whether a shortcut's action runs `otto-stash send`.
fn sends(action: &str) -> bool {
    let mut words = action.split_whitespace().rev();
    let (Some(command), Some(program)) = (words.next(), words.next()) else {
        return false;
    };
    command == "send"
        && std::path::Path::new(program)
            .file_name()
            .is_some_and(|name| name == "otto-stash")
}

/// "Ctrl+Alt+Shift+g" as it reads on a keycap: "Ctrl+Alt+Shift+G".
fn key_label(keys: &str) -> String {
    keys.split('+')
        .map(|key| {
            if key.chars().count() == 1 {
                key.to_uppercase()
            } else {
                key.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_send_shortcut() {
        assert!(sends("run /home/me/.local/bin/otto-stash send"));
        assert!(sends("run otto-stash send"));
        assert!(!sends("run otto-stash cancel"));
        assert!(!sends("run not-stash send"));
        assert_eq!(key_label("Ctrl+Alt+Shift+g"), "Ctrl+Alt+Shift+G");
        assert!(opens_ask("run otto-launcher --ask"));
        assert!(opens_ask("run /usr/bin/otto-ask"));
        assert!(!opens_ask("run otto-launcher --agents"));
        assert!(!opens_ask("run otto-launcher"));
    }
}
