//! `org.otto.Dialog1`: Otto's system dialog, for permissions and questions.
//!
//! Served by otto-islands (`components/otto-islands/src/dbus_service.rs`),
//! under its bus name `org.otto.Island`; see `specs/portal-access-dialog.md`.
//! Every method answers `(response, results)`: `response` is `0` granted,
//! `1` denied or cancelled, `2` ended without an answer and `3` the open
//! button; `results` is `[(group_id, selected_option_id)]`.

use std::collections::HashMap;

/// The well-known bus name (otto-islands', which serves the dialog).
pub const SERVICE: &str = "org.otto.Island";
/// The object path.
pub const PATH: &str = "/org/otto/Dialog";

/// A choice group: `(group_id, label, [(option_id, label, icon)],
/// default_option_id)`.
pub type WireChoice = (String, String, Vec<(String, String, String)>, String);

/// A question for [`DialogProxy::present_questions`]:
/// `(id, label, multi, [(option_id, label, icon)], default_option_ids)`.
pub type WireQuestion = (
    String,
    String,
    bool,
    Vec<(String, String, String)>,
    Vec<String>,
);

#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog"
)]
pub trait Dialog {
    /// A permission dialog, in the shape of
    /// `org.freedesktop.impl.portal.Access`.
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

    /// [`present_access`](Self::present_access) plus an open button. An
    /// empty `grant_label` or `open_label` hides that button.
    #[allow(clippy::too_many_arguments)]
    fn present_question(
        &self,
        app_id: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        open_label: &str,
        modal: bool,
        choices: Vec<WireChoice>,
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;

    /// One question a page, single- or multi-select; `results` carries an
    /// entry per picked option. `cookie` names the dialog for
    /// [`withdraw`](Self::withdraw).
    #[allow(clippy::too_many_arguments)]
    fn present_questions(
        &self,
        app_id: &str,
        cookie: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        open_label: &str,
        labels: HashMap<String, String>,
        modal: bool,
        questions: Vec<WireQuestion>,
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;

    /// Take down the dialog `cookie` names, answering it as ended. Whether
    /// there was one.
    fn withdraw(&self, app_id: &str, cookie: &str) -> zbus::Result<bool>;
}
