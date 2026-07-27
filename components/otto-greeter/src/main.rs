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
use otto_auth_ui::{Action, Appearance, Field, Finger, Panel, PowerAction, Status, User, View};
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
const MARK_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// A request is outstanding. The panel stays live meanwhile — this only
    /// says that Enter would have nothing to answer.
    awaiting_reply: bool,
    client: Client,
    stage: Stage,
    /// The username being authenticated.
    username: String,
    /// Looked up once when the username is submitted, so the panel can show a
    /// real name and avatar without re-reading the password database to draw.
    user: Option<User>,
    /// The current input buffer — username or auth answer depending on stage.
    input: String,
    /// Label shown above the input, from greetd's auth message.
    prompt: String,
    /// Last error, cleared on the next keystroke.
    error: Option<String>,
    /// Informational message from the PAM stack (fingerprint hints, etc.).
    info: Option<String>,
    sessions: Vec<Session>,
    session_index: usize,
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
        Self {
            surface: None,
            panel: None,
            animating_until: None,
            awaiting_reply: false,
            client,
            stage: Stage::Username,
            username: String::new(),
            user: None,
            input: String::new(),
            prompt: "Username".to_string(),
            error: None,
            info: None,
            sessions,
            session_index,
        }
    }

    fn selected_session(&self) -> &Session {
        &self.sessions[self.session_index]
    }

    /// Reset back to the username field, cancelling any half-finished
    /// conversation so greetd is ready for a new `create_session`.
    fn reset(&mut self, error: Option<String>) {
        if !matches!(self.stage, Stage::Username) {
            // greetd will not accept a new `create_session` while one is in
            // flight. The acknowledgement of this comes back through `pump`
            // like any other response.
            let _ = self.client.send(Request::CancelSession);
        }
        self.awaiting_reply = false;
        self.stage = Stage::Username;
        self.input.clear();
        self.username.clear();
        self.user = None;
        self.prompt = "Username".to_string();
        self.info = None;
        self.error = error;
    }

    /// Send `request` and return to the event loop; the reply is picked up by
    /// [`Greeter::pump`].
    fn send(&mut self, request: Request) {
        if let Err(err) = self.client.send(request) {
            tracing::error!(error = %err, "greetd IPC failed");
            self.reset(Some(format!("Login service unavailable: {err}")));
            return;
        }
        self.awaiting_reply = true;
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
                    self.awaiting_reply = false;
                    self.handle(response);
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
                        self.reset(Some("Login service went away".to_string()));
                        changed = true;
                    }
                    self.awaiting_reply = false;
                    return changed;
                }
                Err(err) => {
                    tracing::error!(error = %err, "greetd IPC failed");
                    self.awaiting_reply = false;
                    self.reset(Some(format!("Login service unavailable: {err}")));
                    return true;
                }
            }
        }
    }

    /// Act on one response from greetd, sending a follow-up where the protocol
    /// asks for one.
    fn handle(&mut self, response: Response) {
        match response {
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    self.prompt = auth_message.trim_end_matches(':').to_string();
                    self.stage = Stage::Prompt {
                        secret: auth_message_type == AuthMessageType::Secret,
                    };
                    self.input.clear();
                    // A prompt supersedes whatever hint preceded it: the reader
                    // is no longer what is being waited on.
                    self.info = None;
                }
                // Info and error messages expect no input, only an empty
                // acknowledgement — which is also what lets the PAM module
                // carry on, so the message is on screen before that happens.
                AuthMessageType::Info => {
                    self.info = Some(auth_message);
                    self.send(Request::PostAuthMessageResponse { response: None });
                }
                AuthMessageType::Error => {
                    self.error = Some(auth_message);
                    self.send(Request::PostAuthMessageResponse { response: None });
                }
            },
            Response::Success => match self.stage {
                // The conversation is done. If it ended on a fingerprint, the
                // mark in the field is mid-animation and cutting it off here is
                // the last thing anyone sees of the login; give it its moment
                // first and start the session in `tick`.
                Stage::Username | Stage::Prompt { .. } => {
                    self.error = None;
                    if self.awaiting_finger() {
                        self.stage = Stage::Accepted {
                            since: std::time::Instant::now(),
                        };
                        return;
                    }
                    self.info = None;
                    self.start_session();
                }
                Stage::Accepted { .. } => {}
                // StartSession acknowledged. A real greetd is about to kill this
                // process and exec the session in its place, so there is nothing
                // left to do but wait — `tick` gives up if that never happens.
                Stage::Starting { .. } => {
                    tracing::info!("Session started, waiting to be replaced");
                }
            },
            Response::Error {
                error_type,
                description,
            } => {
                tracing::warn!(?error_type, %description, "greetd rejected the request");
                self.reset(Some(description));
            }
        }
    }

    /// Whether a fingerprint is what the panel is currently illustrating — the
    /// PAM stack announced a reader and nothing has superseded the hint.
    fn awaiting_finger(&self) -> bool {
        self.error.is_none() && self.info.as_deref().is_some_and(mentions_fingerprint)
    }

    /// Ask greetd to exec the session. From here greetd owns what happens next.
    fn start_session(&mut self) {
        self.stage = Stage::Starting {
            since: std::time::Instant::now(),
        };
        let session = self.selected_session().clone();
        tracing::info!(session = %session.name, cmd = ?session.command, "Starting session");
        self.send(Request::StartSession {
            cmd: session.command,
            env: Vec::new(),
        });
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
                self.start_session();
                true
            }
            Stage::Starting { since } if since.elapsed() >= SESSION_START_TIMEOUT => {
                let session = self.selected_session().name.clone();
                tracing::warn!(%session, "Session was started but the greeter is still running");
                self.reset(Some(format!("{session} did not start")));
                true
            }
            _ => false,
        }
    }

    /// Handle Enter for the current stage.
    fn submit(&mut self) {
        // Answering before greetd has asked would desynchronise the
        // conversation — it happens when someone types ahead while a PAM
        // module is still thinking.
        if self.awaiting_reply {
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
                self.send(Request::CreateSession { username });
            }
            Stage::Prompt { .. } => {
                let answer = std::mem::take(&mut self.input);
                self.error = None;
                self.send(Request::PostAuthMessageResponse {
                    response: Some(answer),
                });
            }
            Stage::Accepted { .. } | Stage::Starting { .. } => {}
        }
    }

    /// Translate the greetd conversation into something the panel can draw.
    fn view(&self) -> View<'_> {
        let field = match self.stage {
            // The panel is given only the length of a secret, never the secret.
            Stage::Prompt { secret: true } => Field::Secret(self.input.chars().count()),
            _ => Field::Text(&self.input),
        };

        let status = match (&self.stage, self.error.as_deref(), self.info.as_deref()) {
            // The finger was recognised: the mark, not the wording, is what
            // says so, but the line under it should stop asking for a finger.
            (Stage::Accepted { .. }, ..) => {
                Some(Status::Fingerprint("Authenticated", Finger::Accepted))
            }
            (_, Some(error), _) => Some(Status::Error(error)),
            // A PAM stack announcing a fingerprint reader is the one info
            // message worth illustrating; the rest are shown as plain text.
            (_, None, Some(info)) if mentions_fingerprint(info) => {
                Some(Status::Fingerprint(info, Finger::Awaited))
            }
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
                .then_some("Starting session\u{2026}"),
            // Offering to power off mid-handoff would race greetd's exec.
            power: matches!(self.stage, Stage::Username | Stage::Prompt { .. }),
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

    /// Paint the scene as it currently stands, without touching its state.
    /// This is what an in-flight transition needs on every frame.
    fn paint(&self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        // Continuous animation has to say so before every frame, or the engine
        // replays the picture it recorded for the last one.
        if let Some(panel) = self.panel.as_ref() {
            panel.animate();
        }
        // otto-kit hands over a canvas with the buffer scale already applied,
        // so the scene is laid out in logical points and nothing scales twice.
        let base = surface.base_surface();
        surface.draw(|canvas| base.render_layer_node(canvas));
    }

    /// Act on a click on one of the panel's controls.
    fn activate(&mut self, action: Action) {
        match action {
            Action::CycleSession => self.cycle_session(),
            Action::Power(power) => self.power(power),
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
        let verb = match action {
            PowerAction::Suspend => "suspend",
            PowerAction::Restart => "reboot",
            PowerAction::Shutdown => "poweroff",
        };

        match std::process::Command::new("systemctl").arg(verb).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                tracing::warn!(verb, ?status, "systemctl refused");
                self.error = Some(format!("Not permitted to {verb}"));
            }
            Err(err) => {
                tracing::warn!(verb, %err, "could not run systemctl");
                self.error = Some(format!("Could not {verb}"));
            }
        }
    }
}

/// Whether a PAM message is about a fingerprint reader. The wording comes from
/// `pam_fprintd` and varies with locale and reader, so this matches loosely.
fn mentions_fingerprint(message: &str) -> bool {
    let message = message.to_lowercase();
    ["finger", "fprint", "biometric"]
        .iter()
        .any(|needle| message.contains(needle))
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
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        if let Some(panel) = self.panel.as_mut() {
            panel.set_size(width as f32, height as f32);
        }
        self.draw();
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // Collect anything greetd has said since the last pass. This is what
        // replaces blocking on the socket.
        if self.pump() || self.tick() {
            self.draw();
            return;
        }

        // Keep painting while a transition is in flight. The engine advances
        // the animation on its own thread; this is what puts the result on
        // screen.
        if self.panel.as_ref().is_some_and(Panel::wants_frames) {
            self.paint();
            return;
        }

        match self.animating_until {
            Some(deadline) if std::time::Instant::now() < deadline => self.paint(),
            Some(_) => {
                // One last frame at the settled values, then go quiet.
                self.animating_until = None;
                self.paint();
            }
            None => {}
        }
    }

    /// Tick while the panel is animating, and while waiting for the session to
    /// take over. Otherwise the loop can sleep until the next key press.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        // The Touch ID mark animates for as long as a finger is expected, so
        // the panel asks for frames directly rather than through a deadline.
        if self.panel.as_ref().is_some_and(Panel::wants_frames) || self.animating_until.is_some() {
            return Some(std::time::Duration::from_millis(16));
        }
        // While a reply is outstanding the socket has to be checked; nothing
        // else wakes the loop, because greetd's fd is not one of the two the
        // runner polls.
        if self.awaiting_reply {
            return Some(std::time::Duration::from_millis(16));
        }
        matches!(self.stage, Stage::Starting { .. }).then(|| std::time::Duration::from_millis(250))
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
            Keysym::BackSpace => {
                self.input.pop();
                self.error = None;
            }
            Keysym::Escape => self.reset(None),
            Keysym::Tab => self.cycle_session(),
            _ => {
                // Anything the keymap turned into text goes into the buffer;
                // control characters (Enter, Tab, …) are handled above.
                if let Some(text) = event.utf8.as_deref() {
                    let printable: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !printable.is_empty() {
                        self.input.push_str(&printable);
                        self.error = None;
                    }
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

    let client = Client::connect()?;
    AppRunner::new(Greeter::new(client)).run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn greeter() -> Greeter {
        Greeter::new(Client::Mock {
            awaiting_password: false,
            pending: None,
        })
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

        greeter.handle(Response::Success);
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

        greeter.handle(Response::Success);
        assert!(
            matches!(greeter.stage, Stage::Starting { .. }),
            "nothing to show, so nothing to wait for"
        );
    }

    /// The event loop should only wake up on a timer while it is waiting to be
    /// replaced; at rest it can sleep until the next key press.
    #[test]
    fn only_polls_while_starting() {
        let mut greeter = greeter();
        assert!(greeter.idle_timeout().is_none());

        greeter.stage = Stage::Starting {
            since: Instant::now(),
        };
        assert!(greeter.idle_timeout().is_some());
    }
}
