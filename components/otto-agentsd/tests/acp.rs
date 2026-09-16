//! ACP backend tests: `run_session` drives a fake agent over an in-memory
//! channel, so no agent process or credentials are needed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOptionValue, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason, TextContent,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Responder};
use otto_agentsd::acp::{AcpBackend, Permissions, run_session};
use otto_agentsd::agent::{
    Attachment, Backend, Decision, Question, SessionCommand, SessionEvent, SessionSpec, TurnOutcome,
};
use otto_agentsd::config::{AgentConfig, PermissionPolicy, SkillDelivery};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Starts a fake agent that answers each prompt with `echo: <prompt>` in two
/// chunks, with two exceptions: `wait`, which it holds until cancelled, and
/// `needs-permission`, which asks to run `cargo test` and answers with the
/// option it was given. It records the model it is switched to in `model`.
fn spawn_fake_agent(transport: Channel, model: Arc<Mutex<Option<String>>>) {
    let parked: Arc<Mutex<Option<Responder<PromptResponse>>>> = Arc::default();
    let agent = Agent
        .builder()
        .name("fake-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: NewSessionRequest, responder, _connection| {
                responder.respond(NewSessionResponse::new("fake-session"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _connection| {
                if request.config_id.to_string() == "model"
                    && let SessionConfigOptionValue::ValueId { value } = request.value
                {
                    *model.lock().unwrap() = Some(value.to_string());
                }
                responder.respond(SetSessionConfigOptionResponse::new(Vec::new()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let parked = Arc::clone(&parked);
                async move |request: PromptRequest,
                            responder: Responder<PromptResponse>,
                            connection: ConnectionTo<Client>| {
                    let prompt: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text(text) => Some(text.text.clone()),
                            _ => None,
                        })
                        .collect();
                    let links: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::ResourceLink(link) => {
                                Some(format!(" [{} {}]", link.name, link.uri))
                            }
                            _ => None,
                        })
                        .collect();
                    if prompt == "wait" {
                        *parked.lock().unwrap() = Some(responder);
                        return Ok(());
                    }
                    if prompt == "needs-permission" {
                        let mut fields = ToolCallUpdateFields::new();
                        fields.kind = Some(ToolKind::Execute);
                        fields.title = Some("cargo test".into());
                        let permission = RequestPermissionRequest::new(
                            request.session_id.clone(),
                            ToolCallUpdate::new("call-1", fields),
                            vec![
                                PermissionOption::new(
                                    "allow",
                                    "Allow",
                                    PermissionOptionKind::AllowOnce,
                                ),
                                PermissionOption::new(
                                    "reject",
                                    "Reject",
                                    PermissionOptionKind::RejectOnce,
                                ),
                            ],
                        );
                        let session_id = request.session_id.clone();
                        let spawner = connection.clone();
                        return spawner.spawn(async move {
                            let answer = connection.send_request(permission).block_task().await?;
                            let chosen = match answer.outcome {
                                RequestPermissionOutcome::Selected(selected) => {
                                    selected.option_id.to_string()
                                }
                                _ => "cancelled".to_owned(),
                            };
                            if chosen == "allow" {
                                let mut done = ToolCallUpdateFields::new();
                                done.status = Some(ToolCallStatus::Completed);
                                connection.send_notification(SessionNotification::new(
                                    session_id.clone(),
                                    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                        "call-1", done,
                                    )),
                                ))?;
                            }
                            let chunk =
                                ContentChunk::new(ContentBlock::Text(TextContent::new(chosen)));
                            connection.send_notification(SessionNotification::new(
                                session_id,
                                SessionUpdate::AgentMessageChunk(chunk),
                            ))?;
                            responder.respond(PromptResponse::new(StopReason::EndTurn))
                        });
                    }
                    for piece in ["echo: ", prompt.as_str(), links.as_str()] {
                        if piece.is_empty() {
                            continue;
                        }
                        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(piece)));
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::AgentMessageChunk(chunk),
                        ))?;
                    }
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_cancel: CancelNotification, _connection| {
                if let Some(responder) = parked.lock().unwrap().take() {
                    responder.respond(PromptResponse::new(StopReason::Cancelled))?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        );
    tokio::spawn(async move {
        let _ = agent.connect_to(transport).await;
    });
}

struct Session {
    commands: mpsc::UnboundedSender<SessionCommand>,
    events: mpsc::UnboundedReceiver<SessionEvent>,
    /// The model the fake agent was switched to.
    model: Arc<Mutex<Option<String>>>,
}

fn permissions(policy: PermissionPolicy) -> Permissions {
    Permissions {
        policy,
        agent: "Fake".into(),
    }
}

/// What a turn produced.
struct TurnRecord {
    answer: String,
    outcome: TurnOutcome,
    /// The permission questions the session asked the host.
    asked: Vec<Question>,
    /// The tool calls reported finished, with whether they succeeded.
    finished: Vec<(String, bool)>,
}

impl Session {
    fn start(model: Option<&str>) -> Self {
        Self::launch(model, permissions(PermissionPolicy::Deny))
    }

    fn launch(model: Option<&str>, permissions: Permissions) -> Self {
        Self::launch_with(model, permissions, None)
    }

    fn launch_with(
        model: Option<&str>,
        permissions: Permissions,
        briefing: Option<String>,
    ) -> Self {
        let (client_end, agent_end) = Channel::duplex();
        let chosen = Arc::default();
        spawn_fake_agent(agent_end, Arc::clone(&chosen));
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        tokio::spawn(run_session(
            client_end,
            permissions,
            model.map(str::to_owned),
            None,
            std::env::temp_dir(),
            briefing,
            None,
            command_rx,
            event_tx,
        ));
        Self {
            commands,
            events,
            model: chosen,
        }
    }

    async fn next(&mut self) -> SessionEvent {
        timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("timed out waiting for a session event")
            .expect("the session ended")
    }

    fn prompt(&self, turn_id: &str, text: &str) {
        self.prompt_with(turn_id, text, Vec::new());
    }

    fn prompt_with(&self, turn_id: &str, text: &str, attachments: Vec<Attachment>) {
        let command = SessionCommand::Prompt {
            turn_id: turn_id.into(),
            text: text.into(),
            attachments,
        };
        self.commands.send(command).expect("session is running");
    }

    /// Runs a turn to its end, answering every permission question with
    /// `decision`.
    async fn turn(&mut self, text: &str, decision: Decision) -> TurnRecord {
        self.prompt("t", text);
        let mut record = TurnRecord {
            answer: String::new(),
            outcome: TurnOutcome::Complete,
            asked: Vec::new(),
            finished: Vec::new(),
        };
        loop {
            match self.next().await {
                SessionEvent::MessageChunk { text, .. } => record.answer.push_str(&text),
                SessionEvent::PermissionRequested {
                    question, reply, ..
                } => {
                    record.asked.push(question);
                    let _ = reply.send(decision.clone());
                }
                SessionEvent::ToolCallFinished {
                    tool_call_id,
                    success,
                    ..
                } => record.finished.push((tool_call_id, success)),
                SessionEvent::TurnEnded { outcome, .. } => {
                    record.outcome = outcome;
                    return record;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn under_ask_the_host_is_asked_with_the_agents_options() {
    let mut session = Session::launch(None, permissions(PermissionPolicy::Ask));
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    let allow = Decision {
        approved: true,
        option_id: Some("allow".into()),
    };
    let record = session.turn("needs-permission", allow).await;
    assert!(
        matches!(record.outcome, TurnOutcome::Complete),
        "{:?}",
        record.outcome
    );
    assert_eq!(record.answer, "allow");
    assert_eq!(record.finished, [("call-1".to_owned(), true)]);

    let [question] = &record.asked[..] else {
        panic!("expected one question: {:?}", record.asked);
    };
    assert_eq!(question.tool_call_id, "call-1");
    assert_eq!(question.tool_name, "execute");
    assert_eq!(question.prompt.title, "Fake wants to run a command");
    assert_eq!(question.prompt.subtitle, "cargo test");
    let options: Vec<(&str, &str, bool)> = question
        .options
        .iter()
        .map(|option| (option.id.as_str(), option.label.as_str(), option.allow))
        .collect();
    assert_eq!(
        options,
        [("allow", "Allow", true), ("reject", "Reject", false)]
    );
}

#[tokio::test]
async fn under_ask_a_denial_rejects() {
    let mut session = Session::launch(None, permissions(PermissionPolicy::Ask));
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    let record = session.turn("needs-permission", Decision::deny()).await;
    assert_eq!(record.answer, "reject");
    assert!(record.finished.is_empty());
}

#[tokio::test]
async fn under_deny_nobody_is_asked() {
    let mut session = Session::launch(None, permissions(PermissionPolicy::Deny));
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    let allow = Decision {
        approved: true,
        option_id: None,
    };
    let record = session.turn("needs-permission", allow).await;
    assert_eq!(record.answer, "reject");
    assert!(record.asked.is_empty());
}

#[tokio::test]
async fn a_prompt_streams_chunks_and_completes() {
    let mut session = Session::start(None);
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    session.prompt("t1", "hello");
    let mut text = String::new();
    loop {
        match session.next().await {
            SessionEvent::MessageChunk {
                turn_id,
                text: chunk,
            } => {
                assert_eq!(turn_id, "t1");
                text.push_str(&chunk);
            }
            SessionEvent::TurnEnded { turn_id, outcome } => {
                assert_eq!(turn_id, "t1");
                assert!(matches!(outcome, TurnOutcome::Complete), "{outcome:?}");
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(text, "echo: hello");
}

#[tokio::test]
async fn attachments_reach_the_agent_as_resource_links() {
    let mut session = Session::start(None);
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    let notes = Attachment {
        name: "notes.md".into(),
        uri: "file:///home/me/notes.md".into(),
    };
    session.prompt_with("t", "summarise", vec![notes]);
    let mut text = String::new();
    loop {
        match session.next().await {
            SessionEvent::MessageChunk { text: chunk, .. } => text.push_str(&chunk),
            SessionEvent::TurnEnded { .. } => break,
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(text, "echo: summarise [notes.md file:///home/me/notes.md]");
}

#[tokio::test]
async fn a_configured_model_is_set_before_the_session_is_ready() {
    let mut session = Session::start(Some("haiku"));
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));
    assert_eq!(session.model.lock().unwrap().as_deref(), Some("haiku"));
}

#[tokio::test]
async fn without_a_model_the_agent_keeps_its_own() {
    let mut session = Session::start(None);
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));
    assert_eq!(*session.model.lock().unwrap(), None);
}

#[tokio::test]
async fn cancelling_a_turn_reaches_the_agent() {
    let mut session = Session::start(None);
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    session.prompt("t1", "wait");
    // Give the prompt time to reach the agent before cancelling it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let cancel = SessionCommand::Cancel {
        turn_id: "t1".into(),
    };
    session.commands.send(cancel).expect("session is running");

    match session.next().await {
        SessionEvent::TurnEnded { turn_id, outcome } => {
            assert_eq!(turn_id, "t1");
            assert!(matches!(outcome, TurnOutcome::Cancelled), "{outcome:?}");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn a_missing_agent_binary_fails_session_creation() {
    let agent = AgentConfig {
        command: "/nonexistent/otto-agentsd-test-agent".into(),
        ..AgentConfig::claude()
    };
    let backend = AcpBackend::new(vec![agent], Vec::new());
    let (_commands, command_rx) = mpsc::unbounded_channel();
    let (event_tx, mut events) = mpsc::unbounded_channel();
    let spec = SessionSpec {
        provider: "claude".into(),
        cwd: std::env::temp_dir(),
        resume: None,
    };
    backend.start(spec, command_rx, event_tx);

    let event = timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timed out")
        .expect("an event");
    assert!(
        matches!(event, SessionEvent::CreationFailed(_)),
        "{event:?}"
    );
}

/// The desktop's skills reach the agent, and cost one prompt rather than every
/// prompt. The fake agent echoes the text it was sent, so what comes back is
/// exactly what the agent saw.
#[tokio::test]
async fn the_briefing_goes_ahead_of_the_first_prompt_only() {
    let briefing = Some("Otto skills: /usr/share/otto/plugins/otto\n".to_owned());
    let mut session =
        Session::launch_with(None, permissions(PermissionPolicy::Deny), briefing.clone());
    assert!(matches!(session.next().await, SessionEvent::Ready { .. }));

    let first = session.turn("hello", Decision::deny()).await;
    assert!(
        first.answer.contains("Otto skills:"),
        "the first prompt carries the briefing: {:?}",
        first.answer
    );
    assert!(
        first.answer.ends_with("hello"),
        "and the request comes after it: {:?}",
        first.answer
    );

    let second = session.turn("again", Decision::deny()).await;
    assert_eq!(
        second.answer, "echo: again",
        "the agent has it; sending it twice would be paying twice"
    );
}

/// An agent configured with `skills = false` is told nothing, and publishes no
/// customizations — the list a client reads is what the agent was given.
#[tokio::test]
async fn an_agent_can_be_configured_without_the_desktops_skills() {
    let with = AgentConfig {
        id: "with".into(),
        ..AgentConfig::claude()
    };
    let without = AgentConfig {
        id: "without".into(),
        skills: SkillDelivery::Off,
        ..AgentConfig::claude()
    };
    let root = std::env::temp_dir().join(format!("otto-agentsd-acp-skills-{}", std::process::id()));
    let skill = root.join("otto/skills/configure-otto");
    std::fs::create_dir_all(&skill).expect("a fixture plugin");
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: configure-otto\ndescription: Change settings\n---\n",
    )
    .expect("a fixture skill");
    let plugins = otto_agentsd::skills::discover_in(std::slice::from_ref(&root));
    assert_eq!(plugins.len(), 1);
    let backend = AcpBackend::with_plugins(vec![with, without], Vec::new(), plugins);

    let agents = backend.agents();
    assert!(
        agents[0]
            .customizations
            .as_ref()
            .is_some_and(|list| !list.is_empty()),
        "the default agent publishes what it was given"
    );
    assert_eq!(agents[1].customizations, None);

    std::fs::remove_dir_all(&root).expect("the fixture is ours to remove");
}
