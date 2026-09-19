//! Dialogs: asking the user whether an agent may use a tool, or how to answer
//! an agent's question.
//!
//! Otto's dialog renderer, otto-islands, serves `org.otto.Dialog1` (see
//! `specs/portal-access-dialog.md` in Otto): an island panel with grant,
//! deny and "open in Ask" buttons and, optionally, groups of choices. What no
//! client is watching the chat to answer is asked there: permission requests
//! from agents configured with `permissions = "ask"`, and agents' questions.
//! When a permission dialog cannot be shown the request is denied, so an agent
//! is never allowed something nobody saw; a question that cannot be shown
//! stays open in the chat.
//!
//! Dialogs are asked non-modal: an agent waiting is no reason to take the
//! keyboard. The user can carry on elsewhere; the panel shrinks into a circle
//! in the island row, still waiting, and opens again when clicked.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use agent_client_protocol::schema::v1::{
    PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, ToolCallContent, ToolKind,
};
use fluent_bundle::FluentArgs;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::agent::{Decision, Question, QuestionOption};
use crate::i18n;

/// The label of the button that hands a question to Ask.
pub fn open_in_ask_label() -> String {
    i18n::t("agents-permission-open-in-ask", None)
}

/// What a dialog says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    /// This service's own name for the dialog, so it can take it down again
    /// when the question is settled elsewhere — answered in Ask, or cancelled
    /// with the turn. Empty asks for a dialog nothing will withdraw.
    pub cookie: String,
    /// Who wants to do what: "Claude wants to run a command".
    pub title: String,
    /// The tool call as the agent describes it: the command, the file.
    pub subtitle: String,
    /// Where: the session's folder.
    pub body: String,
    /// The grant button, worded by the desktop after the kind of the option
    /// the agent offered — not after the agent's label for it. Empty hides it.
    pub grant: String,
    /// The deny button, likewise.
    pub deny: String,
    /// The button that hands the question to Ask instead. Empty hides it.
    pub open: String,
    /// The dialog's icon name.
    pub icon: String,
    /// The title is the asker's handle ("@claude"), not a headline: the
    /// renderer then makes the question itself the largest text.
    pub handle_title: bool,
    /// Groups of options, one picked from each, sent back with a grant.
    pub choices: Vec<Choice>,
}

/// A group of options in a [`Prompt`], of which the user picks one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    /// `(id, label)` pairs, in order. A label carries the option's
    /// description after a line break.
    pub options: Vec<(String, String)>,
    /// The option picked to begin with.
    pub default: String,
    /// Any number of options can be picked, not just one.
    pub multi: bool,
    /// The options the agent recommends: what a multi-select question starts
    /// with picked.
    pub recommended: Vec<String>,
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

    /// Takes down the dialog raised with `cookie`, if it is still up.
    ///
    /// Called when the question it was asking has been answered or cancelled
    /// somewhere else. A prompter with nothing to take down does nothing; the
    /// answer such a dialog would have collected is dropped either way, so
    /// this is about not leaving a panel in front of someone.
    fn withdraw(&self, _cookie: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
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

/// What a tool of `kind` is called: the AHP tool name the chat shows, and
/// the catalogue key of the phrase the dialog title uses for it.
fn tool_kind(kind: Option<ToolKind>) -> (&'static str, &'static str) {
    match kind {
        Some(ToolKind::Read) => ("read", "agents-permission-read"),
        Some(ToolKind::Edit) => ("edit", "agents-permission-edit"),
        Some(ToolKind::Delete) => ("delete", "agents-permission-delete"),
        Some(ToolKind::Move) => ("move", "agents-permission-move"),
        Some(ToolKind::Search) => ("search", "agents-permission-search"),
        Some(ToolKind::Execute) => ("execute", "agents-permission-execute"),
        Some(ToolKind::Fetch) => ("fetch", "agents-permission-fetch"),
        Some(ToolKind::SwitchMode) => ("switch-mode", "agents-permission-switch-mode"),
        _ => ("tool", "agents-permission-tool"),
    }
}

/// How the agent would like the request presented: `_meta.permission` on the
/// request, as the Claude and Codex adapters send it.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PermissionHints {
    /// A title of the agent's own, in place of the composed one.
    pub title: Option<String>,
    /// A line about the request, shown before the folder.
    pub description: Option<String>,
    /// The safe answer is no: the dialog and the chat start on the refusing
    /// option.
    pub default_to_no: bool,
}

impl PermissionHints {
    /// The hints on `request`, or none when it carries none it can read.
    pub fn of(request: &RequestPermissionRequest) -> Self {
        request
            .meta
            .as_ref()
            .and_then(|meta| meta.get("permission"))
            .and_then(|hints| serde_json::from_value(hints.clone()).ok())
            .unwrap_or_default()
    }
}

/// How much agent-written text one line of the dialog will show. The agent
/// chooses these words, and a wall of them — or a run of blank lines — would
/// push what the dialog composed itself out of sight.
const AGENT_TEXT_MAX_CHARS: usize = 200;

/// Agent-written `text` as a single line the dialog can find room for.
fn one_line(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if out.chars().count() >= AGENT_TEXT_MAX_CHARS {
            out.push('…');
            return out;
        }
        out.push_str(word);
    }
    match out.char_indices().nth(AGENT_TEXT_MAX_CHARS) {
        Some((end, _)) => {
            out.truncate(end);
            out.push('…');
            out
        }
        None => out,
    }
}

/// The dialog's own word for the narrowest granting or refusing option the
/// agent offered.
///
/// The word is the desktop's, never the agent's. The option's `kind` is what
/// says whether answering with it grants or refuses, so an agent free to
/// label it could put "Cancel" on the granting button and collect consent
/// from someone backing out.
fn option_word(options: &[PermissionOption], approve: bool) -> String {
    let Some(option) = default_option(options, Some(approve), false) else {
        return String::new();
    };
    let key = match option.kind {
        PermissionOptionKind::AllowAlways => "agents-permission-allow-always",
        PermissionOptionKind::AllowOnce => "agents-permission-allow",
        PermissionOptionKind::RejectAlways => "agents-permission-reject-always",
        _ => "agents-permission-reject",
    };
    i18n::t(key, None)
}

/// The plugin agent that is the desktop itself, by the `name` in its agent
/// file. `agents.toml` runs a session as it with `agent = "otto"`.
const OTTO: &str = "otto";

/// The icon dialogs from the plugin agent `plugin_agent` wear, or `None` for
/// the tool icon a dialog would otherwise carry.
///
/// A dialog from Otto's own helper is the desktop speaking to the person, not
/// a third-party agent asking for something, and it says so with Otto's face
/// rather than a tool glyph. The name is Files' icon: it is the Otto mark the
/// desktop installs into the icon theme, and the island resolves icons by
/// theme name.
pub fn agent_icon(plugin_agent: Option<&str>) -> Option<&'static str> {
    (plugin_agent == Some(OTTO)).then_some("otto-files")
}

/// The dialog for `request`, made by the agent called `agent` in `cwd`.
///
/// `icon` is the agent's own face, when it has one; without it the dialog
/// wears the icon for the kind of thing being asked about.
pub fn prompt_for(
    agent: &str,
    icon: Option<&str>,
    cwd: &Path,
    request: &RequestPermissionRequest,
) -> Prompt {
    let hints = PermissionHints::of(request);
    let fields = &request.tool_call.fields;
    // Composed here, never taken from the agent: the title is what names who
    // is asking, and an agent that could write it could pass its own request
    // off as the desktop's.
    let (_, phrase) = tool_kind(fields.kind);
    let mut args = FluentArgs::new();
    args.set("agent", agent);
    args.set("action", i18n::t(phrase, None));
    let title = i18n::t("agents-permission-title", Some(&args));

    let mut args = FluentArgs::new();
    args.set("folder", folder(cwd));
    let where_ = i18n::t("agents-permission-in-folder", Some(&args));
    // What the agent wrote about the request, above the folder and on one
    // line, so it explains the request without displacing it.
    let said: Vec<String> = hints
        .title
        .iter()
        .chain(hints.description.iter())
        .map(|line| one_line(line))
        .filter(|line| !line.is_empty())
        .collect();
    let body = if said.is_empty() {
        where_
    } else {
        format!("{}\n{where_}", said.join(" — "))
    };
    Prompt {
        title,
        subtitle: one_line(fields.title.as_deref().unwrap_or_default()),
        body,
        grant: option_word(&request.options, true),
        deny: option_word(&request.options, false),
        icon: icon.unwrap_or("system-run").into(),
        ..Prompt::default()
    }
}

/// `request` as a [`Question`] the host can put to the user, anywhere.
pub fn question_for(
    agent: &str,
    icon: Option<&str>,
    cwd: &Path,
    request: &RequestPermissionRequest,
) -> Question {
    let fields = &request.tool_call.fields;
    let (tool_name, _) = tool_kind(fields.kind);
    let default_to_no = PermissionHints::of(request).default_to_no;
    let mut tool_input = serde_json::Map::new();
    if let Some(location) = fields
        .locations
        .as_ref()
        .and_then(|locations| locations.first())
    {
        tool_input.insert(
            "path".into(),
            serde_json::Value::String(location.path.display().to_string()),
        );
        if let Some(line) = location.line {
            tool_input.insert("line".into(), line.into());
        }
    }
    if let Some(raw) = &fields.raw_input {
        tool_input.insert("rawInput".into(), raw.clone());
    }
    let edits = fields
        .content
        .iter()
        .flatten()
        .filter_map(|content| match content {
            ToolCallContent::Diff(diff) => Some(serde_json::json!({
                "path": diff.path.display().to_string(),
                "oldText": diff.old_text,
                "newText": diff.new_text,
            })),
            _ => None,
        })
        .collect();
    Question {
        tool_call_id: request.tool_call.tool_call_id.to_string(),
        tool_name: tool_name.to_owned(),
        prompt: prompt_for(agent, icon, cwd, request),
        default_option_id: default_option(&request.options, None, default_to_no)
            .map(|option| option.option_id.to_string()),
        options: request
            .options
            .iter()
            .map(|option| QuestionOption {
                id: option.option_id.to_string(),
                label: option.name.clone(),
                kind: option.kind,
            })
            .collect(),
        tool_input: (!tool_input.is_empty()).then_some(serde_json::Value::Object(tool_input)),
        edits,
    }
}

/// Answers `request` with `decision`: the option it names, when the agent
/// offered one with that id and the right intent, and otherwise the agent's
/// narrowest option for the decision. A cancelled turn gives no answer.
pub fn decide(
    request: &RequestPermissionRequest,
    decision: &Decision,
) -> RequestPermissionResponse {
    if *decision == Decision::Cancelled {
        return RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled);
    }
    let named = decision.option_id().and_then(|id| {
        request.options.iter().find(|option| {
            option.option_id.to_string() == id && allows(*option) == decision.approved()
        })
    });
    match named {
        Some(option) => RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(option.option_id.clone()),
        )),
        None => answer(request, decision.approved()),
    }
}

/// Answers `request` with the agent's narrowest option that allows it, or that
/// rejects it: "once" before "always". Cancelled when the agent offered none.
pub fn answer(request: &RequestPermissionRequest, allow: bool) -> RequestPermissionResponse {
    let outcome = match default_option(&request.options, Some(allow), false) {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
    };
    RequestPermissionResponse::new(outcome)
}

/// Something with a [`PermissionOptionKind`]: the agent's option as it came,
/// or as the host keeps it.
pub trait HasKind {
    fn kind(&self) -> PermissionOptionKind;
}

impl HasKind for PermissionOption {
    fn kind(&self) -> PermissionOptionKind {
        self.kind
    }
}

impl HasKind for QuestionOption {
    fn kind(&self) -> PermissionOptionKind {
        self.kind
    }
}

fn allows(option: &impl HasKind) -> bool {
    matches!(
        option.kind(),
        PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
    )
}

/// The one place an option is picked by kind. For an answer, `Some(approve)`,
/// it is the narrowest option of that intent: "once" before "always". For no
/// answer yet, `None`, it is the option to start on: the narrowest allow, or
/// the narrowest reject when the agent marked the request `default_to_no`;
/// when the agent offers none of that intent, the narrowest of the other.
pub fn default_option<O: HasKind>(
    options: &[O],
    answer: Option<bool>,
    default_to_no: bool,
) -> Option<&O> {
    let narrowest = |approve: bool| {
        let preferred = if approve {
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
        preferred
            .iter()
            .find_map(|kind| options.iter().find(|option| option.kind() == *kind))
    };
    match answer {
        Some(approve) => narrowest(approve),
        None => narrowest(!default_to_no).or_else(|| narrowest(default_to_no)),
    }
}

/// `cwd` for people: under the home folder as `~/…`.
pub fn folder(cwd: &Path) -> String {
    crate::xdg::tilde_from_env(cwd)
}

/// A question as `org.otto.Dialog1.PresentQuestions` takes it:
/// `(id, label, multi, [(option_id, option_label, option_icon)], default_option_ids)`.
type WireQuestion = (
    String,
    String,
    bool,
    Vec<(String, String, String)>,
    Vec<String>,
);

/// How the dialog should set what it is given. Only presentation: the words
/// for getting through the questions — answer, skip, next, back, the counter,
/// the multi-select hint — are the dialog's own, and localised there.
pub(crate) fn question_labels(prompt: &Prompt) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    if prompt.handle_title {
        labels.insert("title-style".to_owned(), "handle".to_owned());
    }
    // A body of several lines is a list of what is being asked: it reads down
    // the left edge, not centred.
    if prompt.body.contains('\n') {
        labels.insert("body-align".to_owned(), "start".to_owned());
    }
    labels
}

fn wire_questions(choices: &[Choice]) -> Vec<WireQuestion> {
    choices
        .iter()
        .map(|choice| {
            let options = choice
                .options
                .iter()
                .map(|(id, label)| (id.clone(), label.clone(), String::new()))
                .collect();
            let defaults = if choice.multi {
                choice.recommended.clone()
            } else {
                vec![choice.default.clone()]
            };
            (
                choice.id.clone(),
                choice.label.clone(),
                choice.multi,
                options,
                defaults,
            )
        })
        .collect()
}

#[zbus::proxy(
    interface = "org.otto.Dialog1",
    default_service = "org.otto.Island",
    default_path = "/org/otto/Dialog",
    gen_blocking = false
)]
trait Dialog {
    /// Presents a dialog and returns once the user answers:
    /// `(response, results)`, where `response` is `0` granted, `1` denied,
    /// `2` ended without an answer and `3` "answer in Ask". One question a
    /// page, single- or multi-select; `results` carries an entry per picked
    /// option. An empty `grant_label` or `open_label` hides that button.
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
    ) -> zbus::Result<(u32, Vec<(String, String)>)>;

    /// Takes down the dialog `cookie` names, answering it as ended.
    async fn withdraw(&self, app_id: &str, cookie: &str) -> zbus::Result<bool>;
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

/// Whether a dialog renderer is on the session bus to be asked. What
/// `otto-agents doctor` reports; nothing else needs to know in advance,
/// since a prompt that cannot be shown is already handled.
pub async fn renderer_present() -> bool {
    let Ok(connection) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&connection).await else {
        return false;
    };
    proxy
        .name_has_owner(
            "org.otto.Island"
                .try_into()
                .expect("a well-formed bus name"),
        )
        .await
        .unwrap_or(false)
}

impl Prompter for Islands {
    fn withdraw(&self, cookie: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let cookie = cookie.to_owned();
        Box::pin(async move {
            let Some(connection) = self.connection().await else {
                return;
            };
            let Ok(proxy) = DialogProxy::new(connection).await else {
                return;
            };
            match proxy.withdraw("otto-agents", &cookie).await {
                Ok(true) => tracing::info!(%cookie, "dialog withdrawn"),
                Ok(false) => tracing::debug!(%cookie, "no dialog left to withdraw"),
                Err(err) => tracing::debug!(%err, %cookie, "could not withdraw the dialog"),
            }
        })
    }

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
            // The subtitle is the command or path in question, so it goes
            // one level down: enough to debug with, not in every journal.
            tracing::info!(title = %prompt.title, "asking in a dialog");
            tracing::debug!(subtitle = %prompt.subtitle, "what it asked about");
            let answer = proxy
                .present_questions(
                    "otto-agents",
                    &prompt.cookie,
                    &prompt.title,
                    &prompt.subtitle,
                    &prompt.body,
                    &prompt.icon,
                    &prompt.grant,
                    &prompt.deny,
                    &prompt.open,
                    question_labels(&prompt),
                    false,
                    wire_questions(&prompt.choices),
                )
                .await;
            match answer {
                Ok((response, results)) => {
                    tracing::info!(response, "dialog answered");
                    reply_from_wire(response, results)
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
        Diff, ToolCallLocation, ToolCallUpdate, ToolCallUpdateFields,
    };
    use serde_json::json;

    fn request(options: Vec<PermissionOption>) -> RequestPermissionRequest {
        let mut fields = ToolCallUpdateFields::new();
        fields.kind = Some(ToolKind::Execute);
        fields.title = Some("cargo test".into());
        RequestPermissionRequest::new("s", ToolCallUpdate::new("call", fields), options)
    }

    /// A request with presentation hints, as the Claude and Codex adapters
    /// send them.
    fn hinted(
        options: Vec<PermissionOption>,
        hints: serde_json::Value,
    ) -> RequestPermissionRequest {
        let mut request = request(options);
        let mut meta = serde_json::Map::new();
        meta.insert("permission".into(), hints);
        request.meta = Some(meta);
        request
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

    /// Only the desktop's own helper wears Otto's face; every other agent
    /// leaves the dialog the icon for what is being asked about.
    #[test]
    fn a_dialog_from_otto_wears_ottos_face() {
        assert_eq!(agent_icon(Some("otto")), Some("otto-files"));
        assert_eq!(agent_icon(Some("claude")), None);
        assert_eq!(agent_icon(None), None);

        let asked = |icon| {
            prompt_for(
                "Otto",
                icon,
                Path::new("/srv/project"),
                &request(claude_options()),
            )
            .icon
        };
        assert_eq!(asked(agent_icon(Some("otto"))), "otto-files");
        assert_eq!(asked(agent_icon(Some("hermes"))), "system-run");
    }

    #[test]
    fn the_dialog_says_who_wants_to_do_what_and_where() {
        let prompt = prompt_for(
            "Claude",
            None,
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
        // A named option of the right intent is taken as it is; of the wrong
        // intent, the decision wins over the name.
        let named = decide(&request, &Decision::Approve(Some("allow_always".into())));
        assert_eq!(selected(&named).as_deref(), Some("allow_always"));
        let crossed = decide(&request, &Decision::Deny(Some("allow_always".into())));
        assert_eq!(selected(&crossed).as_deref(), Some("reject"));
    }

    #[test]
    fn the_option_to_start_on_is_the_narrowest_allow_unless_the_agent_says_no() {
        let options = claude_options();
        let id = |option: Option<&PermissionOption>| option.map(|o| o.option_id.to_string());
        assert_eq!(
            id(default_option(&options, None, false)).as_deref(),
            Some("allow")
        );
        assert_eq!(
            id(default_option(&options, None, true)).as_deref(),
            Some("reject")
        );
        // With nothing of the asked-for intent, the other intent stands in.
        let only_allow = vec![PermissionOption::new(
            "go",
            "Go",
            PermissionOptionKind::AllowAlways,
        )];
        assert_eq!(
            id(default_option(&only_allow, None, true)).as_deref(),
            Some("go")
        );
        assert_eq!(id(default_option(&only_allow, Some(false), false)), None);

        let question = question_for(
            "A",
            None,
            Path::new("/"),
            &hinted(
                claude_options(),
                json!({ "version": 1, "defaultToNo": true }),
            ),
        );
        assert_eq!(question.default_option_id.as_deref(), Some("reject"));
        assert_eq!(question.tool_name, "execute");
        assert_eq!(question.options[0].kind, PermissionOptionKind::AllowAlways);
    }

    #[test]
    fn the_agents_hints_shape_the_dialog() {
        let hints = json!({
            "version": 1,
            "title": "Run cargo test?",
            "description": "Runs the test suite",
            "defaultToNo": false,
        });
        let prompt = prompt_for(
            "Claude",
            None,
            Path::new("/srv/project"),
            &hinted(claude_options(), hints),
        );
        // The title stays the service's, so the dialog always says who is
        // asking; what the agent wrote joins the body.
        assert_eq!(prompt.title, "Claude wants to run a command");
        assert_eq!(
            prompt.body,
            "Run cargo test? — Runs the test suite\nin /srv/project"
        );

        // Hints the service cannot read are ignored, not fatal.
        let odd = prompt_for(
            "Claude",
            None,
            Path::new("/"),
            &hinted(claude_options(), json!(7)),
        );
        assert_eq!(odd.title, "Claude wants to run a command");
        assert_eq!(
            PermissionHints::of(&request(Vec::new())),
            PermissionHints::default()
        );
    }

    #[test]
    fn the_answer_buttons_are_worded_by_the_desktop() {
        // An agent that could word the buttons could put the refusing word on
        // the granting one and collect consent from someone backing out.
        let misleading = vec![
            PermissionOption::new("allow", "Cancel", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Run it", PermissionOptionKind::RejectOnce),
        ];
        let prompt = prompt_for("Claude", None, Path::new("/"), &request(misleading));
        assert_eq!(prompt.grant, "Allow");
        assert_eq!(prompt.deny, "Reject");
    }

    #[test]
    fn agent_text_cannot_crowd_out_what_the_dialog_says() {
        let hints = json!({
            "version": 1,
            "description": format!("padded{}", "\n".repeat(40)),
        });
        let prompt = prompt_for(
            "Claude",
            None,
            Path::new("/srv/project"),
            &hinted(claude_options(), hints),
        );
        // Newlines collapsed, so the folder line stays where the eye lands.
        assert_eq!(prompt.body, "padded\nin /srv/project");

        let long = json!({ "version": 1, "description": "word ".repeat(200) });
        let capped = prompt_for(
            "Claude",
            None,
            Path::new("/"),
            &hinted(claude_options(), long),
        );
        let said = capped.body.lines().next().unwrap();
        assert!(said.chars().count() <= AGENT_TEXT_MAX_CHARS + 1, "{said}");
        assert!(said.ends_with('…'));
    }

    #[test]
    fn a_cancelled_turn_gives_no_answer() {
        let response = decide(&request(claude_options()), &Decision::Cancelled);
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Cancelled
        ));
    }

    #[test]
    fn what_the_tool_would_touch_reaches_the_question() {
        let mut fields = ToolCallUpdateFields::new();
        fields.kind = Some(ToolKind::Edit);
        fields.title = Some("Edit src/main.rs".into());
        fields.locations = Some(vec![
            ToolCallLocation::new("/srv/project/src/main.rs").line(Some(3)),
            ToolCallLocation::new("/srv/project/README.md"),
        ]);
        fields.raw_input = Some(json!({ "file_path": "/srv/project/src/main.rs" }));
        fields.content = Some(vec![
            ToolCallContent::from("a note"),
            ToolCallContent::from(
                Diff::new("/srv/project/src/main.rs", "fn main() {}")
                    .old_text(Some("fn main()".into())),
            ),
        ]);
        let request = RequestPermissionRequest::new(
            "s",
            ToolCallUpdate::new("call", fields),
            claude_options(),
        );
        let question = question_for("Claude", None, Path::new("/srv/project"), &request);
        assert_eq!(question.tool_name, "edit");
        assert_eq!(question.prompt.title, "Claude wants to edit a file");
        assert_eq!(
            question.tool_input,
            Some(json!({
                "path": "/srv/project/src/main.rs",
                "line": 3,
                "rawInput": { "file_path": "/srv/project/src/main.rs" },
            }))
        );
        assert_eq!(
            question.edits,
            [json!({
                "path": "/srv/project/src/main.rs",
                "oldText": "fn main()",
                "newText": "fn main() {}",
            })]
        );

        // Nothing sent, nothing made up.
        let bare = question_for(
            "Claude",
            None,
            Path::new("/"),
            &self::request(claude_options()),
        );
        assert_eq!(bare.tool_input, None);
        assert!(bare.edits.is_empty());
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
        assert_eq!(prompt_for("A", None, Path::new("/"), &request).deny, "");
    }

    fn choice(id: &str, multi: bool) -> Choice {
        Choice {
            id: id.into(),
            label: format!("{id}?"),
            options: vec![
                ("one".into(), "One\nThe first".into()),
                ("two".into(), "Two".into()),
            ],
            default: "one".into(),
            multi,
            recommended: if multi {
                vec!["two".into()]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn questions_go_on_the_wire_with_their_kind_and_defaults() {
        let questions = wire_questions(&[choice("a", false), choice("b", true)]);
        assert_eq!(questions[0].0, "a");
        assert!(!questions[0].2);
        assert_eq!(questions[0].4, ["one"], "a single select starts on one");
        assert!(questions[1].2);
        assert_eq!(
            questions[1].4,
            ["two"],
            "a multi select starts on its recommended"
        );
        assert_eq!(
            questions[0].3[0],
            ("one".to_owned(), "One\nThe first".to_owned(), String::new())
        );
    }

    /// The words for answering, skipping and paging belong to the dialog,
    /// which localises them; this service sends only how to set what it sends.
    #[test]
    fn only_presentation_is_sent_with_the_questions() {
        let one = Prompt {
            choices: vec![choice("a", false)],
            handle_title: true,
            ..Prompt::default()
        };
        let labels = question_labels(&one);
        for word in ["next", "back", "page", "multi-hint", "answer", "skip"] {
            assert!(!labels.contains_key(word), "{word} is the dialog's own");
        }
        assert_eq!(
            labels.get("title-style").map(String::as_str),
            Some("handle")
        );
        assert!(
            !labels.contains_key("body-align"),
            "a one-line body stays centred"
        );

        let several = Prompt {
            choices: vec![choice("a", false), choice("b", true)],
            body: "First?\n\u{2022} One".into(),
            ..Prompt::default()
        };
        let labels = question_labels(&several);
        assert_eq!(labels.get("body-align").map(String::as_str), Some("start"));
        assert!(!labels.contains_key("title-style"));
    }
}
