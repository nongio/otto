//! The password dialog itself: the panel on a layer-shell overlay, and the
//! conversation behind it.
//!
//! The polkit agent ([`crate::polkit`]) opens one per request. The password
//! goes to polkit's own helper ([`Conversation`]), which runs PAM for the
//! identity polkit asked about and tells polkitd itself.

use std::time::{Duration, Instant};

use otto_auth_ui::pam::{self, Attempt, Event, Message, Outcome};
use otto_auth_ui::{reader, Action, Appearance, Field, Finger, Panel, Status, User, View};
use otto_kit::{surfaces::LayerShellSurface, AppContext};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

/// How a dialog ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// polkit's helper accepted the password.
    Confirmed,
    /// Cancel, Escape, or the time ran out.
    Cancelled,
    /// Too many wrong answers.
    Failed,
}

/// How long the question stays up unanswered before it counts as a cancel.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Wrong answers allowed before the confirmation fails. PAM delays each one
/// itself; this is what keeps a confirmation from becoming a place to guess.
const MAX_FAILURES: u32 = 3;

/// The least time between one attempt ending and the next beginning, so a
/// stack that cannot run at all does not spin (as in otto-lock).
const RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How long to let a recognised fingerprint's mark finish before reporting.
const MARK_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long a painted frame is given to reach the screen before painting
/// again, so a compositor that stops sending frame callbacks cannot freeze
/// the panel.
const FRAME_TIMEOUT: Duration = Duration::from_millis(100);

/// Who checks the answers: polkit's helper, which runs PAM for the user and
/// reports to polkitd under `cookie` ([`crate::polkit::session`]).
#[derive(Debug, Clone)]
pub enum Conversation {
    Polkit { cookie: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Talking to PAM.
    Authenticating,
    /// A fingerprint was recognised; the mark is finishing.
    Accepted { since: Instant },
    /// Answered; on the way out.
    Done,
}

pub struct Dialog {
    surface: Option<LayerShellSurface>,
    panel: Option<Panel>,
    /// What polkit says the request is for, and the program that asked.
    message: String,
    requester: String,
    conversation: Conversation,
    verdict: Option<Verdict>,

    stage: Stage,
    user: Option<User>,
    /// Label above the field, as PAM phrased it.
    prompt: String,
    /// What has been typed. Wiped whenever it is dropped.
    input: String,
    /// PAM's `ECHO_OFF`: the field is masked.
    secret: bool,
    /// PAM has asked something that Enter would answer.
    question_pending: bool,
    error: Option<String>,
    info: Option<String>,
    /// A fingerprint reader is what the stack is waiting on.
    finger_pending: bool,
    /// The user chose to type rather than wait for the reader.
    password_requested: bool,
    /// Enter was pressed before PAM asked for the password.
    submit_when_asked: bool,

    attempt: Option<Attempt>,
    failures: u32,
    retry_at: Option<Instant>,
    deadline: Instant,

    animating_until: Option<Instant>,
    painted_at: Option<Instant>,
}

impl Dialog {
    pub fn new(
        message: String,
        requester: String,
        user: Option<User>,
        conversation: Conversation,
    ) -> Self {
        Self {
            surface: None,
            panel: None,
            message,
            requester,
            conversation,
            verdict: None,
            stage: Stage::Authenticating,
            user,
            prompt: otto_kit::t_owned!("lock-prompt-password"),
            input: String::new(),
            secret: true,
            question_pending: false,
            error: None,
            info: None,
            finger_pending: false,
            password_requested: false,
            submit_when_asked: false,
            attempt: None,
            failures: 0,
            retry_at: None,
            deadline: Instant::now() + TIMEOUT,
            animating_until: None,
            painted_at: None,
        }
    }

    /// How it ended, once it has.
    pub fn verdict(&self) -> Option<Verdict> {
        self.verdict
    }

    /// Put the dialog on screen and start the conversation.
    pub fn open(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // The whole output, dimmed, with the card in the middle: an overlay
        // above everything a client can put up, and the keyboard to itself
        // while it is there. No output named, so it opens where the pointer
        // is — where the user just asked for the change.
        let surface = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-authorize",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(-1),
        )?;
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

        let engine = AppContext::layers_renderer(|renderer| renderer.engine().clone())
            .ok_or("the layers engine is unavailable")?;
        let mut panel = Panel::new_dialog(
            Appearance::load(),
            engine,
            surface.base_surface().layer_node(),
        );
        panel.set_reason(&self.message, &self.requester);
        self.panel = Some(panel);
        self.surface = Some(surface);
        self.deadline = Instant::now() + TIMEOUT;
        // Opened from `on_update`, after the run loop's flush for this pass:
        // sent now, or the surface waits for whatever wakes the loop next.
        AppContext::flush();

        self.authenticate();
        Ok(())
    }

    /// Take the dialog off screen. The conversation, if still going, is
    /// dropped with it.
    pub fn close(&mut self) {
        self.attempt = None;
        self.clear_input();
        self.panel = None;
        if let Some(surface) = self.surface.take() {
            // The panel hangs from the surface's node in the process-wide
            // engine; the agent opens one per request, so it has to go too.
            if let Some(node) = surface.layer_node() {
                node.remove();
            }
            surface.destroy();
            AppContext::flush();
        }
    }

    fn awaiting_finger(&self) -> bool {
        self.finger_pending && !self.password_requested
    }

    fn accepts_input(&self) -> bool {
        self.stage == Stage::Authenticating && (self.question_pending || self.password_requested)
    }

    fn clear_input(&mut self) {
        pam::wipe(&mut self.input);
    }

    fn use_password(&mut self) {
        if !self.awaiting_finger() {
            return;
        }
        tracing::info!("password chosen over the fingerprint reader; waiting for PAM to ask");
        self.password_requested = true;
        self.secret = true;
        self.clear_input();
        self.prompt = otto_kit::t_owned!("lock-prompt-password");
        self.info = None;
        self.error = None;
    }

    fn view(&self) -> View<'_> {
        let field = if self.secret || self.password_requested {
            Field::Secret(self.input.chars().count())
        } else {
            Field::Text(&self.input)
        };

        let status = match (self.stage, self.error.as_deref(), self.info.as_deref()) {
            (Stage::Accepted { .. }, ..) => Some(Status::Fingerprint(
                otto_kit::t!("lock-status-authenticated"),
                Finger::Accepted,
            )),
            _ if self.awaiting_finger() => Some(Status::Fingerprint(
                self.error
                    .as_deref()
                    .or(self.info.as_deref())
                    .unwrap_or_else(|| otto_kit::t!("lock-status-place-finger")),
                Finger::Awaited,
            )),
            _ if self.submit_when_asked => {
                Some(Status::Info(otto_kit::t!("lock-status-waiting-for-reader")))
            }
            (_, Some(error), _) => Some(Status::Error(error)),
            (_, None, Some(info)) => Some(Status::Info(info)),
            (_, None, None) => None,
        };

        View {
            user: self.user.as_ref(),
            prompt: &self.prompt,
            field,
            status,
            session: None,
            busy: None,
            power: false,
            offer_password: self.awaiting_finger(),
        }
    }

    /// End with `verdict`. Only the first counts.
    pub fn finish(&mut self, verdict: Verdict) {
        if self.stage == Stage::Done {
            return;
        }
        self.stage = Stage::Done;
        // Dropping the attempt closes its answer channel, which fails the
        // conversation if PAM is still waiting on one.
        self.attempt = None;
        self.clear_input();
        self.verdict = Some(verdict);
    }

    fn authenticate(&mut self) {
        if self.attempt.is_some() || self.stage != Stage::Authenticating {
            return;
        }
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let Some(user) = self.user.as_ref() else {
            tracing::error!("no user to authenticate");
            self.finish(Verdict::Failed);
            return;
        };
        tracing::info!(user = %user.name, "Authenticating");
        let Conversation::Polkit { cookie } = &self.conversation;
        let (user, cookie) = (user.name.clone(), cookie.clone());
        self.attempt = Some(Attempt::with_conversation(move |events, answers| {
            crate::polkit::session::converse(&user, &cookie, events, answers)
        }));
        self.question_pending = false;
    }

    fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Some(event) = self.attempt.as_mut().and_then(Attempt::poll) {
            changed = true;
            match event {
                Event::Said(message) => self.said(message),
                Event::Ended(outcome) => {
                    self.attempt = None;
                    self.ended(outcome);
                }
            }
        }
        changed
    }

    /// Someone is still at it: the minute starts again. polkit's fingerprint
    /// step alone can take half of it before the password is even asked for.
    fn still_going(&mut self) {
        self.deadline = self.deadline.max(Instant::now() + TIMEOUT);
    }

    fn said(&mut self, message: Message) {
        match message {
            Message::Prompt { text, secret } => {
                self.still_going();
                self.prompt = prompt_label(&text);
                self.secret = secret;
                self.question_pending = true;
                self.info = None;
                self.finger_pending = false;
                if self.password_requested && secret {
                    self.password_requested = false;
                    if std::mem::take(&mut self.submit_when_asked) {
                        self.submit();
                    }
                } else {
                    self.password_requested = false;
                    self.submit_when_asked = false;
                    self.clear_input();
                }
            }
            Message::Info(text) => match reader::finger_request(&text) {
                Some(request) => {
                    self.finger_pending = true;
                    self.info = Some(reader::request_line(request, "lock"));
                }
                None => {
                    self.finger_pending |= reader::mentions_fingerprint(&text);
                    self.info = Some(text);
                }
            },
            Message::Error(text) => {
                self.error = Some(if reader::is_no_match(&text) {
                    reader::no_match_line("lock")
                } else {
                    text
                });
            }
        }
    }

    fn ended(&mut self, outcome: Outcome) {
        self.question_pending = false;
        self.clear_input();
        self.password_requested = false;
        self.submit_when_asked = false;

        match outcome {
            Outcome::Authenticated => {
                self.error = None;
                if self.awaiting_finger() {
                    self.stage = Stage::Accepted {
                        since: Instant::now(),
                    };
                    return;
                }
                tracing::info!("Confirmed");
                self.finish(Verdict::Confirmed);
            }
            Outcome::Denied(reason) => {
                self.failures += 1;
                tracing::info!(%reason, failures = self.failures, "Authentication failed");
                if self.failures >= MAX_FAILURES {
                    self.finish(Verdict::Failed);
                    return;
                }
                self.error = Some(reason);
                self.info = None;
                self.finger_pending = false;
                self.retry_at = Some(Instant::now() + RETRY_INTERVAL);
            }
        }
    }

    fn submit(&mut self) {
        if self.password_requested && !self.question_pending {
            tracing::info!("password typed before PAM asked; sending it when it does");
            self.submit_when_asked = true;
            return;
        }
        if !self.question_pending {
            return;
        }
        let answer = std::mem::take(&mut self.input);
        self.error = None;
        self.question_pending = false;
        self.password_requested = false;
        self.submit_when_asked = false;
        if let Some(attempt) = self.attempt.as_ref() {
            tracing::info!("answer handed to the conversation");
            attempt.answer(answer);
        }
    }

    /// Move time along: the fingerprint mark, the deadline, a due attempt.
    fn tick(&mut self) -> bool {
        let mut changed = false;

        if let Stage::Accepted { since } = self.stage {
            let settled = !self.panel.as_ref().is_some_and(Panel::wants_frames);
            if settled || since.elapsed() >= MARK_SETTLE_TIMEOUT {
                self.finish(Verdict::Confirmed);
                return true;
            }
        }

        if self.stage == Stage::Authenticating && Instant::now() >= self.deadline {
            tracing::info!("No answer in time; cancelling");
            self.finish(Verdict::Cancelled);
            return true;
        }

        let before = self.attempt.is_some();
        self.authenticate();
        changed |= self.attempt.is_some() != before;
        changed
    }

    fn activate(&mut self, action: Action) {
        match action {
            Action::Cancel => self.finish(Verdict::Cancelled),
            Action::UsePassword => self.use_password(),
            Action::CycleSession | Action::Power(_) => {}
        }
    }

    fn draw(&mut self) {
        let Some(mut panel) = self.panel.take() else {
            return;
        };
        panel.update(&self.view());
        self.panel = Some(panel);
        self.animating_until = Some(Instant::now() + Duration::from_millis(320));
        self.paint();
    }

    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .surface
                .as_ref()
                .is_some_and(|surface| surface.base_surface().frame_in_flight())
    }

    fn paint(&mut self) {
        if self.stage == Stage::Done {
            return;
        }
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        self.painted_at = Some(Instant::now());
        if let Some(panel) = self.panel.as_ref() {
            panel.animate();
        }
        let base = surface.base_surface();
        surface.draw(|canvas| base.render_layer_node(canvas));
    }

    pub fn configure(&mut self, width: i32, height: i32) {
        if let Some(panel) = self.panel.as_mut() {
            panel.set_size(width as f32, height as f32);
        }
        self.draw();
    }

    /// The compositor took the surface away: nothing was confirmed.
    pub fn closed(&mut self) {
        self.finish(Verdict::Cancelled);
    }

    pub fn update(&mut self) {
        if self.pump() || self.tick() {
            self.draw();
            return;
        }
        if self.frame_in_flight() {
            return;
        }
        let animating = self
            .animating_until
            .is_some_and(|deadline| Instant::now() < deadline);
        if animating || self.panel.as_ref().is_some_and(Panel::frame_due) {
            self.paint();
            return;
        }
        if self.animating_until.take().is_some() {
            self.paint();
        }
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        if self.frame_in_flight() {
            return Some(FRAME_TIMEOUT);
        }
        let mark = self.panel.as_ref().and_then(Panel::next_frame_in);
        let transition = self.animating_until.map(|_| Duration::from_millis(16));
        let retry = self
            .retry_at
            .map(|at| at.saturating_duration_since(Instant::now()));
        let accepted =
            matches!(self.stage, Stage::Accepted { .. }).then(|| Duration::from_millis(50));
        let deadline = Some(self.deadline.saturating_duration_since(Instant::now()));
        [mark, transition, retry, accepted, deadline]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn pointer(&mut self, events: &[PointerEvent]) {
        if self.stage != Stage::Authenticating {
            return;
        }
        let mut acted = false;
        for event in events {
            if !matches!(event.kind, PointerEventKind::Press { .. }) {
                continue;
            }
            let (x, y) = event.position;
            let action = self
                .panel
                .as_ref()
                .and_then(|panel| panel.action_at(x as f32, y as f32));
            if let Some(action) = action {
                self.activate(action);
                acted = true;
            }
        }
        if acted {
            self.draw();
        }
    }

    pub fn key(&mut self, event: &KeyEvent, state: wl_keyboard::KeyState) {
        if state != wl_keyboard::KeyState::Pressed || self.stage != Stage::Authenticating {
            return;
        }

        self.still_going();
        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => self.submit(),
            Keysym::Escape => {
                self.finish(Verdict::Cancelled);
                return;
            }
            Keysym::BackSpace if self.accepts_input() => {
                self.input.pop();
                self.error = None;
            }
            _ => {
                let printable: String = event
                    .utf8
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if printable.is_empty() {
                    return;
                }
                if self.awaiting_finger() {
                    self.use_password();
                }
                if self.accepts_input() {
                    self.input.push_str(&printable);
                    self.error = None;
                }
            }
        }
        self.draw();
    }
}

/// PAM prompts are written for a terminal: `"Password: "`.
fn prompt_label(text: &str) -> String {
    let label = text.trim().trim_end_matches(':').trim_end().to_string();
    if label.is_empty() {
        otto_kit::t_owned!("lock-prompt-password")
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> Dialog {
        Dialog::new(
            "Test".into(),
            "otto-test".into(),
            User::current(),
            Conversation::Polkit {
                cookie: "test".into(),
            },
        )
    }

    /// Three wrong passwords end the confirmation; fewer leave the field up
    /// with the reason.
    #[test]
    fn three_refusals_fail_the_confirmation() {
        let mut dialog = dialog();
        dialog.ended(Outcome::Denied("Authentication failure".into()));
        dialog.ended(Outcome::Denied("Authentication failure".into()));
        assert_eq!(dialog.stage, Stage::Authenticating);
        assert!(dialog.retry_at.is_some());
        dialog.ended(Outcome::Denied("Authentication failure".into()));
        assert_eq!(dialog.verdict(), Some(Verdict::Failed));
    }

    /// A late verdict — Escape after the password was accepted — changes
    /// nothing.
    #[test]
    fn nothing_follows_the_first_verdict() {
        let mut dialog = dialog();
        dialog.ended(Outcome::Authenticated);
        assert_eq!(dialog.verdict(), Some(Verdict::Confirmed));
        dialog.finish(Verdict::Cancelled);
        assert_eq!(dialog.verdict(), Some(Verdict::Confirmed));
    }

    #[test]
    fn the_password_is_masked_and_wiped() {
        let mut dialog = dialog();
        dialog.question_pending = true;
        dialog.input = "hunter2".into();
        assert!(matches!(dialog.view().field, Field::Secret(7)));
        dialog.ended(Outcome::Denied("no".into()));
        assert!(dialog.input.is_empty());
    }
}
