//! Dictation into a host's field.
//!
//! Every agent UI keeps its own field, a [`TextInput`] it lays out and
//! paints, and every one of them dictates into it the same way: Ctrl+D
//! starts listening at the caret and stops it again. While it runs, Escape
//! or Backspace takes back what it typed, Enter stops it and sends the
//! request once the last words are in, and any other key stops it. The keys
//! and the bookkeeping around [`Dictation`] live here so the launcher's Ask
//! and Preview's chat cannot drift apart. The equaliser and the words still
//! being heard are the field's own to draw.
//!
//! Only built with the `dictation` feature, which brings in the microphone
//! and the engine's client.

use std::os::fd::RawFd;
use std::time::Duration;

use otto_kit::components::text_input::TextInput;
use otto_kit::dictation::{self, Dictation, Engine, Status};
use smithay_client_toolkit::seat::keyboard::Keysym;

pub use otto_kit::dictation::Vocabulary;

/// What a key did to the dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationKey {
    /// Not one of the dictation's keys: the host handles it as usual.
    Ignored,
    /// The dictation took the key, to start, stop or carry on. The host
    /// redraws and does nothing else with it.
    Taken,
    /// The dictation took the key and everything it typed back out, so the
    /// field's text has changed.
    Cancelled,
}

/// What a pass over a running dictation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Followed {
    /// The field's text changed.
    pub changed: bool,
    /// The last words are in and Enter asked for the request to go.
    pub send: bool,
}

/// A host field's dictation, while one runs.
#[derive(Default)]
pub struct FieldDictation {
    running: Option<Dictation>,
    /// Enter stopped the dictation: what it typed goes as soon as it is all
    /// in.
    send_when_done: bool,
}

impl FieldDictation {
    /// Whether a dictation is running, or finishing its last pass.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Answer `keysym` if it is one of the dictation's keys. `control` is
    /// the key's Ctrl letter, as [`crate::keys::control_char`] finds it.
    /// `vocabulary` is asked for when a dictation starts: a field that picks
    /// from a list gives the list's names, which are what it expects to hear,
    /// and a request to an agent, which is free speech, gives none.
    ///
    /// Check it before any other key handling: while a dictation runs, it
    /// takes every key.
    pub fn key(
        &mut self,
        keysym: Keysym,
        control: Option<char>,
        field: &mut TextInput,
        vocabulary: impl FnOnce() -> Option<Vocabulary>,
    ) -> DictationKey {
        let Some(running) = self.running.as_mut() else {
            if control != Some('d') {
                return DictationKey::Ignored;
            }
            let mut running = Dictation::start(Engine::from_config(), field);
            if let Some(vocabulary) = vocabulary() {
                running.set_vocabulary(vocabulary);
            }
            self.running = Some(running);
            return DictationKey::Taken;
        };
        match keysym {
            Keysym::Escape | Keysym::BackSpace => {
                if let Some(running) = self.running.take() {
                    running.cancel(field);
                }
                self.send_when_done = false;
                return DictationKey::Cancelled;
            }
            Keysym::Return | Keysym::KP_Enter => {
                running.stop();
                self.send_when_done = true;
            }
            // Modifiers never stop it: Ctrl+D itself starts with one.
            Keysym::Control_L
            | Keysym::Control_R
            | Keysym::Alt_L
            | Keysym::Alt_R
            | Keysym::Super_L
            | Keysym::Super_R
            | Keysym::Meta_L
            | Keysym::Meta_R
            | Keysym::Caps_Lock => {}
            _ => running.stop(),
        }
        DictationKey::Taken
    }

    /// Type in what the dictation heard and move its equaliser. Call it on
    /// every pass of the host's loop; `None` when no dictation runs, which
    /// leaves nothing to redraw. The host redraws on `Some`, and sends the
    /// request when [`Followed::send`] says so.
    pub fn follow(&mut self, field: &mut TextInput) -> Option<Followed> {
        let running = self.running.as_mut()?;
        let before = field.value().len();
        let status = running.update(field);
        let changed = field.value().len() != before;
        let mut send = false;
        if status == Status::Done {
            self.running = None;
            send = std::mem::take(&mut self.send_when_done);
        }
        Some(Followed { changed, send })
    }

    /// Stop listening, as when the field loses the keyboard. The last pass
    /// still types in what is left.
    pub fn stop(&mut self) {
        if let Some(running) = self.running.as_mut() {
            running.stop();
        }
    }

    /// How long the host's loop may sleep: a dictation moves its equaliser
    /// every [`dictation::FRAME`].
    pub fn idle_timeout(&self) -> Option<Duration> {
        self.running.as_ref().map(|_| dictation::FRAME)
    }

    /// The socket that wakes the host's loop when a pass comes back.
    pub fn poll_fd(&self) -> Option<RawFd> {
        self.running.as_ref().map(Dictation::poll_fd)
    }
}
