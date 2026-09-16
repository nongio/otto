//! ACP backend: each session runs its agent as a child process speaking the
//! Agent Client Protocol over stdio.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, InitializeRequest, LoadSessionRequest,
    Meta, NewSessionRequest, PromptRequest, PromptResponse, RequestPermissionRequest,
    RequestPermissionResponse, ResourceLink, ResumeSessionRequest, SessionId, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, StopReason, TextContent, ToolCallStatus,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectTo, ConnectionTo};
use ahp_types::state::AgentInfo;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Backend, Decision, SessionCommand, SessionEvent, SessionSpec, TurnOutcome, agent_info,
};
use crate::config::{self, AgentConfig, PermissionPolicy, SkillDelivery};
use crate::dialog;
use crate::skills::{self, Plugin};

pub struct AcpBackend {
    agents: Vec<AgentConfig>,
    /// The terminal sessions are opened in; see [`config::terminal_command`].
    terminal: Vec<String>,
    /// The desktop's skills, found once at startup. Agents configured with
    /// `skills = true` are told about them; see [`crate::skills`].
    plugins: Vec<Plugin>,
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
                dir = %plugin.dir.display(),
                "offering the desktop's skills to agents",
            );
        }
        Self {
            agents,
            terminal,
            plugins,
        }
    }
}

/// How a session answers its agent's permission requests.
#[derive(Clone)]
pub struct Permissions {
    pub policy: PermissionPolicy,
    /// The agent's name, as questions about it say it.
    pub agent: String,
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
            .args(agent.args.clone())
            .envs(agent.env.clone());
        let transport = AcpAgent::new(config);
        let permissions = Permissions {
            policy: agent.permissions,
            agent: agent.name.clone(),
        };
        // A session taken up again has had the briefing already: it is in the
        // history the agent keeps, and repeating it every restart would pay
        // for it twice.
        let briefing = match agent.skills {
            SkillDelivery::Briefing if spec.resume.is_none() => skills::briefing(&self.plugins),
            _ => None,
        };
        // Claude loads plugins itself, on every session it opens, resumed or
        // not: what it knows of them lives in the process, not the history.
        let meta = match agent.skills {
            SkillDelivery::Claude => skills::claude_session_meta(&self.plugins),
            _ => None,
        };
        tokio::spawn(run_session(
            transport,
            permissions,
            agent.model.clone(),
            spec.resume,
            spec.cwd,
            briefing,
            meta,
            commands,
            events,
        ));
    }

    fn terminal(&self, provider: &str, agent_session: &str, cwd: &Path) -> Option<Vec<String>> {
        let agent = self.agents.iter().find(|agent| agent.id == provider)?;
        config::terminal_command(&self.terminal, agent, agent_session, cwd)
    }
}

type PendingPrompt =
    Pin<Box<dyn Future<Output = Result<PromptResponse, agent_client_protocol::Error>> + Send>>;

/// Drives one ACP session over `transport`: a child process in production,
/// an in-memory channel in tests. When `model` is set, the session switches to
/// it before it is ready, and fails to start if the agent refuses it. When
/// `resume` names a session the agent ran before, the agent takes it up again
/// if it can; otherwise a new session starts. `briefing` is sent ahead of the
/// session's first prompt and nowhere else — see [`crate::skills`]. `meta` goes
/// with the request that opens the session, whichever it is.
#[allow(clippy::too_many_arguments)]
pub async fn run_session(
    transport: impl ConnectTo<Client> + 'static,
    permissions: Permissions,
    model: Option<String>,
    resume: Option<String>,
    cwd: PathBuf,
    briefing: Option<String>,
    meta: Option<Meta>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    events: mpsc::UnboundedSender<SessionEvent>,
) {
    // The turn whose updates are being streamed, shared with the update handler.
    let current_turn: Arc<Mutex<Option<String>>> = Arc::default();
    let ready = Arc::new(AtomicBool::new(false));

    let result = Client
        .builder()
        .name("otto-agentsd")
        .on_receive_notification(
            {
                let events = events.clone();
                let current_turn = Arc::clone(&current_turn);
                async move |notification: SessionNotification, _connection| {
                    forward_update(&current_turn, &events, notification.update);
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
                    let question = dialog::question_for(&permissions.agent, &cwd, &request);
                    let (reply, decision) = oneshot::channel();
                    let asked = SessionEvent::PermissionRequested {
                        turn_id,
                        question,
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
        .connect_with(transport, {
            let events = events.clone();
            let current_turn = Arc::clone(&current_turn);
            let ready = Arc::clone(&ready);
            async move |connection: ConnectionTo<Agent>| {
                let initialized = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                let session_id = open_session(
                    &connection,
                    &initialized.agent_capabilities,
                    resume,
                    &cwd,
                    meta,
                )
                .await?;
                // The agent's own id for the session: for claude-agent-acp,
                // the Claude Code session, which `claude --resume` takes.
                tracing::info!(agent_session = %session_id.0, "agent session started");
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
                ready.store(true, Ordering::Relaxed);
                let _ = events.send(SessionEvent::Ready {
                    agent_session: Some(session_id.0.to_string()),
                });
                drive(
                    &connection,
                    session_id,
                    briefing,
                    &mut commands,
                    &events,
                    &current_turn,
                )
                .await
            }
        })
        .await;

    if let Err(err) = result {
        let message = err.to_string();
        if !ready.load(Ordering::Relaxed) {
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

/// Opens the agent's session: the one `resume` names, when the agent can take
/// it up again, or else a new one.
async fn open_session(
    connection: &ConnectionTo<Agent>,
    capabilities: &AgentCapabilities,
    resume: Option<String>,
    cwd: &Path,
    meta: Option<Meta>,
) -> Result<SessionId, agent_client_protocol::Error> {
    if let Some(id) = resume {
        let taken_up = if capabilities.session_capabilities.resume.is_some() {
            let request = ResumeSessionRequest::new(id.clone(), cwd).meta(meta.clone());
            Some(
                connection
                    .send_request(request)
                    .block_task()
                    .await
                    .map(drop),
            )
        } else if capabilities.load_session {
            // Loading replays the history as updates. No turn is running to
            // claim them, so they are dropped: the chat already has them.
            let request = LoadSessionRequest::new(id.clone(), cwd).meta(meta.clone());
            Some(
                connection
                    .send_request(request)
                    .block_task()
                    .await
                    .map(drop),
            )
        } else {
            None
        };
        match taken_up {
            Some(Ok(())) => {
                tracing::info!(agent_session = %id, "agent session taken up again");
                return Ok(SessionId::from(id));
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
    Ok(session.session_id)
}

/// Runs prompts and cancellations until the host closes the session.
async fn drive(
    connection: &ConnectionTo<Agent>,
    session_id: SessionId,
    mut briefing: Option<String>,
    commands: &mut mpsc::UnboundedReceiver<SessionCommand>,
    events: &mpsc::UnboundedSender<SessionEvent>,
    current_turn: &Mutex<Option<String>>,
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
                    // The desktop's skills, on the first prompt of a new
                    // session only: `take` is what makes it the first. It goes
                    // as its own block ahead of the request, so the agent's
                    // own view of what the person wrote stays untouched, and
                    // it never reaches the AHP chat — the host sent the text,
                    // and the text is what the transcript shows.
                    let briefing = briefing.take().map(|text| ContentBlock::Text(TextContent::new(text)));
                    // Attachments go as links, which every ACP agent accepts:
                    // the agent reads the files with its own tools.
                    let prompt = briefing
                        .into_iter()
                        .chain(std::iter::once(ContentBlock::Text(TextContent::new(text))))
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
    update: SessionUpdate,
) {
    let Some(turn_id) = lock(current_turn).clone() else {
        return;
    };
    let event = match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            text(chunk.content).map(|text| SessionEvent::MessageChunk { turn_id, text })
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            text(chunk.content).map(|text| SessionEvent::ThoughtChunk { turn_id, text })
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let success = match update.fields.status {
                Some(ToolCallStatus::Completed) => true,
                Some(ToolCallStatus::Failed) => false,
                _ => return,
            };
            Some(SessionEvent::ToolCallFinished {
                turn_id,
                tool_call_id: update.tool_call_id.to_string(),
                success,
            })
        }
        _ => None,
    };
    if let Some(event) = event {
        let _ = events.send(event);
    }
}

fn text(content: ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(text) => Some(text.text),
        _ => None,
    }
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
        title = request.tool_call.fields.title.as_deref().unwrap_or(""),
        "answering a permission request by policy"
    );
    dialog::answer(request, policy == PermissionPolicy::Allow)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
