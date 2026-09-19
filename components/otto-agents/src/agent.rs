//! Agent backends: what actually runs a session's turns.
//!
//! The host owns AHP state and never talks to agents directly. For each session
//! it hands a backend a command channel and an event channel, and turns the
//! events it gets back into AHP actions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::PermissionOptionKind;
use ahp_types::state::{AgentInfo, ChatInputAnswer, ChatInputRequest, ChatInputResponseKind};
use tokio::sync::{mpsc, oneshot};

use crate::dialog::Prompt;
use crate::images::SharedImage;

/// What the host needs to start a session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// `AgentInfo.provider` of the agent to run.
    pub provider: String,
    /// The session's working directory.
    pub cwd: PathBuf,
    /// The agent's id for a session it ran before, to take up again rather
    /// than start afresh.
    pub resume: Option<String>,
}

/// A command the host sends to a running session.
#[derive(Debug)]
pub enum SessionCommand {
    Prompt {
        turn_id: String,
        text: String,
        /// Resources the message points the agent at, such as files.
        attachments: Vec<Attachment>,
    },
    Cancel {
        turn_id: String,
    },
    /// Switch the agent to one of the modes it advertised; see
    /// [`SessionEvent::ModesChanged`].
    SetMode {
        mode_id: String,
    },
}

/// A resource attached to a prompt, by reference: the agent reads it itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// What a person calls it, such as the file's name.
    pub name: String,
    pub uri: String,
}

/// One turn of an agent's own history, as `session/load` replays it.
///
/// The replay carries no turn boundaries — only a stream of message chunks and
/// tool calls — so a turn is everything between one user message and the next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryTurn {
    /// What the person asked.
    pub prompt: String,
    /// What the agent said back, in stream order.
    pub parts: Vec<HistoryPart>,
}

/// A piece of what an agent said in a [`HistoryTurn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryPart {
    Message(String),
    Thought(String),
    /// A picture the agent sent, as a file; see [`crate::images`].
    Image(SharedImage),
    /// A tool the agent used, and whether it worked.
    ToolCall {
        tool_call_id: String,
        title: String,
        success: bool,
    },
}

/// Something a session reports back to the host.
#[derive(Debug)]
pub enum SessionEvent {
    /// The agent is up, with its own id for the session when it has one.
    Ready {
        agent_session: Option<String>,
    },
    /// The agent's own history, as `session/load` replayed it. The chat is
    /// rebuilt from it, so turns made while the host was away — in a terminal,
    /// or by another client — come back too.
    HistoryLoaded {
        turns: Vec<HistoryTurn>,
    },
    CreationFailed(String),
    /// The agent's modes — its own permission and sandboxing presets, such as
    /// Claude's "Accept edits" or Codex's "read-only" — and which one it is
    /// in. Sent when the session opens, and again whenever the current mode
    /// changes, from either side.
    ModesChanged {
        current: String,
        available: Vec<Mode>,
    },
    MessageChunk {
        turn_id: String,
        text: String,
    },
    ThoughtChunk {
        turn_id: String,
        text: String,
    },
    /// A picture the agent sent mid-answer, already written to a file. It takes
    /// its place in the answer where it arrived, between what was said before
    /// it and what comes after.
    MessageImage {
        turn_id: String,
        image: SharedImage,
    },
    TurnEnded {
        turn_id: String,
        outcome: TurnOutcome,
    },
    /// The agent needs permission before it uses a tool, and waits for the
    /// decision sent on `reply`. Dropping `reply` denies.
    PermissionRequested {
        turn_id: String,
        /// Boxed: it carries the tool's input and edits, and would otherwise
        /// size every event.
        question: Box<Question>,
        reply: oneshot::Sender<Decision>,
    },
    /// The agent asks the user something, such as Claude's AskUserQuestion or
    /// an MCP server's form, and waits for the answer sent on `reply`.
    /// Dropping `reply` cancels the question.
    InputRequested {
        turn_id: String,
        /// The questions, as the chat shows them. The host keeps `id`.
        request: ChatInputRequest,
        reply: oneshot::Sender<InputAnswer>,
    },
    /// A tool call the agent was given permission for has finished.
    ToolCallFinished {
        turn_id: String,
        tool_call_id: String,
        success: bool,
    },
}

/// A mode an agent can run in, as it describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// A permission an agent asks for mid-turn.
#[derive(Debug, Clone)]
pub struct Question {
    /// The agent's id for the tool call, which the host uses as the AHP one.
    pub tool_call_id: String,
    /// What kind of tool it is, such as `execute` or `edit`.
    pub tool_name: String,
    /// The question as a dialog asks it.
    pub prompt: Prompt,
    /// The answers the agent offers, in its order.
    pub options: Vec<QuestionOption>,
    /// The option to start on: the narrowest allow, or the narrowest reject
    /// when the agent marked the request `defaultToNo`. Clients take it
    /// rather than pick their own.
    pub default_option_id: Option<String>,
    /// What the tool call would touch, as JSON: `{"path": …, "rawInput": …}`
    /// with whichever of the two the agent sent.
    pub tool_input: Option<serde_json::Value>,
    /// The file edits the tool call would make, as a JSON list of
    /// `{"path", "oldText", "newText"}`. Empty when it makes none, or the
    /// agent sent no diff.
    pub edits: Vec<serde_json::Value>,
}

/// One answer an agent offers to a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    /// What choosing it does: allows or rejects, once or always.
    pub kind: PermissionOptionKind,
}

impl QuestionOption {
    /// Whether choosing it lets the tool run.
    pub fn allow(&self) -> bool {
        matches!(
            self.kind,
            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
        )
    }
}

/// The answer to a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Allowed, with the option chosen when the answer came with one. Without
    /// it, the agent's narrowest allowing option is used.
    Approve(Option<String>),
    /// Refused, likewise.
    Deny(Option<String>),
    /// The turn was cancelled while the question waited, so there is no
    /// answer to give.
    Cancelled,
}

impl Decision {
    pub fn deny() -> Self {
        Self::Deny(None)
    }

    /// Whether the tool may run.
    pub fn approved(&self) -> bool {
        matches!(self, Self::Approve(_))
    }

    /// The option the answer named, if it named one.
    pub fn option_id(&self) -> Option<&str> {
        match self {
            Self::Approve(option) | Self::Deny(option) => option.as_deref(),
            Self::Cancelled => None,
        }
    }
}

/// The answer to a [`SessionEvent::InputRequested`].
#[derive(Debug, Clone, PartialEq)]
pub struct InputAnswer {
    pub response: ChatInputResponseKind,
    /// The answers by question id; only submitted ones count.
    pub answers: HashMap<String, ChatInputAnswer>,
}

impl InputAnswer {
    pub fn decline() -> Self {
        Self {
            response: ChatInputResponseKind::Decline,
            answers: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub enum TurnOutcome {
    Complete,
    Cancelled,
    Failed(String),
}

pub trait Backend: Send + Sync + 'static {
    /// The agents this backend can run, as published in `RootState.agents`.
    fn agents(&self) -> Vec<AgentInfo>;

    /// The name of the frosted material the agent `provider`'s surfaces wear,
    /// as `agents.toml` gives it. `None`, the default, keeps them on the
    /// desktop's plain material.
    fn colour(&self, _provider: &str) -> Option<&'static str> {
        None
    }

    /// The folder the agent `provider`'s sessions start in when the client
    /// picks none, as `agents.toml` gives it. `None`, the default, leaves the
    /// choice to the client.
    fn folder(&self, _provider: &str) -> Option<&Path> {
        None
    }

    /// The icon name dialogs from the agent `provider` wear — Otto's own, for
    /// the agent running as the desktop's helper. `None`, the default, leaves
    /// each dialog the icon for the kind of thing it asks about.
    fn icon(&self, _provider: &str) -> Option<&'static str> {
        None
    }

    /// Starts a session in the background. The session ends when `commands`
    /// is closed; it reports through `events` until then.
    fn start(
        &self,
        spec: SessionSpec,
        commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    );

    /// The command that opens `agent_session`, the agent's own id for a
    /// session of `provider`, in a terminal in `cwd`. `written` says whether
    /// the agent has a history for the session; a harness enters one it has
    /// not written yet its own way. `None` when there is no way to, which is
    /// the default.
    fn terminal(
        &self,
        _provider: &str,
        _agent_session: &str,
        _cwd: &Path,
        _written: bool,
    ) -> Option<Vec<String>> {
        None
    }

    /// The same command without a terminal around it: what `otto-agents new`
    /// and `otto-agents enter` run in the terminal they are already in.
    fn enter(
        &self,
        _provider: &str,
        _agent_session: &str,
        _cwd: &Path,
        _written: bool,
    ) -> Option<Vec<String>> {
        None
    }
}

pub fn agent_info(provider: &str, display_name: &str, description: &str) -> AgentInfo {
    AgentInfo {
        provider: provider.to_owned(),
        display_name: display_name.to_owned(),
        description: description.to_owned(),
        models: Vec::new(),
        protected_resources: None,
        customizations: None,
        capabilities: None,
    }
}

/// A backend with one agent, `echo`, that streams every prompt back word by
/// word. Used by tests, and by `otto-agents serve --echo` to try clients without
/// a real agent.
pub struct EchoBackend;

impl Backend for EchoBackend {
    fn agents(&self) -> Vec<AgentInfo> {
        vec![agent_info("echo", "Echo", "Repeats every prompt back")]
    }

    fn start(
        &self,
        _spec: SessionSpec,
        mut commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) {
        tokio::spawn(async move {
            let _ = events.send(SessionEvent::Ready {
                agent_session: None,
            });
            while let Some(command) = commands.recv().await {
                match command {
                    SessionCommand::Prompt { turn_id, text, .. } => {
                        for word in text.split_inclusive(' ') {
                            let _ = events.send(SessionEvent::MessageChunk {
                                turn_id: turn_id.clone(),
                                text: word.to_owned(),
                            });
                        }
                        let _ = events.send(SessionEvent::TurnEnded {
                            turn_id,
                            outcome: TurnOutcome::Complete,
                        });
                    }
                    // Echo turns finish before a cancel can arrive, and echo
                    // has no modes to switch between.
                    SessionCommand::Cancel { .. } | SessionCommand::SetMode { .. } => {}
                }
            }
        });
    }
}
