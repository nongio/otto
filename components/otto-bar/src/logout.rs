//! Log Out: ask first, then hand the compositor its `logout` command.
//!
//! The question is put by otto-islands' system dialog (`org.otto.Dialog1`),
//! the one the portal and the agents use, so it looks and answers to the
//! keyboard like every other question the desktop asks. The compositor then
//! closes the windows one by one and ends the session once they have gone;
//! see `src/state/logout.rs`.

use zbus::Connection;

/// A choice group as the dialog takes it:
/// `(group_id, label, [(option_id, label, icon)], default_option_id)`.
/// Log Out sends none.
type WireChoice = (String, String, Vec<(String, String, String)>, String);

/// `org.otto.Dialog1`, served by otto-islands.
#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog"
)]
trait Dialog {
    /// Returns `(response, results)`: response 0 is the grant button.
    #[allow(clippy::too_many_arguments)]
    fn present_access(
        &self,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        modal: bool,
        choices: Vec<WireChoice>,
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;
}

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
