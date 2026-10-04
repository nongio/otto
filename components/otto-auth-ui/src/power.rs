//! Suspend, restart and shut down, for the panel's power buttons.
//!
//! Asked of logind over the system bus rather than by running `systemctl`:
//! logind is what `systemctl` would have asked anyway, and it says why it
//! refused in a D-Bus error instead of on a stderr nobody reads. Whether the
//! greeter's or the locked session's user may do this is polkit's call;
//! `interactive` lets polkit ask for a password where its policy says to.
//!
//! The call runs on a thread of its own. logind answers suspend only once the
//! machine is on its way down, and polkit can take as long as it likes — the
//! panel keeps drawing meanwhile, and collects the answer with
//! [`PowerRequest::poll`] the way it collects PAM's.

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::PowerAction;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_async = false
)]
trait Login1Manager {
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
}

/// Why logind did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerError {
    /// polkit said no.
    Denied,
    /// logind could not be reached, or failed for a reason of its own.
    Failed(String),
}

impl PowerError {
    /// What the panel says about it, in `client`'s words: `{client}-power-
    /// {action}-denied` or `-failed` from the catalogue, as with
    /// [`crate::reader::request_line`].
    pub fn line(&self, action: PowerAction, client: &str) -> String {
        let action = match action {
            PowerAction::Suspend => "suspend",
            PowerAction::Restart => "restart",
            PowerAction::Shutdown => "shutdown",
        };
        match self {
            PowerError::Denied => {
                otto_kit::i18n::lookup(&format!("{client}-power-{action}-denied"), None)
                    .into_owned()
            }
            PowerError::Failed(error) => {
                let args = otto_kit::i18n::args_from(vec![(
                    "error",
                    otto_kit::i18n::FluentValue::from(error.clone()),
                )]);
                otto_kit::i18n::lookup(&format!("{client}-power-{action}-failed"), Some(&args))
                    .into_owned()
            }
        }
    }
}

/// One power action on its way to logind.
pub struct PowerRequest {
    action: PowerAction,
    answer: Receiver<Result<(), PowerError>>,
}

impl PowerRequest {
    /// Ask logind for `action`, off the calling thread.
    pub fn start(action: PowerAction) -> Self {
        let (tx, answer) = std::sync::mpsc::channel();
        tracing::info!(?action, "power action requested");
        let spawned = std::thread::Builder::new()
            .name("power".to_string())
            .spawn(move || {
                let _ = tx.send(call(action));
                otto_kit::AppContext::request_wakeup();
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the power thread");
            // The sender went with the closure, so `poll` reports a failure.
        }
        Self { action, answer }
    }

    pub fn action(&self) -> PowerAction {
        self.action
    }

    /// logind's answer, once it has given one. Never blocks.
    pub fn poll(&self) -> Option<Result<(), PowerError>> {
        match self.answer.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(PowerError::Failed(
                "the request did not complete".to_string(),
            ))),
        }
    }
}

fn call(action: PowerAction) -> Result<(), PowerError> {
    let failed = |err: zbus::Error| {
        tracing::warn!(?action, %err, "logind unavailable");
        PowerError::Failed(err.to_string())
    };
    let connection = zbus::blocking::Connection::system().map_err(failed)?;
    let manager = Login1ManagerProxy::new(&connection).map_err(failed)?;
    let result = match action {
        PowerAction::Suspend => manager.suspend(true),
        PowerAction::Restart => manager.reboot(true),
        PowerAction::Shutdown => manager.power_off(true),
    };
    match result {
        Ok(()) => {
            tracing::info!(?action, "logind accepted");
            Ok(())
        }
        Err(zbus::Error::MethodError(name, message, _)) if is_refusal(name.as_str()) => {
            tracing::warn!(?action, %name, ?message, "logind refused");
            Err(PowerError::Denied)
        }
        Err(err) => Err(failed(err)),
    }
}

/// The D-Bus errors polkit's refusals come back as.
fn is_refusal(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
    )
}
