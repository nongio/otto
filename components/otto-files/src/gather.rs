//! Hand selected files to otto-gather, which collects things to ask about
//! in Ask.
//!
//! The context menu offers "Add to Gathering" only while otto-gather is on
//! the session bus, so a desktop without it never shows the item.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use zbus::export::futures_util::StreamExt;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

const NAME: &str = "org.otto.Gather1";
const PATH: &str = "/org/otto/Gather1";

/// Whether otto-gather owns its bus name, as last seen.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The runtime the bus calls run on, captured by [`watch`].
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Start following whether otto-gather is running.
///
/// Called from inside the tokio runtime; outside one it does nothing and the
/// menu item never shows.
pub fn watch() {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let _ = RUNTIME.set(handle.clone());
    handle.spawn(async {
        if let Err(error) = follow_owner().await {
            tracing::debug!(%error, "cannot follow otto-gather on the session bus");
        }
    });
}

async fn follow_owner() -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let dbus = DBusProxy::new(&bus).await?;
    let mut changes = dbus
        .receive_name_owner_changed_with_args(&[(0, NAME)])
        .await?;
    let name = BusName::try_from(NAME)?;
    RUNNING.store(dbus.name_has_owner(name).await?, Ordering::Relaxed);
    while let Some(change) = changes.next().await {
        if let Ok(args) = change.args() {
            RUNNING.store(args.new_owner().is_some(), Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Whether otto-gather is there to take files.
pub fn available() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Add `paths` to the gathering, in order. Doesn't wait for otto-gather.
pub fn add(paths: Vec<PathBuf>) {
    let Some(handle) = RUNTIME.get() else {
        return;
    };
    handle.spawn(async move {
        if let Err(error) = add_all(&paths).await {
            tracing::warn!(%error, "cannot add files to the gathering");
        }
    });
}

async fn add_all(paths: &[PathBuf]) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    for path in paths {
        let Some(path) = path.to_str() else {
            tracing::warn!(path = %path.display(), "not UTF-8; left out of the gathering");
            continue;
        };
        bus.call_method(Some(NAME), PATH, Some(NAME), "AddFile", &(path,))
            .await?;
    }
    Ok(())
}
