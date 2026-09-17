use otto_kit::AppContext;
use tokio::sync::oneshot;
use zbus::interface;

use crate::activity::Priority;
use crate::dialog::{ChoiceGroup, ChoiceOption, DialogRequest, RESPONSE_ENDED};
use crate::state::SharedState;

pub const DBUS_NAME: &str = "org.otto.Island";
pub const DBUS_PATH: &str = "/org/otto/Island";
pub const DIALOG_DBUS_PATH: &str = "/org/otto/Dialog";

pub struct IslandService {
    state: SharedState,
}

impl IslandService {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[interface(name = "org.otto.Island1")]
impl IslandService {
    /// Create a new activity in the island.
    ///
    /// Returns the activity ID on success.
    /// `progress`: 0.0–1.0 for a progress bar, negative for no progress.
    /// `priority`: "low", "normal", "high", or "critical".
    async fn create_activity(
        &self,
        app_id: &str,
        title: &str,
        icon: &str,
        progress: f64,
        timeout_ms: u32,
        priority: &str,
        live: bool,
    ) -> zbus::fdo::Result<u64> {
        let priority = Priority::try_from(priority).map_err(zbus::fdo::Error::InvalidArgs)?;

        let progress = if progress < 0.0 {
            None
        } else {
            Some(progress.clamp(0.0, 1.0))
        };

        let mut state = self.state.lock().unwrap();
        let id = state.create_activity(
            app_id.to_string(),
            title.to_string(),
            icon.to_string(),
            progress,
            timeout_ms,
            priority,
            live,
        );
        drop(state);

        AppContext::request_wakeup();
        tracing::info!(id, app_id, title, "activity created");
        Ok(id)
    }

    /// Update an existing activity's title and/or progress.
    ///
    /// Pass an empty string for title to leave it unchanged.
    /// Pass a negative value for progress to clear it.
    async fn update_activity(
        &self,
        id: u64,
        title: &str,
        progress: f64,
    ) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let ok = state.update_activity(id, title, progress);
        drop(state);

        if ok {
            AppContext::request_wakeup();
        }
        Ok(ok)
    }

    /// Dismiss an activity by ID.
    async fn dismiss_activity(&self, id: u64) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let ok = state.dismiss_activity(id);
        drop(state);

        if ok {
            AppContext::request_wakeup();
            tracing::info!(id, "activity dismissed");
        }
        Ok(ok)
    }
}

/// Type alias for a choice group as it arrives over D-Bus:
/// `(group_id, group_label, [(option_id, option_label, option_icon)], default_option_id)`.
type WireChoice = (String, String, Vec<(String, String, String)>, String);

/// `org.otto.Dialog1` — an Access-style permission/choice dialog service.
///
/// Mirrors `org.freedesktop.impl.portal.Access` semantics so `otto-portal` can
/// route both external portal Access requests and internal requests through the
/// island UI. See `specs/portal-access-dialog.md`.
pub struct DialogService {
    state: SharedState,
}

impl DialogService {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

/// Which `org.otto.Dialog1` method a request came in through. The two share
/// one dialog; they differ only in how empty button labels are read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    /// `PresentAccess`: an empty grant label falls back to a default, and
    /// there is no open button.
    Access,
    /// `PresentQuestion`: an empty grant or open label hides that button.
    Question,
}

/// Convert wire choice groups into the dialog model, dropping groups with no
/// options (they would be unanswerable) and resolving each default by id.
fn choice_groups(choices: Vec<WireChoice>) -> Vec<ChoiceGroup> {
    choices
        .into_iter()
        .filter_map(|(id, label, opts, default_id)| {
            let options: Vec<ChoiceOption> = opts
                .into_iter()
                .map(|(oid, olabel, oicon)| ChoiceOption {
                    id: oid,
                    label: olabel,
                    icon: oicon,
                })
                .collect();
            if options.is_empty() {
                return None;
            }
            let default = options.iter().position(|o| o.id == default_id).unwrap_or(0);
            Some(ChoiceGroup {
                id,
                label,
                options,
                default,
            })
        })
        .collect()
}

impl DialogService {
    #[allow(clippy::too_many_arguments)]
    async fn present(
        &self,
        kind: DialogKind,
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
    ) -> (u32, Vec<(String, String)>) {
        let groups = choice_groups(choices);

        let grant_label = if !grant_label.is_empty() || kind == DialogKind::Question {
            grant_label.to_string()
        } else if groups.is_empty() {
            otto_kit::t_owned!("islands-dialog-allow")
        } else {
            otto_kit::t_owned!("islands-dialog-continue")
        };
        let deny_label = if deny_label.is_empty() {
            otto_kit::t_owned!("islands-dialog-deny")
        } else {
            deny_label.to_string()
        };
        let open_label = match kind {
            DialogKind::Access => String::new(),
            DialogKind::Question => open_label.to_string(),
        };

        let (tx, rx) = oneshot::channel();
        let req = DialogRequest {
            id: 0,
            app_id: app_id.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            body: body.to_string(),
            icon: icon.to_string(),
            grant_label,
            deny_label,
            open_label,
            modal,
            choices: groups,
            response_tx: Some(tx),
        };

        {
            let mut state = self.state.lock().unwrap();
            state.add_dialog(req);
        }
        AppContext::request_wakeup();
        tracing::info!(app_id, title, "dialog presented");

        match rx.await {
            Ok(resp) => (resp.response, resp.results),
            Err(_) => (RESPONSE_ENDED, Vec::new()),
        }
    }
}

#[interface(name = "org.otto.Dialog1")]
impl DialogService {
    /// Present a dialog and block until the user answers or the request is
    /// withdrawn (caller aborts / disconnects).
    ///
    /// `choices`: list of `(group_id, group_label, options, default_option_id)`
    ///   where each option is `(option_id, option_label, option_icon)`. An empty
    ///   list makes a plain grant/deny permission prompt.
    ///
    /// Returns `(response, results)`:
    /// - `response`: `0` granted, `1` cancelled/denied, `2` ended.
    /// - `results`: `(group_id, selected_option_id)` for each choice group.
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
    ) -> (u32, Vec<(String, String)>) {
        self.present(
            DialogKind::Access,
            app_id,
            title,
            subtitle,
            body,
            icon,
            grant_label,
            deny_label,
            "",
            modal,
            choices,
        )
        .await
    }

    /// Present a question: the `PresentAccess` dialog, plus an optional open
    /// button that hands the question off to another app.
    ///
    /// - An empty `grant_label` hides the grant button.
    /// - An empty `open_label` hides the open button.
    ///
    /// Returns `(response, results)`:
    /// - `response`: `0` confirmed, `1` cancelled/denied, `2` ended, `3` the
    ///   open button was pressed.
    /// - `results`: `(group_id, selected_option_id)` for each choice group when
    ///   confirmed; empty otherwise.
    #[allow(clippy::too_many_arguments)]
    async fn present_question(
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
    ) -> (u32, Vec<(String, String)>) {
        self.present(
            DialogKind::Question,
            app_id,
            title,
            subtitle,
            body,
            icon,
            grant_label,
            deny_label,
            open_label,
            modal,
            choices,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(id: &str) -> (String, String, String) {
        (id.into(), id.to_uppercase(), String::new())
    }

    #[test]
    fn choice_groups_resolve_defaults_and_drop_empty_groups() {
        let groups = choice_groups(vec![
            (
                "output".into(),
                "Output".into(),
                vec![opt("a"), opt("b")],
                "b".into(),
            ),
            ("empty".into(), "Empty".into(), Vec::new(), String::new()),
            (
                "mode".into(),
                String::new(),
                vec![opt("x")],
                "missing".into(),
            ),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].id, "output");
        assert_eq!(groups[0].default, 1);
        assert_eq!(groups[1].id, "mode");
        assert_eq!(groups[1].default, 0);
    }

    /// The contract otto-agentsd calls: `PresentQuestion` takes the
    /// `PresentAccess` arguments with `open_label` after `deny_label`.
    #[test]
    fn present_question_signature_on_the_bus() {
        use zbus::object_server::Interface;
        let service = DialogService::new(std::sync::Arc::new(std::sync::Mutex::new(
            crate::state::IslandState::new(),
        )));
        let mut xml = String::new();
        service.introspect_to_writer(&mut xml, 0);
        let args = |method: &str| -> String {
            let start = xml.find(&format!("<method name=\"{method}\">")).unwrap();
            let end = start + xml[start..].find("</method>").unwrap();
            xml[start..end]
                .split("type=\"")
                .skip(1)
                .map(|t| t.split('"').next().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            args("PresentAccess"),
            "s s s s s s s b a(ssa(sss)s) u a(ss)"
        );
        assert_eq!(
            args("PresentQuestion"),
            "s s s s s s s s b a(ssa(sss)s) u a(ss)"
        );
    }
}
