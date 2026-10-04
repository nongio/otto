//! The PAM conversation, run off the main thread.
//!
//! PAM is a blocking, serialised API: `pam_authenticate` does not return until
//! the whole stack has had its say, and a module in the middle of it can hold
//! the conversation for as long as it likes — `pam_fprintd` holds it for as
//! long as nobody touches the reader. A lock screen that waited for it on the
//! main thread would stop drawing, so the conversation runs on a thread of its
//! own and speaks to the panel through channels.
//!
//! What crosses those channels is what PAM asked and what the user answered.
//! The prompts are PAM's own wording, which is what lets a stack configured
//! with `pam_fprintd` work here with no code that knows about fingerprints.
//!
//! Otto never sees any of this: the locker runs as the session's user and
//! authenticates that user, exactly as the greeter delegates to greetd.
//!
//! Shared by the two clients that prove the session's own user is at the
//! keyboard: otto-lock, to unlock, and otto-authorize, to confirm a change to
//! a sensitive setting. Each names its own PAM stack with a [`Service`].
//! Behind the `pam` feature, so the greeter (which talks to greetd, not PAM)
//! does not link libpam.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use zeroize::Zeroizing;

// -- libpam ------------------------------------------------------------------

const PAM_SUCCESS: c_int = 0;
const PAM_CONV_ERR: c_int = 19;
const PAM_BUF_ERR: c_int = 5;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;

/// Re-establish credentials that expire with time rather than with the
/// session — a Kerberos ticket, an AFS token. Unlocking is exactly when a
/// session that has been sitting idle wants them refreshed.
const PAM_REFRESH_CRED: c_int = 0x0010;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: Option<
        unsafe extern "C" fn(
            num_msg: c_int,
            msg: *mut *const PamMessage,
            resp: *mut *mut PamResponse,
            appdata_ptr: *mut c_void,
        ) -> c_int,
    >,
    appdata_ptr: *mut c_void,
}

enum PamHandle {}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut PamHandle, pam_status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
}

// -- What the panel sees -----------------------------------------------------

/// Something PAM said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A question with an answer expected. `secret` is PAM's `ECHO_OFF`, and
    /// is the only thing that decides whether the field is masked.
    Prompt { text: String, secret: bool },
    /// A hint. `pam_fprintd` announces the reader this way.
    Info(String),
    /// Something went wrong but the conversation continues — a finger that
    /// did not match is reported like this, and then asked for again.
    Error(String),
}

/// How an attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The user is who they say they are, and the account is in good standing.
    Authenticated,
    /// PAM said no. The text is PAM's own, and is what the panel shows.
    Denied(String),
}

/// One thing the conversation has to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Said(Message),
    Ended(Outcome),
}

/// One run of the PAM stack, from `pam_start` to `pam_end`.
///
/// Dropping an attempt that is still going closes the answer channel, which
/// fails the conversation and unwinds the stack — as far as it can. A module
/// blocked in its own I/O finishes when it finishes; nothing here can hurry it.
pub struct Attempt {
    events: Receiver<Event>,
    /// Each answer is wiped by whichever side drops it last.
    answers: Sender<Zeroizing<String>>,
    /// Set once [`Outcome`] has been read, so a caller cannot answer a
    /// conversation that has ended.
    finished: bool,
}

/// Which PAM stack an [`Attempt`] runs, and how it falls back.
#[derive(Debug, Clone, Copy)]
pub struct Service {
    /// The service Otto ships for this client, `/etc/pam.d/<preferred>`.
    pub preferred: &'static str,
    /// Tried in order when the preferred file is not installed.
    pub fallbacks: &'static [&'static str],
    /// An environment variable that names a different service outright, for
    /// exercising the client against a stack whose answer is known. `None`
    /// for a client whose environment is not the user's to choose.
    pub env_override: Option<&'static str>,
    /// Whether a success refreshes credentials that expire with time
    /// (`PAM_REFRESH_CRED`): right for an unlock, pointless for a one-off
    /// confirmation.
    pub refresh_credentials: bool,
}

impl Service {
    /// otto-lock's stack. See `components/otto-lock/otto-lock.pam`.
    ///
    /// `$OTTO_LOCK_PAM_SERVICE` names a different service. It is no weaker
    /// than the rest of the session's environment: whoever can set it already
    /// runs as this user, and the person a lock screen exists to stop is at
    /// the keyboard of a session they cannot type into.
    pub const LOCK: Service = Service {
        preferred: "otto-lock",
        fallbacks: &["system-auth", "login"],
        env_override: Some("OTTO_LOCK_PAM_SERVICE"),
        refresh_credentials: true,
    };

    /// otto-authorize's stack. See `components/otto-authorize/otto-authorize.pam`.
    ///
    /// Falls back to the lock screen's stack first, which is the same
    /// question — is the session's own user at the keyboard — and then to the
    /// system's. No environment override: the helper is started by the
    /// compositor, and a confirmation that could be pointed at `pam_permit`
    /// would confirm nothing.
    pub const AUTHORIZE: Service = Service {
        preferred: "otto-authorize",
        fallbacks: &["otto-lock", "system-auth", "login"],
        env_override: None,
        refresh_credentials: false,
    };
}

impl Attempt {
    /// Start authenticating `user` against `service`.
    pub fn start(service: &Service, user: &str) -> Self {
        let user = user.to_string();
        let refresh = service.refresh_credentials;
        let service = service_name(service);
        Self::with_conversation(move |events, answers| {
            converse(&service, &user, refresh, events, answers)
        })
    }

    /// Run a conversation that is not this process's own PAM stack — polkit's
    /// setuid helper, which runs PAM on the far side of a pipe — on a thread
    /// of its own, exactly as [`Attempt::start`] runs PAM.
    ///
    /// `converse` sends what it is told as [`Event::Said`], blocks on
    /// `answers` for each prompt, and returns how it ended. A closed answer
    /// channel means the attempt was dropped: it should give up.
    pub fn with_conversation<F>(converse: F) -> Self
    where
        F: FnOnce(&Sender<Event>, &Receiver<Zeroizing<String>>) -> Outcome + Send + 'static,
    {
        let (event_tx, events) = std::sync::mpsc::channel();
        let (answers, answer_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("pam".to_string())
            .spawn(move || {
                let outcome = converse(&event_tx, &answer_rx);
                // A send that fails means the panel is gone — the process is
                // exiting, and there is nobody left to tell.
                let _ = event_tx.send(Event::Ended(outcome));
                otto_kit::AppContext::request_wakeup();
            })
            .expect("could not start the PAM thread");

        Self {
            events,
            answers,
            finished: false,
        }
    }

    /// Whatever PAM has said since the last call, if anything. Never blocks:
    /// the panel keeps drawing while a reader waits for a finger.
    pub fn poll(&mut self) -> Option<Event> {
        if self.finished {
            return None;
        }
        match self.events.try_recv() {
            Ok(event) => {
                if matches!(event, Event::Ended(_)) {
                    self.finished = true;
                }
                Some(event)
            }
            Err(TryRecvError::Empty) => None,
            // The thread died without saying how it went — a panic in the
            // conversation, or a PAM module that took the process's stack with
            // it. Report it as a refusal so the panel offers another go.
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                Some(Event::Ended(Outcome::Denied(otto_kit::t_owned!(
                    "lock-error-service-failed"
                ))))
            }
        }
    }

    /// Answer the prompt PAM is waiting on.
    pub fn answer(&self, text: Zeroizing<String>) {
        if self.finished {
            return;
        }
        let _ = self.answers.send(text);
    }
}

/// The PAM service to authenticate against.
///
/// The preferred service is what Otto ships and what should be used. Without
/// it PAM falls through to `other`, which on a sane system denies everything —
/// so rather than lock the user out of their own session because a file was
/// not installed, fall back to a stack that is known to authenticate a local
/// user.
fn service_name(service: &Service) -> CString {
    resolve_service(service, |name| {
        std::path::Path::new("/etc/pam.d").join(name).is_file()
    })
}

fn resolve_service(service: &Service, installed: impl Fn(&str) -> bool) -> CString {
    let preferred = service.preferred;

    if let Some(name) = service
        .env_override
        .and_then(|var| std::env::var(var).ok())
        .filter(|name| !name.is_empty())
    {
        tracing::warn!(service = name, "using a PAM service from the environment");
        if let Ok(name) = CString::new(name) {
            return name;
        }
    }

    if installed(preferred) {
        return CString::new(preferred).expect("no interior nul");
    }

    let fallback = service.fallbacks.iter().find(|name| installed(name));
    match fallback {
        Some(name) => {
            tracing::warn!(
                service = *name,
                "/etc/pam.d/{preferred} is not installed; falling back"
            );
            CString::new(*name).expect("no interior nul")
        }
        // Nothing to fall back to. Ask for the service that should exist and
        // let PAM report what it makes of it.
        None => {
            tracing::error!("no PAM service found; authentication will fail");
            CString::new(preferred).expect("no interior nul")
        }
    }
}

/// The whole conversation, on the PAM thread.
fn converse(
    service: &CStr,
    user: &str,
    refresh_credentials: bool,
    events: &Sender<Event>,
    answers: &Receiver<Zeroizing<String>>,
) -> Outcome {
    let Ok(user_c) = CString::new(user) else {
        return Outcome::Denied(otto_kit::t_owned!("lock-error-invalid-user"));
    };

    // Lives for as long as the handle does: PAM keeps the pointer and calls
    // back into it from inside `pam_authenticate`.
    let mut state = ConversationState { events, answers };
    let conv = PamConv {
        conv: Some(conversation),
        appdata_ptr: &mut state as *mut ConversationState as *mut c_void,
    };

    let mut pamh: *mut PamHandle = std::ptr::null_mut();
    let status = unsafe { pam_start(service.as_ptr(), user_c.as_ptr(), &conv, &mut pamh) };
    if status != PAM_SUCCESS || pamh.is_null() {
        tracing::error!(status, "pam_start failed");
        return Outcome::Denied(otto_kit::t_owned!("lock-error-unavailable"));
    }

    let outcome = authenticate(pamh, refresh_credentials);

    // `pam_end` takes the last status so the stack can clean up accordingly.
    let end_status = if matches!(outcome, Outcome::Authenticated) {
        PAM_SUCCESS
    } else {
        PAM_CONV_ERR
    };
    unsafe { pam_end(pamh, end_status) };

    outcome
}

fn authenticate(pamh: *mut PamHandle, refresh_credentials: bool) -> Outcome {
    let status = unsafe { pam_authenticate(pamh, 0) };
    if status != PAM_SUCCESS {
        return Outcome::Denied(strerror(pamh, status));
    }

    // Authentication says who they are; this says whether the account may be
    // used at all — expired, locked, out of hours.
    let status = unsafe { pam_acct_mgmt(pamh, 0) };
    if status != PAM_SUCCESS {
        return Outcome::Denied(strerror(pamh, status));
    }

    if !refresh_credentials {
        return Outcome::Authenticated;
    }

    // Best effort: a session whose Kerberos ticket could not be refreshed is
    // still a session its owner just proved they own.
    let status = unsafe { pam_setcred(pamh, PAM_REFRESH_CRED) };
    if status != PAM_SUCCESS {
        tracing::warn!(error = %strerror(pamh, status), "could not refresh credentials");
    }

    Outcome::Authenticated
}

fn strerror(pamh: *mut PamHandle, status: c_int) -> String {
    let message = unsafe { pam_strerror(pamh, status) };
    if message.is_null() {
        return otto_kit::t_owned!("lock-error-auth-failed", status = status.to_string());
    }
    unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned()
}

/// What the conversation callback needs to reach the panel. Borrowed by the
/// PAM handle for the length of one attempt, on the thread that made it.
struct ConversationState<'a> {
    events: &'a Sender<Event>,
    answers: &'a Receiver<Zeroizing<String>>,
}

/// PAM's conversation callback.
///
/// Called from inside `pam_authenticate`, on the PAM thread. Every message is
/// forwarded to the panel; the ones that expect an answer then block here
/// until it comes back, which is what makes a fingerprint reader and a
/// password field the same thing as far as the stack is concerned.
///
/// # Safety
///
/// `appdata_ptr` is the [`ConversationState`] handed to `pam_start`, and the
/// arrays are PAM's, laid out as `pam_message`/`pam_response` on Linux-PAM.
unsafe extern "C" fn conversation(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    let state = &*(appdata_ptr as *const ConversationState);
    let count = num_msg as usize;

    // PAM frees this, so it has to come from the allocator PAM frees with.
    let responses = libc::calloc(count, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }

    for i in 0..count {
        let message = *msg.add(i);
        if message.is_null() {
            libc::free(responses as *mut c_void);
            return PAM_CONV_ERR;
        }
        let text = if (*message).msg.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*message).msg)
                .to_string_lossy()
                .into_owned()
        };

        let answer = match (*message).msg_style {
            style @ (PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON) => {
                let secret = style == PAM_PROMPT_ECHO_OFF;
                if state
                    .events
                    .send(Event::Said(Message::Prompt { text, secret }))
                    .is_err()
                {
                    libc::free(responses as *mut c_void);
                    return PAM_CONV_ERR;
                }
                otto_kit::AppContext::request_wakeup();

                // The panel now owns the conversation until someone types.
                match state.answers.recv() {
                    Ok(answer) => Some(answer),
                    // The lock screen is gone; there will be no answer.
                    Err(_) => {
                        libc::free(responses as *mut c_void);
                        return PAM_CONV_ERR;
                    }
                }
            }
            style => {
                let message = if style == PAM_ERROR_MSG {
                    Message::Error(text)
                } else if style == PAM_TEXT_INFO {
                    Message::Info(text)
                } else {
                    // A style neither this nor any other conversation knows.
                    // Saying nothing is the documented answer.
                    tracing::debug!(style, "ignoring an unknown PAM message style");
                    continue;
                };
                let _ = state.events.send(Event::Said(message));
                otto_kit::AppContext::request_wakeup();
                None
            }
        };

        // A response of NULL is right for a message that asked nothing.
        if let Some(answer) = answer {
            // Copied into a buffer sized for the terminator up front, so the
            // answer is not reallocated (and left behind) on the way to C.
            // Both copies are wiped when they drop, once PAM has its own.
            let mut terminated = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
            terminated.extend_from_slice(answer.as_bytes());
            terminated.push(0);
            drop(answer);
            if terminated[..terminated.len() - 1].contains(&0) {
                libc::free(responses as *mut c_void);
                return PAM_CONV_ERR;
            }
            let copy = libc::strdup(terminated.as_ptr() as *const c_char);
            drop(terminated);
            if copy.is_null() {
                libc::free(responses as *mut c_void);
                return PAM_BUF_ERR;
            }
            (*responses.add(i)).resp = copy;
        }
    }

    *resp = responses;
    PAM_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// otto-authorize without its own file falls back the way otto-lock
    /// does — through the lock screen's stack first, which asks the same
    /// question.
    #[test]
    fn authorize_falls_back_to_the_lock_screen_stack() {
        let only = |present: &'static [&'static str]| move |name: &str| present.contains(&name);
        let pick = |present| resolve_service(&Service::AUTHORIZE, only(present));
        assert_eq!(
            pick(&["otto-authorize", "otto-lock"]).to_str(),
            Ok("otto-authorize")
        );
        assert_eq!(
            pick(&["otto-lock", "system-auth"]).to_str(),
            Ok("otto-lock")
        );
        assert_eq!(pick(&["system-auth"]).to_str(), Ok("system-auth"));
        assert_eq!(pick(&[]).to_str(), Ok("otto-authorize"));
    }
}
