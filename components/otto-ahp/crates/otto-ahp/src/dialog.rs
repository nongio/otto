//! Permission dialogs: asking the user whether an agent may use a tool.
//!
//! Otto's dialog renderer, otto-islands, serves `org.otto.Dialog1` (see
//! `specs/portal-access-dialog.md` in Otto): an Access-style grant or deny
//! prompt, shown as a modal island panel. Agents configured with
//! `permissions = "ask"` have their requests shown there. When the dialog
//! cannot be shown the request is denied, so an agent is never allowed
//! something nobody saw.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use agent_client_protocol::schema::v1::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, ToolKind,
};
use tokio::sync::OnceCell;

use crate::agent::{Decision, Question, QuestionOption};

/// What a permission dialog says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// Who wants to do what: "Claude wants to run a command".
    pub title: String,
    /// The tool call as the agent describes it: the command, the file.
    pub subtitle: String,
    /// Where: the session's folder.
    pub body: String,
    /// The grant button, named after the agent's own option.
    pub grant: String,
    /// The deny button, likewise.
    pub deny: String,
}

/// Puts a [`Prompt`] in front of the user.
pub trait Prompter: Send + Sync {
    /// Resolves to whether the user granted the request. Anything short of a
    /// grant is `false`: a denial, a dismissed dialog, or no dialog at all.
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
}

/// The dialog for `request`, made by the agent called `agent` in `cwd`.
pub fn prompt_for(agent: &str, cwd: &Path, request: &RequestPermissionRequest) -> Prompt {
    let fields = &request.tool_call.fields;
    let action = match fields.kind {
        Some(ToolKind::Read) => "read a file",
        Some(ToolKind::Edit) => "edit a file",
        Some(ToolKind::Delete) => "delete a file",
        Some(ToolKind::Move) => "move a file",
        Some(ToolKind::Search) => "search",
        Some(ToolKind::Execute) => "run a command",
        Some(ToolKind::Fetch) => "fetch from the web",
        Some(ToolKind::SwitchMode) => "change how it works",
        _ => "use a tool",
    };
    let name = |allow| {
        option(request, allow)
            .map(|index| request.options[index].name.clone())
            .unwrap_or_default()
    };
    Prompt {
        title: format!("{agent} wants to {action}"),
        subtitle: fields.title.clone().unwrap_or_default(),
        body: format!("in {}", folder(cwd)),
        grant: name(true),
        deny: name(false),
    }
}

/// `request` as a [`Question`] the host can put to the user, anywhere.
pub fn question_for(agent: &str, cwd: &Path, request: &RequestPermissionRequest) -> Question {
    let tool_name = match request.tool_call.fields.kind {
        Some(ToolKind::Read) => "read",
        Some(ToolKind::Edit) => "edit",
        Some(ToolKind::Delete) => "delete",
        Some(ToolKind::Move) => "move",
        Some(ToolKind::Search) => "search",
        Some(ToolKind::Execute) => "execute",
        Some(ToolKind::Fetch) => "fetch",
        Some(ToolKind::SwitchMode) => "switch-mode",
        _ => "tool",
    };
    Question {
        tool_call_id: request.tool_call.tool_call_id.to_string(),
        tool_name: tool_name.to_owned(),
        prompt: prompt_for(agent, cwd, request),
        options: request
            .options
            .iter()
            .map(|option| QuestionOption {
                id: option.option_id.to_string(),
                label: option.name.clone(),
                allow: matches!(
                    option.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                ),
            })
            .collect(),
    }
}

/// Answers `request` with `decision`: the option it names, when the agent
/// offered one with that id and the right intent, and otherwise the agent's
/// narrowest option for the decision.
pub fn decide(
    request: &RequestPermissionRequest,
    decision: &Decision,
) -> RequestPermissionResponse {
    let named = decision.option_id.as_deref().and_then(|id| {
        request.options.iter().find(|option| {
            option.option_id.to_string() == id
                && matches!(
                    option.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                ) == decision.approved
        })
    });
    match named {
        Some(option) => RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(option.option_id.clone()),
        )),
        None => answer(request, decision.approved),
    }
}

/// The option to pick from `options` for a plain yes or no: the narrowest
/// allowing one for yes, and the narrowest rejecting one for no.
pub fn narrowest(options: &[QuestionOption], allow: bool) -> Option<&QuestionOption> {
    // Agents list "always" before "once", as Claude does, so the last match of
    // the right intent is the narrowest.
    options.iter().rev().find(|option| option.allow == allow)
}

/// Answers `request` with the agent's narrowest option that allows it, or that
/// rejects it: "once" before "always". Cancelled when the agent offered none.
pub fn answer(request: &RequestPermissionRequest, allow: bool) -> RequestPermissionResponse {
    let outcome = match option(request, allow) {
        Some(index) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            request.options[index].option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
    };
    RequestPermissionResponse::new(outcome)
}

fn option(request: &RequestPermissionRequest, allow: bool) -> Option<usize> {
    let preferred = if allow {
        [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ]
    } else {
        [
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ]
    };
    preferred.iter().find_map(|kind| {
        request
            .options
            .iter()
            .position(|option| option.kind == *kind)
    })
}

/// `cwd` for people: under the home folder as `~/…`.
fn folder(cwd: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|home| cwd.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => cwd.display().to_string(),
    }
}

/// A choice group as `org.otto.Dialog1` takes it:
/// `(group_id, group_label, [(option_id, option_label, option_icon)], default_option_id)`.
type WireChoice = (String, String, Vec<(String, String, String)>, String);

#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog",
    gen_blocking = false
)]
trait Dialog {
    /// Presents a dialog and returns once the user answers:
    /// `(response, results)`, where `response` is `0` granted, `1` denied and
    /// `2` ended without an answer.
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
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;
}

/// Shows prompts through otto-islands, on the session bus.
#[derive(Default)]
pub struct Islands {
    /// Connected on the first prompt. `None` when there is no session bus,
    /// which is remembered so it is reported once.
    connection: OnceCell<Option<zbus::Connection>>,
}

impl Islands {
    async fn connection(&self) -> Option<&zbus::Connection> {
        self.connection
            .get_or_init(|| async {
                zbus::Connection::session()
                    .await
                    .inspect_err(|err| {
                        tracing::warn!(%err, "no session bus; permission requests will be denied")
                    })
                    .ok()
            })
            .await
            .as_ref()
    }
}

impl Prompter for Islands {
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move {
            let Some(connection) = self.connection().await else {
                return false;
            };
            let proxy = match DialogProxy::new(connection).await {
                Ok(proxy) => proxy,
                Err(err) => {
                    tracing::warn!(%err, "no dialog renderer; denying the permission request");
                    return false;
                }
            };
            tracing::info!(title = %prompt.title, subtitle = %prompt.subtitle, "asking for permission");
            let answer = proxy
                .present_access(
                    "otto-ahp",
                    &prompt.title,
                    &prompt.subtitle,
                    &prompt.body,
                    "system-run",
                    &prompt.grant,
                    &prompt.deny,
                    true,
                    Vec::new(),
                )
                .await;
            match answer {
                Ok((response, _)) => {
                    tracing::info!(response, "permission dialog answered");
                    response == 0
                }
                Err(err) => {
                    tracing::warn!(%err, "could not show the permission dialog; denying");
                    false
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        PermissionOption, ToolCallUpdate, ToolCallUpdateFields,
    };

    fn request(options: Vec<PermissionOption>) -> RequestPermissionRequest {
        let mut fields = ToolCallUpdateFields::new();
        fields.kind = Some(ToolKind::Execute);
        fields.title = Some("cargo test".into());
        RequestPermissionRequest::new("s", ToolCallUpdate::new("call", fields), options)
    }

    fn claude_options() -> Vec<PermissionOption> {
        vec![
            PermissionOption::new(
                "allow_always",
                "Always Allow",
                PermissionOptionKind::AllowAlways,
            ),
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ]
    }

    fn selected(response: &RequestPermissionResponse) -> Option<String> {
        match &response.outcome {
            RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.to_string()),
            _ => None,
        }
    }

    #[test]
    fn the_dialog_says_who_wants_to_do_what_and_where() {
        let prompt = prompt_for(
            "Claude",
            Path::new("/srv/project"),
            &request(claude_options()),
        );
        assert_eq!(
            prompt,
            Prompt {
                title: "Claude wants to run a command".into(),
                subtitle: "cargo test".into(),
                body: "in /srv/project".into(),
                grant: "Allow".into(),
                deny: "Reject".into(),
            }
        );
    }

    #[test]
    fn answers_pick_the_narrowest_option() {
        let request = request(claude_options());
        assert_eq!(selected(&answer(&request, true)).as_deref(), Some("allow"));
        assert_eq!(
            selected(&answer(&request, false)).as_deref(),
            Some("reject")
        );
    }

    #[test]
    fn with_no_matching_option_the_request_is_cancelled() {
        let only_allow = vec![PermissionOption::new(
            "go",
            "Go",
            PermissionOptionKind::AllowAlways,
        )];
        let request = request(only_allow);
        assert_eq!(selected(&answer(&request, true)).as_deref(), Some("go"));
        assert_eq!(selected(&answer(&request, false)), None);
        assert_eq!(prompt_for("A", Path::new("/"), &request).deny, "");
    }
}
