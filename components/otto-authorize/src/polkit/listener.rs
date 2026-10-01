//! The agent's side of polkit, through `libpolkit-agent-1`.
//!
//! polkit's own agent library registers the agent for the session, serves
//! polkitd's `BeginAuthentication` and `CancelAuthentication`, and registers
//! again when polkitd restarts. This module subclasses its listener and hands
//! each request to the dialog loop on the main thread as a [`Request`].
//!
//! The library runs on a GLib main loop of its own, on a thread of its own.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use polkit_agent_rs::gio::{self, glib, prelude::*};
use polkit_agent_rs::polkit;
use polkit_agent_rs::subclass::ListenerImpl;
use polkit_agent_rs::traits::ListenerExt;
use polkit_agent_rs::{Listener, RegisterFlags};

/// Where the library serves this agent's object on the system bus.
pub const OBJECT_PATH: &str = "/org/otto/PolicyKit1/AuthenticationAgent";

/// Why a request was not completed.
#[derive(Debug)]
pub enum AgentError {
    /// The request could not be completed.
    Failed(String),
    /// The user dismissed it, or it was never shown.
    Cancelled(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Failed(reason) => write!(f, "failed: {reason}"),
            AgentError::Cancelled(reason) => write!(f, "cancelled: {reason}"),
        }
    }
}

/// Who polkit will accept a password from. polkitd expands groups into
/// their users before it asks, so a user is all an agent needs to handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Other,
}

/// One `BeginAuthentication`, as the dialog loop needs it.
pub struct Begin {
    pub action: String,
    pub message: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<Identity>,
    pub reply: tokio::sync::oneshot::Sender<Result<(), AgentError>>,
}

pub enum Request {
    Begin(Begin),
    /// polkitd withdrew the request with this cookie.
    Cancel {
        cookie: String,
    },
    /// The agent cannot serve this session; the process should end, with
    /// this exit status.
    Stop(i32),
}

/// Where the listener sends requests. Set once, before it is registered.
static REQUESTS: OnceLock<Sender<Request>> = OnceLock::new();

fn send(request: Request) -> bool {
    let sent = REQUESTS
        .get()
        .is_some_and(|requests| requests.send(request).is_ok());
    otto_kit::AppContext::request_wakeup();
    sent
}

mod imp {
    use super::*;
    use polkit_agent_rs::gio::glib::subclass::prelude::*;

    #[derive(Default)]
    pub struct AgentListener;

    #[glib::object_subclass]
    impl ObjectSubclass for AgentListener {
        const NAME: &'static str = "OttoAuthorizeListener";
        type Type = super::AgentListener;
        type ParentType = Listener;
    }

    impl ObjectImpl for AgentListener {}

    impl ListenerImpl for AgentListener {
        type Message = String;

        fn initiate_authentication(
            &self,
            action_id: &str,
            message: &str,
            _icon_name: &str,
            details: &polkit::Details,
            cookie: &str,
            identities: Vec<polkit::Identity>,
            cancellable: gio::Cancellable,
            task: gio::Task<Self::Message>,
        ) {
            super::begin(
                action_id,
                message,
                details,
                cookie,
                &identities,
                cancellable,
                task,
            );
        }

        fn initiate_authentication_finish(
            &self,
            result: Result<gio::Task<Self::Message>, glib::Error>,
        ) -> bool {
            // The binding cannot hand polkitd an error from here, and the
            // library would then answer nothing at all. Every request is
            // reported as done: what decides it is whether polkit's helper
            // told polkitd it succeeded, and a dismissed or failed request
            // never gets that far.
            // SAFETY: the library calls this once per task, so the task's
            // result is read once.
            if let Ok(Err(err)) = result.map(|task| unsafe { task.propagate() }) {
                tracing::info!(%err, "request ended without authorization");
            }
            true
        }
    }
}

glib::wrapper! {
    pub struct AgentListener(ObjectSubclass<imp::AgentListener>) @extends Listener;
}

/// Hand one request to the dialog loop, and complete `task` when it answers.
fn begin(
    action: &str,
    message: &str,
    details: &polkit::Details,
    cookie: &str,
    identities: &[polkit::Identity],
    cancellable: gio::Cancellable,
    task: gio::Task<String>,
) {
    tracing::info!(
        action,
        subject_pid = details
            .lookup("polkit.subject-pid")
            .as_deref()
            .unwrap_or("?"),
        caller_pid = details
            .lookup("polkit.caller-pid")
            .as_deref()
            .unwrap_or("?"),
        identities = identities.len(),
        "BeginAuthentication from polkitd"
    );
    let details: HashMap<String, String> = details
        .keys()
        .into_iter()
        .filter_map(|key| {
            let value = details.lookup(&key)?;
            Some((key.to_string(), value.to_string()))
        })
        .collect();
    let (reply, answer) = tokio::sync::oneshot::channel();
    let begin = Begin {
        action: action.to_string(),
        message: message.to_string(),
        details,
        cookie: cookie.to_string(),
        identities: identities.iter().map(identity).collect(),
        reply,
    };

    let withdrawn = cookie.to_string();
    cancellable.connect_cancelled(move |_| {
        tracing::info!("CancelAuthentication from polkitd");
        send(Request::Cancel {
            cookie: withdrawn.clone(),
        });
    });

    if !send(Request::Begin(begin)) {
        // SAFETY: the only result this task gets; the dialog loop never saw it.
        unsafe {
            task.return_result(Err(glib::Error::new(
                gio::IOErrorEnum::Failed,
                "the agent is stopping",
            )));
        }
        return;
    }
    let action = action.to_string();
    glib::MainContext::ref_thread_default().spawn_local(async move {
        // A dialog loop that drops the sender without answering has shown
        // nothing that could be confirmed.
        let result = answer
            .await
            .unwrap_or_else(|_| Err(AgentError::Cancelled("the agent is stopping".into())));
        match &result {
            Ok(()) => tracing::info!(%action, "BeginAuthentication answered: done"),
            Err(err) => tracing::info!(%action, %err, "BeginAuthentication answered"),
        }
        let result = match result {
            Ok(()) => Ok(String::new()),
            Err(AgentError::Cancelled(reason)) => {
                Err(glib::Error::new(gio::IOErrorEnum::Cancelled, &reason))
            }
            Err(AgentError::Failed(reason)) => {
                Err(glib::Error::new(gio::IOErrorEnum::Failed, &reason))
            }
        };
        // SAFETY: the only result this task gets: the request reached the
        // dialog loop, so the early return above did not run.
        unsafe { task.return_result(result) };
    });
}

fn identity(identity: &polkit::Identity) -> Identity {
    identity
        .downcast_ref::<polkit::UnixUser>()
        .and_then(|user| u32::try_from(user.uid()).ok())
        .map(Identity::User)
        .unwrap_or(Identity::Other)
}

/// The logind session to register for: the one this process runs in.
fn session() -> Result<polkit::Subject, glib::Error> {
    if let Some(id) = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
    {
        return Ok(polkit::UnixSession::new(&id).upcast());
    }
    let pid = i32::try_from(std::process::id()).unwrap_or_default();
    polkit::UnixSession::new_for_process_sync(pid, gio::Cancellable::NONE)
        .map(|session| session.upcast())
}

/// Register the agent and serve it until the process ends, on a thread of
/// its own.
pub fn spawn(requests: Sender<Request>) {
    if REQUESTS.set(requests).is_err() {
        tracing::error!("the polkit agent was started twice");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("polkit-agent".into())
        .spawn(serve);
    if let Err(err) = spawned {
        tracing::error!(%err, "cannot start the polkit agent's thread");
        send(Request::Stop(1));
    }
}

fn serve() {
    let context = glib::MainContext::new();
    let pushed = context.with_thread_default(|| {
        let subject = match session() {
            Ok(subject) => subject,
            Err(err) => {
                tracing::error!(%err, "this process is not in a logind session");
                send(Request::Stop(1));
                return;
            }
        };
        let listener: AgentListener = glib::Object::new();
        // Refused most often because another agent already serves this
        // session (a nested Otto inside a desktop that has one). Exit
        // cleanly: starting again cannot help.
        let _registration = match listener.register(
            RegisterFlags::NONE,
            &subject,
            OBJECT_PATH,
            gio::Cancellable::NONE,
        ) {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(%err, "polkit did not accept this agent; leaving it to the one it has");
                send(Request::Stop(0));
                return;
            }
        };
        tracing::info!("registered as the session's polkit authentication agent");
        glib::MainLoop::new(Some(&context), false).run();
    });
    if let Err(err) = pushed {
        tracing::error!(%err, "cannot run the polkit agent's main loop");
        send(Request::Stop(1));
    }
}
