//! Dialogs: asking the user whether an agent may use a tool, or how to answer
//! an agent's question.
//!
//! Otto's dialog renderer, otto-islands, serves `org.otto.Dialog1` (see
//! `specs/portal-access-dialog.md` in Otto): a modal island panel with grant,
//! deny and "open in Ask" buttons and, optionally, groups of choices. What no
//! client is watching the chat to answer is asked there: permission requests
//! from agents configured with `permissions = "ask"`, and agents' questions.
//! When a permission dialog cannot be shown the request is denied, so an agent
//! is never allowed something nobody saw; a question that cannot be shown
//! stays open in the chat.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use agent_client_protocol::schema::v1::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, ToolKind,
};
use tokio::sync::OnceCell;

use crate::agent::{Decision, Question, QuestionOption};

/// The label of the button that hands a question to Ask.
pub const OPEN_IN_ASK: &str = "Open in Ask";

/// What a dialog says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    /// Who wants to do what: "Claude wants to run a command".
    pub title: String,
    /// The tool call as the agent describes it: the command, the file.
    pub subtitle: String,
    /// Where: the session's folder.
    pub body: String,
    /// The grant button, named after the agent's own option. Empty hides it.
    pub grant: String,
    /// The deny button, likewise.
    pub deny: String,
    /// The button that hands the question to Ask instead. Empty hides it.
    pub open: String,
    /// The dialog's icon name.
    pub icon: String,
    /// Groups of options, one picked from each, sent back with a grant.
    pub choices: Vec<Choice>,
}

/// A group of options in a [`Prompt`], of which the user picks one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    /// `(id, label)` pairs, in order.
    pub options: Vec<(String, String)>,
    /// The option picked to begin with.
    pub default: String,
}

/// How the user answered a [`Prompt`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Granted, with the option picked in each choice group: `(group, option)`.
    Granted(Vec<(String, String)>),
    /// Denied, or skipped.
    Denied,
    /// The dialog went away without an answer.
    Ended,
    /// The user asked to answer in Ask instead.
    Open,
    /// No dialog could be shown.
    Unavailable,
}

/// Puts a [`Prompt`] in front of the user.
pub trait Prompter: Send + Sync {
    /// Resolves to what the user said.
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = Reply> + Send + '_>>;

    /// Opens the session at `session_uri` in Ask, for the user to answer there.
    fn open(&self, session_uri: &str) {
        open_in_ask(session_uri);
    }
}

/// Starts `otto-ask --session <session_uri>` in a process group of its own,
/// so it is not stopped along with this service.
pub fn open_in_ask(session_uri: &str) {
    let mut command = tokio::process::Command::new("otto-ask");
    command
        .arg("--session")
        .arg(session_uri)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    match command.spawn() {
        Ok(mut child) => {
            tracing::info!(session_uri, "opening the session in Ask");
            // Reaped once it exits, so it never lingers as a zombie.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
        Err(err) => tracing::warn!(%err, session_uri, "could not start otto-ask"),
    }
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
        icon: "system-run".into(),
        ..Prompt::default()
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
pub fn folder(cwd: &Path) -> String {
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

    /// Like `present_access`, with a button that hands the question elsewhere,
    /// answered with `response` `3`. An empty `grant_label` or `open_label`
    /// hides that button.
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
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;
}

/// The reply a dialog's `(response, results)` stands for.
pub fn reply_from_wire(response: u32, results: Vec<(String, String)>) -> Reply {
    match response {
        0 => Reply::Granted(results),
        1 => Reply::Denied,
        3 => Reply::Open,
        _ => Reply::Ended,
    }
}

/// Whether `err` says the renderer lacks the method: an otto-islands from
/// before `PresentQuestion`.
fn unknown_method(err: &zbus::Error) -> bool {
    match err {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        ),
        zbus::Error::FDO(fdo) => matches!(
            fdo.as_ref(),
            zbus::fdo::Error::UnknownMethod(_) | zbus::fdo::Error::UnknownInterface(_)
        ),
        _ => false,
    }
}

fn wire_choices(choices: &[Choice]) -> Vec<WireChoice> {
    choices
        .iter()
        .map(|choice| {
            let options = choice
                .options
                .iter()
                .map(|(id, label)| (id.clone(), label.clone(), String::new()))
                .collect();
            (
                choice.id.clone(),
                choice.label.clone(),
                options,
                choice.default.clone(),
            )
        })
        .collect()
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
                    .inspect_err(
                        |err| tracing::warn!(%err, "no session bus; no dialogs can be shown"),
                    )
                    .ok()
            })
            .await
            .as_ref()
    }
}

impl Prompter for Islands {
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = Reply> + Send + '_>> {
        // The renderer returns no picks without a grant button to submit them.
        debug_assert!(
            prompt.choices.is_empty() || !prompt.grant.is_empty(),
            "choice groups need a grant label"
        );
        Box::pin(async move {
            let Some(connection) = self.connection().await else {
                return Reply::Unavailable;
            };
            let proxy = match DialogProxy::new(connection).await {
                Ok(proxy) => proxy,
                Err(err) => {
                    tracing::warn!(%err, "no dialog renderer");
                    return Reply::Unavailable;
                }
            };
            tracing::info!(title = %prompt.title, subtitle = %prompt.subtitle, "asking in a dialog");
            let answer = proxy
                .present_question(
                    "otto-agentsd",
                    &prompt.title,
                    &prompt.subtitle,
                    &prompt.body,
                    &prompt.icon,
                    &prompt.grant,
                    &prompt.deny,
                    &prompt.open,
                    true,
                    wire_choices(&prompt.choices),
                )
                .await;
            let err = match answer {
                Ok((response, results)) => {
                    tracing::info!(response, "dialog answered");
                    return reply_from_wire(response, results);
                }
                Err(err) => err,
            };
            // A renderer from before questions can still ask a plain yes or
            // no, only without the button that opens Ask.
            if !unknown_method(&err) || !prompt.choices.is_empty() || prompt.grant.is_empty() {
                tracing::warn!(%err, "could not show the dialog");
                return Reply::Unavailable;
            }
            let answer = proxy
                .present_access(
                    "otto-agentsd",
                    &prompt.title,
                    &prompt.subtitle,
                    &prompt.body,
                    &prompt.icon,
                    &prompt.grant,
                    &prompt.deny,
                    true,
                    Vec::new(),
                )
                .await;
            match answer {
                Ok((response, results)) => {
                    tracing::info!(response, "dialog answered");
                    match reply_from_wire(response, results) {
                        Reply::Open => Reply::Ended,
                        reply => reply,
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "could not show the dialog");
                    Reply::Unavailable
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
                icon: "system-run".into(),
                ..Prompt::default()
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
