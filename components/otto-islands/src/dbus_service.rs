use otto_kit::AppContext;
use std::collections::HashMap;
use tokio::sync::oneshot;

use zbus::interface;

use crate::activity::Priority;
use crate::dialog::{
    ChoiceGroup, ChoiceOption, DialogRequest, DialogView, QuestionStyle, RESPONSE_ENDED,
};
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
    /// `quiet`: report it to the dock icon but do not put it on screen — for
    /// an app whose own window is in front of the user and has already said
    /// what it is doing. See [`crate::activity::Activity::quiet`].
    async fn create_activity(
        &self,
        app_id: &str,
        title: &str,
        icon: &str,
        progress: f64,
        timeout_ms: u32,
        priority: &str,
        live: bool,
        quiet: bool,
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
            quiet,
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

    /// Show or hide a running activity's island without ending it.
    ///
    /// The dock icon keeps filling either way. An app calls this as its own
    /// window comes forward and goes away again, so a job it is already
    /// reporting in its window is not reported twice.
    async fn set_activity_quiet(&self, id: u64, quiet: bool) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let ok = state.set_activity_quiet(id, quiet);
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

/// A question as `PresentQuestions` takes it:
/// `(group_id, label, multi, [(option_id, option_label, option_icon)], default_option_ids)`.
/// A single-select question starts on its first default; a multi-select one
/// starts with every default picked.
type WireQuestion = (
    String,
    String,
    bool,
    Vec<(String, String, String)>,
    Vec<String>,
);

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
    /// `PresentQuestions`: the dialog owns the words for answering — the
    /// asker sends the questions, not the buttons — so an empty grant or deny
    /// label falls back to the dialog's own. An empty open label still hides
    /// that button: only the caller knows whether there is anywhere to open.
    Questions,
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
                ..ChoiceGroup::default()
            })
        })
        .collect()
}

/// Convert `PresentQuestions` questions into the dialog model, dropping
/// questions with no options.
fn question_groups(questions: Vec<WireQuestion>) -> Vec<ChoiceGroup> {
    questions
        .into_iter()
        .filter_map(|(id, label, multi, opts, defaults)| {
            let options: Vec<ChoiceOption> = opts
                .into_iter()
                .map(|(id, label, icon)| ChoiceOption { id, label, icon })
                .collect();
            if options.is_empty() {
                return None;
            }
            let picked: Vec<usize> = defaults
                .iter()
                .filter_map(|d| options.iter().position(|o| o.id == *d))
                .collect();
            let default = if multi {
                0
            } else {
                picked.first().copied().unwrap_or(0)
            };
            Some(ChoiceGroup {
                id,
                label,
                options,
                default,
                multi,
                picked: if multi { picked } else { Vec::new() },
            })
        })
        .collect()
}

/// The `PresentQuestions` style: the dialog's own words for getting through
/// the questions, and the presentation hints the caller does send. A caller
/// that supplies one of the labels itself overrides the dialog's.
fn question_style(groups: usize, labels: &HashMap<String, String>) -> QuestionStyle {
    let label = |key: &str| labels.get(key).cloned().unwrap_or_default();
    QuestionStyle {
        paged: groups > 1,
        // Left empty, each of these becomes the dialog's own word for it —
        // see `DialogView::fill_question_words`, and `dialog_layout` for the
        // counter, which is built with the page numbers in it.
        next_label: label("next"),
        back_label: label("back"),
        page_label: label("page"),
        multi_hint: label("multi-hint"),
        body_start: label("body-align") == "start",
        handle_title: label("title-style") == "handle",
        quiet: label("focus") == "none",
    }
}

impl DialogService {
    #[allow(clippy::too_many_arguments)]
    async fn present(
        &self,
        kind: DialogKind,
        app_id: &str,
        cookie: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        icon: &str,
        grant_label: &str,
        deny_label: &str,
        open_label: &str,
        modal: bool,
        groups: Vec<ChoiceGroup>,
        style: QuestionStyle,
    ) -> (u32, Vec<(String, String)>) {
        let grant_label = match kind {
            _ if !grant_label.is_empty() => grant_label.to_string(),
            // Nothing to answer with, and nothing to answer: the caller only
            // wants the question read, or handed somewhere else. A questions
            // dialog fills in its own words below.
            DialogKind::Question | DialogKind::Questions => String::new(),
            DialogKind::Access if groups.is_empty() => {
                otto_kit::t_owned!("islands-dialog-allow")
            }
            DialogKind::Access => otto_kit::t_owned!("islands-dialog-continue"),
        };
        let deny_label = match kind {
            _ if !deny_label.is_empty() => deny_label.to_string(),
            DialogKind::Questions => String::new(),
            _ => otto_kit::t_owned!("islands-dialog-deny"),
        };
        let open_label = match kind {
            DialogKind::Access => String::new(),
            DialogKind::Question | DialogKind::Questions => open_label.to_string(),
        };

        let (tx, rx) = oneshot::channel();
        let mut view = DialogView {
            id: 0,
            app_id: app_id.to_string(),
            cookie: cookie.to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            body: body.to_string(),
            icon: icon.to_string(),
            grant_label,
            deny_label,
            open_label,
            modal,
            choices: groups,
            style,
        };
        if kind == DialogKind::Questions {
            // The caller sends the questions; the words for getting through
            // them are the dialog's own, in the user's language.
            view.fill_question_words();
        }
        let req = DialogRequest::from_view(view, tx);

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
            "",
            title,
            subtitle,
            body,
            icon,
            grant_label,
            deny_label,
            "",
            modal,
            choice_groups(choices),
            QuestionStyle::default(),
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
            "",
            title,
            subtitle,
            body,
            icon,
            grant_label,
            deny_label,
            open_label,
            modal,
            choice_groups(choices),
            QuestionStyle::default(),
        )
        .await
    }

    /// Present questions: the `PresentQuestion` dialog, with multi-select
    /// questions and several questions asked a page at a time.
    ///
    /// `questions`: `(group_id, label, multi, options, default_option_ids)`,
    /// options as in `PresentQuestion`. An option label may carry a
    /// description after its first line break.
    ///
    /// The dialog owns the words for getting through the questions — answer,
    /// skip, next, back, the counter, the multi-select hint — and localises
    /// them, so a caller need only send the questions themselves and, when
    /// there is somewhere to open them, `open_label`. An empty `grant_label`
    /// or `deny_label` takes the dialog's own.
    ///
    /// `labels` (all optional) overrides those words, for a caller that has
    /// better ones: `next` — the grant button before the last page; `back` —
    /// the back button from the second page on; `page` — a page counter with
    /// `{current}` and `{total}`; `multi-hint` — a line under a multi-select
    /// question. Two are presentation, not words:
    /// `body-align` — `start` for a left-aligned body;
    /// `title-style` — `handle` when the title is the asker's handle
    /// ("@claude") rather than a headline, which makes the question itself the
    /// panel's largest text.
    ///
    /// Returns `(response, results)` as `PresentQuestion` does, except that
    /// `results` has one `(group_id, option_id)` per picked option of a
    /// multi-select question (none when nothing is picked).
    #[allow(clippy::too_many_arguments)]
    async fn present_questions(
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
    ) -> (u32, Vec<(String, String)>) {
        let groups = question_groups(questions);
        let style = question_style(groups.len(), &labels);
        self.present(
            DialogKind::Questions,
            app_id,
            cookie,
            title,
            subtitle,
            body,
            icon,
            grant_label,
            deny_label,
            open_label,
            modal,
            groups,
            style,
        )
        .await
    }

    /// Takes down the dialog `cookie` names, if it is still up, answering it
    /// as ended. For a caller whose question has been settled somewhere else —
    /// answered in another window, or cancelled — so the panel does not sit
    /// there collecting an answer nobody waits for.
    async fn withdraw(&self, app_id: &str, cookie: &str) -> bool {
        let withdrawn = {
            let mut state = self.state.lock().unwrap();
            state.withdraw_dialog(app_id, cookie)
        };
        if withdrawn {
            AppContext::request_wakeup();
            tracing::info!(app_id, cookie, "dialog withdrawn");
        }
        withdrawn
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

    /// The contract otto-agents calls: `PresentQuestion` takes the
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
        // The extra leading `s` over PresentQuestion is the caller's cookie,
        // which is how Withdraw names the dialog to take down again.
        assert_eq!(
            args("PresentQuestions"),
            "s s s s s s s s s a{ss} b a(ssba(sss)as) u a(ss)"
        );
        assert_eq!(args("Withdraw"), "s s b");
    }

    #[test]
    fn questions_carry_multi_select_defaults_and_paging() {
        let groups = question_groups(vec![
            (
                "q0".into(),
                "Role?".into(),
                false,
                vec![opt("a"), opt("b")],
                vec!["b".into()],
            ),
            (
                "q1".into(),
                "Pains?".into(),
                true,
                vec![opt("x"), opt("y"), opt("z")],
                vec!["x".into(), "z".into(), "missing".into()],
            ),
            ("q2".into(), "Empty".into(), true, Vec::new(), Vec::new()),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[0].multi, groups[0].default), (false, 1));
        assert!(groups[1].multi);
        assert_eq!(groups[1].picked, [0, 2]);

        let labels = HashMap::from([
            ("next".to_owned(), "Onward".to_owned()),
            ("body-align".to_owned(), "start".to_owned()),
            ("title-style".to_owned(), "handle".to_owned()),
        ]);
        let style = question_style(groups.len(), &labels);
        assert!(style.paged && style.body_start && style.handle_title);
        // A caller's own label is kept; what it sends none of, the dialog
        // fills in with its own words.
        assert_eq!(style.next_label, "Onward");
        let mut view = DialogView {
            id: 0,
            app_id: String::new(),
            cookie: String::new(),
            title: String::new(),
            subtitle: String::new(),
            body: String::new(),
            icon: String::new(),
            grant_label: String::new(),
            deny_label: String::new(),
            open_label: String::new(),
            modal: false,
            choices: groups,
            style: question_style(2, &HashMap::new()),
        };
        view.fill_question_words();
        assert_eq!(
            view.grant_label,
            otto_kit::t_owned!("islands-dialog-answer")
        );
        assert_eq!(view.deny_label, otto_kit::t_owned!("islands-dialog-skip"));
        assert_eq!(
            view.style.next_label,
            otto_kit::t_owned!("islands-dialog-next")
        );
        assert_eq!(
            view.style.back_label,
            otto_kit::t_owned!("islands-dialog-back")
        );
        assert_eq!(
            view.style.multi_hint,
            otto_kit::t_owned!("islands-dialog-multi-hint")
        );
        assert!(!view.style.handle_title && !view.style.body_start);
        assert!(!question_style(1, &labels).paged);
    }
}
