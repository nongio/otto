//! Client for otto-islands' `org.otto.Dialog1` Access-style dialog renderer.
//!
//! The portal is the broker: it owns which renderer presents a dialog. Today
//! the only renderer is otto-islands. See `specs/portal-access-dialog.md`.

use zbus::Result;

use crate::otto_client::OttoClient;

/// A choice group as sent to the renderer:
/// `(group_id, group_label, [(option_id, option_label, option_icon)], default_option_id)`.
pub type WireChoice = (String, String, Vec<(String, String, String)>, String);

/// D-Bus proxy for `org.otto.Dialog1` (served by otto-islands).
#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog"
)]
trait Dialog {
    /// Present a dialog and block until the user answers or the request is
    /// withdrawn. Returns `(response, results)`:
    /// - `response`: `0` granted, `1` cancelled/denied, `2` ended.
    /// - `results`: `[(group_id, selected_option_id)]`.
    #[allow(clippy::too_many_arguments)]
    async fn present_access(
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
    ) -> Result<(u32, Vec<(String, String)>)>;
}

impl OttoClient {
    /// Build a proxy to the dialog renderer.
    pub async fn dialog_proxy(&self) -> Result<DialogProxy<'_>> {
        DialogProxy::new(&self.connection).await
    }
}
