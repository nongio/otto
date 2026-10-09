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
/// application with unsaved work gets its say either way.
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
            Err(e) => {
                tracing::warn!("no logout dialog ({e}); logging out without asking");
                crate::keyboard_layout::run_shell_command("logout".to_string());
            }
        }
    });
}
