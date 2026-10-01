//! `otto-authorize --polkit-agent`: the session's polkit authentication
//! agent.
//!
//! When a program asks polkit for something that needs a password — `pkexec`,
//! a system service's action — polkitd asks the agent registered for the
//! caller's session to get it. Without one, nothing can ask and every such
//! request fails. Otto's agent shows the same dialog otto-authorize shows for
//! a sensitive setting ([`crate::dialog`]): the name of the user polkit will
//! accept, a reason line naming the program that asked and quoting polkit's
//! message, the password field (and fingerprint reader, if the stack uses
//! one), and Cancel.
//!
//! * The compositor starts it, once per session, on a socketpair it marks as
//!   its own component — so the panel keeps the keyboard as otto-authorize's
//!   does — and starts it again if it dies (`src/polkit_agent.rs`).
//! * It registers for the compositor's logind session and answers only
//!   polkitd ([`dbus`]).
//! * It never answers for anyone. The password goes to polkit's own helper,
//!   which runs PAM and tells polkitd the result ([`helper`]); all this
//!   process can do on its own is cancel.
//! * One request at a time: another arriving while the dialog is up is
//!   cancelled. Escape, Cancel, a minute without an answer and three wrong
//!   passwords all end it, and a program whose requests keep being dismissed
//!   is held off for a while ([`Throttle`]), so nothing can put the panel up
//!   again and again until someone types into it.

mod dbus;
pub mod helper;

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use otto_auth_ui::User;
use otto_kit::{App, AppContext, AppRunner};
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::pointer::PointerEvent;
use wayland_client::protocol::wl_keyboard;

use crate::dialog::{Conversation, Dialog, Verdict};
use dbus::{AgentError, Begin, Identity, Request};

/// The argument that selects this mode.
pub const AGENT_FLAG: &str = "--polkit-agent";

/// The longest polkit message quoted, and the longest program name.
const MAX_MESSAGE_CHARS: usize = 120;
const MAX_PROGRAM_CHARS: usize = 48;

/// How often requests nobody confirmed may come back, per program and for
/// the whole session — the same rule the compositor applies to
/// otto-authorize (`src/authorize.rs`, `Throttle`).
#[derive(Debug, Default)]
pub struct Throttle {
    unconfirmed: VecDeque<Instant>,
    by_caller: HashMap<String, VecDeque<Instant>>,
}

impl Throttle {
    const GAP: Duration = Duration::from_secs(3);
    const LIMIT: usize = 3;
    const GLOBAL_LIMIT: usize = 12;
    const WINDOW: Duration = Duration::from_secs(10 * 60);

    fn prune(times: &mut VecDeque<Instant>, now: Instant) {
        while times
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= Self::WINDOW)
        {
            times.pop_front();
        }
    }

    /// Whether a new dialog may be shown for `caller` at `now`.
    pub fn allows(&mut self, caller: &str, now: Instant) -> bool {
        Self::prune(&mut self.unconfirmed, now);
        self.by_caller.retain(|_, times| {
            Self::prune(times, now);
            !times.is_empty()
        });
        if self.unconfirmed.len() >= Self::GLOBAL_LIMIT {
            return false;
        }
        let Some(times) = self.by_caller.get(caller) else {
            return true;
        };
        times.len() < Self::LIMIT
            && !times
                .back()
                .is_some_and(|at| now.saturating_duration_since(*at) < Self::GAP)
    }

    pub fn note_unconfirmed(&mut self, caller: &str, now: Instant) {
        self.unconfirmed.push_back(now);
        self.by_caller
            .entry(caller.to_string())
            .or_default()
            .push_back(now);
    }

    pub fn note_confirmed(&mut self, caller: &str) {
        self.unconfirmed.clear();
        self.by_caller.remove(caller);
    }
}

/// What the agent puts on screen for one request. [`Dialog`] in the running
/// agent; the tests use a stand-in.
trait Prompt {
    /// Show it and start the conversation.
    fn open(&mut self) -> Result<(), String>;
    /// End it with `verdict`, if it has not ended yet.
    fn finish(&mut self, verdict: Verdict);
    /// How it ended, once it has.
    fn verdict(&self) -> Option<Verdict>;
    /// Take it off screen, dropping the conversation if still going.
    fn close(&mut self);
}

impl Prompt for Dialog {
    fn open(&mut self) -> Result<(), String> {
        Dialog::open(self).map_err(|err| err.to_string())
    }
    fn finish(&mut self, verdict: Verdict) {
        Dialog::finish(self, verdict);
    }
    fn verdict(&self) -> Option<Verdict> {
        Dialog::verdict(self)
    }
    fn close(&mut self) {
        Dialog::close(self);
    }
}

/// What a prompt is made from.
struct Question {
    reason: String,
    user: String,
    cookie: String,
}

/// The request on screen.
struct Active<P> {
    prompt: P,
    cookie: String,
    /// The program that asked, for the [`Throttle`].
    caller: String,
    action: String,
    reply: Option<tokio::sync::oneshot::Sender<Result<(), AgentError>>>,
}

struct Agent<P> {
    requests: Receiver<Request>,
    active: Option<Active<P>>,
    throttle: Throttle,
    make_prompt: fn(Question) -> P,
}

/// The running agent's prompt: the dialog, answered through polkit's helper.
fn dialog_for(question: Question) -> Dialog {
    Dialog::new(
        question.reason,
        Some(User::lookup(&question.user)),
        Conversation::Polkit {
            cookie: question.cookie,
        },
    )
}

impl<P: Prompt> Agent<P> {
    fn new(requests: Receiver<Request>, make_prompt: fn(Question) -> P) -> Self {
        Self {
            requests,
            active: None,
            throttle: Throttle::default(),
            make_prompt,
        }
    }

    /// Take every request the bus thread has handed over, then answer
    /// polkitd if the prompt has ended.
    fn drain(&mut self) {
        loop {
            match self.requests.try_recv() {
                Ok(request) => self.handle(request),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    tracing::error!("the polkit agent's bus thread ended");
                    self.handle(Request::Stop(1));
                    break;
                }
            }
        }
        self.settle();
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Begin(begin) => self.begin(begin),
            Request::Cancel { cookie } => match self.active.as_mut() {
                Some(active) if active.cookie == cookie => {
                    tracing::info!(action = %active.action, "polkit cancelled the request; closing the dialog");
                    active.prompt.finish(Verdict::Cancelled);
                }
                _ => tracing::info!("polkit cancelled a request that is not on screen; ignored"),
            },
            Request::Stop(status) => {
                EXIT_STATUS.store(status, std::sync::atomic::Ordering::Relaxed);
                if let Some(active) = self.active.as_mut() {
                    active.prompt.finish(Verdict::Cancelled);
                }
                self.settle();
                AppContext::request_exit();
            }
        }
    }

    fn begin(&mut self, begin: Begin) {
        let refuse = |begin: Begin, error: AgentError| {
            let _ = begin.reply.send(Err(error));
        };
        if let Some(active) = self.active.as_ref() {
            tracing::warn!(
                action = %begin.action,
                busy_with = %active.action,
                "refused: another authentication is in progress"
            );
            return refuse(
                begin,
                AgentError::Cancelled("another authentication is in progress".into()),
            );
        }
        let Some(user) = choose_identity(&begin.identities, current_uid()).and_then(user_name)
        else {
            tracing::warn!(identities = ?begin.identities, "refused: no user polkit would accept is known here");
            return refuse(
                begin,
                AgentError::Failed("no identity to authenticate".into()),
            );
        };
        let caller = requesting_program(&begin.details);
        if !self.throttle.allows(&caller, Instant::now()) {
            tracing::info!(%caller, "refused: too many unconfirmed requests");
            return refuse(
                begin,
                AgentError::Cancelled("too many unconfirmed requests; try again later".into()),
            );
        }

        let reason = if begin.action == SETTINGS_ACTION {
            settings_reason(&begin.details)
        } else {
            reason_line(&caller, &begin.message)
        };
        let mut prompt = (self.make_prompt)(Question {
            reason,
            user: user.clone(),
            cookie: begin.cookie.clone(),
        });
        if let Err(err) = prompt.open() {
            tracing::error!(%err, "cannot show the dialog");
            prompt.close();
            return refuse(
                begin,
                AgentError::Failed(format!("cannot show the dialog: {err}")),
            );
        }
        tracing::info!(action = %begin.action, %caller, %user, "dialog up; asking for the password");
        self.active = Some(Active {
            prompt,
            cookie: begin.cookie,
            caller,
            action: begin.action,
            reply: Some(begin.reply),
        });
    }

    /// Answer polkitd and take the dialog down once it has a verdict. The
    /// agent is idle again afterwards, whatever the verdict was.
    fn settle(&mut self) {
        let Some(verdict) = self.active.as_ref().and_then(|a| a.prompt.verdict()) else {
            return;
        };
        let Some(mut active) = self.active.take() else {
            return;
        };
        active.prompt.close();
        let now = Instant::now();
        let answer = match verdict {
            Verdict::Confirmed => {
                self.throttle.note_confirmed(&active.caller);
                Ok(())
            }
            Verdict::Cancelled => {
                self.throttle.note_unconfirmed(&active.caller, now);
                Err(AgentError::Cancelled("dismissed".into()))
            }
            Verdict::Failed => {
                self.throttle.note_unconfirmed(&active.caller, now);
                Err(AgentError::Failed("authentication failed".into()))
            }
        };
        tracing::info!(action = %active.action, ?verdict, "answering polkitd; the agent is idle again");
        if let Some(reply) = active.reply.take() {
            if reply.send(answer).is_err() {
                tracing::warn!("polkitd's call was already gone");
            }
        }
    }
}

impl App for Agent<Dialog> {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        AppContext::enable_layer_engine(1920.0, 1080.0);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        if let Some(active) = self.active.as_mut() {
            active.prompt.configure(width, height);
        }
    }

    fn on_close(&mut self) -> bool {
        // The compositor took the dialog's surface away. The agent stays.
        if let Some(active) = self.active.as_mut() {
            active.prompt.closed();
        }
        self.settle();
        false
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        self.drain();
        if let Some(active) = self.active.as_mut() {
            active.prompt.update();
        }
        self.settle();
    }

    fn idle_timeout(&self) -> Option<Duration> {
        self.active.as_ref().and_then(|a| a.prompt.idle_timeout())
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        if let Some(active) = self.active.as_mut() {
            active.prompt.pointer(events);
        }
        self.settle();
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        if let Some(active) = self.active.as_mut() {
            active.prompt.key(event, state);
        }
        self.settle();
    }
}

/// Run the agent until the compositor goes away or polkit turns it down.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, requests) = std::sync::mpsc::channel();
    dbus::spawn(tx);
    let agent = Agent::new(requests, dialog_for);
    AppRunner::new(agent).run()?;
    // The runner owns the agent, so its status travels through a static. A
    // clean exit tells the compositor not to start the agent again.
    let status = EXIT_STATUS.load(std::sync::atomic::Ordering::Relaxed);
    if status != 0 {
        std::process::exit(status);
    }
    Ok(())
}

/// How the agent wants the process to end; see [`run`].
static EXIT_STATUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn current_uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

/// Which of the identities polkit offered to authenticate as: the session's
/// own user when polkit would take them, else root, else the first user.
pub fn choose_identity(identities: &[Identity], own: u32) -> Option<u32> {
    let users: Vec<u32> = identities
        .iter()
        .filter_map(|identity| match identity {
            Identity::User(uid) => Some(*uid),
            Identity::Other => None,
        })
        .collect();
    [own, 0]
        .into_iter()
        .find(|uid| users.contains(uid))
        .or_else(|| users.first().copied())
}

/// The login name of `uid`.
fn user_name(uid: u32) -> Option<String> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to storage that outlives the call; the
    // strings in `entry` point into `buffer`, read before it is dropped.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so `pw_name` is a NUL-terminated string
    // inside `buffer`.
    let name = unsafe { std::ffi::CStr::from_ptr(entry.pw_name) };
    name.to_str().ok().map(str::to_string)
}

/// The program that asked, as the reason line names it: the executable of
/// `polkit.subject-pid` as "name (in /dir)", or its command name when the
/// executable cannot be read.
fn requesting_program(details: &HashMap<String, String>) -> String {
    let pid = details
        .get("polkit.subject-pid")
        .and_then(|pid| pid.trim().parse::<u32>().ok());
    let Some(pid) = pid else {
        return otto_kit::t_owned!("polkit-unknown-program");
    };
    if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        let exe = exe.to_string_lossy();
        let exe = exe.strip_suffix(" (deleted)").unwrap_or(&exe);
        if let Some(described) = describe_path(exe) {
            return described;
        }
    }
    match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        Ok(comm) if !visible(&comm).is_empty() => keep_tail(&visible(&comm), MAX_PROGRAM_CHARS),
        _ => otto_kit::t_owned!("polkit-unknown-program"),
    }
}

/// "name (in /dir)" for an absolute path.
fn describe_path(path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    let name = visible(&path.file_name()?.to_string_lossy());
    let dir = visible(&path.parent()?.to_string_lossy());
    if name.is_empty() {
        return None;
    }
    Some(otto_kit::t_owned!(
        "authorize-path-in",
        name = keep_tail(&name, MAX_PROGRAM_CHARS),
        dir = keep_tail(&dir, MAX_PROGRAM_CHARS)
    ))
}

/// The action Otto's settings service asks for before a protected setting
/// changes (`src/settings/polkit.rs` in the compositor).
const SETTINGS_ACTION: &str = "org.otto.settings.lock";

/// The reason line for a protected setting: the change itself, from the
/// details the compositor passed (`otto.setting`, `otto.value`, `otto.label`).
fn settings_reason(details: &HashMap<String, String>) -> String {
    let setting = details
        .get("otto.setting")
        .map(String::as_str)
        .unwrap_or("");
    let label = visible(
        details
            .get("otto.label")
            .map(String::as_str)
            .unwrap_or(setting),
    );
    let Some(value) = details.get("otto.value").map(|v| visible(v)) else {
        return otto_kit::t_owned!("authorize-reason-generic", setting = label);
    };
    let empty = value.trim().is_empty();
    match setting {
        "lock.locker_command" => {
            otto_kit::t_owned!("authorize-reason-locker-command", value = value)
        }
        "lock.locker_args" if empty => otto_kit::t_owned!("authorize-reason-locker-args-clear"),
        "lock.locker_args" => otto_kit::t_owned!("authorize-reason-locker-args", value = value),
        "lock.auto_lock_timeout" if value == "0" => {
            otto_kit::t_owned!("authorize-reason-auto-lock-off")
        }
        "lock.auto_lock_timeout" => otto_kit::t_owned!("authorize-reason-auto-lock"),
        "lock.on_suspend" if value == "false" => {
            otto_kit::t_owned!("authorize-reason-lock-on-suspend-off")
        }
        "lock.on_suspend" => otto_kit::t_owned!("authorize-reason-lock-on-suspend"),
        "login.greeter_command" => {
            otto_kit::t_owned!("authorize-reason-greeter-command", value = value)
        }
        "login.greeter_args" if empty => otto_kit::t_owned!("authorize-reason-greeter-args-clear"),
        "login.greeter_args" => otto_kit::t_owned!("authorize-reason-greeter-args", value = value),
        _ => otto_kit::t_owned!("authorize-reason-generic", setting = label),
    }
}

/// The dialog's reason line: who asked, then what polkit says it is for.
/// The program comes first so a long message can never push it off the
/// card's two lines.
fn reason_line(program: &str, message: &str) -> String {
    let message = visible(message);
    let message = if message.chars().count() > MAX_MESSAGE_CHARS {
        let cut: String = message.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        message
    };
    otto_kit::t_owned!(
        "polkit-reason",
        program = program.to_string(),
        message = message
    )
}

/// `text` on one line, with control and invisible formatting characters
/// (bidi overrides, zero-width spaces) turned into spaces — polkit's message
/// and a program's name come from outside and must read as what they are.
fn visible(text: &str) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{00AD}'
                    | '\u{061C}'
                    | '\u{180E}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{2028}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
                    | '\u{FFF9}'..='\u{FFFB}'
                    | '\u{E0000}'..='\u{E007F}'
            )
    };
    let cleaned: String = text
        .chars()
        .map(|c| if hidden(c) { ' ' } else { c })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters of `text`, keeping its end.
fn keep_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - (max - 1)).collect();
    format!("…{}", tail.trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_own_user_is_preferred_then_root() {
        let offered = [Identity::User(0), Identity::Other, Identity::User(1000)];
        assert_eq!(choose_identity(&offered, 1000), Some(1000));
        assert_eq!(choose_identity(&offered, 1001), Some(0));
        assert_eq!(choose_identity(&[Identity::User(1002)], 1000), Some(1002));
        assert_eq!(choose_identity(&[Identity::Other], 1000), None);
    }

    #[test]
    fn a_long_message_cannot_hide_the_program() {
        let reason = reason_line("otto-settings (in /usr/bin)", &"x".repeat(400));
        assert!(reason.contains("otto-settings (in /usr/bin)"));
        let position = reason.find("otto-settings").unwrap();
        assert!(position < reason.find("xxx").unwrap());
        assert!(reason.chars().count() < 200);
    }

    #[test]
    fn hidden_characters_are_shown_as_spaces() {
        assert_eq!(visible("Change\u{202E}gnp.exe\n now"), "Change gnp.exe now");
    }

    /// A dismissed request holds its program off; a confirmed one clears it.
    #[test]
    fn dismissals_are_throttled_per_program() {
        let mut throttle = Throttle::default();
        let start = Instant::now();
        assert!(throttle.allows("a", start));
        throttle.note_unconfirmed("a", start);
        assert!(!throttle.allows("a", start + Duration::from_secs(1)));
        assert!(throttle.allows("b", start + Duration::from_secs(1)));
        throttle.note_confirmed("a");
        assert!(throttle.allows("a", start + Duration::from_secs(1)));
    }

    // -- The agent's bookkeeping, with a stand-in for the dialog -------------

    #[derive(Default)]
    struct FakePrompt {
        verdict: Option<Verdict>,
        cookie: String,
        closed: bool,
    }

    impl Prompt for FakePrompt {
        fn open(&mut self) -> Result<(), String> {
            if self.cookie.starts_with("unshowable") {
                return Err("no surface".into());
            }
            Ok(())
        }
        fn finish(&mut self, verdict: Verdict) {
            self.verdict.get_or_insert(verdict);
        }
        fn verdict(&self) -> Option<Verdict> {
            self.verdict
        }
        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn fake_prompt(question: Question) -> FakePrompt {
        FakePrompt {
            cookie: question.cookie,
            ..FakePrompt::default()
        }
    }

    type Reply = tokio::sync::oneshot::Receiver<Result<(), AgentError>>;

    fn agent() -> (Agent<FakePrompt>, std::sync::mpsc::Sender<Request>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Agent::new(rx, fake_prompt), tx)
    }

    /// A BeginAuthentication for `cookie`, asked by process `pid`.
    fn begin(cookie: &str, pid: u32) -> (Request, Reply) {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let request = Request::Begin(Begin {
            action: "org.example.test".into(),
            message: "Test".into(),
            details: HashMap::from([("polkit.subject-pid".to_string(), pid.to_string())]),
            cookie: cookie.into(),
            identities: vec![Identity::User(current_uid())],
            reply,
        });
        (request, answer)
    }

    fn is_cancelled(reply: &mut Reply) -> bool {
        matches!(reply.try_recv(), Ok(Err(AgentError::Cancelled(_))))
    }

    /// polkitd hears nothing until the dialog has a verdict, then exactly
    /// that verdict, and the agent is free for the next request.
    #[test]
    fn agent_answers_polkitd_only_with_the_verdict() {
        let (mut agent, tx) = agent();
        let (request, mut reply) = begin("c1", 1);
        tx.send(request).unwrap();
        agent.drain();
        assert!(agent.active.is_some());
        assert!(
            reply.try_recv().is_err(),
            "answered before the dialog ended"
        );

        agent
            .active
            .as_mut()
            .unwrap()
            .prompt
            .finish(Verdict::Confirmed);
        agent.drain();
        assert!(matches!(reply.try_recv(), Ok(Ok(()))));
        assert!(agent.active.is_none());
    }

    /// A second request while one is on screen is turned down at once; the
    /// first carries on and still gets its own answer.
    #[test]
    fn agent_second_begin_while_one_runs_is_refused() {
        let (mut agent, tx) = agent();
        let (first, mut first_reply) = begin("c1", 1);
        let (second, mut second_reply) = begin("c2", std::process::id());
        tx.send(first).unwrap();
        tx.send(second).unwrap();
        agent.drain();
        assert!(is_cancelled(&mut second_reply));
        assert_eq!(agent.active.as_ref().unwrap().cookie, "c1");
        assert!(first_reply.try_recv().is_err());

        agent
            .active
            .as_mut()
            .unwrap()
            .prompt
            .finish(Verdict::Failed);
        agent.drain();
        assert!(matches!(
            first_reply.try_recv(),
            Ok(Err(AgentError::Failed(_)))
        ));
        assert!(agent.active.is_none());
    }

    /// polkitd's CancelAuthentication closes the dialog (and with it the
    /// helper) and leaves the agent idle; one for another cookie changes
    /// nothing.
    #[test]
    fn agent_cancel_from_polkitd_ends_the_request() {
        let (mut agent, tx) = agent();
        let (request, mut reply) = begin("c1", 1);
        tx.send(request).unwrap();
        tx.send(Request::Cancel {
            cookie: "other".into(),
        })
        .unwrap();
        agent.drain();
        assert!(agent.active.is_some());

        tx.send(Request::Cancel {
            cookie: "c1".into(),
        })
        .unwrap();
        agent.drain();
        assert!(is_cancelled(&mut reply));
        assert!(agent.active.is_none());

        // Idle again: another program's request is shown.
        let (request, _reply) = begin("c2", std::process::id());
        tx.send(request).unwrap();
        agent.drain();
        assert_eq!(agent.active.as_ref().unwrap().cookie, "c2");
    }

    /// A dialog that cannot be shown is a refusal, not a request left
    /// hanging.
    #[test]
    fn agent_unshowable_dialog_is_refused() {
        let (mut agent, tx) = agent();
        let (request, mut reply) = begin("unshowable", 1);
        tx.send(request).unwrap();
        agent.drain();
        assert!(matches!(reply.try_recv(), Ok(Err(AgentError::Failed(_)))));
        assert!(agent.active.is_none());
    }
}
