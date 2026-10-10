//! Log Out: ask first, then hand the compositor its `logout` command.
//!
//! The question is put by otto-islands' system dialog (`org.otto.Dialog1`),
//! the one the portal and the agents use, so it looks and answers to the
//! keyboard like every other question the desktop asks. The compositor then
//! closes the windows one by one and ends the session once they have gone;
//! see `src/state/logout.rs`.

use otto_kit::dbus::dialog::DialogProxy;
use zbus::Connection;

/// The dialog's grant response.
const GRANTED: u32 = 0;

/// Ask whether to log out, and log out on yes.
///
/// Without otto-islands there is nobody to ask, and the logout goes ahead:
/// the compositor still closes each window the polite way, so an
/// application with unsaved work gets its say either way. Any other failure
/// (a timeout, the dialog erroring, no session bus) logs out nobody: a
/// question that could not be answered is not a yes.
pub fn confirm_and_log_out() {
    let title = otto_kit::t!("bar-logout-title");
    let body = otto_kit::t!("bar-logout-body");
    let grant = otto_kit::t!("bar-otto-log-out");
    let deny = otto_kit::t!("common-cancel");
    tokio::spawn(async move {
        let answer = async {
            let conn = Connection::session().await?;
            let dialog = DialogProxy::new(&conn).await?;
            dialog
                .present_access(
                    "",
                    title,
                    "",
                    body,
                    "system-log-out",
                    grant,
                    deny,
                    true,
                    Vec::new(),
                )
                .await
        }
        .await;
        match answer {
            Ok((GRANTED, _)) => crate::keyboard_layout::run_shell_command("logout".to_string()),
            Ok(_) => {}
            Err(e) if dialog_absent(&e) => {
                tracing::warn!("no logout dialog ({e}); logging out without asking");
                crate::keyboard_layout::run_shell_command("logout".to_string());
            }
            Err(e) => tracing::warn!("logout dialog failed ({e}); not logging out"),
        }
    });
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
