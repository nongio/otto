//! otto-lock — Otto's screen locker.
//!
//! It is an ordinary Wayland client. It asks the compositor for the session
//! with `ext-session-lock-v1`, draws the same panel the greeter draws
//! (`components/otto-auth-ui`) on every output, authenticates the session's own
//! user against PAM, and only then asks for the session back.
//!
//! Nothing here can unlock a session it did not lock, and nothing here can
//! reveal one: the lock lives in the compositor, so a locker that crashes
//! leaves the screen blank rather than the desktop. See `specs/lock-screen.md`.
//!
//! ```sh
//! OTTO_LOCKER_COMMAND=target/release/otto-lock   # test an uninstalled build
//! ```

mod pam;

use otto_auth_ui::{Action, Appearance, Field, Finger, Panel, PowerAction, Status, User, View};
use otto_kit::surfaces::{SessionLock, SessionLockSurface};
use otto_kit::{App, AppContext, AppRunner};
use pam::{Attempt, Event, Message, Outcome};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::wl_keyboard;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::Proxy;

/// How long to let the panel finish showing a recognised fingerprint before
/// unlocking regardless. The mark settles well inside this; it is only here so
/// a panel that never stops asking for frames cannot hold the session shut.
const MARK_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// How long a painted frame is given to reach the screen before painting
/// again. Frames are paced by the compositor's callbacks — this is the bound
/// on trusting it to send them, since a lock screen is not something anyone
/// can close and reopen.
const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// The least time between one attempt ending and the next beginning.
///
/// PAM rate-limits a wrong password itself, and that is where the delay
/// belongs. This is for the other case: a stack that cannot run at all —
/// no service file, a broken module — fails instantly, and without a floor
/// the locker would spin starting threads.
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// One output's lock screen.
struct Screen {
    output: WlOutput,
    surface: SessionLockSurface,
    panel: Panel,
    /// Set once the panel has been laid out for this surface's size. Before
    /// that there is no canvas and nothing to draw into.
    sized: bool,
}

/// Where the locker is.
enum Stage {
    /// Talking to PAM. Everything the user can do happens here.
    Authenticating,
    /// Authenticated by fingerprint, holding just long enough for the mark to
    /// finish and be seen. Nothing is outstanding — the pause is ours.
    Accepted { since: std::time::Instant },
    /// The session has been handed back; the process is on its way out.
    Unlocked,
}

/// The conversation, and what the panel should make of it.
///
/// Kept apart from the surfaces so a [`View`] borrowed from it can be handed
/// to panels that are being mutated at the same time.
struct Conversation {
    stage: Stage,
    /// Who is being authenticated: whoever this process runs as. A lock screen
    /// has no name to type, so this is known from the start.
    user: Option<User>,
    /// Label above the field, as PAM phrased it.
    prompt: String,
    /// The current input buffer.
    input: String,
    /// Whether what is being typed is a secret. PAM's `ECHO_OFF`, and the only
    /// thing that decides whether the field is masked.
    secret: bool,
    /// PAM has asked something that Enter would answer.
    question_pending: bool,
    error: Option<String>,
    info: Option<String>,
    /// A fingerprint reader is what the stack is waiting on. Set by the info
    /// message that announces it and cleared by the prompt that supersedes it,
    /// because a failed match replaces that message with an error while the
    /// reader is still waiting.
    finger_pending: bool,
    /// The user would rather type than wait for the reader. PAM is serialised
    /// and cannot be told so, so the panel switches to the field at once and
    /// the answer is kept until PAM asks for it.
    password_requested: bool,
    /// A password was submitted before PAM asked for one.
    submit_when_asked: bool,
}

impl Conversation {
    fn new() -> Self {
        Self {
            stage: Stage::Authenticating,
            user: User::current(),
            prompt: otto_kit::t_owned!("lock-prompt-password"),
            input: String::new(),
            secret: true,
            question_pending: false,
            error: None,
            info: None,
            finger_pending: false,
            password_requested: false,
            submit_when_asked: false,
        }
    }

    /// Whether a fingerprint is what the panel is currently illustrating.
    ///
    /// A failed match does not end this: `pam_fprintd` reports it and then asks
    /// for another finger, so the mark stays up under the error.
    fn awaiting_finger(&self) -> bool {
        self.finger_pending && !self.password_requested
    }

    /// Whether there is anywhere for a keystroke to go.
    ///
    /// Between one attempt and the next prompt PAM has asked nothing; with
    /// `pam_fprintd` that gap lasts as long as the reader waits. Anything typed
    /// into it would be echoed in the clear and then wiped by the prompt that
    /// cleared the buffer — better to take nothing than to take it and lose it.
    fn accepts_input(&self) -> bool {
        match self.stage {
            Stage::Authenticating => self.question_pending || self.password_requested,
            Stage::Accepted { .. } | Stage::Unlocked => false,
        }
    }

    /// Stop waiting for a finger and ask for a password instead.
    ///
    /// Nothing is sent: the module holding the stack will not be hurried. What
    /// changes is who the panel is asking — the field comes back, and whatever
    /// is typed into it waits for the prompt that arrives when the reader gives
    /// up.
    fn use_password(&mut self) {
        if !self.awaiting_finger() {
            return;
        }
        self.password_requested = true;
        self.secret = true;
        self.input.clear();
        self.prompt = otto_kit::t_owned!("lock-prompt-password");
        // The hint was about the reader, which is no longer what is being
        // asked for; the error, if any, was about a finger that missed.
        self.info = None;
        self.error = None;
    }

    /// Translate the conversation into something the panel can draw.
    fn view(&self) -> View<'_> {
        let field = if self.secret || self.password_requested {
            // The panel is given only the length of a secret, never the secret.
            Field::Secret(self.input.chars().count())
        } else {
            Field::Text(&self.input)
        };

        let status = match (&self.stage, self.error.as_deref(), self.info.as_deref()) {
            // The finger was recognised: the mark says so, but the line under
            // it should stop asking for one.
            (Stage::Accepted { .. }, ..) => Some(Status::Fingerprint(
                otto_kit::t!("lock-status-authenticated"),
                Finger::Accepted,
            )),
            // The reader is still what is being waited on, whatever it last
            // said — a missed finger is reported and then asked for again, and
            // taking the mark away for that would say the reader was done.
            _ if self.awaiting_finger() => Some(Status::Fingerprint(
                self.error
                    .as_deref()
                    .or(self.info.as_deref())
                    .unwrap_or_else(|| otto_kit::t!("lock-status-place-finger")),
                Finger::Awaited,
            )),
            // Waiting on a reader that is holding up a password nobody can send
            // yet. Saying so is the difference between a slow unlock and a
            // broken one.
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
            // A lock screen has no session to choose: this one already exists.
            session: None,
            // A greeter has a wait to narrate — the session it asked greetd to
            // exec. Unlocking has none: the session is already there, and the
            // frame that said so would be drawn after the compositor had taken
            // the lock surfaces away.
            busy: None,
            power: matches!(self.stage, Stage::Authenticating),
            // Only while a finger is what is being asked for; everywhere else
            // the field is already there to type into.
            offer_password: self.awaiting_finger(),
        }
    }
}

struct Locker {
    /// The lock itself. `None` before it has been asked for, and after the
    /// compositor has refused it.
    lock: Option<SessionLock>,
    /// Whether the compositor has confirmed the session is hidden. Nothing
    /// authenticates before then: until the blank has reached every screen,
    /// what is on them is still the desktop.
    locked: bool,
    screens: Vec<Screen>,
    session: Conversation,
    appearance: Appearance,
    /// The PAM stack, running on its own thread.
    attempt: Option<Attempt>,
    /// When the next attempt may start. See [`RETRY_INTERVAL`].
    retry_at: Option<std::time::Instant>,
    /// Repaint until this instant, so the panel's transitions are seen through
    /// rather than left frozen at their first step.
    animating_until: Option<std::time::Instant>,
    /// When the last frame was painted, which with the surfaces' frame
    /// callbacks is what paces the panel.
    painted_at: Option<std::time::Instant>,
    /// The minute the clock was last drawn showing. A session can be locked for
    /// hours; a clock that stopped when it was locked is worse than none.
    clock_minute: Option<i64>,
}

impl Locker {
    fn new() -> Self {
        Self {
            lock: None,
            locked: false,
            screens: Vec::new(),
            session: Conversation::new(),
            appearance: Appearance::load(),
            attempt: None,
            retry_at: None,
            animating_until: None,
            painted_at: None,
            clock_minute: None,
        }
    }

    /// Give every output a lock surface, and drop the ones whose output has
    /// gone.
    ///
    /// Reconciled rather than tracked: an output that appears while locked has
    /// to be covered, one that disappears takes its surface with it, and both
    /// are the same comparison. An output with no surface is not a hole in the
    /// lock — the compositor shows its own blank there.
    fn track_outputs(&mut self, ctx: &AppContext) {
        let Some(lock) = self.lock.as_ref() else {
            return;
        };
        if lock.is_released() {
            return;
        }

        let outputs: Vec<WlOutput> = ctx.output_state_ref().outputs().collect();

        self.screens.retain(|screen| {
            let alive = outputs.contains(&screen.output);
            if !alive {
                tracing::info!("output gone; dropping its lock surface");
                screen.surface.destroy();
            }
            alive
        });

        for output in outputs {
            if self
                .screens
                .iter()
                .any(|screen| screen.output.id() == output.id())
            {
                continue;
            }
            match lock.surface_for(&output) {
                Ok(surface) => {
                    let Some(engine) =
                        AppContext::layers_renderer(|renderer| renderer.engine().clone())
                    else {
                        tracing::error!("the layers engine is unavailable");
                        return;
                    };
                    let panel = Panel::new(
                        self.appearance.clone(),
                        engine,
                        surface.base_surface().layer_node(),
                    );
                    tracing::info!("lock surface created for a new output");
                    self.screens.push(Screen {
                        output,
                        surface,
                        panel,
                        sized: false,
                    });
                }
                Err(err) => tracing::error!(%err, "could not create a lock surface"),
            }
        }
    }

    /// Start a PAM conversation, if one is due and none is running.
    fn authenticate(&mut self) {
        if self.attempt.is_some() || !self.locked {
            return;
        }
        if self
            .retry_at
            .is_some_and(|at| std::time::Instant::now() < at)
        {
            return;
        }
        if !matches!(self.session.stage, Stage::Authenticating) {
            return;
        }

        let Some(user) = self.session.user.as_ref() else {
            self.session.error = Some(otto_kit::t_owned!("lock-error-no-user"));
            return;
        };

        tracing::info!(user = %user.name, "Authenticating");
        self.attempt = Some(Attempt::start(&user.name));
        self.session.question_pending = false;
        self.session.prompt = otto_kit::t_owned!("lock-prompt-password");
    }

    /// Collect whatever PAM has said. Returns whether the panel needs
    /// redrawing.
    ///
    /// Called every loop iteration rather than waited on, so a module that
    /// takes its time — a reader waiting for a finger — leaves the panel live.
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

    fn said(&mut self, message: Message) {
        // What the stack asked, in its own words. The only way to tell why a
        // reader is not being offered is to see whether the module announced
        // one at all — and the wording varies by module, locale and reader.
        tracing::debug!(?message, "PAM");
        match message {
            Message::Prompt { text, secret } => {
                self.session.prompt = prompt_label(&text);
                self.session.secret = secret;
                self.session.question_pending = true;
                // A prompt supersedes whatever hint preceded it: the reader is
                // no longer what is being waited on.
                self.session.info = None;
                self.session.finger_pending = false;

                // The question the user answered ahead of time has arrived.
                // Only a secret one, though: a password typed for a password
                // prompt must not be handed to a one-time-code prompt that
                // happens to come first.
                if self.session.password_requested && secret {
                    self.session.password_requested = false;
                    if std::mem::take(&mut self.session.submit_when_asked) {
                        self.submit();
                    }
                } else {
                    self.session.password_requested = false;
                    self.session.submit_when_asked = false;
                    self.session.input.clear();
                }
            }
            Message::Info(text) => {
                self.session.finger_pending |= pam::mentions_fingerprint(&text);
                self.session.info = Some(text);
            }
            Message::Error(text) => self.session.error = Some(text),
        }
    }

    fn ended(&mut self, outcome: Outcome) {
        self.session.question_pending = false;
        self.session.input.clear();
        self.session.password_requested = false;
        self.session.submit_when_asked = false;

        match outcome {
            Outcome::Authenticated => {
                self.session.error = None;
                // If it ended on a fingerprint, the mark is mid-animation and
                // cutting it off here is the last thing anyone sees of the lock
                // screen. Give it its moment; `tick` unlocks after it.
                if self.session.awaiting_finger() {
                    tracing::info!("Fingerprint accepted; holding the mark");
                    self.session.stage = Stage::Accepted {
                        since: std::time::Instant::now(),
                    };
                    return;
                }
                tracing::info!("Authenticated; nothing to show first");
                self.session.info = None;
                self.session.finger_pending = false;
                self.unlock();
            }
            Outcome::Denied(reason) => {
                tracing::info!(%reason, "Authentication failed");
                self.session.error = Some(reason);
                self.session.info = None;
                self.session.finger_pending = false;
                // Straight into another attempt, so the field — or the reader —
                // is there to try again with. PAM's own stack is what makes a
                // wrong password cost time.
                self.retry_at = Some(std::time::Instant::now() + RETRY_INTERVAL);
            }
        }
    }

    /// Hand the session back and leave.
    ///
    /// The unlock request goes out first and the process exits after the run
    /// loop has flushed it — a locker that exited first would leave a session
    /// nobody had unlocked, which is precisely the state this protocol makes
    /// permanent.
    fn unlock(&mut self) {
        let Some(lock) = self.lock.as_ref() else {
            return;
        };
        tracing::info!("Unlocking session");
        self.session.stage = Stage::Unlocked;
        lock.unlock();
        AppContext::request_exit();
    }

    /// Answer the prompt PAM is waiting on.
    fn submit(&mut self) {
        // A password typed ahead of the prompt for it. There is no question to
        // attach it to yet — `pam_fprintd` is still holding the conversation —
        // so it is remembered and sent when one arrives.
        if self.session.password_requested && !self.session.question_pending {
            self.session.submit_when_asked = true;
            return;
        }
        if !self.session.question_pending {
            return;
        }

        let answer = std::mem::take(&mut self.session.input);
        self.session.error = None;
        self.session.question_pending = false;
        self.session.password_requested = false;
        self.session.submit_when_asked = false;
        if let Some(attempt) = self.attempt.as_ref() {
            attempt.answer(answer);
        }
    }

    /// Move the wait for an accepted mark along, and start an attempt that is
    /// due. Returns whether anything changed.
    fn tick(&mut self) -> bool {
        let mut changed = false;

        if let Stage::Accepted { since } = self.session.stage {
            // The panel says when the mark has finished and been read. The
            // timeout is only there so a panel that never settles — or one that
            // failed to build at all — cannot hold the session shut.
            let settled = !self
                .screens
                .iter()
                .any(|screen| screen.panel.wants_frames());
            if settled || since.elapsed() >= MARK_SETTLE_TIMEOUT {
                // How long the mark actually got, and whether it finished or
                // ran out of patience. A hold of a millisecond or two means
                // the panel never asked for frames at all.
                tracing::info!(
                    held_ms = since.elapsed().as_millis() as u64,
                    settled,
                    "Mark done; unlocking"
                );
                self.session.info = None;
                self.session.finger_pending = false;
                self.unlock();
                changed = true;
            }
        }

        let before = self.attempt.is_some();
        self.authenticate();
        changed |= self.attempt.is_some() != before;

        changed
    }

    /// Whether the minute has turned since the clock was last drawn.
    fn clock_stale(&self) -> bool {
        let minute = chrono::Local::now().timestamp() / 60;
        self.clock_minute != Some(minute)
    }

    /// Act on a click on one of the panel's controls.
    fn activate(&mut self, action: Action) {
        match action {
            Action::UsePassword => self.session.use_password(),
            Action::Power(power) => self.power(power),
            // A lock screen shows no session picker, so nothing can ask for
            // this — but the panel's vocabulary is shared with the greeter.
            Action::CycleSession => {}
        }
    }

    /// Suspend, restart or shut down through systemd.
    ///
    /// Whether this is allowed from a locked session is polkit's call, not the
    /// locker's; if it refuses, say so on the panel rather than failing mute.
    fn power(&mut self, action: PowerAction) {
        // The verb is systemctl's, not the user's: it goes on the command
        // line, and the panel gets a message keyed by the action instead.
        let (verb, denied, failed) = match action {
            PowerAction::Suspend => (
                "suspend",
                "lock-power-suspend-denied",
                "lock-power-suspend-failed",
            ),
            PowerAction::Restart => (
                "reboot",
                "lock-power-restart-denied",
                "lock-power-restart-failed",
            ),
            PowerAction::Shutdown => (
                "poweroff",
                "lock-power-shutdown-denied",
                "lock-power-shutdown-failed",
            ),
        };
        tracing::info!(verb, "power action requested");

        // Captured rather than inherited: what systemd has to say about a
        // refusal is the whole diagnosis, and on a lock screen there is no
        // terminal for it to land in.
        match std::process::Command::new("systemctl").arg(verb).output() {
            Ok(output) if output.status.success() => {
                tracing::info!(verb, "systemctl accepted");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let reason = stderr.lines().next().unwrap_or("").trim().to_string();
                tracing::warn!(verb, status = ?output.status, %stderr, "systemctl refused");
                self.session.error = Some(if reason.is_empty() {
                    otto_kit::t_owned!(denied)
                } else {
                    // systemd's own diagnosis, which says more than anything
                    // here could; it comes back in the system's language.
                    reason
                });
            }
            Err(err) => {
                tracing::warn!(verb, %err, "could not run systemctl");
                self.session.error = Some(otto_kit::t_owned!(failed, error = err.to_string()));
            }
        }
    }

    /// Push the current state into every panel and paint.
    ///
    /// A state change starts transitions, which only appear if frames keep
    /// coming while they run — `idle_timeout` keeps them coming until then.
    fn draw(&mut self) {
        let view = self.session.view();
        for screen in self.screens.iter_mut() {
            if screen.sized {
                screen.panel.update(&view);
            }
        }

        self.animating_until =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(320));
        self.paint();
    }

    /// Whether the last painted frame is still on its way to the screen.
    ///
    /// Only until [`FRAME_TIMEOUT`]: a compositor that stops answering with
    /// frame callbacks must not be able to freeze the lock screen.
    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .screens
                .iter()
                .any(|screen| screen.surface.base_surface().frame_in_flight())
    }

    /// Paint every configured screen as its scene currently stands.
    fn paint(&mut self) {
        // The session has been handed back, and with it the lock surfaces: the
        // compositor has taken their role away, and a buffer committed to one
        // now is a commit to a surface that is no longer anything.
        if matches!(self.session.stage, Stage::Unlocked) {
            return;
        }
        if self.screens.iter().all(|screen| !screen.sized) {
            return;
        }
        self.painted_at = Some(std::time::Instant::now());

        if self.clock_stale() {
            self.clock_minute = Some(chrono::Local::now().timestamp() / 60);
            for screen in &self.screens {
                screen.panel.refresh_clock();
            }
            // The engine advances on its own thread, so the frame being painted
            // right now may still carry the old picture. Keep painting for a
            // moment rather than betting the whole minute on this one frame.
            self.animating_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(150));
        }

        for screen in &self.screens {
            if !screen.sized {
                continue;
            }
            // Continuous animation has to say so before every frame, or the
            // engine replays the picture it recorded for the last one.
            screen.panel.animate();
            // otto-kit hands over a canvas with the buffer scale already
            // applied, so the scene is laid out in logical points.
            let base = screen.surface.base_surface();
            screen.surface.draw(|canvas| base.render_layer_node(canvas));
        }
    }
}

/// PAM prompts are written for a terminal: `"Password: "`. The panel puts the
/// label above the field, where the punctuation reads as a typo.
fn prompt_label(text: &str) -> String {
    let label = text.trim().trim_end_matches(':').trim_end().to_string();
    if label.is_empty() {
        otto_kit::t_owned!("lock-prompt-password")
    } else {
        label
    }
}

impl App for Locker {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // The engine has to exist before any surface does: a surface creates
        // its own root layer node on construction, and that node is what a
        // panel's scene hangs off.
        AppContext::enable_layer_engine(1920.0, 1080.0);

        // Ask for the session straight away. The compositor blanks every output
        // before it answers, so the desktop is off the screen from here on —
        // `on_session_locked` is the confirmation that it has actually gone.
        self.lock = Some(SessionLock::acquire()?);
        Ok(())
    }

    fn on_session_locked(&mut self, _ctx: &AppContext) {
        self.locked = true;
        // Only now: until the blank has reached every screen, a fingerprint
        // reader could be authenticating over a visible desktop.
        self.authenticate();
        self.draw();
    }

    fn on_session_lock_finished(&mut self, _ctx: &AppContext) {
        // Either the compositor refused — one lock at a time, and something
        // else holds it — or the lock is over. Both mean the object is dead.
        if matches!(self.session.stage, Stage::Unlocked) {
            return;
        }
        tracing::error!("the compositor refused to lock the session");
        if let Some(lock) = self.lock.as_ref() {
            lock.abandon();
        }
        self.lock = None;
        AppContext::request_exit();
    }

    fn on_configure_lock_surface(
        &mut self,
        _ctx: &AppContext,
        lock_surface: &wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        width: i32,
        height: i32,
        _serial: u32,
    ) {
        let id = lock_surface.id();
        for screen in self.screens.iter_mut() {
            if screen.surface.lock_surface().id() != id {
                continue;
            }
            screen.panel.set_size(width as f32, height as f32);
            screen.sized = true;
        }
        self.draw();
    }

    fn on_update(&mut self, ctx: &AppContext) {
        self.track_outputs(ctx);

        if self.pump() || self.tick() {
            self.draw();
            return;
        }

        // The last frame has not reached the screen yet. Painting another one
        // now would only queue work the compositor has not asked for — and it
        // is the frame callback that wakes the loop for the next one.
        if self.frame_in_flight() {
            return;
        }

        // Keep painting while a transition is in flight, while the Touch ID
        // mark has a frame due, and when the minute turns.
        let animating = self
            .animating_until
            .is_some_and(|deadline| std::time::Instant::now() < deadline);
        let mark_due = self.screens.iter().any(|screen| screen.panel.frame_due());
        if animating || mark_due || self.clock_stale() {
            self.paint();
            return;
        }

        if self.animating_until.take().is_some() {
            // One last frame at the settled values, then go quiet.
            self.paint();
        }
    }

    /// Wake for the mark, for transitions, for an attempt that is due, and once
    /// a minute for the clock. Otherwise the loop can sleep until PAM speaks or
    /// a key is pressed — both of which wake it on their own.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        if self.frame_in_flight() {
            return Some(FRAME_TIMEOUT);
        }

        // The Touch ID mark paces itself; sleep exactly up to its next frame.
        let mark = self
            .screens
            .iter()
            .filter_map(|screen| screen.panel.next_frame_in())
            .min();
        // A transition needs frames until it settles, and unlike the mark it is
        // the engine that advances it, so ask at the rate we can present.
        let transition = self
            .animating_until
            .map(|_| std::time::Duration::from_millis(16));
        let retry = self
            .retry_at
            .map(|at| at.saturating_duration_since(std::time::Instant::now()));
        // The clock only changes on the minute, and nothing else needs the loop
        // awake in between.
        let clock = Some(std::time::Duration::from_secs(
            60 - (chrono::Local::now().timestamp() % 60).unsigned_abs(),
        ));

        [mark, transition, retry, clock].into_iter().flatten().min()
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        if !matches!(self.session.stage, Stage::Authenticating) {
            return;
        }

        let mut acted = false;
        for event in events {
            if !matches!(event.kind, PointerEventKind::Press { .. }) {
                continue;
            }
            // Surface-local logical coordinates, the same space the panel laid
            // its hitboxes out in — so the click has to be read against the
            // panel of the screen it landed on.
            let (x, y) = event.position;
            let action = self
                .screens
                .iter()
                .find(|screen| {
                    screen.surface.base_surface().wl_surface().id() == event.surface.id()
                })
                .and_then(|screen| screen.panel.action_at(x as f32, y as f32));
            tracing::info!(x, y, ?action, "press on the lock surface");
            if let Some(action) = action {
                self.activate(action);
                acted = true;
            }
        }

        if acted {
            self.draw();
        }
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // Nothing typed after the conversation succeeded should be able to
        // change its mind — including the brief pause the accepted mark gets.
        if state != wl_keyboard::KeyState::Pressed
            || !matches!(self.session.stage, Stage::Authenticating)
        {
            return;
        }

        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => self.submit(),
            Keysym::BackSpace if self.session.accepts_input() => {
                self.session.input.pop();
                self.session.error = None;
            }
            // There is nothing to escape to — the session stays locked either
            // way — so Escape clears what has been typed, which is what someone
            // who has lost their place in a masked field wants.
            Keysym::Escape => {
                self.session.input.clear();
                self.session.error = None;
            }
            _ => {
                // Anything the keymap turned into text goes into the buffer;
                // control characters are handled above.
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
                // Typing at a fingerprint prompt is a decision: someone who
                // reaches for the keyboard has stopped waiting for the reader,
                // and having to find the button first would be in the way.
                if self.session.awaiting_finger() {
                    self.session.use_password();
                }
                if self.session.accepts_input() {
                    self.session.input.push_str(&printable);
                    self.session.error = None;
                }
            }
        }

        self.draw();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Before the first string is looked up and before anything is drawn: the
    // catalogue is fixed by the first lookup, and the locker draws at once.
    otto_kit::i18n::init_from_desktop();

    AppRunner::new(Locker::new()).run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> Conversation {
        Conversation::new()
    }

    /// PAM writes its prompts for a terminal. The panel puts the label above
    /// the field, where a trailing colon reads as a mistake.
    #[test]
    fn prompt_labels_lose_their_terminal_punctuation() {
        assert_eq!(prompt_label("Password: "), "Password");
        assert_eq!(prompt_label("Verification code:"), "Verification code");
        assert_eq!(prompt_label("   "), otto_kit::t!("lock-prompt-password"));
    }

    /// A lock screen authenticates the user it runs as, so there is no name to
    /// type and no session to choose.
    #[test]
    fn the_panel_asks_for_nothing_but_the_password() {
        let session = conversation();
        let view = session.view();
        assert!(view.session.is_none(), "there is no session to pick");
        assert!(
            matches!(view.field, Field::Secret(0)),
            "a password is masked from the first keystroke"
        );
    }

    /// Between attempts PAM has asked nothing. Taking keystrokes then would
    /// echo them in the clear and then lose them to the prompt that clears the
    /// buffer.
    #[test]
    fn nothing_is_typed_into_a_conversation_that_asked_nothing() {
        let mut session = conversation();
        assert!(!session.accepts_input());

        session.question_pending = true;
        assert!(session.accepts_input());
    }

    /// The reader is not the only way in. A finger that is never going to be
    /// offered — the wrong hand, no enrolled print — must not leave the user
    /// with nothing to type into and no way to say so.
    #[test]
    fn the_reader_can_be_traded_for_the_password_field() {
        let mut session = conversation();
        session.finger_pending = true;
        session.info = Some("Place your finger on the reader".to_string());

        assert!(
            session.view().offer_password,
            "the way out should be on show"
        );
        assert!(!session.accepts_input(), "nothing to type into yet");

        session.use_password();
        assert!(!session.view().offer_password, "already taken");
        assert!(session.accepts_input(), "the field is what is being asked");
        assert!(
            !matches!(session.view().status, Some(Status::Fingerprint(..))),
            "the mark should go with the request for a finger"
        );
    }

    /// `pam_fprintd` reports a missed finger and then asks for another one.
    /// Taking the mark down for that says the reader is finished when it is
    /// still the thing being waited on — and takes the way out down with it.
    #[test]
    fn a_missed_finger_still_leaves_the_reader_up() {
        let mut locker = Locker::new();
        locker.session.finger_pending = true;
        locker.said(Message::Error("Failed to match fingerprint".to_string()));

        assert!(
            locker.session.awaiting_finger(),
            "the reader is still waiting"
        );
        assert!(locker.session.view().offer_password);
        assert!(matches!(
            locker.session.view().status,
            Some(Status::Fingerprint(
                "Failed to match fingerprint",
                Finger::Awaited
            ))
        ));
    }

    /// PAM is serialised: the password prompt does not exist until the reader
    /// gives up. Typing has to be possible before then anyway, or the way out
    /// is only a way of waiting differently.
    #[test]
    fn a_password_typed_before_the_prompt_is_sent_when_it_arrives() {
        let mut locker = Locker::new();
        locker.session.finger_pending = true;
        locker.session.use_password();
        locker.session.input = "hunter2".to_string();

        locker.submit();
        assert!(
            locker.session.submit_when_asked,
            "Enter should be remembered"
        );
        assert_eq!(
            locker.session.input, "hunter2",
            "the answer must not be lost"
        );
        assert!(matches!(
            locker.session.view().status,
            Some(Status::Info(_))
        ));

        // The reader gives up and `pam_unix` asks its question.
        locker.said(Message::Prompt {
            text: "Password:".to_string(),
            secret: true,
        });
        assert!(locker.session.input.is_empty(), "the buffer went with it");
        assert!(!locker.session.password_requested && !locker.session.submit_when_asked);
    }

    /// The answer was typed for a password prompt. A stack that asks for a
    /// one-time code first must not be handed it.
    #[test]
    fn a_queued_password_is_not_given_to_a_visible_prompt() {
        let mut locker = Locker::new();
        locker.session.finger_pending = true;
        locker.session.use_password();
        locker.session.input = "hunter2".to_string();
        locker.submit();

        locker.said(Message::Prompt {
            text: "Verification code:".to_string(),
            secret: false,
        });
        assert!(
            locker.session.input.is_empty(),
            "the password should be dropped"
        );
        assert!(!locker.session.submit_when_asked);
    }

    /// A wrong password leaves the panel showing why, and another attempt due —
    /// not a lock screen with nothing to type into.
    #[test]
    fn a_refusal_returns_to_the_field_with_the_reason() {
        let mut locker = Locker::new();
        locker.locked = true;
        locker.session.question_pending = true;
        locker.session.input = "wrong".to_string();

        locker.ended(Outcome::Denied("Authentication failure".to_string()));
        assert!(locker.session.input.is_empty(), "the attempt is over");
        assert!(matches!(
            locker.session.view().status,
            Some(Status::Error("Authentication failure"))
        ));
        assert!(locker.retry_at.is_some(), "another attempt should be due");
        assert!(
            matches!(locker.session.stage, Stage::Authenticating),
            "a refusal is not a way out of the lock"
        );
    }

    /// Nothing may authenticate before the compositor has confirmed the session
    /// is hidden: until then a fingerprint on the reader would be unlocking a
    /// desktop that is still on screen.
    #[test]
    fn authentication_waits_for_the_session_to_be_hidden() {
        let mut locker = Locker::new();
        locker.authenticate();
        assert!(locker.attempt.is_none(), "the session is not hidden yet");
    }

    /// A login that ended on a fingerprint pauses before unlocking, so the mark
    /// can finish and be seen.
    #[test]
    fn a_recognised_finger_is_shown_before_the_session_comes_back() {
        let mut locker = Locker::new();
        locker.session.finger_pending = true;
        locker.session.info = Some("Place your finger on the reader".to_string());

        locker.ended(Outcome::Authenticated);
        assert!(
            matches!(locker.session.stage, Stage::Accepted { .. }),
            "should hold on the accepted mark rather than unlock at once"
        );
        assert!(matches!(
            locker.session.view().status,
            Some(Status::Fingerprint(_, Finger::Accepted))
        ));
        assert!(
            locker.session.view().busy.is_none(),
            "a busy panel hides the field, and the mark is inside it"
        );
    }
}
