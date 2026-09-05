//! D-Bus service implementation for `org.otto.Shell1`.
//!
//! The scripting surface: an i3-syntax command language, the tree as JSON,
//! and the two events a status bar needs. The wire contract lives in
//! `docs/developer/shell-dbus-api.md`; the grammar and the phasing are in
//! `docs/developer/tiling-plan.md`.
//!
//! Like [`crate::settings_service`], this module is only the bus end: every
//! request is handed to the compositor thread over the same calloop channel
//! and answered on a `oneshot`, so a call from `otto-msg` takes exactly the
//! path a keystroke does.

use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;
use tracing::info;
use zbus::{interface, Connection, SignalContext};

use crate::screenshare::CompositorCommand;

/// The `RunCommand` and getter errors.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.otto.Shell1.Error")]
pub enum ShellFault {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// The compositor is not listening, or did not answer.
    Unavailable(String),
}

/// The kinds of event `org.otto.Shell1` publishes, matching i3's event names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The focused or visible workspace changed.
    Workspace,
    /// The focused window changed.
    Window,
}

/// One event, ready to go out as JSON.
#[derive(Debug, Clone)]
pub struct ShellEvent {
    pub kind: EventKind,
    /// The payload, in i3's event shape: at least a `change` key and the
    /// subject the change is about.
    pub payload: String,
}

type Announcer = Box<dyn Fn(ShellEvent) + Send + Sync>;

static ANNOUNCER: OnceLock<Mutex<Option<Announcer>>> = OnceLock::new();

fn announcer() -> &'static Mutex<Option<Announcer>> {
    ANNOUNCER.get_or_init(|| Mutex::new(None))
}

/// Install the sink the `WorkspaceChanged` and `WindowChanged` signals go out
/// through. Called once, when the bus connection is up.
pub fn set_announcer(announcer_fn: Announcer) {
    *announcer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(announcer_fn);
}

/// Publish one event.
///
/// Called from the compositor thread, which cannot await; the sink hands it
/// to the bus connection's runtime. With no bus — a headless test, a session
/// whose D-Bus went away — this is a no-op and the compositor carries on.
pub fn announce(kind: EventKind, payload: serde_json::Value) {
    let guard = announcer()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(announcer) = guard.as_ref() {
        announcer(ShellEvent {
            kind,
            payload: payload.to_string(),
        });
    }
}

/// A focus change, in i3's `window` event shape.
pub fn announce_window_focus(container: serde_json::Value) {
    announce(
        EventKind::Window,
        serde_json::json!({"change": "focus", "container": container}),
    );
}

/// A workspace switch, in i3's `workspace` event shape.
pub fn announce_workspace_focus(current: serde_json::Value) {
    announce(
        EventKind::Workspace,
        serde_json::json!({"change": "focus", "current": current}),
    );
}

/// The shell interface, at `/org/otto/Shell1`.
pub struct ShellInterface {
    compositor_tx: smithay::reexports::calloop::channel::Sender<CompositorCommand>,
}

impl ShellInterface {
    /// Ask the compositor thread for something, and wait for its answer.
    async fn ask<T: Send + 'static>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> CompositorCommand,
    ) -> Result<T, ShellFault> {
        let (response_tx, response_rx) = oneshot::channel();
        self.compositor_tx.send(make(response_tx)).map_err(|err| {
            ShellFault::Unavailable(format!("compositor is not listening: {err}"))
        })?;
        response_rx
            .await
            .map_err(|_| ShellFault::Unavailable("compositor did not answer".to_string()))
    }
}

#[interface(name = "org.otto.Shell1")]
impl ShellInterface {
    /// Run a `;`-separated i3-syntax command string.
    ///
    /// Answers one `(success, message)` pair per command, in the order they
    /// were given — the same shape `swaymsg` prints as JSON. A command that
    /// worked carries an empty message; one that did not says why. A string
    /// that does not parse comes back as a single failure naming the
    /// character it stumbled on, because i3 and sway also abandon the whole
    /// string rather than run half of it.
    async fn run_command(&self, command: &str) -> Result<Vec<(bool, String)>, ShellFault> {
        let command = command.to_string();
        let results = self
            .ask(|response_tx| CompositorCommand::RunShellCommand {
                command,
                response_tx,
            })
            .await?;
        Ok(results
            .into_iter()
            .map(|result| match result {
                Ok(()) => (true, String::new()),
                Err(message) => (false, message),
            })
            .collect())
    }

    /// The whole tree as JSON, in i3's `GET_TREE` node shape.
    async fn get_tree(&self) -> Result<String, ShellFault> {
        self.ask(|response_tx| CompositorCommand::GetShellTree { response_tx })
            .await
    }

    /// Every workspace on every output, in i3's `GET_WORKSPACES` shape.
    async fn get_workspaces(&self) -> Result<String, ShellFault> {
        self.ask(|response_tx| CompositorCommand::GetShellWorkspaces { response_tx })
            .await
    }

    /// Every output, in i3's `GET_OUTPUTS` shape.
    async fn get_outputs(&self) -> Result<String, ShellFault> {
        self.ask(|response_tx| CompositorCommand::GetShellOutputs { response_tx })
            .await
    }

    /// The focused or visible workspace changed. The argument is i3's
    /// `workspace` event as JSON.
    #[zbus(signal)]
    async fn workspace_changed(context: &SignalContext<'_>, event: String) -> zbus::Result<()>;

    /// The focused window changed. The argument is i3's `window` event as
    /// JSON.
    #[zbus(signal)]
    async fn window_changed(context: &SignalContext<'_>, event: String) -> zbus::Result<()>;
}

/// Register the shell interface on the existing D-Bus connection.
pub async fn register_shell_interface(
    connection: &Connection,
    compositor_tx: smithay::reexports::calloop::channel::Sender<CompositorCommand>,
) -> zbus::Result<()> {
    connection
        .object_server()
        .at("/org/otto/Shell1", ShellInterface { compositor_tx })
        .await?;

    connection.request_name("org.otto.Shell1").await?;

    // Events originate on the compositor thread, which cannot await, so they
    // are handed to a task on this connection's runtime — the same shape the
    // settings `Changed` signal uses.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<ShellEvent>();
    set_announcer(Box::new(move |event| {
        // A closed channel means the bus went away; the compositor carries on.
        let _ = event_tx.send(event);
    }));

    let connection = connection.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let iface = connection
                .object_server()
                .interface::<_, ShellInterface>("/org/otto/Shell1")
                .await;
            let Ok(iface) = iface else {
                tracing::warn!("Shell interface went away");
                continue;
            };
            let context = iface.signal_context();
            let sent = match event.kind {
                EventKind::Workspace => {
                    ShellInterface::workspace_changed(context, event.payload).await
                }
                EventKind::Window => ShellInterface::window_changed(context, event.payload).await,
            };
            if let Err(err) = sent {
                tracing::warn!("Could not emit a shell event: {err}");
            }
        }
    });

    info!("Shell D-Bus interface registered at org.otto.Shell1");

    Ok(())
}
