//! Agent backends: what actually runs a session's turns.
//!
//! The host owns AHP state and never talks to agents directly. For each session
//! it hands a backend a command channel and an event channel, and turns the
//! events it gets back into AHP actions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ahp_types::state::{AgentInfo, ChatInputAnswer, ChatInputRequest, ChatInputResponseKind};
use tokio::sync::{mpsc, oneshot};

use crate::dialog::Prompt;

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
}

/// A resource attached to a prompt, by reference: the agent reads it itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// What a person calls it, such as the file's name.
    pub name: String,
    pub uri: String,
}

/// Something a session reports back to the host.
#[derive(Debug)]
pub enum SessionEvent {
    /// The agent is up, with its own id for the session when it has one.
    Ready {
        agent_session: Option<String>,
    },
    CreationFailed(String),
    MessageChunk {
        turn_id: String,
        text: String,
    },
    ThoughtChunk {
        turn_id: String,
        text: String,
    },
    TurnEnded {
        turn_id: String,
        outcome: TurnOutcome,
    },
    /// The agent needs permission before it uses a tool, and waits for the
    /// decision sent on `reply`. Dropping `reply` denies.
    PermissionRequested {
        turn_id: String,
        question: Question,
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
}

/// One answer an agent offers to a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    /// Whether choosing it lets the tool run.
    pub allow: bool,
}

/// The answer to a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub approved: bool,
    /// The option chosen, when the answer came with one. Without it, the
    /// agent's narrowest option for `approved` is used.
    pub option_id: Option<String>,
}

impl Decision {
    pub fn deny() -> Self {
        Self {
            approved: false,
            option_id: None,
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

    /// Starts a session in the background. The session ends when `commands`
    /// is closed; it reports through `events` until then.
    fn start(
        &self,
        spec: SessionSpec,
        commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    );

    /// The command that opens `agent_session`, the agent's own id for a
    /// session of `provider`, in a terminal in `cwd`. `None` when there is no
    /// way to, which is the default.
    fn terminal(&self, _provider: &str, _agent_session: &str, _cwd: &Path) -> Option<Vec<String>> {
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
/// word. Used by tests, and by `otto-agentsd serve --echo` to try clients without
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
                    // Echo turns finish before a cancel can arrive.
                    SessionCommand::Cancel { .. } => {}
                }
            }
        });
    }
}
