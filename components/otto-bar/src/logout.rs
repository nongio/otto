//! The Otto menu's way out: Sleep, Restart…, Shut Down… and Log Out.
//!
//! Restart, Shut Down and Log Out ask first. The question is put by
//! otto-islands' system dialog (`org.otto.Dialog1`), the one the portal and
//! the agents use, so it looks and answers to the keyboard like every other
//! question the desktop asks. Logging out hands the compositor its `logout`
//! command, which closes the windows one by one and ends the session once
//! they have gone (see `src/state/logout.rs`). Sleeping, restarting and
//! shutting down are logind's, asked through `otto_kit::power`.

use otto_kit::dbus::dialog::DialogProxy;
use otto_kit::power::PowerAction;
use zbus::Connection;

/// The dialog's grant response.
const GRANTED: u32 = 0;

/// One question the menu asks before acting.
struct Question {
    title: &'static str,
    body: &'static str,
    icon: &'static str,
    grant: &'static str,
}

/// Ask `question`, and run `act` on yes.
///
/// Without otto-islands there is nobody to ask, and `act` goes ahead: the
/// person picked the item from the menu, and the question is a second
/// chance rather than the decision. Any other failure (a timeout, the dialog
/// erroring, no session bus) does nothing: a question that could not be
/// answered is not a yes.
fn confirm_then(question: Question, act: impl FnOnce() + Send + 'static) {
    let deny = otto_kit::t!("common-cancel");
    tokio::spawn(async move {
        let answer = async {
            let conn = Connection::session().await?;
            let dialog = DialogProxy::new(&conn).await?;
            dialog
                .present_access(
                    "",
                    question.title,
                    "",
                    question.body,
                    question.icon,
                    question.grant,
                    deny,
                    true,
                    Vec::new(),
                )
                .await
        }
        .await;
        match answer {
            Ok((GRANTED, _)) => act(),
            Ok(_) => {}
            Err(e) if dialog_absent(&e) => {
                tracing::warn!("no dialog to ask ({e}); going ahead without asking");
                act();
            }
            Err(e) => tracing::warn!("dialog failed ({e}); doing nothing"),
        }
    });
}

/// Ask whether to log out, and log out on yes.
///
/// The compositor still closes each window the polite way, so an
/// application with unsaved work gets its say either way.
pub fn confirm_and_log_out() {
    let question = Question {
        title: otto_kit::t!("bar-logout-title"),
        body: otto_kit::t!("bar-logout-body"),
        icon: "system-log-out",
        grant: otto_kit::t!("bar-otto-log-out"),
    };
    confirm_then(question, || {
        crate::keyboard_layout::run_shell_command("logout".to_string());
    });
}

/// Ask whether to restart, and ask logind to on yes.
pub fn confirm_and_restart() {
    let question = Question {
        title: otto_kit::t!("bar-restart-title"),
        body: otto_kit::t!("bar-restart-body"),
        icon: "system-reboot",
        grant: otto_kit::t!("bar-restart-grant"),
    };
    confirm_then(question, || power(PowerAction::Restart));
}

/// Ask whether to shut down, and ask logind to on yes.
pub fn confirm_and_shut_down() {
    let question = Question {
        title: otto_kit::t!("bar-shutdown-title"),
        body: otto_kit::t!("bar-shutdown-body"),
        icon: "system-shutdown",
        grant: otto_kit::t!("bar-shutdown-grant"),
    };
    confirm_then(question, || power(PowerAction::Shutdown));
}

/// Ask logind for `action` on a thread of its own: a suspend answers only
/// once the machine is on its way down, and polkit may stop to ask for a
/// password. A refusal is logged; polkit has already said why to the person
/// where it asked.
pub fn power(action: PowerAction) {
    let spawned = std::thread::Builder::new()
        .name(format!("power-{}", action.name()))
        .spawn(move || {
            if let Err(err) = otto_kit::power::request(action) {
                tracing::warn!(?action, %err, "logind did not act");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(?action, %err, "could not start the power request");
    }
}

/// Whether `error` says nobody serves the dialog, rather than that the
/// dialog itself went wrong.
fn dialog_absent(error: &zbus::Error) -> bool {
    use zbus::fdo::Error as Fdo;
    match error {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        zbus::Error::FDO(e) => matches!(**e, Fdo::ServiceUnknown(_) | Fdo::NameHasNoOwner(_)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::fdo::Error as Fdo;

    #[test]
    fn only_a_missing_dialog_logs_out_unasked() {
        let fdo = |e: Fdo| zbus::Error::FDO(Box::new(e));
        assert!(dialog_absent(&fdo(Fdo::ServiceUnknown("x".into()))));
        assert!(dialog_absent(&fdo(Fdo::NameHasNoOwner("x".into()))));

        assert!(!dialog_absent(&fdo(Fdo::NoReply("x".into()))));
        assert!(!dialog_absent(&fdo(Fdo::Failed("x".into()))));
        assert!(!dialog_absent(&fdo(Fdo::AccessDenied("x".into()))));
        assert!(!dialog_absent(&zbus::Error::InputOutput(
            std::sync::Arc::new(std::io::Error::other("no bus"))
        )));
    }

    #[test]
    fn reads_the_error_name_of_a_bus_reply() {
        let reply = |name: &str| {
            let msg = zbus::message::Message::method_call("/", "PresentAccess")
                .unwrap()
                .build(&())
                .unwrap();
            zbus::Error::MethodError(name.try_into().unwrap(), None, msg)
        };
        assert!(dialog_absent(&reply(
            "org.freedesktop.DBus.Error.ServiceUnknown"
        )));
        assert!(dialog_absent(&reply(
            "org.freedesktop.DBus.Error.NameHasNoOwner"
        )));
        assert!(!dialog_absent(&reply("org.freedesktop.DBus.Error.NoReply")));
        assert!(!dialog_absent(&reply("org.otto.Dialog1.Error.Busy")));
    }
}
