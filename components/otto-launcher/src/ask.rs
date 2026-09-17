//! Asking an agent.
//!
//! In ask mode, what is typed is a request for an agent rather than a search.
//! Return hands it to otto-agentsd, the agent service, and the launcher becomes a
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

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use ahp::reducers::apply_action_to_chat;
use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::actions::{
    ChatPendingMessageSetAction, ChatToolCallConfirmedAction, ChatTurnCancelledAction, StateAction,
};
use ahp_types::commands::ListSessionsResult;
use ahp_types::common::StringOrMarkdown;
use ahp_types::state::{
    AgentInfo, ChatState, ChildCustomization, ConfirmationOptionKind, Customization, Message,
    MessageAttachment, MessageKind, MessageOrigin, MessageResourceAttachment, PendingMessageKind,
    ResponsePart, SessionStatus, SessionSummary, SnapshotState, ToolCallConfirmationReason,
    ToolCallState, TurnState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use serde_json::{json, Value};
use tokio::sync::mpsc as async_mpsc;

use crate::source::{Activity, Item, Origin};

/// Where otto-agentsd listens unless `OTTO_AGENTS_URL` says otherwise.
const DEFAULT_URL: &str = "ws://127.0.0.1:4800";

const SESSION_SCHEME: &str = "ahp-session:/";

/// Connecting is one round trip to a local service. Past this, the service is
/// not answering, and saying so beats a launcher that looks like it is.
const TIMEOUT: Duration = Duration::from_secs(3);

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// What the connection thread reports.
#[derive(Debug)]
enum Update {
    /// The agents the service offers.
    Agents(Vec<AgentInfo>),
    /// The sessions the service has, most recently changed first.
    Sessions(Vec<SessionSummary>),
    /// The service could not be reached.
    Unreachable(String),
    /// How to open the followed session in a terminal, as the service says.
    Terminal(Option<Terminal>),
    /// The session's chat, as it stood when the launcher subscribed to it.
    Chat(Box<ChatState>),
    /// A change to that chat.
    Action(Box<StateAction>),
    /// The service has one more of the requests.
    HandedOff,
    /// A request, or the session behind them, failed.
    Failed(String),
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
    /// An answer to the agent's question.
    Confirm {
        turn_id: String,
        tool_call_id: String,
        approved: bool,
        option_id: String,
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
}

impl Status {
    pub fn text(&self) -> String {
        match self {
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
    /// The agent's answers, in its order.
    pub choices: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    /// Whether choosing it lets the tool run.
    pub approve: bool,
}

impl Question {
    /// The choice to offer first: the narrowest one that allows, since the
    /// agent lists "always" before "once".
    pub fn default_choice(&self) -> usize {
        self.choices
            .iter()
            .rposition(|choice| choice.approve)
            .unwrap_or(0)
    }
}

/// A skill an agent has, as the service published it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillRef {
    pub name: String,
    /// One sentence saying when the skill applies. Empty when the skill's
    /// frontmatter carries none.
    pub description: String,
}

/// A request as it was sent: what was typed, and the names of the files that
/// went with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub prompt: String,
    pub attachments: Vec<String>,
}

/// The line that names the files going with a request, when there are any.
pub fn attached_text(attachments: &[String]) -> Option<String> {
    (!attachments.is_empty())
        .then(|| otto_kit::t_owned!("launcher-ask-attached", files = attachments.join(", ")))
}

/// One request and what came of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub prompt: String,
    /// The names of the files that went with the request.
    pub attachments: Vec<String>,
    pub answer: String,
    /// The tool calls that were allowed or refused, in order.
    pub steps: Vec<Step>,
    /// The agent's question, while it waits for an answer.
    pub question: Option<Question>,
    pub note: Option<Note>,
}

/// The conversation so far, oldest first, and what the agent is doing now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
}

/// The requests made so far.
struct Run {
    /// The agent's name, for the status line while it starts.
    agent: Option<String>,
    /// Every request sent, in order. An opened session's earlier requests
    /// come first, once its chat arrives.
    sent: Vec<Request>,
    /// How many of them the service has confirmed.
    handed_off: usize,
    /// Questions answered from the launcher, hidden before the service says so.
    answered: Vec<String>,
    chat: Option<ChatState>,
    failure: Option<String>,
    /// Whether the session was already there, opened rather than created.
    resumed: bool,
    /// How to open the session in a terminal, once the service says.
    terminal: Option<Terminal>,
}

/// A command that opens the session in a terminal, with the agent's own
/// interface: otto-agentsd publishes it in the session's `_meta`, under
/// `otto.terminal`, once the agent's id for the session is known and a
/// terminal is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub command: Vec<String>,
    pub cwd: PathBuf,
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
        Some(Self { command, cwd })
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
            .args(args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map(drop)
    }
}

pub struct Ask {
    commands: async_mpsc::UnboundedSender<Command>,
    updates: mpsc::Receiver<Update>,
    wake: UnixStream,
    agents: Vec<AgentInfo>,
    /// The service's sessions, as they stood when the launcher connected.
    sessions: Vec<SessionSummary>,
    sessions_listed: bool,
    /// The files that go with the next request.
    attachments: Vec<PathBuf>,
    unreachable: Option<String>,
    run: Option<Run>,
}

impl Ask {
    /// Connect to otto-agentsd in the background. Sessions are created in `$HOME`.
    pub fn open() -> Self {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
        let folder = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self::connect(url, folder)
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
            .name("otto-agentsd".into())
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
            .expect("cannot start the otto-agentsd connection thread");

        Self {
            commands,
            updates,
            wake,
            agents: Vec::new(),
            sessions: Vec::new(),
            sessions_listed: false,
            attachments: Vec::new(),
            unreachable: None,
            run: None,
        }
    }

    /// Attach `files` to the next request.
    pub fn attach(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        self.attachments.extend(
            files
                .into_iter()
                .map(|file| std::path::absolute(&file).unwrap_or(file)),
        );
    }

    /// The names of the files that go with the next request.
    pub fn attachments(&self) -> Vec<String> {
        self.attachments
            .iter()
            .map(|file| file_label(file))
            .collect()
    }

    /// The files that go with the next request, as rows: each file's name,
    /// over the folder it is in.
    pub fn attachment_rows(&self, source: usize) -> Vec<Item> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        self.attachments
            .iter()
            .enumerate()
            .map(|(index, file)| Item {
                title: file_label(file),
                subtitle: file
                    .parent()
                    .map(|folder| home_relative(folder, home.as_deref())),
                icon: Some(
                    if file.is_dir() {
                        "folder"
                    } else {
                        "text-x-generic"
                    }
                    .to_string(),
                ),
                activity: None,
                search_terms: Vec::new(),
                origin: Origin { source, index },
            })
            .collect()
    }

    /// How to open the session in a terminal, once there is a session and the
    /// service has said.
    pub fn terminal(&self) -> Option<&Terminal> {
        self.run.as_ref()?.terminal.as_ref()
    }

    /// How to open the session at `index` in the list in a terminal, from the
    /// `_meta` the catalogue carries. `None` while the service has not said —
    /// the agent's id for a session it has just started, say.
    pub fn terminal_at(&self, index: usize) -> Option<Terminal> {
        Terminal::from_meta(self.sessions.get(index)?.meta.as_ref())
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
            sent: Vec::new(),
            handed_off: 0,
            answered: Vec::new(),
            chat: None,
            failure: self.unreachable.clone(),
            resumed: true,
            terminal: None,
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
            Update::Terminal(terminal) => {
                if let Some(run) = self.run.as_mut() {
                    run.terminal = terminal;
                }
            }
            Update::Sessions(sessions) => {
                self.sessions = sessions;
                self.sessions_listed = true;
            }
            Update::Unreachable(error) => {
                tracing::warn!(%error, "otto-agentsd is unreachable");
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
                }
            }
            Update::Action(action) => {
                if let Some(chat) = self.run.as_mut().and_then(|run| run.chat.as_mut()) {
                    apply_action_to_chat(chat, &action);
                }
            }
            Update::HandedOff => {
                if let Some(run) = self.run.as_mut() {
                    run.handed_off += 1;
                }
                tracing::info!("request handed to otto-agentsd");
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

    /// Why the service cannot be asked anything, before a request is made.
    pub fn unreachable(&self) -> Option<&str> {
        self.unreachable.as_deref()
    }

    /// Hand `prompt` to the agent, with the files attached so far. The first
    /// request starts a session with the agent at `agent` in the list, or the
    /// service's default agent; later ones queue on the same session, and
    /// `agent` is ignored.
    pub fn send(&mut self, prompt: &str, agent: Option<usize>) {
        let attachments = std::mem::take(&mut self.attachments);
        let request = Request {
            prompt: prompt.to_string(),
            attachments: attachments.iter().map(|file| file_label(file)).collect(),
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
                    sent: vec![request],
                    handed_off: 0,
                    answered: Vec::new(),
                    chat: None,
                    // An unreachable service fails the request at once, rather
                    // than leaving it to look as if it is starting.
                    failure: self.unreachable.clone(),
                    resumed: false,
                    terminal: None,
                });
                chosen.map(|agent| agent.provider.clone())
            }
        };
        let _ = self.commands.send(Command::Ask {
            prompt: prompt.to_string(),
            provider,
            attachments,
        });
    }

    /// Whether a request has been made, and the launcher is showing the log.
    pub fn running(&self) -> bool {
        self.run.is_some()
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
            .filter_map(|attachment| match attachment {
                MessageAttachment::Simple(a) => Some(a.label.clone()),
                MessageAttachment::EmbeddedResource(a) => Some(a.label.clone()),
                MessageAttachment::Resource(a) => Some(a.label.clone()),
                MessageAttachment::Annotations(a) => Some(a.label.clone()),
                MessageAttachment::Chat(a) => Some(a.label.clone()),
                MessageAttachment::Unknown(_) => None,
            })
            .collect(),
    }
}

/// What a file is called in the log: its name.
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

/// The path a `file://` URI names, undoing its percent-encoding.
fn path_from_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        let hex = encoded
            .get(index + 1..index + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (encoded[index], hex) {
            (b'%', Some(byte)) => {
                bytes.push(byte);
                index += 3;
            }
            (byte, _) => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// Picks the session `query` names: its URI, its id, or the start of its id.
fn find_session<'a>(
    sessions: &'a [SessionSummary],
    query: &str,
) -> Result<&'a SessionSummary, String> {
    let id = query.strip_prefix(SESSION_SCHEME).unwrap_or(query);
    let matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|session| {
            session
                .resource
                .strip_prefix(SESSION_SCHEME)
                .is_some_and(|candidate| !id.is_empty() && candidate.starts_with(id))
        })
        .collect();
    match matches.as_slice() {
        [session] => Ok(session),
        [] => Err(format!("no session matches `{query}`")),
        many => Err(format!(
            "`{query}` matches {} sessions; give more of the id",
            many.len()
        )),
    }
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
                note,
            });
        }
        if let Some(turn) = &chat.active_turn {
            let (steps, question) = tool_calls(&turn.id, &turn.response_parts);
            let question = question.filter(|question| !answered.contains(&question.tool_call_id));
            let thinking = matches!(turn.response_parts.last(), Some(ResponsePart::Reasoning(_)));
            status = Some(if question.is_some() {
                Status::Waiting
            } else if thinking {
                Status::Thinking
            } else {
                Status::Working
            });
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
            answer: String::new(),
            steps: Vec::new(),
            question: None,
            note,
        });
    }

    if let Some(error) = failure {
        status = Some(Status::Failed(error.to_string()));
    }
    Transcript { entries, status }
}

/// The answer in `parts`: its markdown, without the reasoning.
fn answer(parts: &[ResponsePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ResponsePart::Markdown(markdown) => Some(markdown.content.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
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
            reporter.send(Update::Unreachable(format!("otto-agentsd at {url}: {err}")));
            return;
        }
        Err(_) => {
            reporter.send(Update::Unreachable(format!(
                "otto-agentsd at {url} did not answer"
            )));
            return;
        }
    };

    match client.subscribe(ROOT_RESOURCE_URI.to_string()).await {
        Ok((subscribed, _root)) => {
            if let Some(SnapshotState::Root(root)) = subscribed.snapshot.map(|s| s.state) {
                reporter.send(Update::Agents(root.agents));
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
                let handed =
                    hand_off(&client, &prompt, &attachments, provider, folder, reporter).await;
                if handed.is_ok() {
                    reporter.send(Update::HandedOff);
                }
                break handed;
            }
            Some(Command::Resume { session }) => break resume(&client, &session, reporter).await,
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
                    Command::Resume { .. } | Command::Stop { .. } => continue,
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
                    }
                    _ => {}
                },
                Some(_) => {}
                None => {
                    reporter.send(Update::Failed("otto-agentsd closed the connection".into()));
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
                    Some(reason) => reporter.send(Update::Failed(reason)),
                    None => reporter.send(Update::Action(Box::new(envelope.action))),
                },
                Some(_) => {}
                None => {
                    reporter.send(Update::Failed("otto-agentsd closed the connection".into()));
                    break;
                }
            },
        }
    }
    client.shutdown().await;
}

async fn connect(url: &str) -> Result<Client, BoxError> {
    let transport = ahp_ws::WebSocketTransport::connect(url).await?;
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
            reporter.send(Update::Terminal(Terminal::from_meta(state.meta.as_ref())));
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

/// `path` as a `file://` URI, percent-encoding everything but unreserved
/// characters and slashes.
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
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

    fn entry(prompt: &str, answer: &str, note: Option<Note>) -> Entry {
        Entry {
            prompt: prompt.into(),
            attachments: Vec::new(),
            answer: answer.into(),
            steps: Vec::new(),
            question: None,
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

        // Answered here, it is not offered again while the service catches up.
        let answered = transcript(Some(&chat), &prompts, &["call-1".into()], None, None);
        assert!(answered.question().is_none());
        assert_eq!(answered.status, Some(Status::Working));
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

    /// An agent publishing `skills` as a plugin's children, the way otto-agentsd
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
    fn files_go_with_the_next_request_only() {
        let mut ask = offline();
        ask.attach([
            PathBuf::from("/home/me/My Notes.md"),
            PathBuf::from("/tmp/a.png"),
        ]);
        assert_eq!(ask.attachments(), ["My Notes.md", "a.png"]);
        let rows = ask.attachment_rows(1);
        let shown: Vec<(&str, Option<&str>)> = rows
            .iter()
            .map(|row| (row.title.as_str(), row.subtitle.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [("My Notes.md", Some("/home/me")), ("a.png", Some("/tmp"))]
        );
        assert!(rows.iter().all(|row| row.origin.source == 1));

        ask.send("summarise", None);
        ask.send("and again", None);
        assert!(ask.attachments().is_empty());
        assert!(ask.attachment_rows(1).is_empty());
        let entries = ask.transcript().expect("a conversation").entries;
        assert_eq!(entries[0].attachments, ["My Notes.md", "a.png"]);
        assert!(entries[1].attachments.is_empty());
    }

    /// otto-agentsd says how to open a session in a terminal in the session's
    /// `_meta`; anything short of a command and a folder is no way to.
    #[test]
    fn a_session_says_how_to_open_it_in_a_terminal() {
        let meta = |value: Value| value.as_object().cloned();
        let terminal = meta(json!({
            "otto": { "terminal": {
                "command": ["ghostty", "-e", "claude", "--resume", "abc"],
                "cwd": "/home/me",
            }}
        }));
        assert_eq!(
            Terminal::from_meta(terminal.as_ref()),
            Some(Terminal {
                command: ["ghostty", "-e", "claude", "--resume", "abc"]
                    .map(String::from)
                    .to_vec(),
                cwd: PathBuf::from("/home/me"),
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
                    attachments: vec!["notes.md".into()],
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

    /// Against a live service: `otto-agentsd serve --echo`, with `OTTO_AGENTS_URL`
    /// pointing at it when it is not on the default port.
    #[test]
    #[ignore = "needs a running `otto-agentsd serve --echo`"]
    fn follows_a_conversation_to_its_answers() {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| DEFAULT_URL.into());
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
                        && t.entries[count - 1].answer.contains(word)
                })
            }
        };
        // The second request is sent straight after the first, so it queues.
        ask.send("hello launcher", None);
        ask.send("and again", None);
        let done = pump_until(&mut ask, answered(2, "again"));
        let transcript = ask.transcript();
        assert!(done, "the conversation never finished: {transcript:?}");
        assert!(transcript.unwrap().entries[0].answer.contains("launcher"));
        assert!(!ask.handing_off());
    }

    /// Against a live service, like the test above.
    #[test]
    #[ignore = "needs a running `otto-agentsd serve --echo`"]
    fn opens_a_session_with_its_attachments_and_carries_it_on() {
        let url = std::env::var("OTTO_AGENTS_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let mut first = Ask::connect(url.clone(), std::env::temp_dir());
        first.attach([PathBuf::from("/tmp/notes.md")]);
        first.send("resume me", None);
        assert!(pump_until(&mut first, |ask| ask.transcript().is_some_and(
            |t| t.status.is_none() && t.entries[0].answer.contains("resume")
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
        assert_eq!(entries[0].attachments, ["notes.md"]);

        second.send("carried on", None);
        let done = pump_until(&mut second, |ask| {
            ask.transcript().is_some_and(|t| {
                t.status.is_none()
                    && t.entries.len() == 2
                    && t.entries[1].answer.contains("carried")
            })
        });
        assert!(done, "{:?}", second.transcript());
    }
}
