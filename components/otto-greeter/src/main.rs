//! otto-greeter — the login screen Otto shows in `--login` mode.
//!
//! It is an ordinary Wayland client: a fullscreen `wlr-layer-shell` surface on
//! the overlay layer with exclusive keyboard interactivity. Authentication is
//! delegated entirely to greetd over `$GREETD_SOCK` (see [`greetd`]).
//!
//! Without `GREETD_SOCK` it runs against a mock backend, so the UI can be
//! developed inside a normal Otto session:
//!
//! ```sh
//! cargo run -p otto-greeter          # password: otto
//! ```

mod greetd;
mod session;

use greetd::{AuthMessageType, Client, Request, Response};
use otto_auth_ui::{
    reader, Action, Appearance, Field, Finger, Panel, PowerAction, Status, User, View,
};
use otto_kit::{surfaces::LayerShellSurface, App, AppContext, AppRunner};
use session::Session;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};

/// How long to wait after `start_session` before concluding that the session
/// is not coming. greetd replaces this process on success, so staying alive
/// past this means the exec failed — or that a test daemon is standing in.
/// Without this the greeter would hang on "Starting session…" forever.
const SESSION_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long to let the panel finish showing a recognised fingerprint before
/// starting the session regardless. greetd replaces this process the moment
/// `start_session` succeeds, so the mark's result has to be shown *before* the
/// request goes out — and a panel that never stopped asking for frames would
/// otherwise hold up the login for good.
///
/// It is a safety net, not a deadline: the panel finishes well inside it, and
/// it only has to stay clear of however long the mark takes to settle.
const MARK_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// How long a painted frame is given to reach the screen before the greeter
/// paints regardless. Frames are paced by the compositor's frame callbacks —
/// that is what keeps an animating panel from painting faster than anyone can
/// see — and this is the bound on trusting it to send them.
const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// What greetd was asked, remembered until it answers.
///
/// greetd replies to every request with the same three responses, so the reply
/// alone does not say what it is a reply to — and `Success` means "the user is
/// authenticated" after one request and "the conversation you abandoned is
/// gone" after another. Reading a cancellation as an authentication is how
/// Escape used to start a session nobody had logged into, and, once greetd
/// refused that, do it again for as long as anyone watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    /// `create_session` or `post_auth_message_response`: the two requests that
    /// carry the conversation forward, and the only ones whose `Success` means
    /// the user is through.
    Auth,
    /// `start_session`.
    Start,
    /// `cancel_session`.
    Cancel,
    /// A request whose conversation was abandoned before it was answered —
    /// Escape pressed while a PAM module was still thinking. Its answer is
    /// about a login that no longer exists, so nothing acts on it.
    Abandoned,
}

/// Where the greeter is in the greetd conversation.
enum Stage {
    /// Collecting the username, before any session exists.
    Username,
    /// Answering an auth message from greetd.
    Prompt { secret: bool },
    /// Authenticated by fingerprint, and holding just long enough for the mark
    /// to finish and be seen. Nothing is outstanding with greetd here — the
    /// pause is ours, and `start_session` follows it.
    Accepted { since: std::time::Instant },
    /// Authenticated; greetd should be replacing this process any moment.
    Starting { since: std::time::Instant },
}

struct Greeter {
    surface: Option<LayerShellSurface>,
    /// The panel, shared with the lock screen. It knows nothing about greetd.
    /// Built in `on_app_ready`, once there is a surface to parent its scene to.
    panel: Option<Panel>,
    /// Repaint until this instant, so the panel's transitions are seen through
    /// rather than left frozen at their first step.
    animating_until: Option<std::time::Instant>,
    /// When the last frame was painted, which with the surface's frame callback
    /// is what paces the panel. `None` before anything has been drawn.
    painted_at: Option<std::time::Instant>,
    /// Requests sent and not yet answered, oldest first. greetd answers in
    /// order, so the front of this is what the next response is about. Usually
    /// at most one — a second joins it when the conversation is cancelled from
    /// under a PAM module that has yet to reply.
    outstanding: std::collections::VecDeque<Asked>,
    /// greetd is holding a session for us: `create_session` was accepted and
    /// nothing has cancelled it since. It has to be cancelled before another
    /// can be created, and cancelling one that does not exist is an error.
    conversation: bool,
    client: Client,
    stage: Stage,
    /// The username being authenticated.
    username: String,
    /// Looked up once when the username is submitted, so the panel can show a
    /// real name and avatar without re-reading the password database to draw.
    user: Option<User>,
    /// The current input buffer — username or auth answer depending on stage.
    input: String,
    /// The buffer holds a suggested username nobody has typed, so the first
    /// edit replaces it rather than adding to it. A field cannot show a
    /// selection, so this stands in for one: the offer is either taken with
    /// Enter or typed straight over, and it is never possible to end up
    /// submitting the default with a stray character on the end of it.
    input_is_a_suggestion: bool,
    /// Label shown above the input, from greetd's auth message.
    prompt: String,
    /// Last error, cleared on the next keystroke.
    error: Option<String>,
    /// Informational message from the PAM stack (fingerprint hints, etc.).
    info: Option<String>,
    /// A fingerprint reader is what the PAM stack is waiting on. Set by the
    /// info message that announces it and cleared by the prompt that
    /// supersedes it — kept as a fact of its own rather than re-read from the
    /// message, because a failed match replaces that message with an error and
    /// the reader is still waiting.
    finger_pending: bool,
    /// The user would rather type a password than wait for the reader. PAM is
    /// serialised and cannot be told so — `pam_fprintd` holds the stack until
    /// it times out or gives up — so the panel switches to the field at once
    /// and the answer is kept until PAM asks for it.
    password_requested: bool,
    /// A password was typed and submitted before PAM asked for one. It goes
    /// out with the first `secret` prompt to arrive.
    submit_when_asked: bool,
    sessions: Vec<Session>,
    session_index: usize,
    /// The minute the clock was last drawn showing. A login screen left up
    /// overnight otherwise keeps the time it appeared at — the clock draws
    /// from a closure the engine records once and replays.
    clock_minute: Option<i64>,
}

impl Greeter {
    fn new(client: Client) -> Self {
        let sessions = session::discover();
        let session_index = session::default_index(&sessions);
        tracing::info!(
            default = %sessions[session_index].name,
            available = sessions.len(),
            "Sessions discovered"
        );
        // Most logins are the same person on the same machine, and typing a
        // name you have typed a thousand times is work the login screen can do
        // for you. The suggestion is in the field, not applied behind it: it is
        // the same field, editable, and Escape empties it (see `reset`).
        let suggested = User::default_login();
        if let Some(user) = &suggested {
            tracing::info!(user = %user.name, "Offering the default account");
        }

        Self {
            surface: None,
            panel: None,
            animating_until: None,
            painted_at: None,
            outstanding: std::collections::VecDeque::new(),
            conversation: false,
            client,
            stage: Stage::Username,
            username: String::new(),
            input: suggested
                .as_ref()
                .map(|user| user.name.clone())
                .unwrap_or_default(),
            input_is_a_suggestion: suggested.is_some(),
            user: suggested,
            prompt: otto_kit::t_owned!("greeter-prompt-username"),
            error: None,
            info: None,
            finger_pending: false,
            password_requested: false,
            submit_when_asked: false,
            sessions,
            session_index,
            clock_minute: None,
        }
    }

    fn selected_session(&self) -> &Session {
        &self.sessions[self.session_index]
    }

    /// Whether greetd owes an answer to the conversation on screen. The panel
    /// stays live meanwhile — this only says that Enter would have nothing to
    /// answer.
    ///
    /// What is left over from a login somebody walked away from does not count.
    /// Those replies are outstanding for as long as the PAM module holding them
    /// takes — `pam_fprintd` waits out its whole timeout, and greetd reads
    /// nothing from us until it does — and Enter cannot be dead for that long:
    /// Escape and a name typed after it is somebody logging in, not somebody
    /// answering the question they just left.
    fn awaiting_reply(&self) -> bool {
        self.outstanding
            .iter()
            .any(|asked| !matches!(asked, Asked::Abandoned | Asked::Cancel))
    }

    /// Whether greetd has asked something that Enter could answer right now.
    fn has_a_question_pending(&self) -> bool {
        matches!(self.stage, Stage::Prompt { .. }) && !self.awaiting_reply()
    }

    /// Reset back to the username field, cancelling any half-finished
    /// conversation so greetd is ready for a new `create_session`.
    fn reset(&mut self, error: Option<String>) {
        if self.conversation {
            // Anything still in flight belongs to the conversation being
            // abandoned — a PAM module that has not answered yet, typically —
            // and its reply must not be read as this one's.
            for asked in self.outstanding.iter_mut() {
                *asked = Asked::Abandoned;
            }
            // greetd will not accept a new `create_session` while one is in
            // flight. The acknowledgement of this comes back through `pump`
            // like any other response. Sent directly rather than through
            // `send`, whose failure path is this function.
            if self.client.send(Request::CancelSession).is_ok() {
                self.outstanding.push_back(Asked::Cancel);
            }
            self.conversation = false;
        }
        self.stage = Stage::Username;
        // Emptied, not re-offered. Escape out of a login is how someone says
        // they are not the person the screen assumed they were, and putting
        // that name straight back would be answering a different question.
        self.input.clear();
        self.input_is_a_suggestion = false;
        self.username.clear();
        self.user = None;
        self.prompt = otto_kit::t_owned!("greeter-prompt-username");
        self.info = None;
        self.finger_pending = false;
        self.password_requested = false;
        self.submit_when_asked = false;
        self.error = error;
    }

    /// Send `request` and return to the event loop; the reply is picked up by
    /// [`Greeter::pump`] and read in the light of `asked`.
    fn send(&mut self, asked: Asked, request: Request) {
        if let Err(err) = self.client.send(request) {
            tracing::error!(error = %err, "greetd IPC failed");
            self.reset(Some(otto_kit::t_owned!(
                "greeter-error-service-unavailable",
                error = err.to_string()
            )));
            return;
        }
        self.outstanding.push_back(asked);
    }

    /// Collect whatever greetd has answered, if anything.
    ///
    /// Returns whether the panel needs redrawing. Called every loop iteration
    /// rather than waited on, so a PAM module that takes its time — a
    /// fingerprint reader waiting for a finger — leaves the panel live.
    fn pump(&mut self) -> bool {
        let mut changed = false;

        loop {
            match self.client.poll() {
                Ok(Some(response)) => {
                    match self.outstanding.pop_front() {
                        Some(asked) => self.handle(asked, response),
                        // greetd only ever speaks when spoken to, so this is
                        // either a protocol violation or a bug here; either
                        // way, acting on it would be guessing.
                        None => {
                            tracing::warn!(?response, "greetd answered a question we did not ask")
                        }
                    }
                    changed = true;
                }
                Ok(None) => return changed,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // greetd hung up. After `start_session` that is the handoff
                    // starting and this process is about to be replaced, so it
                    // is not worth alarming anyone about.
                    if matches!(self.stage, Stage::Starting { .. }) {
                        tracing::info!("greetd closed the connection; session is taking over");
                    } else {
                        tracing::error!("greetd closed the connection unexpectedly");
                        self.reset(Some(otto_kit::t_owned!("greeter-error-service-gone")));
                        changed = true;
                    }
                    // Nothing will answer the rest, and a queue that never
                    // drains would leave Enter dead for good.
                    self.outstanding.clear();
                    return changed;
                }
                Err(err) => {
                    tracing::error!(error = %err, "greetd IPC failed");
                    self.outstanding.clear();
                    self.reset(Some(otto_kit::t_owned!(
                        "greeter-error-service-unavailable",
                        error = err.to_string()
                    )));
                    return true;
                }
            }
        }
    }

    /// Act on one response from greetd, sending a follow-up where the protocol
    /// asks for one. `asked` is what this is a reply to — see [`Asked`].
    fn handle(&mut self, asked: Asked, response: Response) {
        // The conversation this belongs to was abandoned. Answering an auth
        // message for it would revive a login the user walked away from, and
        // its errors are about that login, not the one on screen.
        if asked == Asked::Abandoned {
            tracing::debug!(?response, "ignoring the reply to an abandoned request");
            return;
        }

        match response {
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    let secret = auth_message_type == AuthMessageType::Secret;
                    self.prompt = auth_message.trim_end_matches(':').to_string();
                    self.stage = Stage::Prompt { secret };
                    // A prompt supersedes whatever hint preceded it: the reader
                    // is no longer what is being waited on.
                    self.info = None;
                    self.finger_pending = false;

                    // The question the user answered ahead of time has arrived.
                    // Only a secret one, though: a password typed for a
                    // password prompt must not be handed to a one-time-code
                    // prompt that happens to come first.
                    if self.password_requested && secret {
                        self.password_requested = false;
                        if std::mem::take(&mut self.submit_when_asked) {
                            self.submit();
                        }
                    } else {
                        self.password_requested = false;
                        self.submit_when_asked = false;
                        self.input.clear();
                    }
                }
                // Info and error messages expect no input, only an empty
                // acknowledgement — which is also what lets the PAM module
                // carry on, so the message is on screen before that happens.
                AuthMessageType::Info => {
                    // greetd relays the module's own words, which are in the
                    // module's locale rather than the panel's — and greetd's
                    // environment is barer than a session's, so they are
                    // usually English. A request for a finger is said again
                    // from the catalogues; the rest keeps its wording.
                    match reader::finger_request(&auth_message) {
                        Some(request) => {
                            self.finger_pending = true;
                            self.info = Some(reader::request_line(request, "greeter"));
                        }
                        None => {
                            self.finger_pending |= reader::mentions_fingerprint(&auth_message);
                            self.info = Some(auth_message);
                        }
                    }
                    self.send(
                        Asked::Auth,
                        Request::PostAuthMessageResponse { response: None },
                    );
                }
                AuthMessageType::Error => {
                    self.error = Some(if reader::is_no_match(&auth_message) {
                        reader::no_match_line("greeter")
                    } else {
                        auth_message
                    });
                    self.send(
                        Asked::Auth,
                        Request::PostAuthMessageResponse { response: None },
                    );
                }
            },
            Response::Success => match asked {
                // The conversation is done. If it ended on a fingerprint, the
                // mark in the field is mid-animation and cutting it off here is
                // the last thing anyone sees of the login; give it its moment
                // first and start the session in `tick`.
                Asked::Auth => {
                    self.error = None;
                    if self.awaiting_finger() {
                        self.stage = Stage::Accepted {
                            since: std::time::Instant::now(),
                        };
                        return;
                    }
                    self.info = None;
                    self.finger_pending = false;
                    self.start_session();
                }
                // A real greetd is about to kill this process and exec the
                // session in its place, so there is nothing left to do but
                // wait — `tick` gives up if that never happens.
                Asked::Start => tracing::info!("Session started, waiting to be replaced"),
                // The conversation is gone, which is what was asked for. The
                // panel is already back at the username field.
                Asked::Cancel => tracing::debug!("Conversation cancelled"),
                Asked::Abandoned => unreachable!("returned above"),
            },
            Response::Error {
                error_type,
                description,
            } => {
                tracing::warn!(?error_type, %description, "greetd rejected the request");
                // A cancellation that failed leaves nothing to cancel, which is
                // where it was trying to get to. Resetting here would send
                // another one and answer it the same way, for ever.
                if asked == Asked::Cancel {
                    return;
                }
                self.reset(Some(description));
            }
        }
    }

    /// Whether a fingerprint is what the panel is currently illustrating — the
    /// PAM stack announced a reader, and the user has not asked to type a
    /// password instead.
    ///
    /// A failed match does not end this: `pam_fprintd` reports it and then
    /// asks for another finger, so the mark stays up under the error.
    fn awaiting_finger(&self) -> bool {
        self.finger_pending && !self.password_requested
    }

    /// Stop waiting for a finger and ask for a password instead.
    ///
    /// Nothing is sent: PAM is serialised, and the module holding the stack
    /// will not be hurried by anything the greeter says. What changes is who
    /// the panel is asking — the field comes back, and whatever is typed into
    /// it waits for the prompt that arrives when the reader gives up.
    fn use_password(&mut self) {
        if !self.awaiting_finger() {
            return;
        }
        self.password_requested = true;
        self.input.clear();
        self.prompt = otto_kit::t_owned!("greeter-prompt-password");
        // The hint was about the reader, which is no longer what is being
        // asked for; the error, if any, was about a finger that missed.
        self.info = None;
        self.error = None;
    }

    /// Ask greetd to exec the session. From here greetd owns what happens next.
    fn start_session(&mut self) {
        self.stage = Stage::Starting {
            since: std::time::Instant::now(),
        };
        let session = self.selected_session().clone();
        tracing::info!(session = %session.name, cmd = ?session.command, "Starting session");
        self.send(
            Asked::Start,
            Request::StartSession {
                cmd: session.command,
                env: Vec::new(),
            },
        );
    }

    /// Move the two waits along: letting an accepted fingerprint be seen, and
    /// giving up on a session that never took over.
    ///
    /// Returns whether anything changed, so the caller knows to redraw.
    fn tick(&mut self) -> bool {
        match self.stage {
            // The panel says when the mark has finished and been read. The
            // timeout is only there so a panel that never settles — or one that
            // failed to build at all — cannot strand the login.
            Stage::Accepted { since } => {
                let settled = !self.panel.as_ref().is_some_and(Panel::wants_frames);
                if !settled && since.elapsed() < MARK_SETTLE_TIMEOUT {
                    return false;
                }
                self.info = None;
                self.finger_pending = false;
                self.start_session();
                true
            }
            Stage::Starting { since } if since.elapsed() >= SESSION_START_TIMEOUT => {
                let session = self.selected_session().name.clone();
                tracing::warn!(%session, "Session was started but the greeter is still running");
                self.reset(Some(otto_kit::t_owned!(
                    "greeter-error-session-did-not-start",
                    session = session
                )));
                true
            }
            _ => false,
        }
    }

    /// Whether there is anything for a keystroke to go into.
    ///
    /// Between `create_session` and the first prompt greetd has asked nothing:
    /// with `pam_fprintd` that gap lasts as long as the reader waits. Anything
    /// typed into it would be echoed in the clear — the field is only masked
    /// once a `secret` prompt says so — and then wiped when that prompt
    /// arrived and cleared the buffer. Better to take nothing than to take it
    /// and lose it.
    fn accepts_input(&self) -> bool {
        match self.stage {
            // Once the password has been asked for there is somewhere for
            // keystrokes to go, even though greetd has not asked yet: the
            // field is masked, and the answer is held until the prompt comes.
            Stage::Username => !self.conversation || self.password_requested,
            Stage::Prompt { .. } => true,
            Stage::Accepted { .. } | Stage::Starting { .. } => false,
        }
    }

    /// Clear a suggested username the moment it is edited, and say whether
    /// there was one. The card goes back to nobody with it: the avatar and
    /// name on it belong to the account being offered, and the next keystroke
    /// is somebody saying it is not theirs.
    fn take_over_the_suggestion(&mut self) -> bool {
        if !self.input_is_a_suggestion {
            return false;
        }
        self.input.clear();
        self.input_is_a_suggestion = false;
        self.user = None;
        true
    }

    /// Handle Enter for the current stage.
    fn submit(&mut self) {
        // A password typed ahead of the prompt for it. There is no question to
        // attach it to yet — `pam_fprintd` is still holding the conversation —
        // so it is remembered and sent when one arrives.
        if self.password_requested && !self.has_a_question_pending() {
            self.submit_when_asked = true;
            return;
        }

        // Answering before greetd has asked would desynchronise the
        // conversation — it happens when someone types ahead while a PAM
        // module is still thinking.
        if self.awaiting_reply() {
            return;
        }

        match self.stage {
            Stage::Username => {
                let username = self.input.trim().to_string();
                if username.is_empty() {
                    return;
                }
                self.username = username.clone();
                self.user = Some(User::lookup(&username));
                self.error = None;
                // The name has been taken; who it is now shows as the avatar
                // and the name on the card. Leaving it in the field as well
                // makes the wait that follows — `pam_fprintd` can hold it for
                // as long as nobody touches the reader — look like the step
                // before it, as though Enter had done nothing.
                self.input.clear();
                self.input_is_a_suggestion = false;
                self.prompt = otto_kit::t_owned!("greeter-prompt-authenticating");
                // From here greetd is holding a session that has to be
                // cancelled before another can be created — including when the
                // reply to this is an error.
                self.conversation = true;
                self.send(Asked::Auth, Request::CreateSession { username });
            }
            Stage::Prompt { .. } => {
                let answer = std::mem::take(&mut self.input);
                self.error = None;
                // Whatever was queued has now gone out as this answer.
                self.password_requested = false;
                self.submit_when_asked = false;
                self.send(
                    Asked::Auth,
                    Request::PostAuthMessageResponse {
                        response: Some(answer),
                    },
                );
            }
            Stage::Accepted { .. } | Stage::Starting { .. } => {}
        }
    }

    /// Translate the greetd conversation into something the panel can draw.
    fn view(&self) -> View<'_> {
        let field = match self.stage {
            // The panel is given only the length of a secret, never the secret.
            Stage::Prompt { secret: true } => Field::Secret(self.input.chars().count()),
            // A password typed before PAM has asked for it is a secret too,
            // and must not be echoed while it waits.
            _ if self.password_requested => Field::Secret(self.input.chars().count()),
            _ => Field::Text(&self.input),
        };

        let status = match (&self.stage, self.error.as_deref(), self.info.as_deref()) {
            // The finger was recognised: the mark, not the wording, is what
            // says so, but the line under it should stop asking for a finger.
            (Stage::Accepted { .. }, ..) => Some(Status::Fingerprint(
                otto_kit::t!("greeter-status-authenticated"),
                Finger::Accepted,
            )),
            // The reader is still what is being waited on, whatever it last
            // said — a missed finger is reported and then asked for again, and
            // taking the mark away for that would say the reader was done.
            _ if self.awaiting_finger() => Some(Status::Fingerprint(
                self.error
                    .as_deref()
                    .or(self.info.as_deref())
                    .unwrap_or_else(|| otto_kit::t!("greeter-status-place-finger")),
                Finger::Awaited,
            )),
            // Waiting on a reader that is holding up a password nobody can
            // send yet. Saying so is the difference between a slow login and a
            // broken one.
            _ if self.submit_when_asked => Some(Status::Info(otto_kit::t!(
                "greeter-status-waiting-for-reader"
            ))),
            (_, Some(error), _) => Some(Status::Error(error)),
            (_, None, Some(info)) => Some(Status::Info(info)),
            (_, None, None) => None,
        };

        View {
            user: self.user.as_ref(),
            prompt: &self.prompt,
            field,
            status,
            session: Some(&self.selected_session().name),
            // Not while accepted: the mark is inside the field, and a busy
            // panel fades the field away with it.
            busy: matches!(self.stage, Stage::Starting { .. })
                .then_some(otto_kit::t!("greeter-status-starting-session")),
            // Offering to power off mid-handoff would race greetd's exec.
            power: matches!(self.stage, Stage::Username | Stage::Prompt { .. }),
            // Only while a finger is what is being asked for: everywhere else
            // the field is already there to type into.
            offer_password: self.awaiting_finger(),
        }
    }

    /// Push the current state into the panel's scene and paint a frame.
    ///
    /// A state change starts transitions, which only appear if frames keep
    /// coming while they run — `idle_timeout` keeps them coming until then.
    fn draw(&mut self) {
        // The view borrows the conversation state and `update` needs the panel
        // mutably — both parts of `self`. Moving the panel aside for the call
        // is simpler than splitting the struct to satisfy the borrow checker.
        let Some(mut panel) = self.panel.take() else {
            return;
        };
        panel.update(&self.view());
        self.panel = Some(panel);

        self.animating_until =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(320));
        self.paint();
    }

    /// Whether the last painted frame is still on its way to the screen, in
    /// which case there is nothing to gain by painting another one yet.
    ///
    /// Only until [`FRAME_TIMEOUT`], though: a compositor that stops answering
    /// with frame callbacks must not be able to freeze the login screen, which
    /// is not something anyone can close and reopen.
    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .surface
                .as_ref()
                .is_some_and(|surface| surface.base_surface().frame_in_flight())
    }

    /// Whether the minute has turned since the clock was last drawn.
    fn clock_stale(&self) -> bool {
        self.clock_minute != Some(chrono::Local::now().timestamp() / 60)
    }

    /// Paint the scene as it currently stands, without touching its state.
    /// This is what an in-flight transition needs on every frame.
    fn paint(&mut self) {
        if self.surface.is_none() {
            return;
        }
        self.painted_at = Some(std::time::Instant::now());

        if self.clock_stale() {
            self.clock_minute = Some(chrono::Local::now().timestamp() / 60);
            if let Some(panel) = self.panel.as_ref() {
                panel.refresh_clock();
            }
            // The engine advances on its own thread, so this frame may still
            // carry the old picture; keep painting for a moment.
            self.animating_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(150));
        }

        // Continuous animation has to say so before every frame, or the engine
        // replays the picture it recorded for the last one.
        if let Some(panel) = self.panel.as_ref() {
            panel.animate();
        }
        // otto-kit hands over a canvas with the buffer scale already applied,
        // so the scene is laid out in logical points and nothing scales twice.
        let surface = self.surface.as_ref().expect("checked just above");
        let base = surface.base_surface();
        surface.draw(|canvas| base.render_layer_node(canvas));
    }

    /// Act on a click on one of the panel's controls.
    fn activate(&mut self, action: Action) {
        match action {
            Action::CycleSession => self.cycle_session(),
            Action::Power(power) => self.power(power),
            Action::UsePassword => self.use_password(),
        }
    }

    fn cycle_session(&mut self) {
        if !self.sessions.is_empty() {
            self.session_index = (self.session_index + 1) % self.sessions.len();
        }
    }

    /// Suspend, restart or shut down through systemd.
    ///
    /// Whether an unprivileged greeter may do this is polkit's call, not the
    /// greeter's; if it refuses, say so on the panel rather than failing mute.
    fn power(&mut self, action: PowerAction) {
        // The verb is systemctl's, not the user's: it goes on the command
        // line, and the panel gets a message keyed by the action instead.
        let (verb, denied, failed) = match action {
            PowerAction::Suspend => (
                "suspend",
                "greeter-power-suspend-denied",
                "greeter-power-suspend-failed",
            ),
            PowerAction::Restart => (
                "reboot",
                "greeter-power-restart-denied",
                "greeter-power-restart-failed",
            ),
            PowerAction::Shutdown => (
                "poweroff",
                "greeter-power-shutdown-denied",
                "greeter-power-shutdown-failed",
            ),
        };

        match std::process::Command::new("systemctl").arg(verb).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::warn!(verb, ?status, "systemctl refused");
                self.error = Some(otto_kit::t_owned!(denied));
            }
            Err(err) => {
                tracing::warn!(verb, %err, "could not run systemctl");
                self.error = Some(otto_kit::t_owned!(failed));
            }
        }
    }
}

impl App for Greeter {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        // The engine has to exist before the surface does: a surface creates
        // its own root layer node on construction, and that node is what the
        // panel's scene hangs off. The size here is provisional — the real one
        // arrives with the first configure.
        AppContext::enable_layer_engine(1920.0, 1080.0);

        // Anchoring all four edges with size 0x0 makes the compositor size the
        // surface to the whole output. Exclusive keyboard interactivity means
        // nothing else can receive input while the greeter is up.
        let surface = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-greeter",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(-1),
        )?;
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);

        let engine = AppContext::layers_renderer(|renderer| renderer.engine().clone())
            .ok_or("the layers engine is unavailable")?;
        self.panel = Some(Panel::new(
            Appearance::load(),
            engine,
            surface.base_surface().layer_node(),
        ));

        self.surface = Some(surface);

        // A name the screen filled in itself is a question already answered:
        // making someone press Enter to confirm it puts a step in front of the
        // thing they actually came to do. Submitting it here means the first
        // screen is the password field or the reader. Escape still goes back to
        // an empty field, for whoever is not the account being offered.
        //
        // Done here rather than in `new` so the conversation starts with a
        // panel to draw its result on: `pam_fprintd` announces the reader
        // almost at once, and there would be nothing to show it with.
        if self.input_is_a_suggestion {
            self.submit();
        }

        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        if let Some(panel) = self.panel.as_mut() {
            panel.set_size(width as f32, height as f32);
        }
        self.draw();
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // Collect anything greetd has said since the last pass. The socket is
        // in the loop's poll set, so this runs when it has something to say.
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

        // Keep painting while a transition is in flight, and while the Touch ID
        // mark has a frame due. The engine advances transitions on its own
        // thread; this is what puts the result on screen.
        let animating = self
            .animating_until
            .is_some_and(|deadline| std::time::Instant::now() < deadline);
        if animating || self.panel.as_ref().is_some_and(Panel::frame_due) || self.clock_stale() {
            self.paint();
            return;
        }

        if self.animating_until.take().is_some() {
            // One last frame at the settled values, then go quiet.
            self.paint();
        }
    }

    /// Tick while the panel is animating, and while waiting for the session to
    /// take over. Otherwise the loop can sleep until the next key press.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        // A frame is in flight: the compositor's callback is what wakes the
        // loop for the next one. The timer is only the way out of a callback
        // that never comes — see `frame_in_flight`.
        if self.frame_in_flight() {
            return Some(FRAME_TIMEOUT);
        }

        // The Touch ID mark paces itself; sleep exactly up to its next frame.
        let mark = self.panel.as_ref().and_then(Panel::next_frame_in);
        // A transition needs frames until it settles, and unlike the mark it is
        // the engine that advances it, so ask at the rate we can present.
        let transition = self
            .animating_until
            .map(|_| std::time::Duration::from_millis(16));
        // greetd replaces this process on a successful login; if that never
        // happens, `tick` has to be reached to say so.
        let session = matches!(self.stage, Stage::Starting { .. })
            .then(|| std::time::Duration::from_millis(250));

        // The clock only changes on the minute, and nothing else needs the
        // loop awake in between.
        let clock = Some(std::time::Duration::from_secs(
            60 - (chrono::Local::now().timestamp() % 60).unsigned_abs(),
        ));

        [mark, transition, session, clock]
            .into_iter()
            .flatten()
            .min()
    }

    /// Wake the loop when greetd speaks. Without this the conversation would
    /// have to be polled, which for a fingerprint reader means polling for as
    /// long as nobody touches it.
    fn poll_fds(&self) -> Vec<std::os::fd::RawFd> {
        self.client.as_raw_fd().into_iter().collect()
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        // Once authentication has succeeded the panel is inert, and its
        // controls are not drawn — so there is nothing left to click.
        if matches!(self.stage, Stage::Accepted { .. } | Stage::Starting { .. }) {
            return;
        }

        let mut acted = false;
        for event in events {
            if !matches!(event.kind, PointerEventKind::Press { .. }) {
                continue;
            }
            // Surface-local logical coordinates, the same space the panel laid
            // its hitboxes out in.
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

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // Nothing typed after the conversation succeeded should be able to
        // change its mind — including the brief pause the accepted mark gets,
        // where a stray Escape would otherwise cancel a session that greetd has
        // already authenticated.
        if state != wl_keyboard::KeyState::Pressed
            || matches!(self.stage, Stage::Accepted { .. } | Stage::Starting { .. })
        {
            return;
        }

        match event.keysym {
            Keysym::Return | Keysym::KP_Enter => self.submit(),
            Keysym::BackSpace if self.accepts_input() => {
                if self.take_over_the_suggestion() {
                    // Backspace over a suggestion clears all of it, the way it
                    // would over a selection. Rubbing out a name character by
                    // character to type another one is not an edit anybody
                    // wants to make.
                } else {
                    self.input.pop();
                }
                self.error = None;
            }
            // Escape is exempt: a login waiting on a finger nobody is going to
            // give it is exactly when there has to be a way out.
            Keysym::Escape => self.reset(None),
            Keysym::Tab => self.cycle_session(),
            _ => {
                // Anything the keymap turned into text goes into the buffer;
                // control characters (Enter, Tab, …) are handled above.
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
                if self.awaiting_finger() {
                    self.use_password();
                }
                if self.accepts_input() {
                    self.take_over_the_suggestion();
                    self.input.push_str(&printable);
                    self.error = None;
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
    // catalogue is fixed by the first lookup, and the greeter draws at once.
    otto_kit::i18n::init_from_desktop();

    let client = Client::connect()?;
    AppRunner::new(Greeter::new(client)).run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use greetd::ErrorType;
    use std::time::{Duration, Instant};

    fn greeter() -> Greeter {
        Greeter::new(Client::Mock {
            awaiting_password: false,
            pending: None,
        })
    }

    /// A greeter that is offering a default account, whatever the password
    /// database of the machine running the test happens to hold.
    fn greeter_offering(name: &str) -> Greeter {
        let mut greeter = greeter();
        greeter.input = name.to_string();
        greeter.input_is_a_suggestion = true;
        greeter.user = Some(User::lookup(name));
        greeter
    }

    /// The offered name is a suggestion, not a prefix. Typing over it has to
    /// replace it — a field cannot show a selection, so the first keystroke is
    /// what stands in for one, and appending would silently build a username
    /// nobody meant to type.
    #[test]
    fn typing_replaces_the_suggested_name() {
        let mut greeter = greeter_offering("riccardo");

        assert!(greeter.take_over_the_suggestion());
        greeter.input.push('a');
        assert_eq!(greeter.input, "a");
        assert!(
            greeter.user.is_none(),
            "the card must stop showing an account that is being typed over"
        );

        // Only the first edit; after that the field is an ordinary one.
        assert!(!greeter.take_over_the_suggestion());
        greeter.input.push_str("da");
        assert_eq!(greeter.input, "ada");
    }

    /// Backspace over a suggestion clears the whole thing, as it would over a
    /// selection — rubbing a name out letter by letter to type another is not
    /// an edit anyone wants to make.
    #[test]
    fn backspace_clears_the_whole_suggested_name() {
        let mut greeter = greeter_offering("riccardo");

        assert!(greeter.take_over_the_suggestion());
        assert!(greeter.input.is_empty());
    }

    /// Enter takes the suggestion as typed, and it stops being one: the name
    /// has been submitted, and the field it came from is cleared behind it.
    #[test]
    fn the_suggested_name_can_be_submitted_as_it_stands() {
        let mut greeter = greeter_offering("riccardo");

        greeter.submit();
        assert_eq!(greeter.username, "riccardo");
        assert!(greeter.conversation);
        assert!(!greeter.input_is_a_suggestion);
        assert!(greeter.input.is_empty());
    }

    /// Escape out of a login is somebody saying they are not who the screen
    /// assumed. Putting the same name straight back would answer a different
    /// question — the field is emptied and left ready to type into.
    #[test]
    fn escape_empties_the_suggested_name_rather_than_re_offering_it() {
        let mut greeter = greeter_offering("riccardo");
        greeter.submit();

        greeter.reset(None);
        assert!(matches!(greeter.stage, Stage::Username));
        assert!(greeter.input.is_empty(), "nothing to type over");
        assert!(!greeter.input_is_a_suggestion);
        assert!(greeter.user.is_none());
        assert!(
            greeter.accepts_input(),
            "the field has to take a name straight away"
        );
    }

    /// And then logging in again is the whole point of having escaped. The
    /// reader still holds the request from the login that was left, and greetd
    /// answers nothing — not even the cancellation — until it lets go: waiting
    /// for that queue to drain left Enter dead for the reader's entire timeout,
    /// with a name typed and nothing happening.
    #[test]
    fn a_name_typed_after_escape_logs_in_without_waiting_for_the_reader() {
        let mut greeter = waiting_for_a_finger();
        greeter.reset(None);
        assert!(
            !greeter.outstanding.is_empty(),
            "the abandoned request and the cancellation are both still owed"
        );

        greeter.input = "riccardo".to_string();
        greeter.submit();

        assert_eq!(greeter.username, "riccardo", "Enter has to start the login");
        assert!(greeter.conversation);
        assert_eq!(
            greeter.outstanding.back(),
            Some(&Asked::Auth),
            "the new create_session is queued behind the cancellation"
        );
    }

    #[test]
    fn waiting_for_the_session_is_not_an_error_yet() {
        let mut greeter = greeter();
        greeter.stage = Stage::Starting {
            since: Instant::now(),
        };
        assert!(!greeter.tick(), "should still be waiting");
        assert!(matches!(greeter.stage, Stage::Starting { .. }));
    }

    /// greetd replaces this process on a successful `start_session`. Still
    /// being alive well past that means the exec failed, and the greeter has
    /// to recover rather than sit on "Starting session…" forever.
    #[test]
    fn gives_up_once_the_session_fails_to_take_over() {
        let mut greeter = greeter();
        greeter.username = "riccardo".to_string();
        greeter.stage = Stage::Starting {
            since: Instant::now() - SESSION_START_TIMEOUT - Duration::from_millis(1),
        };

        assert!(greeter.tick(), "should have given up");
        assert!(
            matches!(greeter.stage, Stage::Username),
            "must return to the username field so the user can retry"
        );
        assert!(greeter.error.is_some(), "must say why it gave up");
        assert!(greeter.input.is_empty());
        assert!(greeter.username.is_empty());
    }

    #[test]
    fn ticking_does_nothing_while_the_conversation_is_live() {
        let mut greeter = greeter();
        assert!(!greeter.tick());

        greeter.stage = Stage::Prompt { secret: true };
        assert!(!greeter.tick());
        assert!(matches!(greeter.stage, Stage::Prompt { .. }));
    }

    /// A login that ended on a fingerprint pauses before starting the session,
    /// so the mark can finish and be seen. greetd kills this process the moment
    /// `start_session` succeeds, so a pause taken afterwards would never be
    /// drawn — it has to come first.
    #[test]
    fn a_recognised_finger_is_shown_before_the_session_starts() {
        let mut greeter = greeter();
        greeter.stage = Stage::Prompt { secret: true };
        greeter.info = Some("Place your finger on the reader".to_string());
        greeter.finger_pending = true;

        greeter.handle(Asked::Auth, Response::Success);
        assert!(
            matches!(greeter.stage, Stage::Accepted { .. }),
            "should hold on the accepted mark rather than start straight away"
        );
        assert!(
            matches!(
                greeter.view().status,
                Some(Status::Fingerprint(_, Finger::Accepted))
            ),
            "the panel should be told the finger was accepted"
        );
        assert!(
            greeter.view().busy.is_none(),
            "a busy panel hides the field, and the mark is inside it"
        );

        // No panel here, so `tick` falls back to its timeout — which is the
        // safety net a panel that never settles would rely on.
        greeter.stage = Stage::Accepted {
            since: Instant::now() - MARK_SETTLE_TIMEOUT - Duration::from_millis(1),
        };
        assert!(greeter.tick(), "the hold should end");
        assert!(matches!(greeter.stage, Stage::Starting { .. }));
    }

    /// A password login has no mark to wait for and must not be slowed down.
    #[test]
    fn a_password_login_starts_the_session_at_once() {
        let mut greeter = greeter();
        greeter.stage = Stage::Prompt { secret: true };

        greeter.handle(Asked::Auth, Response::Success);
        assert!(
            matches!(greeter.stage, Stage::Starting { .. }),
            "nothing to show, so nothing to wait for"
        );
    }

    /// `pam_fprintd` can leave the greeter with nothing to do for as long as
    /// nobody touches the reader. The panel has to show that the login moved
    /// on — a field still holding the name under a "Username" label, with the
    /// mark pulsing away below it, reads as an Enter that did nothing — and it
    /// must not take keystrokes it would echo in the clear and then lose to
    /// the prompt that eventually clears the buffer.
    #[test]
    fn the_username_step_ends_when_the_username_is_taken() {
        let mut greeter = greeter();
        greeter.input = "riccardo".to_string();

        greeter.submit();
        assert!(greeter.conversation, "greetd is holding a session now");
        assert!(
            greeter.input.is_empty(),
            "the field should not still be showing the name"
        );
        assert!(
            !greeter.accepts_input(),
            "greetd has not asked for anything yet"
        );

        // The prompt that eventually arrives is what reopens the field.
        greeter.outstanding.pop_front();
        greeter.handle(
            Asked::Auth,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".to_string(),
            },
        );
        assert!(greeter.accepts_input(), "there is a question to answer now");
        assert_eq!(greeter.prompt, otto_kit::t!("greeter-prompt-password"));
    }

    /// Escape cancels the conversation, and greetd acknowledges a cancellation
    /// with the same `Success` it uses for a completed authentication. Reading
    /// the one as the other started a session nobody had logged into; greetd
    /// answered "no session active", which reset the greeter, which cancelled
    /// again — thousands of times a second, with "Starting session…" on screen
    /// throughout. The reply has to be read as an answer to what was asked.
    #[test]
    fn cancelling_does_not_start_a_session() {
        let mut greeter = greeter();
        greeter.conversation = true;
        greeter.stage = Stage::Prompt { secret: true };

        greeter.reset(None);
        assert_eq!(
            greeter.outstanding.pop_front(),
            Some(Asked::Cancel),
            "the half-finished conversation should have been cancelled"
        );

        greeter.handle(Asked::Cancel, Response::Success);
        assert!(
            matches!(greeter.stage, Stage::Username),
            "cancelling leaves the panel at the username field, not logging in"
        );
        assert!(greeter.outstanding.is_empty(), "nothing more was asked");
    }

    /// The other half of the loop: greetd refusing a cancellation used to reset
    /// the greeter, which cancelled again. There is nothing to recover from —
    /// a cancellation that failed leaves no conversation, which is the point.
    #[test]
    fn a_refused_cancellation_is_not_cancelled_again() {
        let mut greeter = greeter();
        greeter.handle(
            Asked::Cancel,
            Response::Error {
                error_type: ErrorType::Error,
                description: "no session active".to_string(),
            },
        );

        assert!(greeter.outstanding.is_empty(), "must not ask again");
        assert!(matches!(greeter.stage, Stage::Username));
        assert!(
            greeter.error.is_none(),
            "the user cancelled; there is nothing to tell them"
        );
    }

    /// `pam_fprintd` leaves a request outstanding for as long as nobody touches
    /// the reader, so Escape is pressed *while* one is in flight. The reply,
    /// when it comes, is about the login that was walked away from.
    #[test]
    fn a_reply_to_an_abandoned_request_changes_nothing() {
        let mut greeter = greeter();
        greeter.conversation = true;
        greeter.username = "riccardo".to_string();
        greeter.info = Some("Place your finger on the reader".to_string());
        greeter.finger_pending = true;
        greeter.outstanding.push_back(Asked::Auth);

        greeter.reset(None);
        assert_eq!(
            greeter.outstanding.pop_front(),
            Some(Asked::Abandoned),
            "the in-flight request belongs to the cancelled conversation"
        );

        // The finger that arrived after Escape must not log anyone in.
        greeter.handle(Asked::Abandoned, Response::Success);
        assert!(matches!(greeter.stage, Stage::Username));

        // Nor should the error PAM reports for the interrupted conversation be
        // shown, or acted on.
        greeter.handle(
            Asked::Abandoned,
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "pam_authenticate: conversation failed".to_string(),
            },
        );
        assert!(greeter.error.is_none(), "not this login's error");
        assert_eq!(
            greeter.outstanding.pop_front(),
            Some(Asked::Cancel),
            "only the cancellation is still owed an answer"
        );
    }

    /// A greeter where `create_session` has been answered with a fingerprint
    /// hint and `pam_fprintd` is now holding the conversation: the username is
    /// taken, a request is outstanding, and nothing is being asked.
    fn waiting_for_a_finger() -> Greeter {
        let mut greeter = greeter();
        greeter.username = "riccardo".to_string();
        greeter.conversation = true;
        greeter.finger_pending = true;
        greeter.info = Some("Place your finger on the reader".to_string());
        greeter.prompt = otto_kit::t_owned!("greeter-prompt-authenticating");
        // The acknowledgement of the info message; `pam_fprintd` will not
        // answer it until a finger arrives or it gives up.
        greeter.outstanding.push_back(Asked::Auth);
        greeter
    }

    /// The reader is not the only way in. A finger that is never going to be
    /// offered — the wrong hand, a hand holding something, no enrolled print —
    /// used to leave nothing to do but wait for PAM to time out, with no way
    /// to say so and no field to type into.
    #[test]
    fn the_reader_can_be_traded_for_the_password_field() {
        let mut greeter = waiting_for_a_finger();
        assert!(
            greeter.view().offer_password,
            "the way out should be on the card while the reader is waiting"
        );
        assert!(!greeter.accepts_input(), "nothing to type into yet");

        greeter.use_password();
        assert!(!greeter.view().offer_password, "already taken");
        assert!(greeter.accepts_input(), "the field is what is being asked");
        assert!(
            matches!(greeter.view().field, Field::Secret(0)),
            "a password is masked from the first keystroke, prompt or no prompt"
        );
        assert!(
            !matches!(greeter.view().status, Some(Status::Fingerprint(..))),
            "the mark should go with the request for a finger"
        );
    }

    /// PAM is serialised: the password prompt does not exist until the reader
    /// gives up. Typing has to be possible before then anyway, or the way out
    /// is only a way of waiting differently.
    #[test]
    fn a_password_typed_before_the_prompt_is_sent_when_it_arrives() {
        let mut greeter = waiting_for_a_finger();
        greeter.use_password();
        greeter.input = "hunter2".to_string();

        greeter.submit();
        assert!(greeter.submit_when_asked, "Enter should be remembered");
        assert_eq!(
            greeter.outstanding.len(),
            1,
            "nothing may be sent while the reader still owns the conversation"
        );
        assert_eq!(greeter.input, "hunter2", "the answer must not be lost");
        assert!(matches!(greeter.view().status, Some(Status::Info(_))));

        // The reader gives up and `pam_unix` asks its question.
        greeter.outstanding.pop_front();
        greeter.handle(
            Asked::Auth,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".to_string(),
            },
        );
        assert_eq!(
            greeter.outstanding.pop_front(),
            Some(Asked::Auth),
            "the answer should have gone out with the prompt that asked for it"
        );
        assert!(greeter.input.is_empty(), "the buffer went with it");
        assert!(!greeter.password_requested && !greeter.submit_when_asked);
    }

    /// The answer was typed for a password prompt. A stack that asks for a
    /// one-time code first must not be handed it.
    #[test]
    fn a_queued_password_is_not_given_to_a_visible_prompt() {
        let mut greeter = waiting_for_a_finger();
        greeter.use_password();
        greeter.input = "hunter2".to_string();
        greeter.submit();

        greeter.outstanding.pop_front();
        greeter.handle(
            Asked::Auth,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Visible,
                auth_message: "Verification code:".to_string(),
            },
        );
        assert!(
            greeter.outstanding.is_empty(),
            "nothing should have been sent"
        );
        assert!(greeter.input.is_empty(), "the password should be dropped");
        assert!(!greeter.submit_when_asked);
    }

    /// `pam_fprintd` reports a missed finger and then asks for another one.
    /// Taking the mark down for that says the reader is finished when it is
    /// still the thing being waited on — and takes the way out down with it.
    #[test]
    fn a_missed_finger_still_leaves_the_reader_up() {
        let mut greeter = waiting_for_a_finger();
        greeter.outstanding.pop_front();
        greeter.handle(
            Asked::Auth,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Error,
                auth_message: "Failed to match fingerprint".to_string(),
            },
        );

        assert!(greeter.awaiting_finger(), "the reader is still waiting");
        assert!(greeter.view().offer_password, "and so is the way out of it");
        assert!(
            matches!(
                greeter.view().status,
                Some(Status::Fingerprint(text, Finger::Awaited))
                    if text == reader::no_match_line("greeter"),
            ),
            "the miss should be reported without retiring the mark"
        );
    }

    /// greetd relays the module's request for a finger word for word, hardware
    /// name and all, in whatever language the module was running in.
    #[test]
    fn a_named_finger_is_asked_for_in_the_panel_s_words() {
        let mut greeter = greeter();
        greeter.conversation = true;
        greeter.handle(
            Asked::Auth,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Info,
                auth_message: "Place your right index finger on Elan Fingerprint Sensor"
                    .to_string(),
            },
        );

        assert!(
            greeter.awaiting_finger(),
            "a request for a finger puts the mark up"
        );
        assert_eq!(
            greeter.info.as_deref(),
            Some(
                reader::request_line(
                    reader::FingerRequest {
                        swipe: false,
                        finger: Some(reader::FingerName::RightIndex),
                    },
                    "greeter",
                )
                .as_str()
            ),
            "the reader's own wording must not reach the card"
        );
    }

    /// At rest the loop sleeps until the next key press, give or take the
    /// clock: the panel shows the time, so it wakes on the minute and not a
    /// moment sooner. While waiting to be replaced it polls far more often,
    /// because `tick` has to be reached to give up on a session that never
    /// took over.
    #[test]
    fn only_polls_while_starting() {
        let mut greeter = greeter();
        let at_rest = greeter.idle_timeout().expect("the clock needs a wake-up");
        assert!(
            at_rest > Duration::from_secs(1),
            "nothing but the clock should be waking an idle login screen"
        );

        greeter.stage = Stage::Starting {
            since: Instant::now(),
        };
        assert!(greeter.idle_timeout().expect("waiting to be replaced") < at_rest);
    }

    /// An outstanding request used to be waited on with a 16ms timer, because
    /// greetd's socket was not one of the descriptors the loop polled. It is
    /// one now — and `pam_fprintd` leaves a request outstanding for as long as
    /// nobody touches the reader, so this is the difference between an idle
    /// login screen and one waking sixty times a second to read nothing.
    #[test]
    fn an_outstanding_request_is_waited_on_not_polled() {
        let (greeter_end, _greetd_end) =
            std::os::unix::net::UnixStream::pair().expect("socketpair");
        greeter_end.set_nonblocking(true).expect("non-blocking");
        let mut greeter = Greeter::new(Client::Real {
            stream: greeter_end,
            inbox: Vec::new(),
            closed: false,
        });
        greeter.outstanding.push_back(Asked::Auth);

        assert!(
            greeter
                .idle_timeout()
                .is_none_or(|timeout| timeout > Duration::from_secs(1)),
            "the socket is polled, so nothing but the clock needs a timer"
        );
        assert_eq!(
            greeter.poll_fds().len(),
            1,
            "the loop has to wait on greetd's socket"
        );
    }
}
