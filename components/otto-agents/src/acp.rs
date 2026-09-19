//! ACP backend: each session runs its agent as a child process speaking the
//! Agent Client Protocol over stdio.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ClientCapabilities, ContentBlock,
    CreateElicitationRequest, CreateElicitationResponse, ElicitationAction,
    ElicitationCapabilities, ElicitationFormCapabilities, ElicitationMode, InitializeRequest,
    LoadSessionRequest, Meta, NewSessionRequest, PromptRequest, PromptResponse,
    RequestPermissionRequest, RequestPermissionResponse, ResourceLink, ResumeSessionRequest,
    SessionConfigOptionValue, SessionId, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason, TextContent, ToolCallContent,
    ToolCallStatus,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectTo, ConnectionTo};
use ahp_types::state::AgentInfo;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Backend, Decision, HistoryPart, HistoryTurn, Mode, SessionCommand, SessionEvent, SessionSpec,
    TurnOutcome, agent_info,
};
use crate::config::{self, AgentConfig, PermissionPolicy, SkillDelivery};
use crate::dialog;
use crate::elicitation;
use crate::images::{ImageCache, SharedImage};
use crate::skills::{self, Plugin};

pub struct AcpBackend {
    agents: Vec<AgentConfig>,
    /// The terminal sessions are opened in; see [`config::terminal_command`].
    terminal: Vec<String>,
    /// The desktop's skills, found once at startup. Agents configured with
    /// `skills = true` are told about them; see [`crate::skills`].
    plugins: Vec<Plugin>,
    /// Where pictures agents send are kept. `None` when there is nowhere to
    /// keep them, and pictures are then dropped.
    images: Option<ImageCache>,
}

impl AcpBackend {
    pub fn new(agents: Vec<AgentConfig>, terminal: Vec<String>) -> Self {
        Self::with_plugins(agents, terminal, skills::discover())
    }

    pub fn with_plugins(
        agents: Vec<AgentConfig>,
        terminal: Vec<String>,
        plugins: Vec<Plugin>,
    ) -> Self {
        for plugin in &plugins {
            tracing::info!(
                plugin = %plugin.name,
                skills = plugin.skills.len(),
                agents = plugin.agents.len(),
                dir = %plugin.dir.display(),
                "offering the desktop's skills to agents",
            );
        }
        Self {
            agents,
            terminal,
            plugins,
            images: ImageCache::open(),
        }
    }
}

/// How a session answers its agent's permission requests.
#[derive(Clone)]
pub struct Permissions {
    pub policy: PermissionPolicy,
    /// The agent's name, as questions about it say it.
    pub agent: String,
    /// The agent's own icon, as dialogs about it wear it. `None` leaves each
    /// dialog the icon for the kind of thing being asked about.
    pub icon: Option<&'static str>,
}

impl Backend for AcpBackend {
    fn agents(&self) -> Vec<AgentInfo> {
        self.agents
            .iter()
            .map(|agent| {
                let mut info = agent_info(&agent.id, &agent.name, &agent.description);
                // What this agent is given, where a client can see it. An
                // agent with `skills = false` publishes none, so the list is
                // the truth rather than the catalogue.
                info.customizations = agent
                    .skills
                    .enabled()
                    .then(|| skills::customizations(&self.plugins))
                    .filter(|published| !published.is_empty());
                info
            })
            .collect()
    }

    fn colour(&self, provider: &str) -> Option<&'static str> {
        self.agents
            .iter()
            .find(|agent| agent.id == provider)?
            .colour
            .map(|colour| colour.name())
    }

    fn folder(&self, provider: &str) -> Option<&Path> {
        self.agents
            .iter()
            .find(|agent| agent.id == provider)?
            .folder
            .as_deref()
    }

    fn icon(&self, provider: &str) -> Option<&'static str> {
        let agent = self.agents.iter().find(|agent| agent.id == provider)?;
        dialog::agent_icon(agent.agent.as_deref())
    }

    fn start(
        &self,
        spec: SessionSpec,
        commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) {
        let Some(agent) = self.agents.iter().find(|agent| agent.id == spec.provider) else {
            let _ = events.send(SessionEvent::CreationFailed(format!(
                "unknown agent: {}",
                spec.provider
            )));
            return;
        };
        let config = AcpAgentConfig::new(&agent.command)
            .args(crate::config::expand_args(&agent.args))
            .envs(crate::config::expand_env(&agent.env));
        let transport = AcpAgent::new(config);
        let permissions = Permissions {
            policy: agent.permissions,
            agent: agent.name.clone(),
            icon: dialog::agent_icon(agent.agent.as_deref()),
        };
        // Claude loads plugins itself, on every session it opens, resumed or
        // not: what it knows of them lives in the process, not the history.
        let run_as = agent.agent.as_deref().and_then(|name| {
            let found = skills::agent_file(&self.plugins, name);
            if found.is_none() {
                tracing::warn!(agent = name, "no plugin has an agent file by that name");
            } else if agent.skills != SkillDelivery::Claude {
                tracing::warn!(
                    agent = name,
                    "an agent file rides the Claude route only; set skills = \"claude\""
                );
            }
            if let Some((run_as, file)) = &found {
                tracing::info!(agent = %run_as, path = %file.path.display(), "running as a plugin agent");
            }
            found.map(|(run_as, _)| run_as)
        });
        let meta = match agent.skills {
            SkillDelivery::Claude => skills::claude_session_meta(&self.plugins, run_as.as_deref()),
            _ => None,
        };
        tokio::spawn(run_session(
            transport,
            permissions,
            agent.model.clone(),
            agent.mode.clone(),
            agent.config.clone(),
            spec.resume,
            spec.cwd,
            meta,
            self.images.clone(),
            commands,
            events,
        ));
    }

    fn terminal(
        &self,
        provider: &str,
        agent_session: &str,
        cwd: &Path,
        written: bool,
    ) -> Option<Vec<String>> {
        let agent = self.agents.iter().find(|agent| agent.id == provider)?;
        config::terminal_command(&self.terminal, agent, agent_session, cwd, written)
    }

    fn enter(
        &self,
        provider: &str,
        agent_session: &str,
        cwd: &Path,
        written: bool,
    ) -> Option<Vec<String>> {
        let agent = self.agents.iter().find(|agent| agent.id == provider)?;
        config::enter_command(agent, agent_session, cwd, written)
    }
}

type PendingPrompt =
    Pin<Box<dyn Future<Output = Result<PromptResponse, agent_client_protocol::Error>> + Send>>;

/// Drives one ACP session over `transport`: a child process in production,
/// an in-memory channel in tests. When `model` is set, the session switches to
/// it before it is ready, and fails to start if the agent refuses it. When
/// `mode` names one of the modes the agent advertises, a new session switches
/// to it before it is ready; a session taken up again keeps the mode it was
/// in, and an id the agent does not know is logged and ignored. `config`
/// carries the agent's own session options, set after the model; one the
/// agent refuses is logged and the session carries on. When
/// `resume` names a session the agent ran before, the agent takes it up again
/// if it can; otherwise a new session starts. `meta` goes with the request
/// that opens the session, whichever it is — see [`crate::skills`]. `images`
/// is where pictures the agent sends are kept; without one they are dropped.
#[allow(clippy::too_many_arguments)]
pub async fn run_session(
    transport: impl ConnectTo<Client> + 'static,
    permissions: Permissions,
    model: Option<String>,
    mode: Option<String>,
    config: BTreeMap<String, String>,
    resume: Option<String>,
    cwd: PathBuf,
    meta: Option<Meta>,
    images: Option<ImageCache>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    events: mpsc::UnboundedSender<SessionEvent>,
) {
    // The turn whose updates are being streamed, shared with the update handler.
    let current_turn: Arc<Mutex<Option<String>>> = Arc::default();
    // The modes the agent advertised when the session opened and the
    // current one, kept so a change can be reported with the whole list
    // and a refused mode withdrawn from it.
    let modes: Arc<Mutex<Modes>> = Arc::default();
    // Set while `session/load` replays the agent's history, so its updates go
    // to the replay; see [`Replay`].
    let replay: Arc<Mutex<Option<Replay>>> = Arc::default();
    // The pictures each tool call has already put in the answer. A tool call is
    // updated several times and may repeat its content, which would otherwise
    // show the same picture again on every update.
    let shown: Arc<Mutex<BTreeSet<(String, PathBuf)>>> = Arc::default();
    let ready = Arc::new(AtomicBool::new(false));

    let result = Client
        .builder()
        .name("otto-agents")
        .on_receive_notification(
            {
                let events = events.clone();
                let current_turn = Arc::clone(&current_turn);
                let replay = Arc::clone(&replay);
                let modes = Arc::clone(&modes);
                let images = images.clone();
                let shown = Arc::clone(&shown);
                async move |notification: SessionNotification, _connection| {
                    // A mode change belongs to the session, not to a turn or
                    // the replay: the agent switched itself, or someone did
                    // from its own interface.
                    if let SessionUpdate::CurrentModeUpdate(update) = &notification.update {
                        announce_modes(&events, &modes, update.current_mode_id.to_string());
                        return Ok(());
                    }
                    // No turn runs while the agent replays its history, so
                    // those updates belong to the replay.
                    if let Some(replay) = lock(&replay).as_mut() {
                        replay.take(notification.update);
                        return Ok(());
                    }
                    forward_update(
                        &current_turn,
                        &events,
                        images.as_ref(),
                        &shown,
                        notification.update,
                    );
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let cwd = cwd.clone();
                let events = events.clone();
                let current_turn = Arc::clone(&current_turn);
                async move |request: RequestPermissionRequest,
                            responder,
                            connection: ConnectionTo<Agent>| {
                    if permissions.policy != PermissionPolicy::Ask {
                        return responder.respond(answer_by_policy(permissions.policy, &request));
                    }
                    let Some(turn_id) = lock(&current_turn).clone() else {
                        return responder.respond(dialog::answer(&request, false));
                    };
                    // The host puts the question to the user: in the chat, for
                    // a client to answer, or in a dialog when nobody watches.
                    let question =
                        dialog::question_for(&permissions.agent, permissions.icon, &cwd, &request);
                    let (reply, decision) = oneshot::channel();
                    let asked = SessionEvent::PermissionRequested {
                        turn_id,
                        question: Box::new(question),
                        reply,
                    };
                    if events.send(asked).is_err() {
                        return responder.respond(dialog::answer(&request, false));
                    }
                    // The answer can take minutes, and a handler holds up every
                    // other message from the agent until it returns, so it is
                    // awaited in a task of its own.
                    connection.spawn(async move {
                        let decision = decision.await.unwrap_or_else(|_| Decision::deny());
                        responder.respond(dialog::decide(&request, &decision))
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let events = events.clone();
                let current_turn = Arc::clone(&current_turn);
                async move |request: CreateElicitationRequest,
                            responder,
                            connection: ConnectionTo<Agent>| {
                    let decline = || CreateElicitationResponse::new(ElicitationAction::Decline);
                    // Only forms are asked; a URL to open has nowhere to go.
                    let ElicitationMode::Form(mode) = &request.mode else {
                        return responder.respond(decline());
                    };
                    // A question outside a turn has nobody to answer it.
                    let Some(turn_id) = lock(&current_turn).clone() else {
                        return responder.respond(decline());
                    };
                    let schema = serde_json::to_value(&mode.requested_schema).unwrap_or_default();
                    let id = uuid::Uuid::new_v4().to_string();
                    let form = elicitation::form(&id, &request.message, &schema);
                    let (reply, answer) = oneshot::channel();
                    let asked = SessionEvent::InputRequested {
                        turn_id,
                        request: form.request.clone(),
                        reply,
                    };
                    if events.send(asked).is_err() {
                        return responder.respond(decline());
                    }
                    // Answered in a task of its own, like a permission request.
                    connection.spawn(async move {
                        let answer = answer.await.ok();
                        responder.respond(elicitation::respond(&form, answer))
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, {
            let events = events.clone();
            let current_turn = Arc::clone(&current_turn);
            let modes = Arc::clone(&modes);
            let ready = Arc::clone(&ready);
            let images = images.clone();
            async move |connection: ConnectionTo<Agent>| {
                // Forms are what the chat can ask; declaring them is what
                // turns Claude's AskUserQuestion on.
                let capabilities = ClientCapabilities::new().elicitation(
                    ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()),
                );
                let initialized = connection
                    .send_request(
                        InitializeRequest::new(ProtocolVersion::V1)
                            .client_capabilities(capabilities),
                    )
                    .block_task()
                    .await?;
                let Opened {
                    session_id,
                    history,
                    modes: advertised,
                    fresh,
                } = open_session(
                    &connection,
                    &initialized.agent_capabilities,
                    resume,
                    &cwd,
                    meta,
                    images.as_ref(),
                    &replay,
                )
                .await?;
                // The agent's own id for the session: for claude-agent-acp,
                // the Claude Code session, which `claude --resume` takes.
                tracing::info!(agent_session = %session_id.0, "agent session started");
                let current = advertised.map(|state| {
                    let (current, available) = modes_of(state);
                    lock(&modes).available = available;
                    announce_modes(&events, &modes, current.clone());
                    current
                });
                // The configured mode, on a new session only: one taken up
                // again keeps whatever it was last switched to, here or in
                // a terminal.
                if let (true, Some(mode)) = (fresh, mode) {
                    let known = lock(&modes).available.iter().any(|known| known.id == mode);
                    if !known {
                        tracing::warn!(%mode, "the agent offers no such mode; keeping its own");
                    } else if current.as_deref() != Some(mode.as_str()) {
                        let request = SetSessionModeRequest::new(session_id.clone(), mode.clone());
                        match connection.send_request(request).block_task().await {
                            Ok(_) => {
                                tracing::info!(%mode, "session mode set");
                                announce_modes(&events, &modes, mode);
                            }
                            Err(err) => {
                                tracing::warn!(%mode, %err, "the agent refused the mode");
                                withdraw_mode(&events, &modes, &mode);
                            }
                        }
                    }
                }
                if let Some(model) = model {
                    let request = SetSessionConfigOptionRequest::new(
                        session_id.clone(),
                        "model",
                        model.as_str(),
                    );
                    connection
                        .send_request(request)
                        .block_task()
                        .await
                        .inspect_err(
                            |err| tracing::warn!(%model, %err, "the agent refused the model"),
                        )?;
                    tracing::info!(%model, "session model set");
                }
                // The agent's own session options. A refusal is not fatal:
                // an option is a preference, and the session is worth having
                // without it.
                for (option, value) in config {
                    let request = SetSessionConfigOptionRequest::new(
                        session_id.clone(),
                        option.clone(),
                        SessionConfigOptionValue::from(value.as_str()),
                    );
                    // The key, never the value: an option is a place a
                    // person may reasonably have put a secret, and the
                    // journal is not the place for it.
                    match connection.send_request(request).block_task().await {
                        Ok(_) => tracing::info!(%option, "session option set"),
                        Err(err) => {
                            tracing::warn!(%option, %err, "the agent refused the option");
                        }
                    }
                }
                // Before the session is ready, so a queued turn starts
                // against a chat that already holds the agent's history.
                if let Some(turns) = history {
                    tracing::info!(turns = turns.len(), "history replayed by the agent");
                    let _ = events.send(SessionEvent::HistoryLoaded { turns });
                }
                ready.store(true, Ordering::Relaxed);
                let _ = events.send(SessionEvent::Ready {
                    agent_session: Some(session_id.0.to_string()),
                });
                drive(
                    &connection,
                    session_id,
                    &mut commands,
                    &events,
                    &current_turn,
                    &modes,
                )
                .await
            }
        })
        .await;

    if let Err(err) = result {
        let message = err.to_string();
        if !ready.load(Ordering::Relaxed) {
            tracing::warn!(%message, "the agent did not get as far as a session");
            let _ = events.send(SessionEvent::CreationFailed(message));
        } else if let Some(turn_id) = lock(&current_turn).take() {
            let _ = events.send(SessionEvent::TurnEnded {
                turn_id,
                outcome: TurnOutcome::Failed(message),
            });
        } else {
            tracing::warn!("agent connection ended: {message}");
        }
    }
}

/// Collects the history an agent replays during `session/load`.
///
/// The replay is a flat stream of chunks and tool calls with no turn
/// boundaries, so a turn is taken to be everything from one user message up to
/// the next: `user_message_chunk` opens a turn, and everything after it
/// belongs to that turn.
#[derive(Debug, Default)]
struct Replay {
    turns: Vec<HistoryTurn>,
    /// Where a picture in the replay is kept. A replayed picture is the same
    /// bytes as when it first arrived, so it lands on the file already there.
    images: Option<ImageCache>,
    /// The pictures each tool call has already put in the replay, as in
    /// [`forward_tool_pictures`].
    shown: BTreeSet<(String, PathBuf)>,
}

impl Replay {
    fn take(&mut self, update: SessionUpdate) {
        match update {
            SessionUpdate::UserMessageChunk(chunk) => {
                let Some(text) = text(chunk.content) else {
                    return;
                };
                // Chunks of one message arrive in pieces; a chunk that follows
                // nothing the agent said carries on the same prompt.
                match self.turns.last_mut() {
                    Some(turn) if turn.parts.is_empty() => turn.prompt.push_str(&text),
                    _ => self.turns.push(HistoryTurn {
                        prompt: text,
                        parts: Vec::new(),
                    }),
                }
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                match said(chunk.content, self.images.as_ref()) {
                    Some(Said::Text(text)) => self.push(HistoryPart::Message(text)),
                    Some(Said::Image(image)) => self.push(HistoryPart::Image(image)),
                    None => {}
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let Some(text) = text(chunk.content) {
                    self.push(HistoryPart::Thought(text));
                }
            }
            SessionUpdate::ToolCall(call) => {
                let title = call.title.clone();
                let success = !matches!(call.status, ToolCallStatus::Failed);
                let id = call.tool_call_id.to_string();
                self.push(HistoryPart::ToolCall {
                    tool_call_id: id.clone(),
                    title,
                    success,
                });
                self.take_pictures(&id, &call.content);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let id = update.tool_call_id.to_string();
                if let Some(content) = update.fields.content.clone() {
                    self.take_pictures(&id, &content);
                }
                let status = update.fields.status;
                let title = update.fields.title.clone();
                for turn in self.turns.iter_mut() {
                    for part in turn.parts.iter_mut() {
                        let HistoryPart::ToolCall {
                            tool_call_id,
                            title: current,
                            success,
                        } = part
                        else {
                            continue;
                        };
                        if *tool_call_id != id {
                            continue;
                        }
                        if let Some(title) = title.clone() {
                            *current = title;
                        }
                        match status {
                            Some(ToolCallStatus::Completed) => *success = true,
                            Some(ToolCallStatus::Failed) => *success = false,
                            _ => {}
                        }
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    /// Puts every picture in a replayed tool call's `content` into the turn, as
    /// [`forward_tool_pictures`] does for a turn as it happens.
    fn take_pictures(&mut self, tool_call_id: &str, content: &[ToolCallContent]) {
        for piece in content {
            let ToolCallContent::Content(piece) = piece else {
                continue;
            };
            let Some(Said::Image(image)) = said(piece.content.clone(), self.images.as_ref()) else {
                continue;
            };
            if !self
                .shown
                .insert((tool_call_id.to_owned(), image.path.clone()))
            {
                continue;
            }
            self.push(HistoryPart::Image(image));
        }
    }

    /// Adds a part to the turn being replayed. A part that arrives before any
    /// user message opens a turn with no prompt rather than being dropped.
    fn push(&mut self, part: HistoryPart) {
        if self.turns.is_empty() {
            self.turns.push(HistoryTurn::default());
        }
        if let Some(turn) = self.turns.last_mut() {
            turn.parts.push(part);
        }
    }
}

/// What opening the agent's session gave.
struct Opened {
    session_id: SessionId,
    /// The history the agent replayed, when it loaded the session.
    history: Option<Vec<HistoryTurn>>,
    /// The agent's modes and the current one, when it has any.
    modes: Option<SessionModeState>,
    /// Whether the session is a new one rather than taken up again.
    fresh: bool,
}

/// Opens the agent's session and, when it can, the history that goes with it.
///
/// `session/load` comes first: it replays the history back, which is what the
/// chat is rebuilt from. `session/resume` restores the agent alone and is the
/// fallback for agents that cannot load; both cost the same, since the agent
/// reads its own history either way.
async fn open_session(
    connection: &ConnectionTo<Agent>,
    capabilities: &AgentCapabilities,
    resume: Option<String>,
    cwd: &Path,
    meta: Option<Meta>,
    images: Option<&ImageCache>,
    replay: &Mutex<Option<Replay>>,
) -> Result<Opened, agent_client_protocol::Error> {
    if let Some(id) = resume {
        if capabilities.load_session {
            *lock(replay) = Some(Replay {
                images: images.cloned(),
                ..Replay::default()
            });
            let request = LoadSessionRequest::new(id.clone(), cwd).meta(meta.clone());
            let loaded = connection.send_request(request).block_task().await;
            let collected = lock(replay).take();
            match loaded {
                Ok(loaded) => {
                    tracing::info!(agent_session = %id, "agent session loaded with its history");
                    return Ok(Opened {
                        session_id: SessionId::from(id),
                        history: collected.map(|replay| replay.turns),
                        modes: loaded.modes,
                        fresh: false,
                    });
                }
                Err(err) => tracing::warn!(
                    agent_session = %id,
                    %err,
                    "the agent could not load its session; trying to resume it",
                ),
            }
        }
        let taken_up = if capabilities.session_capabilities.resume.is_some() {
            let request = ResumeSessionRequest::new(id.clone(), cwd).meta(meta.clone());
            Some(connection.send_request(request).block_task().await)
        } else {
            None
        };
        match taken_up {
            Some(Ok(resumed)) => {
                // Resumed without a replay, so the chat keeps what the host
                // last saw.
                tracing::info!(agent_session = %id, "agent session taken up again");
                return Ok(Opened {
                    session_id: SessionId::from(id),
                    history: None,
                    modes: resumed.modes,
                    fresh: false,
                });
            }
            Some(Err(err)) => tracing::warn!(
                agent_session = %id,
                %err,
                "the agent could not take its session up again; starting a new one"
            ),
            None => tracing::warn!(
                agent_session = %id,
                "the agent cannot take sessions up again; starting a new one"
            ),
        }
    }
    let session = connection
        .send_request(NewSessionRequest::new(cwd).meta(meta))
        .block_task()
        .await?;
    Ok(Opened {
        session_id: session.session_id,
        history: None,
        modes: session.modes,
        fresh: true,
    })
}

/// The agent's modes as the host keeps them: the current one's id, and the
/// list in the agent's order.
fn modes_of(state: SessionModeState) -> (String, Vec<Mode>) {
    let available = state
        .available_modes
        .into_iter()
        .map(|mode| Mode {
            id: mode.id.to_string(),
            name: mode.name,
            description: mode.description,
        })
        .collect();
    (state.current_mode_id.to_string(), available)
}

/// The agent's modes as the driver keeps them: the list it advertised, in
/// its order, and the id of the current one.
#[derive(Debug, Default)]
struct Modes {
    current: String,
    available: Vec<Mode>,
}

/// Reports `current` as the session's mode, with the modes the agent
/// advertised. An agent that named a current mode without advertising any
/// list still has that much said about it.
fn announce_modes(
    events: &mpsc::UnboundedSender<SessionEvent>,
    modes: &Mutex<Modes>,
    current: String,
) {
    let available = {
        let mut modes = lock(modes);
        modes.current = current.clone();
        modes.available.clone()
    };
    let _ = events.send(SessionEvent::ModesChanged { current, available });
}

/// Takes a mode the agent refused out of the list, so nobody is offered it
/// again: an agent advertises every mode it knows, not every mode the
/// session can enter, as Claude does with `auto` on a model without it.
fn withdraw_mode(events: &mpsc::UnboundedSender<SessionEvent>, modes: &Mutex<Modes>, mode: &str) {
    let current = {
        let mut modes = lock(modes);
        modes.available.retain(|known| known.id != mode);
        modes.current.clone()
    };
    announce_modes(events, modes, current);
}

/// Runs prompts, cancellations and mode changes until the host closes the
/// session.
async fn drive(
    connection: &ConnectionTo<Agent>,
    session_id: SessionId,
    commands: &mut mpsc::UnboundedReceiver<SessionCommand>,
    events: &mpsc::UnboundedSender<SessionEvent>,
    current_turn: &Mutex<Option<String>>,
    modes: &Arc<Mutex<Modes>>,
) -> Result<(), agent_client_protocol::Error> {
    let mut pending: Option<(String, PendingPrompt)> = None;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None => return Ok(()),
                Some(SessionCommand::Prompt { turn_id, text, attachments }) => {
                    if pending.is_some() {
                        let _ = events.send(SessionEvent::TurnEnded {
                            turn_id,
                            outcome: TurnOutcome::Failed("a turn is already running".into()),
                        });
                        continue;
                    }
                    *lock(current_turn) = Some(turn_id.clone());
                    // Attachments go as links, which every ACP agent accepts:
                    // the agent reads the files with its own tools.
                    let prompt = std::iter::once(ContentBlock::Text(TextContent::new(text)))
                        .chain(attachments.into_iter().map(|attachment| {
                            ContentBlock::ResourceLink(ResourceLink::new(attachment.name, attachment.uri))
                        }))
                        .collect();
                    let request = PromptRequest::new(session_id.clone(), prompt);
                    pending = Some((turn_id, Box::pin(connection.send_request(request).block_task())));
                }
                Some(SessionCommand::Cancel { turn_id }) => {
                    if pending.as_ref().is_some_and(|(running, _)| *running == turn_id) {
                        connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    }
                }
                Some(SessionCommand::SetMode { mode_id }) => {
                    // Answered in a task of its own: a turn may be running,
                    // and its prompt must go on being polled meanwhile. A
                    // refusal is logged; the agent's mode is what it says.
                    let request = SetSessionModeRequest::new(session_id.clone(), mode_id.clone());
                    let sent = connection.send_request(request).block_task();
                    let events = events.clone();
                    let modes = Arc::clone(modes);
                    connection.spawn(async move {
                        match sent.await {
                            Ok(_) => {
                                tracing::info!(mode = %mode_id, "session mode set");
                                announce_modes(&events, &modes, mode_id);
                            }
                            Err(err) => {
                                tracing::warn!(mode = %mode_id, %err, "the agent refused the mode");
                                withdraw_mode(&events, &modes, &mode_id);
                            }
                        }
                        Ok(())
                    })?;
                }
            },
            response = async {
                match pending.as_mut() {
                    Some((_, prompt)) => prompt.await,
                    None => std::future::pending().await,
                }
            } => {
                let Some((turn_id, _)) = pending.take() else { continue };
                *lock(current_turn) = None;
                let outcome = match response {
                    Ok(response) => match response.stop_reason {
                        StopReason::EndTurn => TurnOutcome::Complete,
                        StopReason::Cancelled => TurnOutcome::Cancelled,
                        other => TurnOutcome::Failed(format!("the agent stopped early: {other:?}")),
                    },
                    Err(err) => TurnOutcome::Failed(err.to_string()),
                };
                let _ = events.send(SessionEvent::TurnEnded { turn_id, outcome });
            }
        }
    }
}

fn forward_update(
    current_turn: &Mutex<Option<String>>,
    events: &mpsc::UnboundedSender<SessionEvent>,
    images: Option<&ImageCache>,
    shown: &Mutex<BTreeSet<(String, PathBuf)>>,
    update: SessionUpdate,
) {
    let Some(turn_id) = lock(current_turn).clone() else {
        return;
    };
    let event = match update {
        SessionUpdate::AgentMessageChunk(chunk) => match said(chunk.content, images) {
            Some(Said::Text(text)) => Some(SessionEvent::MessageChunk { turn_id, text }),
            Some(Said::Image(image)) => Some(SessionEvent::MessageImage { turn_id, image }),
            None => None,
        },
        SessionUpdate::AgentThoughtChunk(chunk) => {
            // A picture in the reasoning is not shown: the log folds thinking
            // into a line of its own, with nowhere to put one.
            text(chunk.content).map(|text| SessionEvent::ThoughtChunk { turn_id, text })
        }
        SessionUpdate::ToolCall(call) => {
            let id = call.tool_call_id.to_string();
            forward_tool_pictures(events, images, shown, &turn_id, &id, &call.content);
            None
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let id = update.tool_call_id.to_string();
            if let Some(content) = &update.fields.content {
                forward_tool_pictures(events, images, shown, &turn_id, &id, content);
            }
            let success = match update.fields.status {
                Some(ToolCallStatus::Completed) => true,
                Some(ToolCallStatus::Failed) => false,
                _ => return,
            };
            Some(SessionEvent::ToolCallFinished {
                turn_id,
                tool_call_id: id,
                success,
            })
        }
        _ => None,
    };
    if let Some(event) = event {
        let _ = events.send(event);
    }
}

/// Sends every picture in a tool call's `content` on as part of the answer.
///
/// This, not the agent's own message, is where pictures actually come from: a
/// model does not emit an image, it calls a tool that returns one — Claude
/// reading a JPEG, a browser tool taking a screenshot — and the picture arrives
/// as that call's content. It is shown in the answer rather than folded into the
/// tool call's line, because the log draws a tool call as one dimmed line and
/// because the agent's next words usually describe what the picture shows.
///
/// A tool call is updated more than once and may repeat its content each time,
/// so `shown` remembers what has already gone out for each call. Pictures are
/// named after their contents, which is what makes that comparison cheap and
/// exact.
fn forward_tool_pictures(
    events: &mpsc::UnboundedSender<SessionEvent>,
    images: Option<&ImageCache>,
    shown: &Mutex<BTreeSet<(String, PathBuf)>>,
    turn_id: &str,
    tool_call_id: &str,
    content: &[ToolCallContent],
) {
    for piece in content {
        let ToolCallContent::Content(piece) = piece else {
            continue;
        };
        let Some(Said::Image(image)) = said(piece.content.clone(), images) else {
            continue;
        };
        if !lock(shown).insert((tool_call_id.to_owned(), image.path.clone())) {
            continue;
        }
        tracing::debug!(
            tool_call_id,
            path = %image.path.display(),
            "a picture from a tool call goes in the answer",
        );
        let shared = SessionEvent::MessageImage {
            turn_id: turn_id.to_owned(),
            image,
        };
        let _ = events.send(shared);
    }
}

fn text(content: ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(text) => Some(text.text),
        _ => None,
    }
}

/// A piece of what an agent said: words, or a picture.
enum Said {
    Text(String),
    Image(SharedImage),
}

/// What `content` amounts to, with pictures kept in `images`.
///
/// ACP sends a picture either as base64 in the message — which is written to
/// the cache — or as a link to a file, which is pointed at where it lies. Audio
/// and embedded resources are not shown, and neither is a link to anything that
/// is not a picture: those are context for the agent, not for the reader.
fn said(content: ContentBlock, images: Option<&ImageCache>) -> Option<Said> {
    match content {
        ContentBlock::Text(text) => Some(Said::Text(text.text)),
        ContentBlock::Image(image) => {
            let Some(cache) = images else {
                tracing::warn!("nowhere to keep a picture the agent sent; dropping it");
                return None;
            };
            // An image block carries no name of its own. Where it says which
            // resource it came from, that file's name is the best label there
            // is; otherwise the picture is just called a picture.
            let label = image.uri.as_deref().and_then(file_name);
            cache
                .store(&image.data, &image.mime_type, label.as_deref())
                .map(Said::Image)
        }
        ContentBlock::ResourceLink(link) => {
            let path = crate::uri::to_path(&link.uri)?;
            ImageCache::adopt(path, link.mime_type.as_deref()).map(Said::Image)
        }
        _ => None,
    }
}

/// The last part of a `file://` URI, or of a bare path, as a name for a
/// picture. `None` for a URI that names no file, such as a `data:` one.
fn file_name(uri: &str) -> Option<String> {
    let path = crate::uri::to_path(uri).unwrap_or_else(|| PathBuf::from(uri));
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some(name).filter(|name| !name.is_empty())
}

/// Answers a permission request from the agent's policy alone, without asking
/// anyone.
fn answer_by_policy(
    policy: PermissionPolicy,
    request: &RequestPermissionRequest,
) -> RequestPermissionResponse {
    tracing::info!(
        ?policy,
        tool_call = ?request.tool_call.tool_call_id,
        "answering a permission request by policy"
    );
    // The title is the command or the path the agent is asking about, which
    // is the person's business and not the default journal's.
    tracing::debug!(
        title = request.tool_call.fields.title.as_deref().unwrap_or(""),
        "what it asked about"
    );
    dialog::answer(request, policy == PermissionPolicy::Allow)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        ContentChunk, ToolCall, ToolCallUpdate, ToolCallUpdateFields,
    };

    fn said(text: &str) -> ContentBlock {
        ContentBlock::Text(TextContent::new(text))
    }

    fn asked(text: &str) -> SessionUpdate {
        SessionUpdate::UserMessageChunk(ContentChunk::new(said(text)))
    }

    fn answered(text: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(said(text)))
    }

    #[test]
    fn a_turn_runs_from_one_user_message_to_the_next() {
        let mut replay = Replay::default();
        replay.take(asked("first"));
        replay.take(answered("one"));
        replay.take(asked("second"));
        replay.take(answered("two"));

        assert_eq!(replay.turns.len(), 2, "one turn per user message");
        assert_eq!(replay.turns[0].prompt, "first");
        assert_eq!(
            replay.turns[0].parts,
            vec![HistoryPart::Message("one".into())]
        );
        assert_eq!(replay.turns[1].prompt, "second");
        assert_eq!(
            replay.turns[1].parts,
            vec![HistoryPart::Message("two".into())]
        );
    }

    #[test]
    fn chunks_of_one_prompt_join_up() {
        let mut replay = Replay::default();
        replay.take(asked("a question "));
        replay.take(asked("in two parts"));
        replay.take(answered("answered"));

        assert_eq!(replay.turns.len(), 1, "no agent reply divided them");
        assert_eq!(replay.turns[0].prompt, "a question in two parts");
    }

    #[test]
    fn a_tool_call_is_replayed_with_what_became_of_it() {
        let mut replay = Replay::default();
        replay.take(asked("do the thing"));
        replay.take(SessionUpdate::ToolCall(ToolCall::new("call-1", "Bash")));
        replay.take(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "call-1",
            {
                let mut fields = ToolCallUpdateFields::default();
                fields.status = Some(ToolCallStatus::Failed);
                fields
            },
        )));

        assert_eq!(
            replay.turns[0].parts,
            vec![HistoryPart::ToolCall {
                tool_call_id: "call-1".into(),
                title: "Bash".into(),
                success: false,
            }],
            "the update says how it ended, and the call keeps its title"
        );
    }

    #[test]
    fn what_an_agent_says_before_any_prompt_is_kept() {
        let mut replay = Replay::default();
        replay.take(answered("a preamble of its own"));

        assert_eq!(replay.turns.len(), 1);
        assert_eq!(replay.turns[0].prompt, "", "nobody asked for it");
        assert_eq!(
            replay.turns[0].parts,
            vec![HistoryPart::Message("a preamble of its own".into())]
        );
    }
}
