//! Asking about a program and acting on the answer — see
//! [`crate::program_access`] for what is asked and who is trusted.
//!
//! The store is read, watched and written, and the user asked, on threads of
//! their own; what they learn comes back to the compositor over a channel.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use smithay::reexports::calloop::channel::{channel, Event as ChannelEvent, Sender};

use super::{screencopy::PendingScreencopy, Backend, Otto};
use crate::program_access::{self, store, Capability};

/// What the threads tell the compositor.
#[derive(Debug)]
pub enum AccessEvent {
    /// The store's answers, read at startup and again whenever they change.
    Loaded(HashMap<(Capability, PathBuf), bool>),
    /// The user answered about a program, or could not be asked (`None`).
    Answered {
        capability: Capability,
        exe: PathBuf,
        allowed: Option<bool>,
    },
}

pub struct ProgramAccessState {
    /// Clients in Otto's own process are trusted: the headless tests' clients
    /// are, and tests of the questions turn it off.
    pub trust_own_process: bool,
    /// Whether the user can be asked. Not in the headless backend, whose
    /// tests answer through [`Otto::answer_program_access`].
    pub prompts: bool,
    /// The questions on screen, so a program is asked once at a time.
    pub asking: HashSet<(Capability, PathBuf)>,
    /// Screen captures waiting on an answer about their program.
    pub awaiting_frames: Vec<(PathBuf, PendingScreencopy)>,
    events: Option<Sender<AccessEvent>>,
}

impl Default for ProgramAccessState {
    fn default() -> Self {
        Self {
            trust_own_process: true,
            prompts: false,
            asking: HashSet::new(),
            awaiting_frames: Vec::new(),
            events: None,
        }
    }
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Read the answers, keep them up to date, and listen for the user's.
    pub fn watch_program_access(&mut self) {
        let (tx, rx) = channel::<AccessEvent>();
        if let Err(err) = self.handle.insert_source(rx, |event, _, state| {
            if let ChannelEvent::Msg(event) = event {
                state.on_access_event(event);
            }
        }) {
            tracing::warn!(%err, "cannot listen for answers about programs");
            return;
        }
        self.program_access.prompts = self.backend_data.backend_name() != "headless";
        if self.program_access.prompts {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("program-access".into())
                .spawn(move || {
                    let Ok(conn) = zbus::blocking::Connection::session() else {
                        return;
                    };
                    let send = |tx: &Sender<AccessEvent>| {
                        if let Some(answers) = store::load(&conn) {
                            let _ = tx.send(AccessEvent::Loaded(answers));
                        }
                    };
                    send(&tx);
                    store::watch(&conn, || send(&tx));
                })
                .ok();
        }
        self.program_access.events = Some(tx);
    }

    /// Ask the user about the program at `exe`, unless they are being asked
    /// already. The answer arrives as [`AccessEvent::Answered`].
    pub fn ask_program_access(&mut self, capability: Capability, exe: PathBuf) {
        if !self.program_access.asking.insert((capability, exe.clone())) {
            return;
        }
        tracing::info!(capability = capability.id(), exe = %exe.display(), "asking about a program");
        if !self.program_access.prompts {
            return;
        }
        let Some(tx) = self.program_access.events.clone() else {
            return;
        };
        std::thread::Builder::new()
            .name("program-access-ask".into())
            .spawn(move || {
                let allowed = zbus::blocking::Connection::session().ok().and_then(|conn| {
                    let allowed = store::ask(&conn, capability, &exe)?;
                    store::save(&conn, capability, &exe, allowed);
                    Some(allowed)
                });
                let _ = tx.send(AccessEvent::Answered {
                    capability,
                    exe,
                    allowed,
                });
            })
            .ok();
    }

    /// Act on the user's answer about the program at `exe`, as though it
    /// came from the dialog.
    pub fn answer_program_access(
        &mut self,
        capability: Capability,
        exe: PathBuf,
        allowed: Option<bool>,
    ) {
        self.on_access_event(AccessEvent::Answered {
            capability,
            exe,
            allowed,
        });
    }

    fn on_access_event(&mut self, event: AccessEvent) {
        match event {
            AccessEvent::Loaded(answers) => program_access::replace_all(answers),
            AccessEvent::Answered {
                capability,
                exe,
                allowed,
            } => {
                self.program_access
                    .asking
                    .remove(&(capability, exe.clone()));
                if let Some(allowed) = allowed {
                    program_access::record(capability, &exe, allowed);
                }
                tracing::info!(
                    capability = capability.id(),
                    exe = %exe.display(),
                    ?allowed,
                    "answer about a program"
                );
                if capability == Capability::ScreenCapture {
                    self.release_awaiting_frames(&exe, allowed == Some(true));
                }
            }
        }
    }

    /// Capture the frames waiting on `exe`, or fail them.
    fn release_awaiting_frames(&mut self, exe: &std::path::Path, allowed: bool) {
        let (released, waiting): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.program_access.awaiting_frames)
                .into_iter()
                .partition(|(program, _)| program == exe);
        self.program_access.awaiting_frames = waiting;
        if released.is_empty() {
            return;
        }
        for (_, frame) in released {
            if allowed && !self.is_session_locked() {
                self.pending_screencopy_frames.push(frame);
            } else {
                frame.frame.failed();
            }
        }
        self.backend_data.request_redraw();
    }
}
