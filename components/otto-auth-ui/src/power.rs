//! Sleep, hibernate, restart and shut down, for the panel's power buttons.
//!
//! The asking is [`otto_kit::power`]'s, shared with the bar's Otto menu.
//! Whether the greeter's or the locked session's user may do this is
//! polkit's call.
//!
//! The call runs on a thread of its own. logind answers suspend only once the
//! machine is on its way down, and polkit can take as long as it likes — the
//! panel keeps drawing meanwhile, and collects the answer with
//! [`PowerRequest::poll`] the way it collects PAM's.

use std::sync::mpsc::{Receiver, TryRecvError};

pub use otto_kit::power::PowerError;

use crate::PowerAction;

/// What the panel says about a refusal or failure, in `client`'s words:
/// `{client}-power-{action}-denied` or `-failed` from the catalogue, as with
/// [`crate::reader::request_line`].
pub fn error_line(error: &PowerError, action: PowerAction, client: &str) -> String {
    let action = action.name();
    match error {
        PowerError::Denied => {
            otto_kit::i18n::lookup(&format!("{client}-power-{action}-denied"), None).into_owned()
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
                let _ = tx.send(otto_kit::power::request(action));
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
