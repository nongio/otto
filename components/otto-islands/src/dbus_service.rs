use otto_kit::AppContext;
use tokio::sync::oneshot;
use zbus::interface;

use crate::activity::Priority;
use crate::dialog::{ChoiceGroup, ChoiceOption, DialogRequest};
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
        let groups: Vec<ChoiceGroup> = choices
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
                // A group with no options is unanswerable — drop it.
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
            .collect();

        let grant_label = if !grant_label.is_empty() {
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
            Err(_) => (2, Vec::new()),
        }
    }
}
