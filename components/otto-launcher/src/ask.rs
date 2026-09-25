//! Asking an agent.
//!
//! In ask mode, what is typed is a request for an agent rather than a search.
//! Return hands it to otto-agents, the agent service, and the launcher becomes a
//! conversation: the log above the field shows each request and its answer,
//! and the field takes the next request, which queues behind whatever the
//! agent is doing.
//!
//! When the agent needs permission, its question opens in the chat as a tool
//! call waiting for confirmation, and the launcher offers the agent's answers
//! as rows under the field. Because the launcher is watching the chat, the
//! service leaves the question to it; closing the launcher hands the question
//! to a dialog instead.
//!
//! When the agent asks the person something — a choice, a value, a link to
//! open — the request sits in the chat as an input request, and the launcher
//! asks its questions one at a time, with the answers as rows and the field
//! taking typed ones. Drafts and answers are shared as they are given, so a
//! request answered somewhere else closes here too; see [`crate::input`].
//!
//! Files handed to the launcher go with the next request, as attachments that
//! point the agent at them. And instead of starting a session, the launcher
//! can open one that is already there — named on the command line, or picked
//! from the list in agents mode — to follow it and carry it on.
//!
//! The connection lives on a thread of its own, because the launcher's loop
//! has no async runtime. The thread connects as soon as the launcher opens —
//! so the list of agents is there by the time anyone looks — and reports back
//! over a channel, waking the loop through a socket the launcher polls.
//!
//! Every request is queued on the session's chat rather than started as a
//! turn. A turn can only start once the agent is up, which takes seconds, and
//! only when the previous turn is over; the service starts each queued request
//! as soon as it can. Closing the launcher after a hand-off therefore costs
//! nothing: the service already owns the requests, and the session carries on
//! without anyone watching.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use ahp::reducers::apply_action_to_chat;
use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::actions::{
    ChatInputAnswerChangedAction, ChatInputCompletedAction, ChatPendingMessageSetAction,
    ChatToolCallConfirmedAction, ChatTurnCancelledAction, StateAction,
};
use ahp_types::commands::ListSessionsResult;
use ahp_types::common::StringOrMarkdown;
use ahp_types::state::{
    AgentInfo, ChatInputAnswer, ChatInputResponseKind, ChatState, ChildCustomization,
    ConfirmationOptionKind, Customization, Message, MessageAttachment, MessageKind, MessageOrigin,
    MessageResourceAttachment, PendingMessageKind, ResponsePart, SessionStatus, SessionSummary,
    SnapshotState, ToolCallConfirmationReason, ToolCallState, ToolInput, TurnState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_agents_client::default_url;
use otto_agents_client::session::{self, SESSION_SCHEME};
use otto_agents_client::uri::{from_path as file_uri, to_path as path_from_uri};
pub use otto_kit::components::attachments::Attachment;
use serde_json::{json, Value};
use tokio::sync::mpsc as async_mpsc;

use crate::input::{self, Change, InputRequest, Outcome};
use crate::log::Style;
use crate::source::{Activity, Item, Origin};

/// The folder a session starts in when neither the agent nor anyone else
/// names one: a scratch folder of Ask's own, `$XDG_STATE_HOME/otto/ask`.
///
/// The folder is the reach the agent is given — everything under it is
/// something it can read, and a permission policy only covers what it thinks
/// to ask about — so the fallback is somewhere with nothing in it rather than
/// the home folder and everything in that. Desktop requests need no folder:
/// those go through the settings service and the skill tree. An agent that
/// should start somewhere else says so with `folder` in `agents.toml`, which
/// arrives as `otto.folders.<id>`.
fn default_folder() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".local/state")));
    let Some(folder) = state.map(|state| state.join("otto/ask")) else {
        return PathBuf::from("/");
    };
    // The service refuses a folder that is not there, so it is made here.
    if let Err(err) = std::fs::create_dir_all(&folder) {
        tracing::warn!(%err, folder = %folder.display(), "could not make the scratch folder");
        return home.unwrap_or_else(|| PathBuf::from("/"));
    }
    folder
}

/// Where each agent's sessions start, from the root state's `_meta`:
/// `otto.folders` maps provider ids to file URIs, as `agents.toml` set them.
fn folders_from_meta(meta: Option<&serde_json::Map<String, Value>>) -> HashMap<String, PathBuf> {
    meta.and_then(|meta| meta.get("otto")?.get("folders")?.as_object())
        .map(|folders| {
            folders
                .iter()
                .filter_map(|(provider, uri)| {
                    Some((provider.clone(), path_from_uri(uri.as_str()?)?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Connecting is one round trip to a local service. Past this, the service is
/// not answering, and saying so beats a launcher that looks like it is.
const TIMEOUT: Duration = Duration::from_secs(3);

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// What the connection thread reports.
#[derive(Debug)]
enum Update {
    /// The agents the service offers.
    Agents(Vec<AgentInfo>),
    /// The frosted material each coloured agent wears, by provider id, as the
    /// root state's `_meta` says under `otto.colours`.
    Colours(HashMap<String, String>),
    /// The agent of the followed session, by provider id.
    Provider(String),
    /// The sessions the service has, most recently changed first.
    Sessions(Vec<SessionSummary>),
    /// The service could not be reached.
    Unreachable(String),
    /// How to open the followed session in a terminal, as the service says.
    Terminal(Option<Terminal>),
    /// Whether the session's history is still on its way from the agent.
    Loading(bool),
    /// The agent's modes and the one it is in, as the service says in the
    /// session's `_meta`; `None` for an agent without any.
    Modes(Option<Modes>),
    /// The session's chat, as it stood when the launcher subscribed to it.
    Chat(Box<ChatState>),
    /// A change to that chat.
    Action(Box<StateAction>),
    /// The service has one more of the requests.
    HandedOff,
    /// A request, or the session behind them, failed.
    Failed(String),
    /// The service refused a change to the input request with this id: it
    /// was settled elsewhere first, or the answers did not do.
    InputRefused(String),
}

/// What the launcher asks of the connection thread.
enum Command {
    /// A request: the first creates the session with `provider`, unless one
    /// was opened, and the rest queue on it.
    Ask {
        prompt: String,
        provider: Option<String>,
        attachments: Vec<PathBuf>,
    },
    /// Open the session `session` names, a URI or the start of its id, in
    /// place of creating one.
    Resume {
        session: String,
    },
    Cancel {
        turn_id: String,
    },
    /// Stop the turn of the session `session`, a URI, from the list.
    Stop {
        session: String,
    },
    /// Remove the session `session`, a URI, for good.
    Delete {
        session: String,
    },
    /// Hand the session to its terminal: `session` is a URI from the list,
    /// or `None` for the open one.
    Release {
        session: Option<String>,
    },
    /// Switch the open session's agent to the mode `mode_id`, one of those
    /// it advertised.
    SetMode {
        mode_id: String,
    },
    /// An answer to the agent's question.
    Confirm {
        turn_id: String,
        tool_call_id: String,
        approved: bool,
        option_id: String,
    },
    /// An answer to one of the questions of an input request, draft or final.
    InputAnswer {
        request_id: String,
        question_id: String,
        answer: ChatInputAnswer,
    },
    /// An input request answered, declined or dismissed.
    InputComplete {
        request_id: String,
        response: ChatInputResponseKind,
        answers: Option<HashMap<String, ChatInputAnswer>>,
    },
}

/// What the agent is doing now, as the log's closing line says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// An existing session is being opened.
    Opening,
    /// The session is being created, or the agent is starting. Carries the
    /// agent's name, when it is known.
    Starting(Option<String>),
    /// The agent is reasoning. Transient: the reasoning itself is never shown.
    Thinking,
    /// The agent is answering, or doing something it does not narrate.
    Working,
    /// The agent asked something, and waits for an answer from the rows.
    Waiting,
    /// The session failed, and nothing more will come.
    Failed(String),
    /// The session is going to its terminal. `busy` while a turn has to
    /// finish first.
    HandingOver { busy: bool },
    /// The conversation is on its way from the agent.
    Loading,
}

impl Status {
    pub fn text(&self) -> String {
        match self {
            Status::Loading => otto_kit::t_owned!("launcher-ask-loading"),
            Status::HandingOver { busy: false } => otto_kit::t_owned!("launcher-ask-handing-over"),
            Status::HandingOver { busy: true } => {
                otto_kit::t_owned!("launcher-ask-handing-over-busy")
            }
            Status::Opening => otto_kit::t_owned!("launcher-ask-opening"),
            Status::Starting(Some(agent)) => {
                otto_kit::t_owned!("launcher-ask-starting", agent = agent.as_str())
            }
            Status::Starting(None) => otto_kit::t_owned!("launcher-ask-starting-agent"),
            Status::Thinking => otto_kit::t_owned!("launcher-ask-thinking"),
            Status::Working => otto_kit::t_owned!("launcher-ask-working"),
            Status::Waiting => otto_kit::t_owned!("launcher-ask-waiting"),
            Status::Failed(error) => {
                otto_kit::t_owned!("launcher-ask-failed", error = error.as_str())
            }
        }
    }
}

/// What the log says under a request, when there is something to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Waiting behind another request.
    Queued,
    Cancelled,
    Failed(String),
}

impl Note {
    pub fn text(&self) -> String {
        match self {
            Note::Queued => otto_kit::t_owned!("launcher-ask-queued"),
            Note::Cancelled => otto_kit::t_owned!("launcher-ask-cancelled"),
            Note::Failed(error) => {
                otto_kit::t_owned!("launcher-ask-failed", error = error.as_str())
            }
        }
    }
}

/// A tool call the agent made, once it was allowed or refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// What the tool call does, as the agent names it: the command, the file.
    pub tool: String,
    pub state: StepState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
    Denied,
}

impl Step {
    pub fn text(&self) -> String {
        let tool = self.tool.as_str();
        match self.state {
            StepState::Running => otto_kit::t_owned!("launcher-ask-step-running", tool = tool),
            StepState::Done => otto_kit::t_owned!("launcher-ask-step-done", tool = tool),
            StepState::Failed => otto_kit::t_owned!("launcher-ask-step-failed", tool = tool),
            StepState::Denied => otto_kit::t_owned!("launcher-ask-step-denied", tool = tool),
        }
    }
}

/// A question from the agent, waiting for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub turn_id: String,
    pub tool_call_id: String,
    /// Who wants to do what: "Claude wants to run a command".
    pub title: String,
    /// The tool call itself: the command, the file.
    pub detail: String,
    /// What the tool call would touch, as lines under the detail: the file,
    /// and the edit to it as `-`/`+` lines. Cut short past [`ACTION_LINES`].
    pub action: Vec<String>,
    /// The answer to start on, when the service says which.
    pub default_id: Option<String>,
    /// The agent's answers, in its order.
    pub choices: Vec<Choice>,
}

/// How many lines of what a tool call would touch the log shows before
/// cutting it short with an ellipsis.
pub const ACTION_LINES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    /// Whether choosing it lets the tool run.
    pub approve: bool,
}

impl Question {
    /// The choice to offer first: the one the service picked, and failing
    /// that the narrowest one that allows, since the agent lists "always"
    /// before "once".
    pub fn default_choice(&self) -> usize {
        self.default_id
            .as_deref()
            .and_then(|id| self.choices.iter().position(|choice| choice.id == id))
            .or_else(|| self.choices.iter().rposition(|choice| choice.approve))
            .unwrap_or(0)
    }
}

/// What a tool call would touch, as the log shows it: the file it names,
/// then each edit as its old lines with `-` and its new ones with `+`. The
/// service sends `tool_input` as JSON with `path` and `rawInput`, and `edits`
/// as a list of `{path, oldText, newText}`; anything shaped otherwise is
/// left out rather than guessed at. Kept to [`ACTION_LINES`] lines, the last
/// an ellipsis when there was more.
pub fn action_lines(tool_input: Option<&ToolInput>, edits: Option<&Value>) -> Vec<String> {
    let mut lines = Vec::new();
    let input = match tool_input {
        Some(ToolInput::Inline(text)) => serde_json::from_str::<Value>(text).ok(),
        _ => None,
    };
    let path_of = |value: &Value| {
        value
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
    };
    if let Some(path) = input.as_ref().and_then(path_of) {
        lines.push(path);
    }
    for edit in edits.and_then(Value::as_array).into_iter().flatten() {
        if let Some(path) = path_of(edit).filter(|path| !lines.contains(path)) {
            lines.push(path);
        }
        let text = |key: &str| {
            edit.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        lines.extend(text("oldText").into_iter().map(|line| format!("- {line}")));
        lines.extend(text("newText").into_iter().map(|line| format!("+ {line}")));
    }
    if lines.len() > ACTION_LINES {
        lines.truncate(ACTION_LINES - 1);
        lines.push("\u{2026}".to_owned());
    }
    lines
}

/// A skill an agent has, as the service published it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillRef {
    pub name: String,
    /// One sentence saying when the skill applies. Empty when the skill's
    /// frontmatter carries none.
    pub description: String,
}

/// A request as it was sent: what was typed, and what went with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub prompt: String,
    pub attachments: Vec<Attachment>,
}

/// A piece of what the agent answered.
///
/// An answer is mostly Markdown, and the pieces of it that arrive one after
/// another read as one document. A picture breaks it in two: what was said
/// before it, the picture, then the rest — so a diagram sits where the agent put
/// it rather than at the end.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// Markdown, as [`otto_md_kit`] reads it.
    Text(String),
    /// A picture the agent sent, as the file the service keeps it in.
    Image(Picture),
}

/// A picture in an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub path: PathBuf,
    /// What it is called, for a screen reader and for when it cannot be drawn:
    /// the file's name without the digest the service appends to it.
    pub label: String,
}

impl Picture {
    /// The picture at `uri`, when it is a local file the launcher can read.
    ///
    /// The service writes pictures under its own cache and names them
    /// `<label>-<digest>.<extension>`; the digest is how the same picture stays
    /// one file, and is not something to show anyone.
    fn at(uri: &str, content_type: Option<&str>) -> Option<Self> {
        let path = path_from_uri(uri)?;
        if !content_type.is_none_or(|kind| kind.starts_with("image/")) {
            return None;
        }
        let stem = path.file_stem()?.to_string_lossy();
        let label = match stem.rsplit_once('-') {
            Some((label, digest)) if is_digest(digest) && !label.is_empty() => {
                label.replace('-', " ")
            }
            _ => stem.into_owned(),
        };
        Some(Self { path, label })
    }
}

/// Whether `text` is the hexadecimal digest the service appends to a picture's
/// name, rather than part of what the picture is called.
fn is_digest(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// One request and what came of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub prompt: String,
    /// What went with the request.
    pub attachments: Vec<Attachment>,
    /// What the agent answered, in the order it said it.
    pub answer: Vec<Said>,
    /// The tool calls that were allowed or refused, in order.
    pub steps: Vec<Step>,
    /// The agent's question, while it waits for an answer.
    pub question: Option<Question>,
    /// What the agent asked the person in the turn, open or settled, in order.
    pub inputs: Vec<InputRequest>,
    pub note: Option<Note>,
}

/// The conversation so far, oldest first, and what the agent is doing now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub entries: Vec<Entry>,
    /// `None` when the agent has nothing in hand.
    pub status: Option<Status>,
}

impl Transcript {
    /// The question waiting for an answer, if the agent asked one.
    pub fn question(&self) -> Option<&Question> {
        self.entries
            .iter()
            .find_map(|entry| entry.question.as_ref())
    }

    /// The first input request still waiting for an answer.
    pub fn input(&self) -> Option<&InputRequest> {
        self.entries
            .iter()
            .flat_map(|entry| &entry.inputs)
            .find(|request| request.is_open())
    }
}

/// Input requests as answered from the launcher, ahead of the service saying
/// so.
#[derive(Default)]
struct Inputs {
    /// Answers given here, by request and question id, until the chat carries
    /// them.
    answers: HashMap<String, HashMap<String, ChatInputAnswer>>,
    /// Requests settled here, and how.
    completed: Vec<(String, ChatInputResponseKind)>,
    /// The question being asked, when it is not the first unanswered one:
    /// the person went back, or on past one answered elsewhere.
    cursor: Option<(String, usize)>,
    /// Why the last answer to a request was not taken.
    invalid: Option<(String, String)>,
}

/// What answering an input request did, for the launcher to follow up on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputAnswered {
    /// What was typed went into the answer, so the field can empty.
    pub took_text: bool,
    /// The request's link was opened.
    pub opened: bool,
}

/// The requests made so far.
struct Run {
    /// The agent's name, for the status line while it starts.
    agent: Option<String>,
    /// The agent's provider id: the one the request went to, or the one the
    /// service says an opened session belongs to.
    provider: Option<String>,
    /// Every request sent, in order. An opened session's earlier requests
    /// come first, once its chat arrives.
    sent: Vec<Request>,
    /// How many of them the service has confirmed.
    handed_off: usize,
    /// Questions answered from the launcher, hidden before the service says so.
    answered: Vec<String>,
    chat: Option<ChatState>,
    inputs: Inputs,
    failure: Option<String>,
    /// Whether the session was already there, opened rather than created.
    resumed: bool,
    /// How to open the session in a terminal, once the service says.
    terminal: Option<Terminal>,
    /// The history is on its way from the agent: the chat is not all there
    /// is to show yet.
    loading: bool,
    /// The agent's modes, once the service says; `None` for an agent
    /// without any, or before its session opened.
    modes: Option<Modes>,
}

/// The modes an agent can run in — its own permission and sandboxing presets,
/// such as Claude's "Accept edits" — and the one it is in, as otto-agents
/// publishes them in the session's `_meta` under `otto.modes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modes {
    /// The id of the current mode.
    pub current: String,
    /// The agent's list, in its order.
    pub available: Vec<ModeInfo>,
}

/// One of an agent's modes, as it describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
}

/// The agent and its mode, closing the ask log.
pub struct ModeLine {
    /// The agent's name as it is shown: an @ and what it is called.
    pub agent: String,
    /// The mode it is in, which the footer draws as a pill.
    pub mode: String,
    /// How to change the mode, when there is another to change to.
    pub hint: Option<String>,
}

impl Modes {
    /// What the current mode is called, falling back to its id when the
    /// agent's list does not have it.
    pub fn current_name(&self) -> &str {
        self.available
            .iter()
            .find(|mode| mode.id == self.current)
            .map_or(self.current.as_str(), |mode| mode.name.as_str())
    }

    /// The mode after the current one in the agent's list, round the end;
    /// `None` when there is nothing to switch to.
    pub fn next(&self) -> Option<&ModeInfo> {
        if self.available.len() < 2 {
            return None;
        }
        let at = self
            .available
            .iter()
            .position(|mode| mode.id == self.current)
            .map_or(0, |at| (at + 1) % self.available.len());
        self.available.get(at)
    }
}

/// The agent's modes, as the service says in the session's `_meta`.
fn modes_from_meta(meta: Option<&serde_json::Map<String, Value>>) -> Option<Modes> {
    let modes = meta?.get("otto")?.get("modes")?;
    let current = modes.get("current")?.as_str()?.to_owned();
    let available = modes
        .get("available")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mode| {
            let id = mode.get("id")?.as_str()?.to_owned();
            let name = mode
                .get("name")
                .and_then(Value::as_str)
                .map_or_else(|| id.clone(), str::to_owned);
            Some(ModeInfo { id, name })
        })
        .collect();
    Some(Modes { current, available })
}

impl Run {
    /// Drop what the launcher holds about input requests once the chat has
    /// caught up: answers the chat now carries, and anything about a request
    /// no longer open — settled here, in a dialog or on another client.
    fn forget_settled_inputs(&mut self) {
        let Some(chat) = self.chat.as_ref() else {
            return;
        };
        let open: Vec<InputRequest> = chat
            .active_turn
            .iter()
            .flat_map(|turn| input_requests(&turn.response_parts, true))
            .filter(InputRequest::is_open)
            .collect();
        let find = |id: &str| open.iter().find(|request| request.id == id);
        let inputs = &mut self.inputs;
        inputs.answers.retain(|id, answers| {
            let Some(request) = find(id) else {
                return false;
            };
            answers.retain(|question, answer| request.answers.get(question) != Some(answer));
            !answers.is_empty()
        });
        inputs.completed.retain(|(id, _)| find(id).is_some());
        if inputs
            .cursor
            .as_ref()
            .is_some_and(|(id, _)| find(id).is_none())
        {
            inputs.cursor = None;
        }
        if inputs
            .invalid
            .as_ref()
            .is_some_and(|(id, _)| find(id).is_none())
        {
            inputs.invalid = None;
        }
    }
}

/// A command that opens the session in a terminal, with the agent's own
/// interface: otto-agents publishes it in the session's `_meta`, under
/// `otto.terminal`, once the agent's id for the session is known and a
/// terminal is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    /// The agent's id for the session, when the service says it.
    pub session: Option<String>,
    /// What the window is called, standing in for `{title}` in the command.
    pub title: String,
}

impl Terminal {
    fn from_meta(meta: Option<&serde_json::Map<String, Value>>) -> Option<Self> {
        let terminal = meta?.get("otto")?.get("terminal")?;
        let command: Vec<String> = terminal
            .get("command")?
            .as_array()?
            .iter()
            .map(|arg| arg.as_str().map(str::to_owned))
            .collect::<Option<_>>()?;
        if command.is_empty() {
            return None;
        }
        let cwd = PathBuf::from(terminal.get("cwd")?.as_str()?);
        let session = terminal
            .get("session")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(Self {
            command,
            cwd,
            session,
            title: String::new(),
        })
    }

    /// Brings the terminal that has the session open to the front, when there
    /// is one and its window can be told apart. Says whether it did.
    pub fn focus(&self) -> bool {
        self.already_open()
            && self
                .session
                .as_deref()
                .is_some_and(crate::windows::focus_matching)
    }

    /// Whether a terminal already has the session open: some process names
    /// the agent's id for it on its command line and has a controlling
    /// terminal. The agent otto-agents runs names the id too, but talks over
    /// pipes and has none.
    pub fn already_open(&self) -> bool {
        let Some(session) = self.session.as_deref().filter(|id| !id.is_empty()) else {
            return false;
        };
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            let Ok(cmdline) = std::fs::read(path.join("cmdline")) else {
                return false;
            };
            if !String::from_utf8_lossy(&cmdline).contains(session) {
                return false;
            }
            // Field 7 of `stat` is the controlling terminal, 0 for none. The
            // fields are counted after the command's closing parenthesis, as
            // the command itself may hold spaces.
            std::fs::read_to_string(path.join("stat"))
                .ok()
                .and_then(|stat| {
                    let rest = stat.rsplit_once(')')?.1;
                    rest.split_whitespace().nth(4)?.parse::<i64>().ok()
                })
                .is_some_and(|tty| tty != 0)
        })
    }

    /// Start the terminal in a process group of its own, so it outlives the
    /// launcher.
    pub fn open(&self) -> std::io::Result<()> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let (program, args) = self
            .command
            .split_first()
            .ok_or_else(|| std::io::Error::other("the terminal command is empty"))?;
        Command::new(program)
            .args(args.iter().map(|arg| arg.replace("{title}", &self.title)))
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map(drop)
    }
}

/// The material each coloured agent wears, from the root state's `_meta`:
/// `otto.colours` maps provider ids to the names otto-kit's `Frosted` takes.
fn colours_from_meta(meta: Option<&serde_json::Map<String, Value>>) -> HashMap<String, String> {
    meta.and_then(|meta| meta.get("otto")?.get("colours")?.as_object())
        .map(|colours| {
            colours
                .iter()
                .filter_map(|(provider, name)| Some((provider.clone(), name.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the service says, in the session's `_meta`, that the history is
/// still on its way from the agent.
fn loading_from_meta(meta: Option<&serde_json::Map<String, Value>>) -> bool {
    meta.and_then(|meta| meta.get("otto")?.get("loading")?.as_bool())
        .unwrap_or(false)
}

/// What a session's terminal window is called: the feature, the agent, and
/// what the session is about, cut to a length a dock label can show.
fn window_title(agent: Option<&str>, title: &str) -> String {
    const MOST: usize = 60;
    let mut title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > MOST {
        title = title.chars().take(MOST - 1).collect::<String>() + "…";
    }
    match (agent, title.is_empty()) {
        (Some(agent), false) => otto_kit::t_owned!(
            "launcher-ask-window-title",
            agent = agent,
            title = title.as_str()
        ),
        (Some(agent), true) => otto_kit::t_owned!("launcher-ask-window-title-agent", agent = agent),
        (None, false) => {
            otto_kit::t_owned!("launcher-ask-window-title-plain", title = title.as_str())
        }
        (None, true) => otto_kit::t_owned!("launcher-ask-window-title-bare"),
    }
}

pub struct Ask {
    commands: async_mpsc::UnboundedSender<Command>,
    updates: mpsc::Receiver<Update>,
    wake: UnixStream,
    agents: Vec<AgentInfo>,
    /// The material each coloured agent wears, by provider id.
    colours: HashMap<String, String>,
    /// The service's sessions, as they stood when the launcher connected.
    sessions: Vec<SessionSummary>,
    sessions_listed: bool,
    /// The files that go with the next request…
    attachments: Vec<PathBuf>,
    /// …but for those struck out, which stay listed and stay behind.
    struck: Vec<bool>,
    /// What otto-stash has stashed, after the files above: it goes with
    /// the next request too, but otto-stash keeps it, and changes to it go
    /// there. Each with its file and whether it is struck out.
    stashed: Vec<(PathBuf, Attachment, bool)>,
    /// The open session is going to its terminal; `Some(true)` while a turn
    /// has to finish first.
    handing_over: Option<bool>,
    unreachable: Option<String>,
    run: Option<Run>,
}

impl Ask {
    /// Connect to otto-agents in the background, with sessions created in
    /// [`default_folder`].
    pub fn open() -> Self {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| default_url());
        Self::connect(url, default_folder())
    }

    pub fn connect(url: String, folder: PathBuf) -> Self {
        let (commands, command_rx) = async_mpsc::unbounded_channel();
        let (update_tx, updates) = mpsc::channel();
        let (wake, wake_tx) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => panic!("cannot create the launcher's wake-up socket: {err}"),
        };
        let _ = wake.set_nonblocking(true);
        let _ = wake_tx.set_nonblocking(true);
        let reporter = Reporter {
            updates: update_tx,
            wake: wake_tx,
        };

        std::thread::Builder::new()
            .name("otto-agents".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        reporter.send(Update::Unreachable(err.to_string()));
                        return;
                    }
                };
                runtime.block_on(serve(&url, &folder, command_rx, &reporter));
            })
            .expect("cannot start the otto-agents connection thread");

        Self {
            commands,
            updates,
            wake,
            agents: Vec::new(),
            colours: HashMap::new(),
            sessions: Vec::new(),
            sessions_listed: false,
            attachments: Vec::new(),
            struck: Vec::new(),
            stashed: Vec::new(),
            handing_over: None,
            unreachable: None,
            run: None,
        }
    }

    /// Attach `files` to the next request.
    pub fn attach(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        self.attach_struck(files.into_iter().map(|file| (file, false)));
    }

    /// Attach `files` to the next request, each struck out or not.
    pub fn attach_struck(&mut self, files: impl IntoIterator<Item = (PathBuf, bool)>) {
        for (file, struck) in files {
            self.attachments
                .push(std::path::absolute(&file).unwrap_or(file));
            self.struck.push(struck);
        }
    }

    /// What otto-stash has stashed now, each file with whether it is
    /// struck out.
    pub fn set_stashed(&mut self, items: &[(PathBuf, bool)]) {
        // Read once per change rather than on every layout: a text item's
        // file is read to show it.
        let known: HashMap<&Path, &Attachment> = self
            .stashed
            .iter()
            .map(|(file, attachment, _)| (file.as_path(), attachment))
            .collect();
        let stashed = items
            .iter()
            .map(|(file, struck)| {
                let attachment = known
                    .get(file.as_path())
                    .map_or_else(|| Attachment::for_file(file), |known| (*known).clone());
                (file.clone(), attachment, *struck)
            })
            .collect();
        self.stashed = stashed;
    }

    /// What goes with the next request, each with whether it is struck out:
    /// the files attached, then what is stashed.
    pub fn pending(&self) -> Vec<(Attachment, bool)> {
        self.attachments
            .iter()
            .zip(&self.struck)
            .map(|(file, struck)| (Attachment::for_file(file), *struck))
            .chain(
                self.stashed
                    .iter()
                    .map(|(_, attachment, struck)| (attachment.clone(), *struck)),
            )
            .collect()
    }

    /// Whether anything goes with the next request: attached or stashed,
    /// and not struck out.
    pub fn has_attachments(&self) -> bool {
        self.struck.iter().any(|struck| !struck)
            || self.stashed.iter().any(|(_, _, struck)| !struck)
    }

    /// Take the attachment at `index` in [`Self::pending`] off the next
    /// request. A stashed one is otto-stash's to take out: its index in
    /// the stash is returned for that.
    pub fn remove_attachment(&mut self, index: usize) -> Option<usize> {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            self.struck.remove(index);
            return None;
        }
        let stashed = index - self.attachments.len();
        (stashed < self.stashed.len()).then(|| {
            self.stashed.remove(stashed);
            stashed
        })
    }

    /// Strike the attachment at `index` in [`Self::pending`] out, or bring
    /// it back. A stashed one is struck out in otto-stash too: its index
    /// in the stash is returned for that.
    pub fn toggle_attachment(&mut self, index: usize) -> Option<usize> {
        if let Some(struck) = self.struck.get_mut(index) {
            *struck = !*struck;
            return None;
        }
        let stashed = index - self.attachments.len();
        let (_, _, struck) = self.stashed.get_mut(stashed)?;
        *struck = !*struck;
        Some(stashed)
    }

    /// How to open the session in a terminal, once there is a session and the
    /// service has said.
    pub fn terminal(&self) -> Option<Terminal> {
        let run = self.run.as_ref()?;
        let mut terminal = run.terminal.clone()?;
        let title = run
            .chat
            .as_ref()
            .map(requests)
            .and_then(|requests| requests.first().map(|request| request.prompt.clone()))
            .or_else(|| run.sent.first().map(|request| request.prompt.clone()))
            .unwrap_or_default();
        terminal.title = window_title(run.agent.as_deref(), &title);
        Some(terminal)
    }

    /// How to open the session at `index` in the list in a terminal, from the
    /// `_meta` the catalogue carries. `None` while the service has not said —
    /// the agent's id for a session it has just started, say.
    pub fn terminal_at(&self, index: usize) -> Option<Terminal> {
        let session = self.sessions.get(index)?;
        let mut terminal = Terminal::from_meta(session.meta.as_ref())?;
        terminal.title = window_title(self.agent_name(&session.provider), &session.title);
        Some(terminal)
    }

    /// What the agent whose provider id is `provider` is called.
    fn agent_name(&self, provider: &str) -> Option<&str> {
        self.agents
            .iter()
            .find(|agent| agent.provider == provider)
            .map(|agent| agent.display_name.as_str())
    }

    /// Open the session `session` names — its URI, its id, or the start of its
    /// id — to follow it and carry it on, instead of starting one. Does nothing
    /// once a request is made.
    pub fn resume(&mut self, session: &str) {
        if self.run.is_some() {
            return;
        }
        self.run = Some(Run {
            agent: None,
            provider: None,
            sent: Vec::new(),
            handed_off: 0,
            answered: Vec::new(),
            chat: None,
            inputs: Inputs::default(),
            failure: self.unreachable.clone(),
            resumed: true,
            terminal: None,
            loading: false,
            modes: None,
        });
        let _ = self.commands.send(Command::Resume {
            session: session.to_string(),
        });
    }

    /// Open the session at `index` in the list. Returns whether there was one.
    pub fn resume_at(&mut self, index: usize) -> bool {
        let Some(session) = self.sessions.get(index).map(|s| s.resource.clone()) else {
            return false;
        };
        self.resume(&session);
        true
    }

    /// The URI of the session at `index` in the list.
    pub fn session_at(&self, index: usize) -> Option<&str> {
        self.sessions.get(index).map(|s| s.resource.as_str())
    }

    /// Whether the service has said which sessions it has.
    pub fn sessions_listed(&self) -> bool {
        self.sessions_listed
    }

    /// The sessions whose titles contain `query`, as rows.
    pub fn session_rows(&self, source: usize, query: &str) -> Vec<Item> {
        let query = query.trim().to_lowercase();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                query.is_empty() || session.title.to_lowercase().contains(&query)
            })
            .map(|(index, session)| Item {
                title: if session.title.is_empty() {
                    otto_kit::t_owned!("launcher-agents-untitled")
                } else {
                    session.title.clone()
                },
                subtitle: Some(session_subtitle(
                    session,
                    self.agent_name(&session.provider),
                    home.as_deref(),
                )),
                icon: None,
                activity: Some(session_activity(session)),
                search_terms: Vec::new(),
                origin: Origin { source, index },
            })
            .collect()
    }

    /// The socket that becomes readable when there is news.
    pub fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Take in whatever the connection thread has reported. Never blocks.
    /// Returns whether anything changed.
    pub fn pump(&mut self) -> bool {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            changed = true;
            self.apply(update);
        }
        changed
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Agents(agents) => self.agents = agents,
            Update::Colours(colours) => self.colours = colours,
            Update::Provider(provider) => {
                if let Some(run) = self.run.as_mut() {
                    run.provider = Some(provider);
                }
            }
            Update::Terminal(terminal) => {
                if let Some(run) = self.run.as_mut() {
                    run.terminal = terminal;
                }
            }
            Update::Loading(loading) => {
                if let Some(run) = self.run.as_mut() {
                    run.loading = loading;
                }
            }
            Update::Modes(modes) => {
                if let Some(run) = self.run.as_mut() {
                    run.modes = modes;
                }
            }
            Update::Sessions(sessions) => {
                self.sessions = sessions;
                self.sessions_listed = true;
            }
            Update::Unreachable(error) => {
                tracing::warn!(%error, "otto-agents is unreachable");
                match self.run.as_mut() {
                    Some(run) => run.failure = Some(error),
                    None => self.unreachable = Some(error),
                }
            }
            Update::Chat(chat) => {
                if let Some(run) = self.run.as_mut() {
                    if run.resumed && run.chat.is_none() {
                        // The requests made before the launcher opened the
                        // session are the service's already.
                        let earlier = requests(&chat);
                        run.handed_off += earlier.len();
                        run.sent.splice(0..0, earlier);
                    }
                    run.chat = Some(*chat);
                    run.forget_settled_inputs();
                }
            }
            Update::Action(action) => {
                if let Some(run) = self.run.as_mut() {
                    if let Some(chat) = run.chat.as_mut() {
                        apply_action_to_chat(chat, &action);
                        run.forget_settled_inputs();
                    }
                }
            }
            Update::InputRefused(request_id) => {
                // Whatever the chat says about the request is the truth: shown
                // settled if it was, asked again if it was not.
                if let Some(run) = self.run.as_mut() {
                    run.inputs.answers.remove(&request_id);
                    run.inputs.completed.retain(|(id, _)| *id != request_id);
                }
            }
            Update::HandedOff => {
                if let Some(run) = self.run.as_mut() {
                    run.handed_off += 1;
                }
                tracing::info!("request handed to otto-agents");
            }
            Update::Failed(error) => {
                tracing::error!(%error, "the request failed");
                if let Some(run) = self.run.as_mut() {
                    run.failure = Some(error);
                }
            }
        }
    }

    /// The agents to pick from, as rows. None when there is only one agent to
    /// ask, because a list of one is not a choice.
    pub fn agent_rows(&self, source: usize) -> Vec<Item> {
        if self.agents.len() < 2 {
            return Vec::new();
        }
        self.agents
            .iter()
            .enumerate()
            .map(|(index, agent)| Item {
                title: agent.display_name.clone(),
                subtitle: (!agent.description.is_empty()).then(|| agent.description.clone()),
                icon: None,
                activity: None,
                search_terms: Vec::new(),
                origin: Origin { source, index },
            })
            .collect()
    }

    /// What the agent in row `index` of [`Ask::agent_rows`] is called.
    pub fn agent_display_name(&self, index: usize) -> Option<&str> {
        self.agents
            .get(index)
            .map(|agent| agent.display_name.as_str())
    }

    /// The skills the agent has, as the service published them: the desktop's
    /// own, and any the person installed. `agent` is the row chosen in the
    /// agent list, if any; before a session exists the completion follows
    /// whichever agent the request would go to, because a skill one agent has
    /// and another does not should not be offered for the wrong one.
    pub fn skills(&self, agent: Option<usize>) -> Vec<SkillRef> {
        let chosen = match self.run.as_ref() {
            // Once a session is running, the agent is settled: its name is
            // what `send` recorded.
            Some(run) => run
                .agent
                .as_ref()
                .and_then(|name| self.agents.iter().find(|agent| &agent.display_name == name))
                .or_else(|| self.agents.first()),
            None => agent
                .and_then(|index| self.agents.get(index))
                .or_else(|| self.agents.first()),
        };
        let Some(chosen) = chosen else {
            return Vec::new();
        };
        chosen
            .customizations
            .iter()
            .flatten()
            .filter_map(|customization| match customization {
                Customization::Plugin(plugin) => plugin.children.as_ref(),
                Customization::Directory(directory) => directory.children.as_ref(),
                _ => None,
            })
            .flatten()
            .filter_map(|child| match child {
                ChildCustomization::Skill(skill) if skill.enabled != Some(false) => {
                    Some(SkillRef {
                        name: skill.name.clone(),
                        description: skill.description.clone().unwrap_or_default(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// What Tab would add to `text`: the rest of the skill name being typed.
    ///
    /// A request naming a skill starts with `/`, which is the only thing that
    /// turns the completion on — otherwise every sentence starting with a word
    /// a skill also starts with would sprout grey text mid-thought. The match
    /// is on the first word alone, and only while the caret is at the end of
    /// it, so `/conf` completes and `/configure-otto the dock` does not.
    pub fn completion(&self, text: &str, agent: Option<usize>) -> Option<String> {
        let typed = text.strip_prefix('/')?;
        if typed.is_empty() || typed.contains(char::is_whitespace) {
            return None;
        }
        let typed = typed.to_lowercase();
        let mut names: Vec<String> = self
            .skills(agent)
            .into_iter()
            .map(|skill| skill.name)
            .filter(|name| name.to_lowercase().starts_with(&typed) && name.len() > typed.len())
            .collect();
        // One skill's name may be another's prefix. Completing to the shortest
        // is the choice that is never wrong to accept: the longer one is still
        // a keystroke away.
        names.sort_by_key(String::len);
        Some(names.into_iter().next()?[typed.len()..].to_owned())
    }

    /// The answers to the agent's question, as rows, when it asked one.
    pub fn question_rows(&self, source: usize) -> Vec<Item> {
        let Some(question) = self.question() else {
            return Vec::new();
        };
        question
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| Item {
                title: choice.label.clone(),
                subtitle: None,
                icon: None,
                activity: None,
                search_terms: Vec::new(),
                origin: Origin { source, index },
            })
            .collect()
    }

    /// The agent's question waiting for an answer from the launcher.
    pub fn question(&self) -> Option<Question> {
        self.transcript()?.question().cloned()
    }

    /// Answer the agent's question with its choice at `index`. Returns
    /// whether there was a question to answer.
    pub fn answer(&mut self, index: usize) -> bool {
        let Some(question) = self.question() else {
            return false;
        };
        let Some(choice) = question.choices.get(index) else {
            return false;
        };
        if let Some(run) = self.run.as_mut() {
            run.answered.push(question.tool_call_id.clone());
        }
        self.commands
            .send(Command::Confirm {
                turn_id: question.turn_id,
                tool_call_id: question.tool_call_id,
                approved: choice.approve,
                option_id: choice.id.clone(),
            })
            .is_ok()
    }

    /// The input request waiting for an answer from the launcher, with the
    /// answers given here over the chat's, and the question to ask now —
    /// `None` once no question is left and the request is ready to send.
    pub fn input(&self) -> Option<(InputRequest, Option<usize>)> {
        let request = self.transcript()?.input()?.clone();
        let current = self.input_current(&request);
        Some((request, current))
    }

    fn input_current(&self, request: &InputRequest) -> Option<usize> {
        match self.run.as_ref().and_then(|run| run.inputs.cursor.as_ref()) {
            Some((id, index)) if *id == request.id && *index < request.fields.len() => Some(*index),
            _ => request.first_unsettled(),
        }
    }

    /// Names the question being asked, so the launcher can tell when it moves
    /// on to another.
    pub fn input_key(&self) -> Option<String> {
        self.input()
            .map(|(request, current)| format!("input:{}:{current:?}", request.id))
    }

    /// The answers to the question being asked, as rows.
    pub fn input_rows(&self, source: usize) -> Vec<Item> {
        let Some((request, current)) = self.input() else {
            return Vec::new();
        };
        request
            .rows(current)
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let (title, subtitle) = request.row_text(row, current);
                Item {
                    title,
                    subtitle,
                    icon: None,
                    activity: None,
                    search_terms: Vec::new(),
                    origin: Origin { source, index },
                }
            })
            .collect()
    }

    /// The row to start from on the question being asked.
    pub fn input_default_row(&self) -> usize {
        self.input()
            .map_or(0, |(request, current)| request.default_row(current))
    }

    /// What the field starts with on the question being asked.
    pub fn input_prefill(&self) -> Option<String> {
        let (request, current) = self.input()?;
        request.prefill(current)
    }

    /// What the empty field says on the question being asked.
    pub fn input_placeholder(&self) -> Option<&'static str> {
        let (request, current) = self.input()?;
        request.placeholder(current)
    }

    /// Whether typing answers the question being asked.
    pub fn input_takes_text(&self) -> bool {
        self.input().is_some_and(|(request, current)| {
            current
                .and_then(|index| request.fields.get(index))
                .is_some_and(input::Field::takes_text)
        })
    }

    /// What the log says about `request`, one of the transcript's.
    pub fn input_lines(&self, request: &InputRequest) -> Vec<(String, Style)> {
        let invalid = self
            .run
            .as_ref()
            .and_then(|run| run.inputs.invalid.as_ref())
            .filter(|(id, _)| *id == request.id)
            .map(|(_, text)| text.as_str());
        request.lines(self.input_current(request), invalid)
    }

    /// Choose the row at `index` on the question being asked, with `typed` in
    /// the field.
    pub fn choose_input(&mut self, index: usize, typed: &str) -> InputAnswered {
        let Some((request, current)) = self.input() else {
            return InputAnswered::default();
        };
        let Some(row) = request.rows(current).get(index).copied() else {
            return InputAnswered::default();
        };
        let step = request.choose(row, current, typed);
        self.take_input_step(&request, step)
    }

    /// Answer the question being asked with what is typed.
    pub fn answer_input(&mut self, typed: &str) -> InputAnswered {
        let Some((request, current)) = self.input() else {
            return InputAnswered::default();
        };
        let step = request.answer_text(current, typed);
        self.take_input_step(&request, step)
    }

    /// Go back to the question before the one being asked. Returns whether
    /// there was one.
    pub fn input_back(&mut self) -> bool {
        let Some((request, current)) = self.input() else {
            return false;
        };
        let previous = match current {
            Some(0) => return false,
            Some(index) => index - 1,
            None if request.fields.is_empty() => return false,
            None => request.fields.len() - 1,
        };
        if let Some(run) = self.run.as_mut() {
            run.inputs.cursor = Some((request.id.clone(), previous));
            run.inputs.invalid = None;
        }
        true
    }

    fn take_input_step(&mut self, request: &InputRequest, step: input::Step) -> InputAnswered {
        let Some(run) = self.run.as_mut() else {
            return InputAnswered::default();
        };
        let mut answered = InputAnswered {
            took_text: step.took_text,
            opened: false,
        };
        if let Some(url) = step.open.as_deref() {
            match input::open_link(url) {
                Ok(()) => answered.opened = true,
                Err(err) => tracing::warn!(%err, "could not open the link"),
            }
        }
        run.inputs.invalid = step
            .invalid
            .map(|invalid| (request.id.clone(), invalid.text()));
        run.inputs.cursor = step.next.map(|next| (request.id.clone(), next));
        for change in step.changes {
            let command = match change {
                Change::Answer {
                    question_id,
                    answer,
                } => {
                    run.inputs
                        .answers
                        .entry(request.id.clone())
                        .or_default()
                        .insert(question_id.clone(), answer.clone());
                    Command::InputAnswer {
                        request_id: request.id.clone(),
                        question_id,
                        answer,
                    }
                }
                Change::Complete { response, answers } => {
                    run.inputs.completed.push((request.id.clone(), response));
                    Command::InputComplete {
                        request_id: request.id.clone(),
                        response,
                        answers,
                    }
                }
            };
            let _ = self.commands.send(command);
        }
        answered
    }

    /// Why the service cannot be asked anything, before a request is made.
    pub fn unreachable(&self) -> Option<&str> {
        self.unreachable.as_deref()
    }

    /// Hand `prompt` to the agent, with the files attached and stashed so
    /// far. The first request starts a session with the agent at `agent` in
    /// the list, or the service's default agent; later ones queue on the same
    /// session, and `agent` is ignored.
    ///
    /// Returns whether what was stashed went with it, so the stash can
    /// be told it is over.
    pub fn send(&mut self, prompt: &str, agent: Option<usize>) -> bool {
        let struck = std::mem::take(&mut self.struck);
        let stashed = std::mem::take(&mut self.stashed);
        let took_stashed = !stashed.is_empty();
        let attachments: Vec<PathBuf> = std::mem::take(&mut self.attachments)
            .into_iter()
            .zip(struck)
            .chain(stashed.into_iter().map(|(file, _, struck)| (file, struck)))
            .filter(|(_, struck)| !struck)
            .map(|(file, _)| file)
            .collect();
        let request = Request {
            prompt: prompt.to_string(),
            attachments: attachments
                .iter()
                .map(|file| Attachment::for_file(file))
                .collect(),
        };
        let provider = match self.run.as_mut() {
            Some(run) => {
                run.sent.push(request);
                None
            }
            None => {
                let chosen = agent
                    .and_then(|index| self.agents.get(index))
                    .or_else(|| self.agents.first());
                self.run = Some(Run {
                    agent: chosen.map(|agent| agent.display_name.clone()),
                    provider: chosen.map(|agent| agent.provider.clone()),
                    sent: vec![request],
                    handed_off: 0,
                    answered: Vec::new(),
                    chat: None,
                    inputs: Inputs::default(),
                    // An unreachable service fails the request at once, rather
                    // than leaving it to look as if it is starting.
                    failure: self.unreachable.clone(),
                    resumed: false,
                    terminal: None,
                    loading: false,
                    modes: None,
                });
                chosen.map(|agent| agent.provider.clone())
            }
        };
        let _ = self.commands.send(Command::Ask {
            prompt: prompt.to_string(),
            provider,
            attachments,
        });
        took_stashed
    }

    /// The name of the frosted material the card wears: the running session's
    /// agent's, once there is one, and before that the agent the request
    /// would go to — the row at `agent` in the agent list, or the default.
    /// `None` for an agent without a colour, which is the plain material.
    pub fn colour(&self, agent: Option<usize>) -> Option<&str> {
        let provider = match self.run.as_ref() {
            Some(run) => run.provider.as_deref()?,
            None => agent
                .and_then(|index| self.agents.get(index))
                .or_else(|| self.agents.first())?
                .provider
                .as_str(),
        };
        self.colours.get(provider).map(String::as_str)
    }

    /// Whether a request has been made, and the launcher is showing the log.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The line under the status naming the agent and the mode it is in,
    /// such as "Claude · Accept edits", with how to switch when the agent has
    /// more than one mode. `None` until the session's agent has said.
    /// The agent and its mode, as the log's footer draws them: the agent's
    /// name, the mode on its own, and how to change it when it can be
    /// changed.
    pub fn mode_line(&self) -> Option<ModeLine> {
        let run = self.run.as_ref()?;
        let modes = run.modes.as_ref()?;
        let agent = run
            .agent
            .clone()
            .or_else(|| {
                run.provider
                    .as_deref()
                    .map(|provider| self.agent_name(provider).unwrap_or(provider).to_owned())
            })
            .unwrap_or_default();
        Some(ModeLine {
            agent: otto_kit::t_owned!("launcher-ask-agent", agent = agent),
            mode: modes.current_name().to_string(),
            hint: modes
                .next()
                .is_some()
                .then(|| otto_kit::t_owned!("launcher-ask-mode-hint")),
        })
    }

    /// Switch the agent to the next of its modes. The line changes once the
    /// agent has, as the service reports it. Returns whether there was a
    /// mode to switch to.
    pub fn cycle_mode(&mut self) -> bool {
        let Some(next) = self
            .run
            .as_ref()
            .and_then(|run| run.modes.as_ref())
            .and_then(Modes::next)
        else {
            return false;
        };
        self.commands
            .send(Command::SetMode {
                mode_id: next.id.clone(),
            })
            .is_ok()
    }

    /// Whether a request is still on its way to the service, and closing now
    /// would lose it.
    pub fn handing_off(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.handed_off < run.sent.len() && run.failure.is_none())
    }

    /// Stop the agent's turn. Returns whether there was one to stop.
    pub fn cancel(&mut self) -> bool {
        let Some(turn) = self
            .run
            .as_ref()
            .and_then(|run| run.chat.as_ref())
            .and_then(|chat| chat.active_turn.as_ref())
        else {
            return false;
        };
        self.commands
            .send(Command::Cancel {
                turn_id: turn.id.clone(),
            })
            .is_ok()
    }

    /// Stop the turn of the session at `index` in the list. Returns whether it
    /// had one going, as far as the list knows.
    /// Tells the service the session is going to its terminal: the one at
    /// `index` in the list, or the open one. The service stops its own agent
    /// once it is idle, so the terminal's is the only one writing.
    pub fn release(&mut self, index: Option<usize>) -> bool {
        let session = match index {
            Some(index) => match self.sessions.get(index) {
                Some(session) => Some(session.resource.clone()),
                None => return false,
            },
            None => {
                let busy = self.run.as_ref().is_some_and(|run| {
                    run.chat.as_ref().is_some_and(|chat| {
                        chat.active_turn.is_some()
                            || chat
                                .queued_messages
                                .as_ref()
                                .is_some_and(|queue| !queue.is_empty())
                    }) || self.handing_off()
                });
                self.handing_over = Some(busy);
                None
            }
        };
        self.commands.send(Command::Release { session }).is_ok()
    }

    /// Removes the session at `index` in the list for good. The list is
    /// asked for again once the service has done it.
    pub fn delete_at(&mut self, index: usize) -> bool {
        let Some(session) = self.sessions.get(index) else {
            return false;
        };
        self.commands
            .send(Command::Delete {
                session: session.resource.clone(),
            })
            .is_ok()
    }

    pub fn stop_at(&mut self, index: usize) -> bool {
        let Some(session) = self.sessions.get(index) else {
            return false;
        };
        let status = SessionStatus::from_bits(session.status);
        if !status.contains(SessionStatus::InProgress)
            && !status.contains(SessionStatus::InputNeeded)
        {
            return false;
        }
        self.commands
            .send(Command::Stop {
                session: session.resource.clone(),
            })
            .is_ok()
    }

    /// The conversation so far.
    pub fn transcript(&self) -> Option<Transcript> {
        let run = self.run.as_ref()?;
        let mut transcript = transcript(
            run.chat.as_ref(),
            &run.sent,
            &run.answered,
            run.failure.as_deref(),
            run.agent.as_deref(),
        );
        if run.resumed && run.chat.is_none() && run.failure.is_none() {
            transcript.status = Some(Status::Opening);
        }
        let turn_running = run
            .chat
            .as_ref()
            .is_some_and(|chat| chat.active_turn.is_some());
        if run.loading && !turn_running && run.failure.is_none() {
            transcript.status = Some(Status::Loading);
        }
        if let Some(busy) = self.handing_over {
            transcript.status = Some(Status::HandingOver { busy });
        }
        // Answers given here show before the service carries them.
        for request in transcript
            .entries
            .iter_mut()
            .flat_map(|entry| entry.inputs.iter_mut())
            .filter(|request| request.is_open())
        {
            if let Some(answers) = run.inputs.answers.get(&request.id) {
                request.answers.extend(answers.clone());
            }
            let completed = run
                .inputs
                .completed
                .iter()
                .find(|(id, _)| *id == request.id);
            if let Some((_, response)) = completed {
                request.outcome = Outcome::Responded(*response);
            }
        }
        if transcript.status == Some(Status::Waiting)
            && transcript.question().is_none()
            && transcript.input().is_none()
        {
            transcript.status = Some(Status::Working);
        }
        Some(transcript)
    }
}

/// Every request the chat knows of, in the order they were made: the ended
/// turns, the active one, then the queue.
fn requests(chat: &ChatState) -> Vec<Request> {
    chat.turns
        .iter()
        .map(|turn| &turn.message)
        .chain(chat.active_turn.iter().map(|turn| &turn.message))
        .chain(
            chat.queued_messages
                .iter()
                .flatten()
                .map(|pending| &pending.message),
        )
        .map(request)
        .collect()
}

fn request(message: &Message) -> Request {
    Request {
        prompt: message.text.clone(),
        attachments: message
            .attachments
            .iter()
            .flatten()
            .filter_map(|attachment| {
                // A file attached by reference reads as the file; anything
                // else as what it is called.
                let (label, uri) = match attachment {
                    MessageAttachment::Simple(a) => (&a.label, None),
                    MessageAttachment::EmbeddedResource(a) => (&a.label, None),
                    MessageAttachment::Resource(a) => (&a.label, Some(a.uri.as_str())),
                    MessageAttachment::Annotations(a) => (&a.label, None),
                    MessageAttachment::Chat(a) => (&a.label, None),
                    MessageAttachment::Unknown(_) => return None,
                };
                let path = uri
                    .and_then(path_from_uri)
                    .unwrap_or_else(|| PathBuf::from(label));
                Some(Attachment::for_file(&path))
            })
            .collect(),
    }
}

/// What an attached file is called: its name.
fn file_label(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

/// Which agent a session belongs to, and what it is doing, and in which folder.
/// `agent` is the agent's name, when the service has said what its providers
/// are called.
/// The dot beside a session in the list. A failed session has stopped, so it
/// reads as idle; the subtitle says why.
fn session_activity(session: &SessionSummary) -> Activity {
    let status = SessionStatus::from_bits(session.status);
    if status.contains(SessionStatus::InputNeeded) {
        Activity::Waiting
    } else if status.contains(SessionStatus::InProgress) {
        Activity::Working
    } else {
        Activity::Idle
    }
}

fn session_subtitle(session: &SessionSummary, agent: Option<&str>, home: Option<&Path>) -> String {
    let status = SessionStatus::from_bits(session.status);
    let status = if status.contains(SessionStatus::InputNeeded) {
        otto_kit::t_owned!("launcher-agents-needs-input")
    } else if status.contains(SessionStatus::InProgress) {
        otto_kit::t_owned!("launcher-agents-working")
    } else if status.contains(SessionStatus::Error) {
        otto_kit::t_owned!("launcher-agents-error")
    } else {
        otto_kit::t_owned!("launcher-agents-idle")
    };
    let folder = session
        .working_directories
        .iter()
        .flatten()
        .next()
        .and_then(|uri| path_from_uri(uri));
    // The agent leads, as a handle: whose session this is comes before what it
    // is doing. Unknown providers still name themselves, since the id is what
    // the configuration calls them.
    let agent = agent.unwrap_or(session.provider.as_str());
    let head = if agent.is_empty() {
        status
    } else {
        format!("@{agent} · {status}")
    };
    match folder {
        Some(folder) => format!("{head} · {}", home_relative(&folder, home)),
        None => head,
    }
}

fn home_relative(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Picks the session `query` names: its URI, its id, or the start of its id.
fn find_session<'a>(
    sessions: &'a [SessionSummary],
    query: &str,
) -> Result<&'a SessionSummary, String> {
    session::find(sessions, Some(query)).map_err(|err| err.to_string())
}

/// Works out the conversation from the chat, and from the requests sent that
/// the chat does not show yet.
///
/// The chat lists finished turns, then the active one, then the queue, in the
/// order the requests were sent; anything sent past those is still on its way.
/// Questions in `answered` were answered from here, and are not asked again.
fn transcript(
    chat: Option<&ChatState>,
    sent: &[Request],
    answered: &[String],
    failure: Option<&str>,
    agent: Option<&str>,
) -> Transcript {
    let mut entries = Vec::new();
    let mut status = None;
    let mut queued: Vec<Request> = Vec::new();

    if let Some(chat) = chat {
        for turn in &chat.turns {
            let note = match turn.state {
                TurnState::Complete => None,
                TurnState::Cancelled => Some(Note::Cancelled),
                TurnState::Error => Some(Note::Failed(error_message(&turn.response_parts))),
            };
            let (steps, _) = tool_calls(&turn.id, &turn.response_parts);
            let Request {
                prompt,
                attachments,
            } = request(&turn.message);
            entries.push(Entry {
                prompt,
                attachments,
                answer: answer(&turn.response_parts),
                steps,
                question: None,
                inputs: input_requests(&turn.response_parts, false),
                note,
            });
        }
        if let Some(turn) = &chat.active_turn {
            let (steps, question) = tool_calls(&turn.id, &turn.response_parts);
            let question = question.filter(|question| !answered.contains(&question.tool_call_id));
            let thinking = matches!(turn.response_parts.last(), Some(ResponsePart::Reasoning(_)));
            let inputs = input_requests(&turn.response_parts, true);
            status = Some(
                if question.is_some() || inputs.iter().any(InputRequest::is_open) {
                    Status::Waiting
                } else if thinking {
                    Status::Thinking
                } else {
                    Status::Working
                },
            );
            let Request {
                prompt,
                attachments,
            } = request(&turn.message);
            entries.push(Entry {
                prompt,
                attachments,
                answer: answer(&turn.response_parts),
                steps,
                question,
                inputs,
                note: None,
            });
        }
        queued.extend(
            chat.queued_messages
                .iter()
                .flatten()
                .map(|pending| request(&pending.message)),
        );
    }

    let known = entries.len() + queued.len();
    queued.extend(sent.iter().skip(known).cloned());

    if status.is_none() && !queued.is_empty() {
        // Nothing is running, but a request is about to: the agent is still
        // starting, or the service is between two turns.
        status = Some(if entries.is_empty() {
            Status::Starting(agent.map(str::to_string))
        } else {
            Status::Working
        });
    }
    let next_is_starting = chat.is_none_or(|chat| chat.active_turn.is_none());
    for (index, request) in queued.into_iter().enumerate() {
        let note = (index > 0 || !next_is_starting).then_some(Note::Queued);
        entries.push(Entry {
            prompt: request.prompt,
            attachments: request.attachments,
            answer: Vec::new(),
            steps: Vec::new(),
            question: None,
            inputs: Vec::new(),
            note,
        });
    }

    if let Some(error) = failure {
        status = Some(Status::Failed(error.to_string()));
    }
    Transcript { entries, status }
}

/// The answer in `parts`, in order: the Markdown the agent wrote and the
/// pictures it sent, without the reasoning.
///
/// Markdown parts that follow one another are one piece, joined as paragraphs
/// are, so wrapping and headings read across the chunks the agent streamed.
fn answer(parts: &[ResponsePart]) -> Vec<Said> {
    let mut answer: Vec<Said> = Vec::new();
    for part in parts {
        match part {
            ResponsePart::Markdown(markdown) => {
                let text = markdown.content.trim();
                if text.is_empty() {
                    continue;
                }
                match answer.last_mut() {
                    Some(Said::Text(said)) => {
                        said.push_str("\n\n");
                        said.push_str(text);
                    }
                    _ => answer.push(Said::Text(text.to_owned())),
                }
            }
            ResponsePart::ContentRef(resource) => {
                if let Some(picture) = Picture::at(&resource.uri, resource.content_type.as_deref())
                {
                    answer.push(Said::Image(picture));
                }
            }
            _ => {}
        }
    }
    answer
}

/// The tool calls in the turn `turn_id`'s `parts`: those already decided, and
/// the one waiting for an answer.
fn tool_calls(turn_id: &str, parts: &[ResponsePart]) -> (Vec<Step>, Option<Question>) {
    let mut steps = Vec::new();
    let mut question = None;
    for part in parts {
        let ResponsePart::ToolCall(call) = part else {
            continue;
        };
        let step = |tool: &StringOrMarkdown, state| Step {
            tool: plain(tool),
            state,
        };
        match &call.tool_call {
            ToolCallState::PendingConfirmation(pending) => {
                question = Some(Question {
                    turn_id: turn_id.to_string(),
                    tool_call_id: pending.tool_call_id.clone(),
                    title: pending
                        .confirmation_title
                        .as_ref()
                        .map(plain)
                        .unwrap_or_else(|| pending.display_name.clone()),
                    detail: plain(&pending.invocation_message),
                    action: action_lines(pending.tool_input.as_ref(), pending.edits.as_ref()),
                    default_id: pending
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("otto")?.get("defaultOption")?.as_str())
                        .map(str::to_owned),
                    choices: pending
                        .options
                        .iter()
                        .flatten()
                        .map(|option| Choice {
                            id: option.id.clone(),
                            label: option.label.clone(),
                            approve: matches!(option.kind, ConfirmationOptionKind::Approve),
                        })
                        .collect(),
                });
            }
            ToolCallState::Running(running) => {
                steps.push(step(&running.invocation_message, StepState::Running))
            }
            ToolCallState::Completed(done) => {
                let state = if done.success {
                    StepState::Done
                } else {
                    StepState::Failed
                };
                steps.push(step(&done.invocation_message, state));
            }
            ToolCallState::Cancelled(cancelled) => {
                steps.push(step(&cancelled.invocation_message, StepState::Denied))
            }
            _ => {}
        }
    }
    (steps, question)
}

/// The input requests in `parts`, in order. `active` says whether the turn is
/// still running, which is the only time one can be answered.
fn input_requests(parts: &[ResponsePart], active: bool) -> Vec<InputRequest> {
    parts
        .iter()
        .filter_map(|part| match part {
            ResponsePart::InputRequest(request) => Some(InputRequest::from_part(request, active)),
            _ => None,
        })
        .collect()
}

/// The action that carries an answer to an input request, or its completion.
fn input_action(command: Command) -> Option<StateAction> {
    Some(match command {
        Command::InputAnswer {
            request_id,
            question_id,
            answer,
        } => StateAction::ChatInputAnswerChanged(ChatInputAnswerChangedAction {
            request_id,
            question_id,
            answer: Some(answer),
        }),
        Command::InputComplete {
            request_id,
            response,
            answers,
        } => StateAction::ChatInputCompleted(ChatInputCompletedAction {
            request_id,
            response,
            answers,
        }),
        _ => return None,
    })
}

fn plain(text: &StringOrMarkdown) -> String {
    match text {
        StringOrMarkdown::Plain(text) => text.clone(),
        StringOrMarkdown::Markdown { markdown } => markdown.clone(),
    }
}

fn error_message(parts: &[ResponsePart]) -> String {
    parts
        .iter()
        .find_map(|part| match part {
            ResponsePart::Error(error) => Some(error.error.message.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The connection thread's side of the channel.
struct Reporter {
    updates: mpsc::Sender<Update>,
    wake: UnixStream,
}

impl Reporter {
    fn send(&self, update: Update) {
        if self.updates.send(update).is_ok() {
            // A full socket already has a wake-up in it, which is all a byte
            // is for.
            let _ = (&self.wake).write(&[1]);
        }
    }
}

/// The connection thread: connect and list the agents, wait for the first
/// request and hand it off, then follow the chat, queue the requests that
/// follow and send the answers, until the launcher goes.
async fn serve(
    url: &str,
    folder: &Path,
    mut commands: async_mpsc::UnboundedReceiver<Command>,
    reporter: &Reporter,
) {
    let client = match tokio::time::timeout(TIMEOUT, connect(url)).await {
        Ok(Ok(client)) => client,
        Ok(Err(err)) => {
            reporter.send(Update::Unreachable(format!("otto-agents at {url}: {err}")));
            return;
        }
        Err(_) => {
            reporter.send(Update::Unreachable(format!(
                "otto-agents at {url} did not answer"
            )));
            return;
        }
    };

    // Where each agent wants its sessions, which beats the fallback folder.
    let mut folders: HashMap<String, PathBuf> = HashMap::new();
    match client.subscribe(ROOT_RESOURCE_URI.to_string()).await {
        Ok((subscribed, _root)) => {
            if let Some(SnapshotState::Root(root)) = subscribed.snapshot.map(|s| s.state) {
                folders = folders_from_meta(root.meta.as_ref());
                reporter.send(Update::Agents(root.agents));
                reporter.send(Update::Colours(colours_from_meta(root.meta.as_ref())));
            }
        }
        Err(err) => tracing::warn!(%err, "could not list the agents"),
    }
    match list_sessions(&client).await {
        Ok(sessions) => reporter.send(Update::Sessions(sessions)),
        Err(err) => tracing::warn!(%err, "could not list the sessions"),
    }

    let followed = loop {
        match commands.recv().await {
            Some(Command::Ask {
                prompt,
                provider,
                attachments,
            }) => {
                let chosen = provider
                    .as_deref()
                    .and_then(|provider| folders.get(provider))
                    .map_or(folder, PathBuf::as_path);
                let handed =
                    hand_off(&client, &prompt, &attachments, provider, chosen, reporter).await;
                if handed.is_ok() {
                    reporter.send(Update::HandedOff);
                }
                break handed;
            }
            Some(Command::Resume { session }) => break resume(&client, &session, reporter).await,
            Some(Command::Release {
                session: Some(session),
            }) => {
                release(&client, &session).await;
                continue;
            }
            Some(Command::Delete { session }) => {
                let disposed: Result<Value, _> = client
                    .request("disposeSession", json!({ "channel": session }))
                    .await;
                if let Err(err) = disposed {
                    tracing::warn!(%err, %session, "could not remove the session");
                }
                match list_sessions(&client).await {
                    Ok(sessions) => reporter.send(Update::Sessions(sessions)),
                    Err(err) => tracing::warn!(%err, "could not list the sessions"),
                }
                continue;
            }
            Some(Command::Stop { session }) => {
                if let Err(err) = stop(&client, &session).await {
                    tracing::warn!(%err, %session, "could not stop the session");
                }
                // The list says what the session is doing, and it is not that
                // any more.
                match list_sessions(&client).await {
                    Ok(sessions) => reporter.send(Update::Sessions(sessions)),
                    Err(err) => tracing::warn!(%err, "could not list the sessions"),
                }
                continue;
            }
            Some(_) => continue,
            None => return,
        }
    };
    let (chat_uri, mut session_events, mut chat_events) = match followed {
        Ok(followed) => followed,
        Err(err) => {
            reporter.send(Update::Failed(err.to_string()));
            return;
        }
    };

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                let action = match command {
                    // The session is open already, and the list is gone.
                    Command::Resume { .. } | Command::Stop { .. } | Command::Delete { .. } => {
                        continue
                    }
                    Command::Release { session } => {
                        let session = session.as_deref().unwrap_or_else(|| session_events.uri()).to_owned();
                        release(&client, &session).await;
                        continue;
                    }
                    Command::SetMode { mode_id } => {
                        set_mode(&client, session_events.uri(), &mode_id).await;
                        continue;
                    }
                    Command::Ask { prompt, attachments, .. } => {
                        match queue(&client, &chat_uri, &prompt, &attachments).await {
                            Ok(()) => reporter.send(Update::HandedOff),
                            Err(err) => reporter.send(Update::Failed(err.to_string())),
                        }
                        continue;
                    }
                    Command::Cancel { turn_id } => {
                        StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
                            turn_id,
                            duration: 0,
                            meta: None,
                        })
                    }
                    Command::Confirm { turn_id, tool_call_id, approved, option_id } => {
                        StateAction::ChatToolCallConfirmed(ChatToolCallConfirmedAction {
                            turn_id,
                            tool_call_id,
                            meta: None,
                            approved,
                            confirmed: approved.then_some(ToolCallConfirmationReason::UserAction),
                            reason: None,
                            edited_tool_input: None,
                            user_suggestion: None,
                            reason_message: None,
                            selected_option_id: Some(option_id),
                        })
                    }
                    command @ (Command::InputAnswer { .. } | Command::InputComplete { .. }) => {
                        match input_action(command) {
                            Some(action) => action,
                            None => continue,
                        }
                    }
                };
                if let Err(err) = client.dispatch(chat_uri.clone(), action).await {
                    reporter.send(Update::Failed(err.to_string()));
                }
            },
            event = session_events.recv() => match event {
                Some(SubscriptionEvent::Action(envelope)) => match envelope.action {
                    StateAction::SessionCreationFailed(failed) => {
                        reporter.send(Update::Failed(failed.error.message));
                    }
                    StateAction::SessionMetaChanged(changed) => {
                        reporter.send(Update::Terminal(Terminal::from_meta(changed.meta.as_ref())));
                        reporter.send(Update::Loading(loading_from_meta(changed.meta.as_ref())));
                        reporter.send(Update::Modes(modes_from_meta(changed.meta.as_ref())));
                    }
                    _ => {}
                },
                Some(_) => {}
                None => {
                    reporter.send(Update::Failed("otto-agents closed the connection".into()));
                    break;
                }
            },
            event = chat_events.recv() => match event {
                Some(SubscriptionEvent::Action(envelope)) => match envelope.rejection_reason {
                    // An answer that lost to one given elsewhere, in a dialog
                    // or another client, is refused. The question is settled
                    // either way, so the conversation carries on.
                    Some(reason)
                        if matches!(envelope.action, StateAction::ChatToolCallConfirmed(_)) =>
                    {
                        tracing::info!(%reason, "the answer came too late")
                    }
                    // The same goes for an input request: settled elsewhere,
                    // or refused, the chat says where it stands.
                    Some(reason) => match &envelope.action {
                        StateAction::ChatInputAnswerChanged(ChatInputAnswerChangedAction {
                            request_id,
                            ..
                        })
                        | StateAction::ChatInputCompleted(ChatInputCompletedAction {
                            request_id,
                            ..
                        }) => {
                            tracing::info!(%reason, %request_id, "the answer was refused");
                            reporter.send(Update::InputRefused(request_id.clone()));
                        }
                        _ => reporter.send(Update::Failed(reason)),
                    },
                    None => reporter.send(Update::Action(Box::new(envelope.action))),
                },
                Some(_) => {}
                None => {
                    reporter.send(Update::Failed("otto-agents closed the connection".into()));
                    break;
                }
            },
        }
    }
    client.shutdown().await;
}

async fn connect(url: &str) -> Result<Client, BoxError> {
    let transport = otto_agents_client::connect(url).await?;
    let client = Client::connect(transport, ClientConfig::default()).await?;
    client
        .initialize(
            "otto-launcher".into(),
            vec![PROTOCOL_VERSION.into()],
            Vec::new(),
        )
        .await?;
    Ok(client)
}

/// A followed session: its chat's URI, with the session's and the chat's event
/// streams.
type Followed = (String, ahp::SessionSubscription, ahp::SessionSubscription);

async fn list_sessions(client: &Client) -> Result<Vec<SessionSummary>, BoxError> {
    let listed: ListSessionsResult = client
        .request("listSessions", json!({ "channel": ROOT_RESOURCE_URI }))
        .await?;
    Ok(listed.items)
}

/// Creates a session in `folder` with `request` and its `attachments` queued
/// on its chat, and follows it.
async fn hand_off(
    client: &Client,
    request: &str,
    attachments: &[PathBuf],
    provider: Option<String>,
    folder: &Path,
    reporter: &Reporter,
) -> Result<Followed, BoxError> {
    let session = format!("{SESSION_SCHEME}{}", uuid::Uuid::new_v4());
    let mut params = json!({ "channel": session, "workingDirectories": [file_uri(folder)] });
    if let Some(provider) = provider {
        params["provider"] = Value::String(provider);
    }
    client.request::<_, Value>("createSession", params).await?;

    let followed = follow(client, session.clone(), reporter).await?;
    queue(client, &followed.0, request, attachments).await?;
    tracing::info!(%session, "session created");
    Ok(followed)
}

/// Follows the session `query` names, as it is.
async fn resume(client: &Client, query: &str, reporter: &Reporter) -> Result<Followed, BoxError> {
    let sessions = list_sessions(client).await?;
    let session = find_session(&sessions, query)?.resource.clone();
    tracing::info!(%session, "session opened");
    follow(client, session, reporter).await
}

/// Subscribes to `session` and its chat, reporting the chat as it stands.
async fn follow(
    client: &Client,
    session: String,
    reporter: &Reporter,
) -> Result<Followed, BoxError> {
    let (subscribed, session_events) = client.subscribe(session.clone()).await?;
    let chat_uri = match subscribed.snapshot.map(|snapshot| snapshot.state) {
        Some(SnapshotState::Session(state)) => {
            reporter.send(Update::Provider(state.provider.clone()));
            reporter.send(Update::Terminal(Terminal::from_meta(state.meta.as_ref())));
            reporter.send(Update::Loading(loading_from_meta(state.meta.as_ref())));
            reporter.send(Update::Modes(modes_from_meta(state.meta.as_ref())));
            state.default_chat.ok_or("the session has no chat")?
        }
        _ => return Err("the service sent no session".into()),
    };

    // Subscribed before the request is queued, so every change it causes
    // arrives after the snapshot it applies to. Being subscribed is also what
    // tells the service that questions can be answered here.
    let (subscribed, chat_events) = client.subscribe(chat_uri.clone()).await?;
    match subscribed.snapshot.map(|snapshot| snapshot.state) {
        Some(SnapshotState::Chat(chat)) => reporter.send(Update::Chat(chat)),
        _ => return Err("the service sent no chat".into()),
    }
    Ok((chat_uri, session_events, chat_events))
}

/// Cancels the active turn of `session`, if it has one, without following it.
/// Hands `session` to its terminal, so the service lets go of its agent once
/// the agent is idle. Failing is logged and nothing more: the terminal opens
/// either way, and the idle timeout gets there in the end.
async fn release(client: &Client, session: &str) {
    let released: Result<Value, _> = client
        .request("releaseSession", json!({ "channel": session }))
        .await;
    if let Err(err) = released {
        tracing::warn!(%err, %session, "could not hand the session to its terminal");
    }
}

/// Asks the service to switch `session`'s agent to the mode `mode_id`. A
/// refusal is logged and nothing more: the line shows the mode the agent is
/// in, which the agent's own answer moves.
async fn set_mode(client: &Client, session: &str, mode_id: &str) {
    let switched: Result<Value, _> = client
        .request("setMode", json!({ "session": session, "modeId": mode_id }))
        .await;
    if let Err(err) = switched {
        tracing::warn!(%err, %session, %mode_id, "could not switch the agent's mode");
    }
}

async fn stop(client: &Client, session: &str) -> Result<(), BoxError> {
    let (subscribed, _events) = client.subscribe(session.to_string()).await?;
    let chat_uri = match subscribed.snapshot.map(|snapshot| snapshot.state) {
        Some(SnapshotState::Session(state)) => state.default_chat,
        _ => None,
    };
    let _ = client.unsubscribe(session.to_string()).await;
    let chat_uri = chat_uri.ok_or("the session has no chat")?;

    // Only long enough to learn the turn: staying subscribed would tell the
    // service that questions can be answered here.
    let (subscribed, _events) = client.subscribe(chat_uri.clone()).await?;
    let turn = match subscribed.snapshot.map(|snapshot| snapshot.state) {
        Some(SnapshotState::Chat(chat)) => chat.active_turn.map(|turn| turn.id),
        _ => None,
    };
    let _ = client.unsubscribe(chat_uri.clone()).await;
    let Some(turn_id) = turn else {
        return Ok(());
    };
    tracing::info!(%session, %turn_id, "stopping the session's turn");
    client
        .dispatch(
            chat_uri,
            StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
                turn_id,
                duration: 0,
                meta: None,
            }),
        )
        .await?;
    Ok(())
}

/// Queues `request` on the chat with `attachments`, returning once the
/// service has it.
async fn queue(
    client: &Client,
    chat_uri: &str,
    request: &str,
    attachments: &[PathBuf],
) -> Result<(), BoxError> {
    let queued = ChatPendingMessageSetAction {
        kind: PendingMessageKind::Queued,
        id: uuid::Uuid::new_v4().to_string(),
        message: Message {
            text: request.to_string(),
            origin: MessageOrigin {
                kind: MessageKind::User,
            },
            attachments: (!attachments.is_empty())
                .then(|| attachments.iter().map(|file| attachment(file)).collect()),
            model: None,
            agent: None,
            meta: None,
        },
    };
    client
        .dispatch(
            chat_uri.to_string(),
            StateAction::ChatPendingMessageSet(queued),
        )
        .await?;
    // `dispatchAction` is a notification, so nothing answers it. The service
    // handles a connection's messages in order, so the ping coming back means
    // the request ahead of it has been taken.
    client.ping().await?;
    Ok(())
}

/// `file` attached by reference: the agent reads it itself.
fn attachment(file: &Path) -> MessageAttachment {
    MessageAttachment::Resource(MessageResourceAttachment {
        label: file_label(file),
        range: None,
        display_kind: file.is_dir().then(|| "directory".to_string()),
        meta: None,
        uri: file_uri(file),
        size_hint: None,
        content_type: None,
        nonce: None,
        selection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahp_types::state::{
        ActiveTurn, ConfirmationOption, ErrorInfo, ErrorResponsePart, MarkdownResponsePart,
        PendingMessage, ReasoningResponsePart, ToolCallCancellationReason, ToolCallCancelledState,
        ToolCallPendingConfirmationState, ToolCallResponsePart, Turn,
    };
    use std::time::Instant;

    #[test]
    fn a_folder_becomes_a_file_uri() {
        assert_eq!(
            file_uri(Path::new("/home/me/My Projects")),
            "file:///home/me/My%20Projects"
        );
    }

    #[test]
    fn an_unreachable_service_is_reported_rather_than_waited_on() {
        // Port 9 is the discard port; nothing speaks WebSocket there.
        let mut ask = Ask::connect("ws://127.0.0.1:9".into(), PathBuf::from("/"));
        let deadline = Instant::now() + TIMEOUT + Duration::from_secs(2);
        while ask.unreachable().is_none() && Instant::now() < deadline {
            ask.pump();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(ask.unreachable().is_some());

        // Asking anyway fails the request instead of pretending to start it.
        ask.send("hello", None);
        assert!(!ask.handing_off());
        assert!(matches!(
            ask.transcript().and_then(|t| t.status),
            Some(Status::Failed(_))
        ));
    }

    fn message(text: &str) -> Message {
        Message {
            text: text.into(),
            origin: MessageOrigin {
                kind: MessageKind::User,
            },
            attachments: None,
            model: None,
            agent: None,
            meta: None,
        }
    }

    fn markdown(content: &str) -> ResponsePart {
        ResponsePart::Markdown(MarkdownResponsePart {
            id: content.into(),
            content: content.into(),
        })
    }

    /// A picture in an answer, as the service puts it there: a `contentRef` at
    /// the file it wrote.
    fn picture(name: &str) -> ResponsePart {
        ResponsePart::ContentRef(ahp_types::state::ResourceResponsePart {
            uri: file_uri(&PathBuf::from(format!("/tmp/otto/{name}"))),
            size_hint: Some(64),
            content_type: Some("image/png".into()),
            nonce: None,
        })
    }

    fn reasoning(content: &str) -> ResponsePart {
        ResponsePart::Reasoning(ReasoningResponsePart {
            id: content.into(),
            content: content.into(),
        })
    }

    fn asking(tool_call_id: &str) -> ResponsePart {
        let option = |id: &str, label: &str, kind| ConfirmationOption {
            id: id.into(),
            label: label.into(),
            kind,
            group: None,
        };
        ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
            tool_call: ToolCallState::PendingConfirmation(ToolCallPendingConfirmationState {
                tool_call_id: tool_call_id.into(),
                tool_name: "execute".into(),
                display_name: "cargo test".into(),
                intention: None,
                contributor: None,
                meta: None,
                invocation_message: StringOrMarkdown::Plain("cargo test".into()),
                tool_input: None,
                confirmation_title: Some(StringOrMarkdown::Plain(
                    "Claude wants to run a command".into(),
                )),
                risk_assessment: None,
                edits: None,
                editable: None,
                options: Some(vec![
                    option(
                        "allow_always",
                        "Always Allow",
                        ConfirmationOptionKind::Approve,
                    ),
                    option("allow", "Allow", ConfirmationOptionKind::Approve),
                    option("reject", "Reject", ConfirmationOptionKind::Deny),
                ]),
            }),
        }))
    }

    #[test]
    fn the_service_says_which_answer_to_start_on() {
        let mut question = Question {
            turn_id: "t".into(),
            tool_call_id: "c".into(),
            title: String::new(),
            detail: String::new(),
            action: Vec::new(),
            default_id: Some("reject".into()),
            choices: ["allow_always", "allow", "reject"]
                .iter()
                .map(|id| Choice {
                    id: (*id).into(),
                    label: (*id).into(),
                    approve: id.starts_with("allow"),
                })
                .collect(),
        };
        assert_eq!(question.default_choice(), 2, "the service's pick");
        question.default_id = Some("no-such".into());
        assert_eq!(question.default_choice(), 1, "an unknown pick falls back");
        question.default_id = None;
        assert_eq!(question.default_choice(), 1);

        // And it travels in the pending state's `_meta`.
        let ResponsePart::ToolCall(mut call) = asking("call-1") else {
            unreachable!()
        };
        if let ToolCallState::PendingConfirmation(pending) = &mut call.tool_call {
            pending.meta = Some(
                json!({ "otto": { "defaultOption": "reject" } })
                    .as_object()
                    .cloned()
                    .unwrap(),
            );
        }
        let (_, question) = tool_calls("t", &[ResponsePart::ToolCall(call)]);
        assert_eq!(question.unwrap().default_choice(), 2);
    }

    #[test]
    fn what_the_tool_would_touch_is_shown_in_lines() {
        let input = ToolInput::Inline(
            json!({ "path": "/srv/a.rs", "rawInput": { "file_path": "/srv/a.rs" } }).to_string(),
        );
        let edits = json!([
            { "path": "/srv/a.rs", "oldText": "one\ntwo", "newText": "uno\ndue" },
            { "path": "/srv/b.rs", "oldText": null, "newText": "new" },
        ]);
        assert_eq!(
            action_lines(Some(&input), Some(&edits)),
            [
                "/srv/a.rs",
                "- one",
                "- two",
                "+ uno",
                "+ due",
                "/srv/b.rs",
                "+ new"
            ]
        );
        // Only the file, without an edit.
        assert_eq!(action_lines(Some(&input), None), ["/srv/a.rs"]);
        // Nothing usable, nothing shown.
        assert!(action_lines(Some(&ToolInput::Inline("not json".into())), None).is_empty());
        assert!(action_lines(None, Some(&json!({ "path": 1 }))).is_empty());

        // A long edit is cut short.
        let long: String = (0..30).map(|n| format!("line {n}\n")).collect();
        let edits = json!([{ "path": "/srv/c.rs", "newText": long }]);
        let lines = action_lines(None, Some(&edits));
        assert_eq!(lines.len(), ACTION_LINES);
        assert_eq!(lines.last().map(String::as_str), Some("\u{2026}"));
        assert_eq!(lines[ACTION_LINES - 2], "+ line 9");
    }

    fn refused(tool_call_id: &str) -> ResponsePart {
        ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
            tool_call: ToolCallState::Cancelled(ToolCallCancelledState {
                tool_call_id: tool_call_id.into(),
                tool_name: "execute".into(),
                display_name: "rm -rf build".into(),
                intention: None,
                contributor: None,
                meta: None,
                invocation_message: StringOrMarkdown::Plain("rm -rf build".into()),
                tool_input: None,
                reason: ToolCallCancellationReason::Denied,
                reason_message: None,
                user_suggestion: None,
                selected_option: None,
            }),
        }))
    }

    fn empty_chat() -> ChatState {
        serde_json::from_value(json!({
            "resource": "ahp-chat:/1",
            "title": "",
            "status": 0,
            "modifiedAt": "2026-09-15T10:00:00.000Z",
            "turns": [],
        }))
        .expect("a chat")
    }

    fn with_active(mut chat: ChatState, prompt: &str, parts: Vec<ResponsePart>) -> ChatState {
        chat.active_turn = Some(ActiveTurn {
            id: "t".into(),
            started_at: "2026-09-15T10:00:00.000Z".into(),
            message: message(prompt),
            response_parts: parts,
            usage: None,
        });
        chat
    }

    fn with_ended(
        mut chat: ChatState,
        prompt: &str,
        state: TurnState,
        parts: Vec<ResponsePart>,
    ) -> ChatState {
        chat.turns.push(Turn {
            id: prompt.into(),
            started_at: None,
            duration: None,
            message: message(prompt),
            response_parts: parts,
            usage: None,
            state,
        });
        chat
    }

    fn with_queued(mut chat: ChatState, prompt: &str) -> ChatState {
        chat.queued_messages
            .get_or_insert_with(Vec::new)
            .push(PendingMessage {
                id: prompt.into(),
                message: message(prompt),
            });
        chat
    }

    /// Everything an entry's answer says, as one string: what a test asserting
    /// on the words of an answer wants, whatever pieces it arrived in.
    fn said(entry: &Entry) -> String {
        entry
            .answer
            .iter()
            .filter_map(|said| match said {
                Said::Text(text) => Some(text.as_str()),
                Said::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn entry(prompt: &str, answer: &str, note: Option<Note>) -> Entry {
        Entry {
            prompt: prompt.into(),
            attachments: Vec::new(),
            answer: match answer.is_empty() {
                true => Vec::new(),
                false => vec![Said::Text(answer.to_owned())],
            },
            steps: Vec::new(),
            question: None,
            inputs: Vec::new(),
            note,
        }
    }

    fn sent(prompts: &[&str]) -> Vec<Request> {
        prompts
            .iter()
            .map(|prompt| Request {
                prompt: prompt.to_string(),
                attachments: Vec::new(),
            })
            .collect()
    }

    fn of(chat: Option<&ChatState>, prompts: &[Request]) -> Transcript {
        transcript(chat, prompts, &[], None, None)
    }

    #[test]
    fn a_first_request_starts_until_its_turn_does() {
        let starting = Transcript {
            entries: vec![entry("hi", "", None)],
            status: Some(Status::Starting(Some("Claude".into()))),
        };
        let hi = sent(&["hi"]);
        assert_eq!(transcript(None, &hi, &[], None, Some("Claude")), starting);
        let chat = with_queued(empty_chat(), "hi");
        assert_eq!(
            transcript(Some(&chat), &hi, &[], None, Some("Claude")),
            starting
        );
    }

    /// A picture sits where the agent sent it, so a diagram stays with the
    /// paragraph that introduces it rather than falling to the end.
    #[test]
    fn a_picture_takes_its_place_in_the_answer() {
        let parts = vec![
            markdown("here:"),
            picture("mock-up-0123456789abcdef.png"),
            markdown("and"),
            markdown("that is all"),
        ];
        let chat = with_ended(empty_chat(), "draw", TurnState::Complete, parts);
        let transcript = transcript(Some(&chat), &sent(&["draw"]), &[], None, None);
        let answer = &transcript.entries[0].answer;
        assert_eq!(answer.len(), 3, "{answer:?}");
        assert_eq!(answer[0], Said::Text("here:".into()));
        let Said::Image(picture) = &answer[1] else {
            panic!("the picture is in the middle: {answer:?}");
        };
        assert_eq!(
            picture.path,
            PathBuf::from("/tmp/otto/mock-up-0123456789abcdef.png")
        );
        // The digest the service appends is how the file stays one file; it is
        // not what the picture is called.
        assert_eq!(picture.label, "mock up");
        // The two pieces of Markdown after it read as one document.
        assert_eq!(answer[2], Said::Text("and\n\nthat is all".into()));
    }

    /// What is pointed at has to be a picture on this machine. Anything else is
    /// context for the agent, and drawing a frame for it would be a lie.
    #[test]
    fn a_reference_that_is_not_a_local_picture_is_not_shown() {
        let not_a_picture = ResponsePart::ContentRef(ahp_types::state::ResourceResponsePart {
            uri: file_uri(&PathBuf::from("/tmp/otto/notes.md")),
            size_hint: None,
            content_type: Some("text/markdown".into()),
            nonce: None,
        });
        let elsewhere = ResponsePart::ContentRef(ahp_types::state::ResourceResponsePart {
            uri: "https://example.invalid/shot.png".into(),
            size_hint: None,
            content_type: Some("image/png".into()),
            nonce: None,
        });
        let parts = vec![markdown("see"), not_a_picture, elsewhere];
        let chat = with_ended(empty_chat(), "look", TurnState::Complete, parts);
        let transcript = transcript(Some(&chat), &sent(&["look"]), &[], None, None);
        assert_eq!(transcript.entries[0].answer, [Said::Text("see".into())]);
    }

    #[test]
    fn reasoning_shows_as_thinking_and_never_in_the_answer() {
        let hi = sent(&["hi"]);
        let chat = with_active(empty_chat(), "hi", vec![reasoning("hmm")]);
        let thinking = of(Some(&chat), &hi);
        assert_eq!(thinking.status, Some(Status::Thinking));
        assert_eq!(thinking.entries, vec![entry("hi", "", None)]);

        let parts = vec![reasoning("hmm"), markdown("Hello"), markdown("world")];
        let chat = with_active(empty_chat(), "hi", parts);
        let answering = of(Some(&chat), &hi);
        assert_eq!(answering.status, Some(Status::Working));
        assert_eq!(answering.entries, vec![entry("hi", "Hello\n\nworld", None)]);
    }

    #[test]
    fn requests_sent_during_a_turn_queue_behind_it() {
        let prompts = sent(&["one", "two", "three"]);
        // "two" has reached the service's queue; "three" is still on its way.
        let chat = with_queued(with_active(empty_chat(), "one", vec![]), "two");
        let during = of(Some(&chat), &prompts);
        assert_eq!(
            during.entries,
            vec![
                entry("one", "", None),
                entry("two", "", Some(Note::Queued)),
                entry("three", "", Some(Note::Queued)),
            ]
        );
        assert_eq!(during.status, Some(Status::Working));
    }

    #[test]
    fn a_question_waits_for_an_answer_offering_the_narrowest_allow_first() {
        let prompts = sent(&["run the tests"]);
        let chat = with_active(
            empty_chat(),
            "run the tests",
            vec![refused("call-0"), asking("call-1")],
        );
        let waiting = of(Some(&chat), &prompts);
        assert_eq!(waiting.status, Some(Status::Waiting));
        let entry = &waiting.entries[0];
        assert_eq!(
            entry.steps,
            vec![Step {
                tool: "rm -rf build".into(),
                state: StepState::Denied
            }]
        );
        let question = waiting.question().expect("a question");
        assert_eq!(question.title, "Claude wants to run a command");
        assert_eq!(question.detail, "cargo test");
        assert_eq!(question.turn_id, "t");
        let labels: Vec<&str> = question.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Always Allow", "Allow", "Reject"]);
        assert_eq!(question.default_choice(), 1);
        assert!(question.action.is_empty());

        // Answered here, it is not offered again while the service catches up.
        let answered = transcript(Some(&chat), &prompts, &["call-1".into()], None, None);
        assert!(answered.question().is_none());
        assert_eq!(answered.status, Some(Status::Working));
    }

    fn input_request(response: Option<&str>) -> ResponsePart {
        let mut part = json!({
            "kind": "inputRequest",
            "request": {
                "id": "req-1",
                "questions": [
                    { "kind": "boolean", "id": "tests", "message": "Write tests?", "required": true }
                ]
            }
        });
        if let Some(response) = response {
            part["response"] = json!(response);
        }
        serde_json::from_value(part).expect("an input request part")
    }

    #[test]
    fn an_open_input_request_waits_and_an_ended_turn_leaves_it_unanswered() {
        let prompts = sent(&["set it up"]);
        let chat = with_active(empty_chat(), "set it up", vec![input_request(None)]);
        let waiting = of(Some(&chat), &prompts);
        assert_eq!(waiting.status, Some(Status::Waiting));
        let request = waiting.input().expect("an open input request");
        assert_eq!(request.id, "req-1");

        let chat = with_active(
            empty_chat(),
            "set it up",
            vec![input_request(Some("accept"))],
        );
        let answered = of(Some(&chat), &prompts);
        assert!(answered.input().is_none());
        assert_eq!(answered.status, Some(Status::Working));

        let chat = with_ended(
            empty_chat(),
            "set it up",
            TurnState::Complete,
            vec![input_request(None)],
        );
        let ended = of(Some(&chat), &prompts);
        assert!(ended.input().is_none());
        assert_eq!(ended.entries[0].inputs[0].outcome, Outcome::Unanswered);
    }

    #[test]
    fn ended_turns_say_how_they_ended_and_leave_nothing_in_hand() {
        let error = ResponsePart::Error(ErrorResponsePart {
            error: ErrorInfo {
                error_type: "agentError".into(),
                message: "out of tokens".into(),
                stack: None,
                meta: None,
            },
            resumable: None,
        });
        let chat = with_ended(
            empty_chat(),
            "one",
            TurnState::Complete,
            vec![markdown("Hi")],
        );
        let chat = with_ended(chat, "two", TurnState::Cancelled, vec![]);
        let chat = with_ended(chat, "three", TurnState::Error, vec![error]);
        let prompts = sent(&["one", "two", "three"]);
        assert_eq!(
            of(Some(&chat), &prompts),
            Transcript {
                entries: vec![
                    entry("one", "Hi", None),
                    entry("two", "", Some(Note::Cancelled)),
                    entry("three", "", Some(Note::Failed("out of tokens".into()))),
                ],
                status: None,
            }
        );
    }

    #[test]
    fn a_failure_outranks_whatever_the_chat_says() {
        let chat = with_active(empty_chat(), "hi", vec![markdown("Hel")]);
        let failed = transcript(Some(&chat), &sent(&["hi"]), &[], Some("gone"), None);
        assert_eq!(failed.status, Some(Status::Failed("gone".into())));
        assert_eq!(failed.entries, vec![entry("hi", "Hel", None)]);
    }

    /// An `Ask` whose connection never gets anywhere, for driving its state by
    /// hand. Nothing is pumped, so the connection's failure never lands.
    fn offline() -> Ask {
        Ask::connect("ws://127.0.0.1:9".into(), PathBuf::from("/"))
    }

    fn attached(file: &str) -> MessageAttachment {
        attachment(Path::new(file))
    }

    /// An agent publishing `skills` as a plugin's children, the way otto-agents
    /// publishes the desktop's own.
    fn agent_with_skills(provider: &str, skills: &[&str]) -> AgentInfo {
        use ahp_types::state::{PluginCustomization, SkillCustomization};
        let children = skills
            .iter()
            .map(|name| {
                ChildCustomization::Skill(SkillCustomization {
                    id: format!("otto-skill:{name}"),
                    uri: format!("file:///usr/share/otto/plugins/otto/skills/{name}/SKILL.md"),
                    name: (*name).to_owned(),
                    icons: None,
                    range: None,
                    meta: None,
                    enabled: None,
                    description: Some(format!("what {name} is for")),
                    disable_model_invocation: None,
                    disable_user_invocation: None,
                })
            })
            .collect();
        AgentInfo {
            provider: provider.to_owned(),
            display_name: provider.to_owned(),
            description: String::new(),
            models: Vec::new(),
            protected_resources: None,
            customizations: Some(vec![Customization::Plugin(PluginCustomization {
                id: "otto-plugin:otto".into(),
                uri: "file:///usr/share/otto/plugins/otto".into(),
                name: "otto".into(),
                icons: None,
                range: None,
                meta: None,
                client_id: None,
                load: None,
                children: Some(children),
                enablement: None,
                version: None,
            })]),
            capabilities: None,
        }
    }

    #[test]
    fn a_skill_name_completes_from_a_slash() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![agent_with_skills(
            "claude",
            &["configure-otto", "extend-otto-files"],
        )]));

        assert_eq!(ask.completion("/conf", None).as_deref(), Some("igure-otto"));
        assert_eq!(
            ask.completion("/extend", None).as_deref(),
            Some("-otto-files")
        );
        // Case is not what a person is thinking about while typing.
        assert_eq!(ask.completion("/CONF", None).as_deref(), Some("igure-otto"));
    }

    /// The completion only ever fires on a request that opens with a skill
    /// name, so an ordinary sentence never sprouts grey text.
    #[test]
    fn nothing_completes_without_a_slash_or_past_the_first_word() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![agent_with_skills(
            "claude",
            &["configure-otto"],
        )]));

        assert_eq!(ask.completion("configure", None), None, "no slash");
        assert_eq!(ask.completion("/", None), None, "nothing typed yet");
        assert_eq!(
            ask.completion("/configure-otto the dock", None),
            None,
            "the name is behind us"
        );
        assert_eq!(ask.completion("/nope", None), None, "no such skill");
        assert_eq!(
            ask.completion("/configure-otto", None),
            None,
            "a name already whole has nothing to add"
        );
    }

    /// Two agents, different skills: the completion follows the one the
    /// request would go to.
    #[test]
    fn the_completion_follows_the_chosen_agent() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![
            agent_with_skills("claude", &["configure-otto"]),
            agent_with_skills("other", &["count-sheep"]),
        ]));

        assert_eq!(
            ask.completion("/co", Some(0)).as_deref(),
            Some("nfigure-otto")
        );
        assert_eq!(ask.completion("/co", Some(1)).as_deref(), Some("unt-sheep"));
        // With none picked, the default agent is the one that would answer.
        assert_eq!(ask.completion("/co", None).as_deref(), Some("nfigure-otto"));
    }

    /// The agent picked from the list is the one the field names: the row
    /// and the name come from the same list, in the same order.
    #[test]
    fn a_picked_row_names_its_agent() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![
            agent_with_skills("claude", &[]),
            agent_with_skills("hermes", &[]),
        ]));

        let rows = ask.agent_rows(0);
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(
                ask.agent_display_name(row.origin.index),
                Some(row.title.as_str())
            );
        }
        assert_eq!(ask.agent_display_name(2), None);
    }

    /// One name being another's prefix: the shorter completion is the one
    /// that is never wrong to accept.
    #[test]
    fn the_shortest_matching_name_wins() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![agent_with_skills(
            "claude",
            &["files-extra", "files"],
        )]));
        assert_eq!(ask.completion("/fil", None).as_deref(), Some("es"));
    }

    #[test]
    fn stashed_items_follow_the_attached_ones_and_stay_otto_stashes() {
        let mut ask = offline();
        ask.attach([PathBuf::from("/tmp/attached.md")]);
        ask.set_stashed(&[
            (PathBuf::from("/tmp/one.png"), false),
            (PathBuf::from("/tmp/two.png"), true),
        ]);
        assert_eq!(ask.pending().len(), 3);
        // Changes to stashed items say where in the stash they are.
        assert_eq!(ask.toggle_attachment(0), None);
        assert_eq!(ask.toggle_attachment(2), Some(1));
        assert_eq!(ask.remove_attachment(1), Some(0));
        assert!(ask.has_attachments());
        assert!(ask.send("", None));
        assert!(ask.pending().is_empty());
        let entries = ask.transcript().expect("a conversation").entries;
        assert_eq!(
            entries[0].attachments,
            [Attachment::File("/tmp/two.png".into())]
        );
        // Nothing stashed, nothing for otto-stash to end.
        assert!(!ask.send("again", None));
    }

    #[test]
    fn files_go_with_the_next_request_only() {
        let mut ask = offline();
        ask.attach([
            PathBuf::from("/home/me/My Notes.md"),
            PathBuf::from("/tmp/a.png"),
            PathBuf::from("/tmp/left-out.txt"),
            PathBuf::from("/tmp/removed.txt"),
        ]);
        ask.remove_attachment(3);
        ask.toggle_attachment(2);
        assert_eq!(
            ask.pending(),
            [
                (Attachment::File("/home/me/My Notes.md".into()), false),
                (Attachment::File("/tmp/a.png".into()), false),
                (Attachment::File("/tmp/left-out.txt".into()), true),
            ]
        );

        assert!(ask.has_attachments());
        ask.send("summarise", None);
        assert!(!ask.has_attachments());
        ask.send("and again", None);
        assert!(ask.pending().is_empty());
        let entries = ask.transcript().expect("a conversation").entries;
        // A struck attachment stays behind.
        assert_eq!(
            entries[0].attachments,
            [
                Attachment::File("/home/me/My Notes.md".into()),
                Attachment::File("/tmp/a.png".into()),
            ]
        );
        assert!(entries[1].attachments.is_empty());
    }

    /// otto-agents says how to open a session in a terminal in the session's
    /// `_meta`; anything short of a command and a folder is no way to.
    #[test]
    fn a_terminal_is_open_only_when_a_process_with_a_tty_names_the_session() {
        let terminal = |session: Option<&str>| Terminal {
            command: vec!["true".into()],
            cwd: PathBuf::from("/"),
            session: session.map(str::to_owned),
            title: String::new(),
        };
        assert!(
            !terminal(Some("no-process-names-this-0f3c1e")).already_open(),
            "an id on nobody's command line"
        );
        assert!(!terminal(None).already_open(), "no id to look for");
        assert!(
            !terminal(Some("")).already_open(),
            "an empty id matches everything"
        );
    }

    #[test]
    fn a_session_says_how_to_open_it_in_a_terminal() {
        let meta = |value: Value| value.as_object().cloned();
        let terminal = meta(json!({
            "otto": { "terminal": {
                "command": ["ghostty", "-e", "claude", "--resume", "abc"],
                "cwd": "/home/me",
                "session": "abc",
            }}
        }));
        assert_eq!(
            Terminal::from_meta(terminal.as_ref()),
            Some(Terminal {
                command: ["ghostty", "-e", "claude", "--resume", "abc"]
                    .map(String::from)
                    .to_vec(),
                cwd: PathBuf::from("/home/me"),
                session: Some("abc".into()),
                title: String::new(),
            })
        );

        assert_eq!(Terminal::from_meta(None), None);
        let other = meta(json!({ "someone": { "else": true } }));
        assert_eq!(Terminal::from_meta(other.as_ref()), None);
        let empty = meta(json!({ "otto": { "terminal": { "command": [], "cwd": "/" } } }));
        assert_eq!(Terminal::from_meta(empty.as_ref()), None);
    }

    #[test]
    fn an_attachment_points_at_its_file() {
        let MessageAttachment::Resource(resource) = attached("/home/me/My Notes.md") else {
            panic!("a file is attached as a resource");
        };
        assert_eq!(resource.label, "My Notes.md");
        assert_eq!(resource.uri, "file:///home/me/My%20Notes.md");
        assert_eq!(
            path_from_uri(&resource.uri).as_deref(),
            Some(Path::new("/home/me/My Notes.md"))
        );
    }

    #[test]
    fn an_opened_session_shows_what_came_before_and_carries_on() {
        let mut ask = offline();
        ask.resume("abc");
        assert_eq!(
            ask.transcript().and_then(|t| t.status),
            Some(Status::Opening)
        );
        // Typed before the session arrived: it queues behind what is there.
        ask.send("more", None);

        let mut earlier = message("one");
        earlier.attachments = Some(vec![attached("/home/me/notes.md")]);
        let mut chat = with_ended(
            empty_chat(),
            "one",
            TurnState::Complete,
            vec![markdown("Hi")],
        );
        chat.turns[0].message = earlier;
        ask.apply(Update::Chat(Box::new(chat)));

        let transcript = ask.transcript().expect("a conversation");
        assert_eq!(
            transcript.entries,
            vec![
                Entry {
                    attachments: vec![Attachment::File("/home/me/notes.md".into())],
                    ..entry("one", "Hi", None)
                },
                entry("more", "", None),
            ]
        );
        assert_eq!(transcript.status, Some(Status::Working));
        assert!(ask.handing_off(), "the new request is still on its way");
    }

    fn summary(id: &str, title: &str) -> SessionSummary {
        serde_json::from_value(json!({
            "resource": format!("ahp-session:/{id}"),
            "provider": "claude",
            "title": title,
            "status": 1,
            "createdAt": "2026-09-15T10:00:00.000Z",
            "modifiedAt": "2026-09-15T10:00:00.000Z",
            "workingDirectories": ["file:///home/me/My%20Projects"],
        }))
        .expect("a session summary")
    }

    #[test]
    fn a_session_row_carries_what_the_session_is_doing() {
        let with_status = |bits: u32| {
            let mut session = summary("abc", "one");
            session.status = bits;
            session_activity(&session)
        };
        assert_eq!(with_status(SessionStatus::Idle.bits()), Activity::Idle);
        assert_eq!(with_status(SessionStatus::Error.bits()), Activity::Idle);
        assert_eq!(
            with_status(SessionStatus::InProgress.bits()),
            Activity::Working
        );
        // Waiting on input is a turn in progress too; the wait is what shows.
        assert_eq!(
            with_status(SessionStatus::InputNeeded.bits()),
            Activity::Waiting
        );
        assert_eq!(
            with_status(SessionStatus::InputNeeded.bits() | SessionStatus::IsRead.bits()),
            Activity::Waiting
        );
    }

    #[test]
    fn a_session_is_found_by_its_uri_its_id_or_the_start_of_it() {
        let sessions = [summary("abc123", "one"), summary("abd456", "two")];
        let title = |query| find_session(&sessions, query).map(|s| s.title.as_str());
        assert_eq!(title("ahp-session:/abc123"), Ok("one"));
        assert_eq!(title("abd"), Ok("two"));
        assert!(title("ab").unwrap_err().contains("2 sessions"));
        assert!(title("zzz").is_err());
        assert!(title("").is_err());
    }

    /// Each agent's own folder, as `folder` in `agents.toml` set it and the
    /// root state published it. An agent naming none is the client's to place.
    #[test]
    fn each_agent_can_start_in_its_own_folder() {
        let root_meta = serde_json::from_value(json!({
            "otto": {
                "folders": {
                    "claude": "file:///home/me/dev",
                    "codex": "file:///home/me/My%20Projects",
                    // Not a path, so not a folder: it would otherwise be
                    // taken as one relative to nothing.
                    "hermes": "file://relative/dir",
                }
            }
        }))
        .expect("the root's meta");
        let folders = folders_from_meta(Some(&root_meta));

        assert_eq!(folders.get("claude"), Some(&PathBuf::from("/home/me/dev")));
        assert_eq!(
            folders.get("codex"),
            Some(&PathBuf::from("/home/me/My Projects")),
            "percent-encoding undone"
        );
        assert_eq!(folders.get("hermes"), None);
        assert_eq!(folders.get("pi"), None, "no folder of its own");
        assert!(folders_from_meta(None).is_empty());
    }

    /// The card's material follows the agent: the one picked before a request,
    /// the one the request went to after, and the one an opened session
    /// belongs to. Agents without a colour, and services that publish none,
    /// leave the card on the plain material.
    #[test]
    fn the_material_follows_the_agent() {
        let mut ask = offline();
        ask.apply(Update::Agents(vec![
            agent_with_skills("claude", &[]),
            agent_with_skills("hermes", &[]),
            agent_with_skills("pi", &[]),
        ]));
        assert_eq!(ask.colour(None), None, "no colours published yet");

        let root_meta = serde_json::from_value(json!({
            "otto": { "colours": { "claude": "orange", "hermes": "violet" } }
        }))
        .expect("the root's meta");
        ask.apply(Update::Colours(colours_from_meta(Some(&root_meta))));
        assert_eq!(ask.colour(None), Some("orange"), "the default agent's");
        assert_eq!(ask.colour(Some(1)), Some("violet"), "the picked agent's");
        assert_eq!(ask.colour(Some(2)), None, "pi has no colour");

        ask.send("hello", Some(1));
        assert_eq!(ask.colour(Some(0)), Some("violet"), "settled once sent");

        let mut ask = offline();
        ask.apply(Update::Colours(colours_from_meta(Some(&root_meta))));
        ask.resume("ahp-session:/abc");
        assert_eq!(ask.colour(None), None, "unknown until the service says");
        ask.apply(Update::Provider("claude".into()));
        assert_eq!(ask.colour(None), Some("orange"));

        assert!(colours_from_meta(None).is_empty());
    }

    #[test]
    fn a_listed_session_carries_the_terminal_the_catalogue_gave_it() {
        let mut ask = offline();
        let mut with_meta = summary("abc", "Fix the build");
        with_meta.meta = serde_json::from_value(json!({
            "otto": { "terminal": { "command": ["ghostty", "-e", "claude"], "cwd": "/home/me" } }
        }))
        .expect("the session's meta");
        ask.apply(Update::Sessions(vec![with_meta, summary("def", "")]));
        assert_eq!(
            ask.terminal_at(0),
            Some(Terminal {
                command: vec!["ghostty".into(), "-e".into(), "claude".into()],
                cwd: PathBuf::from("/home/me"),
                // A service that says no id still opens the session.
                session: None,
                title: "Ask: Fix the build".into(),
            })
        );
        // A session the service has said nothing about opens nowhere.
        assert_eq!(ask.terminal_at(1), None);
        assert_eq!(ask.terminal_at(2), None);
    }

    #[test]
    fn sessions_are_listed_by_title_with_their_folder() {
        let mut ask = offline();
        ask.apply(Update::Sessions(vec![
            summary("abc", "Fix the build"),
            summary("def", ""),
        ]));
        let home = Path::new("/home/me");
        assert_eq!(
            session_subtitle(&ask.sessions[0], Some("Claude"), Some(home)),
            format!(
                "@Claude · {} · ~/My Projects",
                otto_kit::t_owned!("launcher-agents-idle")
            )
        );
        // An agent the service did not name is still named, by its id.
        assert_eq!(
            session_subtitle(&ask.sessions[0], None, Some(home)),
            format!(
                "@claude · {} · ~/My Projects",
                otto_kit::t_owned!("launcher-agents-idle")
            )
        );
        let rows = ask.session_rows(0, "build");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Fix the build");
        assert_eq!(ask.session_rows(0, "").len(), 2);

        assert!(ask.resume_at(1));
        assert!(ask.running());
    }

    fn pump_until(ask: &mut Ask, done: impl Fn(&Ask) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            ask.pump();
            if done(ask) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Against a live service: `otto-agents serve --echo`, with `OTTO_AGENTS_URL`
    /// pointing at it when it is not on the default port.
    #[test]
    #[ignore = "needs a running `otto-agents serve --echo`"]
    fn follows_a_conversation_to_its_answers() {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| default_url());
        let mut ask = Ask::connect(url, std::env::temp_dir());

        assert!(pump_until(&mut ask, |ask| !ask.agents.is_empty()));
        assert!(
            ask.agent_rows(0).is_empty(),
            "one agent is not a choice, so there is no list"
        );

        let answered = |count: usize, word: &'static str| {
            move |ask: &Ask| {
                ask.transcript().is_some_and(|t| {
                    t.status.is_none()
                        && t.entries.len() == count
                        && said(&t.entries[count - 1]).contains(word)
                })
            }
        };
        // The second request is sent straight after the first, so it queues.
        ask.send("hello launcher", None);
        ask.send("and again", None);
        let done = pump_until(&mut ask, answered(2, "again"));
        let transcript = ask.transcript();
        assert!(done, "the conversation never finished: {transcript:?}");
        assert!(said(&transcript.unwrap().entries[0]).contains("launcher"));
        assert!(!ask.handing_off());
    }

    /// Against a live service, like the test above.
    #[test]
    #[ignore = "needs a running `otto-agents serve --echo`"]
    fn opens_a_session_with_its_attachments_and_carries_it_on() {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| default_url());
        let mut first = Ask::connect(url.clone(), std::env::temp_dir());
        first.attach([PathBuf::from("/tmp/notes.md")]);
        first.send("resume me", None);
        assert!(pump_until(&mut first, |ask| ask.transcript().is_some_and(
            |t| t.status.is_none() && said(&t.entries[0]).contains("resume")
        )));
        drop(first);

        // The newest session is the one just made.
        let mut second = Ask::connect(url, std::env::temp_dir());
        assert!(pump_until(&mut second, Ask::sessions_listed));
        assert!(second.resume_at(0));
        assert!(pump_until(&mut second, |ask| ask
            .transcript()
            .is_some_and(|t| t.status.is_none() && !t.entries.is_empty())));
        let entries = second.transcript().unwrap().entries;
        assert_eq!(entries[0].prompt, "resume me");
        assert_eq!(
            entries[0].attachments,
            [Attachment::File("/tmp/notes.md".into())]
        );

        second.send("carried on", None);
        let done = pump_until(&mut second, |ask| {
            ask.transcript().is_some_and(|t| {
                t.status.is_none()
                    && t.entries.len() == 2
                    && said(&t.entries[1]).contains("carried")
            })
        });
        assert!(done, "{:?}", second.transcript());
    }
}
