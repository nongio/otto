//! Checking the answers, through polkit's own agent session.
//!
//! `libpolkit-agent-1`'s session runs polkit's helper for the user named in
//! the request, relays the PAM conversation, and has the helper report the
//! result to polkitd under the request's cookie. polkitd believes only the
//! helper; nothing in this process can say yes for the user.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use otto_auth_ui::pam::{self, Event, Message, Outcome};
use polkit_agent_rs::gio::glib;
use polkit_agent_rs::polkit;
use polkit_agent_rs::Session;

/// How often the conversation looks at both sides, the session's signals and
/// the dialog's answers, while neither has anything to say.
const POLL: Duration = Duration::from_millis(50);

/// The whole conversation for one attempt, on the attempt's thread; see
/// [`otto_auth_ui::pam::Attempt::with_conversation`].
pub fn converse(
    user: &str,
    cookie: &str,
    events: &Sender<Event>,
    answers: &Receiver<String>,
) -> Outcome {
    let context = glib::MainContext::new();
    let ran = context.with_thread_default(|| run(&context, user, cookie, events, answers));
    ran.unwrap_or_else(|err| {
        tracing::error!(%err, "cannot run the polkit session");
        Outcome::Denied(otto_kit::t_owned!("lock-error-service-failed"))
    })
}

fn run(
    context: &glib::MainContext,
    user: &str,
    cookie: &str,
    events: &Sender<Event>,
    answers: &Receiver<String>,
) -> Outcome {
    let identity = match polkit::UnixUser::new_for_name(user) {
        Ok(identity) => identity,
        Err(err) => {
            tracing::error!(%err, "no such user for polkit");
            return Outcome::Denied(otto_kit::t_owned!("lock-error-service-failed"));
        }
    };
    let session = Session::new(&identity, cookie);

    let said = events.clone();
    session.connect_request(move |_, text, echo_on| {
        let _ = said.send(Event::Said(Message::Prompt {
            text: text.to_string(),
            secret: !echo_on,
        }));
        otto_kit::AppContext::request_wakeup();
    });
    let said = events.clone();
    session.connect_show_error(move |_, text| {
        let _ = said.send(Event::Said(Message::Error(text.to_string())));
        otto_kit::AppContext::request_wakeup();
    });
    let said = events.clone();
    session.connect_show_info(move |_, text| {
        let _ = said.send(Event::Said(Message::Info(text.to_string())));
        otto_kit::AppContext::request_wakeup();
    });
    let gained: Rc<Cell<Option<bool>>> = Rc::default();
    let completed = gained.clone();
    session.connect_completed(move |_, authorized| completed.set(Some(authorized)));

    session.initiate();
    loop {
        while context.iteration(false) {}
        if let Some(authorized) = gained.get() {
            tracing::info!(authorized, "polkit session completed");
            return if authorized {
                Outcome::Authenticated
            } else {
                Outcome::Denied(otto_kit::t_owned!("authorize-error-failed"))
            };
        }
        match answers.recv_timeout(POLL) {
            Ok(mut answer) => {
                session.response(&answer);
                pam::wipe(&mut answer);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                tracing::info!("the dialog ended first; cancelling the polkit session");
                session.cancel();
                // Nobody is left to read it.
                return Outcome::Denied(String::new());
            }
        }
    }
}
