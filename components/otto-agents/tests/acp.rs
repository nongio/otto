//! ACP backend tests: `run_session` drives a fake agent over an in-memory
//! channel, so no agent process or credentials are needed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::CurrentModeUpdate;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, ImageContent,
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOptionValue, SessionMode, SessionModeState,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse, StopReason,
    TextContent, ToolCall, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
    ToolKind,
};
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Responder};
use otto_agents::acp::{AcpBackend, Permissions, run_session};
use otto_agents::agent::{
    Attachment, Backend, Decision, Question, SessionCommand, SessionEvent, SessionSpec, TurnOutcome,
};
use otto_agents::config::{AgentConfig, PermissionPolicy, SkillDelivery};
use otto_agents::images::ImageCache;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// What the fake agent was switched to.
#[derive(Default)]
struct Recorded {
    model: Option<String>,
    /// The session options it was set to, other than the model.
    options: BTreeMap<String, String>,
    mode: Option<String>,
}

/// A one-pixel PNG, as an agent sends a picture: base64 in the message.
const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

/// A mode the fake agent advertises but refuses to enter, as Claude does
/// with `auto` on a model without it.
const REFUSED_MODE: (&str, &str) = ("auto", "Auto");

/// A session option the fake agent refuses, as Codex does with one its
/// build does not carry.
const REFUSED_OPTION: &str = "no-such-option";

/// The modes the fake agent advertises and accepts.
const MODES: [(&str, &str); 3] = [
    ("default", "Manual"),
    ("acceptEdits", "Accept edits"),
    ("plan", "Plan"),
];

fn advertised_modes() -> SessionModeState {
    SessionModeState::new(
        "default",
        MODES
            .iter()
            .chain(std::iter::once(&REFUSED_MODE))
            .map(|(id, name)| SessionMode::new(*id, *name))
            .collect(),
    )
}

/// Starts a fake agent that answers each prompt with `echo: <prompt>` in two
/// chunks, with three exceptions: `wait`, which it holds until cancelled,
/// `needs-permission`, which asks to run `cargo test` and answers with the
/// option it was given, `read:<path>`, which asks to read `<path>` and does
/// the same, and `change-mode`, which switches itself to `plan`
/// and says so. It advertises [`MODES`] and [`REFUSED_MODE`], starting in
/// `default`, refuses to enter the latter, and records
/// the model and mode it is switched to in `recorded`.
fn spawn_fake_agent(transport: Channel, recorded: Arc<Mutex<Recorded>>) {
    let parked: Arc<Mutex<Option<Responder<PromptResponse>>>> = Arc::default();
    let switched = Arc::clone(&recorded);
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
                responder.respond(NewSessionResponse::new("fake-session").modes(advertised_modes()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _connection| {
                let option = request.config_id.to_string();
                if option == REFUSED_OPTION {
                    return Err(agent_client_protocol::Error::invalid_params());
                }
                if let SessionConfigOptionValue::ValueId { value } = request.value {
                    let mut recorded = recorded.lock().unwrap();
                    if option == "model" {
                        recorded.model = Some(value.to_string());
                    } else {
                        recorded.options.insert(option, value.to_string());
                    }
                }
                responder.respond(SetSessionConfigOptionResponse::new(Vec::new()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, _connection| {
                let mode = request.mode_id.to_string();
                if !MODES.iter().any(|(id, _)| *id == mode) {
                    return Err(agent_client_protocol::Error::invalid_params());
                }
                switched.lock().unwrap().mode = Some(mode);
                responder.respond(SetSessionModeResponse::new())
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
                    if prompt == "change-mode" {
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("plan")),
                        ))?;
                        return responder.respond(PromptResponse::new(StopReason::EndTurn));
                    }
                    let read = prompt.strip_prefix("read:").map(str::to_owned);
                    if prompt == "needs-permission" || read.is_some() {
                        let mut fields = ToolCallUpdateFields::new();
                        match read {
                            Some(path) => {
                                fields.kind = Some(ToolKind::Read);
                                fields.title = Some(format!("Read {path}"));
                                fields.locations = Some(vec![ToolCallLocation::new(&path)]);
                                fields.raw_input = Some(serde_json::json!({ "file_path": path }));
                            }
                            None => {
                                fields.kind = Some(ToolKind::Execute);
                                fields.title = Some("cargo test".into());
                            }
                        }
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
                    if prompt == "read-a-picture" {
                        // How a picture really arrives: the agent calls a tool
                        // — Claude reading a JPEG — and the picture comes back
                        // as that call's content, not in the agent's message.
                        let call = ToolCall::new("call-read", "Read screenshot.png");
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::ToolCall(call),
                        ))?;
                        let mut image = ImageContent::new(PIXEL, "image/jpeg");
                        image.uri = Some("file:///home/me/Screenshots/Shot%201.jpg".to_owned());
                        let mut fields = ToolCallUpdateFields::new();
                        fields.status = Some(ToolCallStatus::Completed);
                        fields.content = Some(vec![ContentBlock::Image(image).into()]);
                        let finished = ToolCallUpdate::new("call-read", fields.clone());
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::ToolCallUpdate(finished),
                        ))?;
                        // The same content again, as an agent that resends it
                        // on every update does.
                        let again = ToolCallUpdate::new("call-read", fields);
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::ToolCallUpdate(again),
                        ))?;
                        let words = ContentBlock::Text(TextContent::new("that is your screenshot"));
                        connection.send_notification(SessionNotification::new(
                            request.session_id.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(words)),
                        ))?;
                        return responder.respond(PromptResponse::new(StopReason::EndTurn));
                    }
                    if prompt == "draw" {
                        // Words, then a picture, then more words: the order the
                        // answer has to keep.
                        let said = |block| {
                            SessionNotification::new(
                                request.session_id.clone(),
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(block)),
                            )
                        };
                        connection.send_notification(said(ContentBlock::Text(
                            TextContent::new("here:"),
                        )))?;
                        let mut image = ImageContent::new(PIXEL, "image/png");
                        image.uri = Some("file:///tmp/Mock%20Up.png".to_owned());
                        connection.send_notification(said(ContentBlock::Image(image)))?;
                        connection.send_notification(said(ContentBlock::Text(
                            TextContent::new("that is all"),
                        )))?;
                        return responder.respond(PromptResponse::new(StopReason::EndTurn));
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
    /// What the fake agent was switched to.
    recorded: Arc<Mutex<Recorded>>,
}

/// The modes as the last `ModesChanged` before the session was ready had them.
struct Modes {
    current: String,
    available: Vec<otto_agents::agent::Mode>,
}

fn permissions(policy: PermissionPolicy) -> Permissions {
    Permissions {
        policy,
        agent: "Fake".into(),
        icon: None,
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
    /// The current mode, each time the session said it changed.
    modes: Vec<String>,
}

impl Session {
    fn start(model: Option<&str>) -> Self {
        Self::launch(model, permissions(PermissionPolicy::Deny))
    }

    /// A session configured to start in `mode`.
    fn start_in_mode(mode: &str) -> Self {
        Self::launch_with(None, Some(mode), permissions(PermissionPolicy::Deny))
    }

    fn launch(model: Option<&str>, permissions: Permissions) -> Self {
        Self::launch_with(model, None, permissions)
    }

    /// A session configured with the agent's own options.
    fn start_with_options(options: &[(&str, &str)]) -> Self {
        Self::launch_all(None, None, permissions(PermissionPolicy::Deny), options)
    }

    fn launch_with(model: Option<&str>, mode: Option<&str>, permissions: Permissions) -> Self {
        Self::launch_all(model, mode, permissions, &[])
    }

    fn launch_all(
        model: Option<&str>,
        mode: Option<&str>,
        permissions: Permissions,
        options: &[(&str, &str)],
    ) -> Self {
        Self::launch_into(model, mode, permissions, options, None)
    }

    /// A session that keeps the pictures its agent sends in `images`.
    fn start_with_images(images: ImageCache) -> Self {
        Self::launch_into(
            None,
            None,
            permissions(PermissionPolicy::Deny),
            &[],
            Some(images),
        )
    }

    fn launch_into(
        model: Option<&str>,
        mode: Option<&str>,
        permissions: Permissions,
        options: &[(&str, &str)],
        images: Option<ImageCache>,
    ) -> Self {
        // Every session in this file goes through here, and the wording the
        // tests assert is the source catalogue's, not the developer's locale.
        otto_agents::i18n::pin_source_locale();
        let (client_end, agent_end) = Channel::duplex();
        let recorded = Arc::default();
        spawn_fake_agent(agent_end, Arc::clone(&recorded));
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        tokio::spawn(run_session(
            client_end,
            permissions,
            model.map(str::to_owned),
            mode.map(str::to_owned),
            options
                .iter()
                .map(|(option, value)| ((*option).to_owned(), (*value).to_owned()))
                .collect(),
            None,
            std::env::temp_dir(),
            Vec::new(),
            None,
            images,
            command_rx,
            event_tx,
        ));
        Self {
            commands,
            events,
            recorded,
        }
    }

    /// Waits until the session is ready, and returns the modes it announced
    /// on the way there.
    async fn ready(&mut self) -> Option<Modes> {
        let mut modes = None;
        loop {
            match self.next().await {
                SessionEvent::Ready { .. } => return modes,
                SessionEvent::ModesChanged { current, available } => {
                    modes = Some(Modes { current, available });
                }
                other => panic!("unexpected event before ready: {other:?}"),
            }
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
        self.turn_with(text, Vec::new(), decision).await
    }

    /// [`Session::turn`], with `attachments` sent along.
    async fn turn_with(
        &mut self,
        text: &str,
        attachments: Vec<Attachment>,
        decision: Decision,
    ) -> TurnRecord {
        self.prompt_with("t", text, attachments);
        let mut record = TurnRecord {
            answer: String::new(),
            outcome: TurnOutcome::Complete,
            asked: Vec::new(),
            finished: Vec::new(),
            modes: Vec::new(),
        };
        loop {
            match self.next().await {
                SessionEvent::MessageChunk { text, .. } => record.answer.push_str(&text),
                SessionEvent::PermissionRequested {
                    question, reply, ..
                } => {
                    record.asked.push(*question);
                    let _ = reply.send(decision.clone());
                }
                SessionEvent::ToolCallFinished {
                    tool_call_id,
                    success,
                    ..
                } => record.finished.push((tool_call_id, success)),
                SessionEvent::ModesChanged { current, .. } => record.modes.push(current),
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
    otto_agents::i18n::pin_source_locale();
    let mut session = Session::launch(None, permissions(PermissionPolicy::Ask));
    session.ready().await;

    let allow = Decision::Approve(Some("allow".into()));
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
        .map(|option| (option.id.as_str(), option.label.as_str(), option.allow()))
        .collect();
    assert_eq!(
        options,
        [("allow", "Allow", true), ("reject", "Reject", false)]
    );
}

#[tokio::test]
async fn under_ask_a_denial_rejects() {
    let mut session = Session::launch(None, permissions(PermissionPolicy::Ask));
    session.ready().await;

    let record = session.turn("needs-permission", Decision::deny()).await;
    assert_eq!(record.answer, "reject");
    assert!(record.finished.is_empty());
}

#[tokio::test]
async fn under_deny_nobody_is_asked() {
    let mut session = Session::launch(None, permissions(PermissionPolicy::Deny));
    session.ready().await;

    let allow = Decision::Approve(None);
    let record = session.turn("needs-permission", allow).await;
    assert_eq!(record.answer, "reject");
    assert!(record.asked.is_empty());
}

#[tokio::test]
async fn a_prompt_streams_chunks_and_completes() {
    let mut session = Session::start(None);
    session.ready().await;

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

/// A picture the agent sends arrives as a file, in its place in the answer:
/// between the words said before it and the words said after.
#[tokio::test]
async fn a_picture_the_agent_sends_becomes_a_file_in_the_answer() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let cache = ImageCache::at(dir.path().join("images")).expect("a cache");
    let mut session = Session::start_with_images(cache);
    session.ready().await;

    session.prompt("t1", "draw");
    let mut said = Vec::new();
    loop {
        match session.next().await {
            SessionEvent::MessageChunk { text, .. } => said.push(text),
            SessionEvent::MessageImage { turn_id, image } => {
                assert_eq!(turn_id, "t1");
                assert_eq!(image.media_type, "image/png");
                assert!(image.path.is_file(), "{}", image.path.display());
                assert!(image.bytes > 0);
                // The URI the agent gave names the picture, so the file the
                // chat points at is named after it rather than after nothing.
                let name = image.path.file_name().unwrap().to_string_lossy();
                assert!(name.starts_with("mock-up-png-"), "{name}");
                said.push(format!("<{name}>"));
            }
            SessionEvent::TurnEnded { outcome, .. } => {
                assert!(matches!(outcome, TurnOutcome::Complete), "{outcome:?}");
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(said.len(), 3, "{said:?}");
    assert_eq!(said[0], "here:");
    assert!(said[1].starts_with("<mock-up-png-"), "{said:?}");
    assert_eq!(said[2], "that is all");
}

/// The path a picture actually takes: a tool returns it. The model never emits
/// an image of its own, so a picture that only arrived through
/// `agent_message_chunk` would never be seen.
#[tokio::test]
async fn a_picture_a_tool_returns_goes_in_the_answer() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let cache = ImageCache::at(dir.path().join("images")).expect("a cache");
    let mut session = Session::start_with_images(cache);
    session.ready().await;

    session.prompt("t1", "read-a-picture");
    let mut pictures = Vec::new();
    let mut words = Vec::new();
    let mut finished = Vec::new();
    loop {
        match session.next().await {
            SessionEvent::MessageImage { image, .. } => pictures.push(image),
            SessionEvent::MessageChunk { text, .. } => words.push(text),
            SessionEvent::ToolCallFinished { tool_call_id, .. } => finished.push(tool_call_id),
            SessionEvent::TurnEnded { outcome, .. } => {
                assert!(matches!(outcome, TurnOutcome::Complete), "{outcome:?}");
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    // Resent content shows the picture once, not once per update.
    assert_eq!(pictures.len(), 1, "{pictures:?}");
    assert_eq!(pictures[0].media_type, "image/jpeg");
    assert!(pictures[0].path.is_file());
    let name = pictures[0].path.file_name().unwrap().to_string_lossy();
    assert!(name.starts_with("shot-1-jpg-"), "{name}");
    assert_eq!(words, ["that is your screenshot"]);
    assert_eq!(finished.len(), 2, "both updates said the call had finished");
}

/// Without somewhere to keep it, a picture is dropped and the words around it
/// still arrive: the answer reads short, not broken.
#[tokio::test]
async fn a_picture_with_nowhere_to_go_is_dropped() {
    let mut session = Session::start(None);
    session.ready().await;

    session.prompt("t1", "draw");
    let mut said = Vec::new();
    loop {
        match session.next().await {
            SessionEvent::MessageChunk { text, .. } => said.push(text),
            SessionEvent::TurnEnded { .. } => break,
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(said, ["here:", "that is all"]);
}

#[tokio::test]
async fn attachments_reach_the_agent_as_resource_links() {
    let mut session = Session::start(None);
    session.ready().await;

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

/// A folder holding `notes.md` and `other.md`, and `notes.md` as an
/// attachment.
fn attached_notes() -> (tempfile::TempDir, Attachment) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["notes.md", "other.md"] {
        std::fs::write(dir.path().join(name), "x").unwrap();
    }
    let notes = Attachment {
        name: "notes.md".into(),
        uri: otto_agents::uri::from_path(&dir.path().join("notes.md")),
    };
    (dir, notes)
}

#[tokio::test]
async fn reading_an_attached_file_needs_no_permission() {
    let (dir, notes) = attached_notes();
    for policy in [PermissionPolicy::Ask, PermissionPolicy::Deny] {
        let mut session = Session::launch(None, permissions(policy));
        session.ready().await;

        let read = format!("read:{}", dir.path().join("notes.md").display());
        let record = session
            .turn_with(&read, vec![notes.clone()], Decision::deny())
            .await;
        assert_eq!(record.answer, "allow", "{policy:?}");
        assert!(record.asked.is_empty(), "{policy:?}: nobody is asked");

        // The allowance lasts for the session, not just the turn.
        let record = session.turn(&read, Decision::deny()).await;
        assert_eq!(record.answer, "allow", "{policy:?}");
        assert!(record.asked.is_empty(), "{policy:?}");
    }
}

#[tokio::test]
async fn reading_anything_else_still_asks() {
    let (dir, notes) = attached_notes();
    let mut session = Session::launch(None, permissions(PermissionPolicy::Ask));
    session.ready().await;

    let read = format!("read:{}", dir.path().join("other.md").display());
    let record = session
        .turn_with(&read, vec![notes], Decision::deny())
        .await;
    assert_eq!(record.answer, "reject");
    assert_eq!(record.asked.len(), 1, "a file next to the attached one");

    let record = session.turn("needs-permission", Decision::deny()).await;
    assert_eq!(record.answer, "reject");
    assert_eq!(record.asked.len(), 1, "a command, with a file attached");
}

#[tokio::test]
async fn an_attachment_allows_nothing_in_another_session() {
    let (dir, notes) = attached_notes();
    let mut first = Session::launch(None, permissions(PermissionPolicy::Ask));
    first.ready().await;
    let read = format!("read:{}", dir.path().join("notes.md").display());
    let record = first.turn_with(&read, vec![notes], Decision::deny()).await;
    assert_eq!(record.answer, "allow");

    let mut second = Session::launch(None, permissions(PermissionPolicy::Ask));
    second.ready().await;
    let record = second.turn(&read, Decision::deny()).await;
    assert_eq!(record.answer, "reject");
    assert_eq!(record.asked.len(), 1);
}

#[tokio::test]
async fn a_configured_model_is_set_before_the_session_is_ready() {
    let mut session = Session::start(Some("haiku"));
    session.ready().await;
    assert_eq!(
        session.recorded.lock().unwrap().model.as_deref(),
        Some("haiku")
    );
}

#[tokio::test]
async fn without_a_model_the_agent_keeps_its_own() {
    let mut session = Session::start(None);
    session.ready().await;
    assert_eq!(session.recorded.lock().unwrap().model, None);
}

/// Codex only offers its question tool in the `plan` collaboration mode, and
/// that mode is an option, not an ACP mode: what `config` is for.
#[tokio::test]
async fn configured_options_are_set_before_the_session_is_ready() {
    let mut session = Session::start_with_options(&[("collaboration_mode", "plan")]);
    session.ready().await;
    assert_eq!(
        session
            .recorded
            .lock()
            .unwrap()
            .options
            .get("collaboration_mode"),
        Some(&"plan".to_string())
    );
}

/// An option is a preference, so a session the agent refuses it for is still
/// worth having — and the options after it are still set.
#[tokio::test]
async fn an_option_the_agent_refuses_leaves_the_session_running() {
    let mut session =
        Session::start_with_options(&[(REFUSED_OPTION, "on"), ("collaboration_mode", "plan")]);
    session.ready().await;
    let recorded = session.recorded.lock().unwrap();
    assert_eq!(
        recorded.options.get("collaboration_mode"),
        Some(&"plan".to_string())
    );
    assert!(!recorded.options.contains_key(REFUSED_OPTION));
}

#[tokio::test]
async fn the_agents_modes_are_announced_when_the_session_opens() {
    let mut session = Session::start(None);
    let modes = session.ready().await.expect("the agent advertised modes");
    assert_eq!(modes.current, "default");
    assert_eq!(
        modes
            .available
            .iter()
            .map(|mode| (mode.id.as_str(), mode.name.as_str()))
            .collect::<Vec<_>>(),
        MODES
            .iter()
            .chain([&REFUSED_MODE])
            .copied()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        session.recorded.lock().unwrap().mode,
        None,
        "nothing was asked of it"
    );
}

#[tokio::test]
async fn a_configured_mode_is_set_on_a_new_session() {
    let mut session = Session::start_in_mode("acceptEdits");
    let modes = session.ready().await.expect("the agent advertised modes");
    assert_eq!(modes.current, "acceptEdits");
    assert_eq!(
        modes.available.len(),
        MODES.len() + 1,
        "the list stays the agent's"
    );
    assert_eq!(
        session.recorded.lock().unwrap().mode.as_deref(),
        Some("acceptEdits")
    );
}

#[tokio::test]
async fn a_configured_mode_the_agent_does_not_offer_is_ignored() {
    let mut session = Session::start_in_mode("yolo");
    let modes = session.ready().await.expect("the agent advertised modes");
    assert_eq!(modes.current, "default", "the agent keeps its own");
    assert_eq!(session.recorded.lock().unwrap().mode, None);
}

#[tokio::test]
async fn a_mode_change_from_the_agent_reaches_the_host() {
    let mut session = Session::start(None);
    session.ready().await;
    let record = session.turn("change-mode", Decision::deny()).await;
    assert_eq!(record.modes, vec!["plan".to_owned()]);
}

#[tokio::test]
async fn set_mode_reaches_the_agent() {
    let mut session = Session::start(None);
    session.ready().await;
    let command = SessionCommand::SetMode {
        mode_id: "plan".into(),
    };
    session.commands.send(command).expect("session is running");
    match session.next().await {
        SessionEvent::ModesChanged { current, available } => {
            assert_eq!(current, "plan");
            assert_eq!(available.len(), MODES.len() + 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(
        session.recorded.lock().unwrap().mode.as_deref(),
        Some("plan")
    );
}

#[tokio::test]
async fn a_mode_the_agent_refuses_is_withdrawn_from_the_list() {
    let mut session = Session::start(None);
    session.ready().await;
    let command = SessionCommand::SetMode {
        mode_id: REFUSED_MODE.0.into(),
    };
    session.commands.send(command).expect("session is running");
    match session.next().await {
        SessionEvent::ModesChanged { current, available } => {
            assert_eq!(current, "default", "the agent kept its mode");
            let ids: Vec<&str> = available.iter().map(|mode| mode.id.as_str()).collect();
            assert_eq!(ids, MODES.map(|(id, _)| id).to_vec());
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(session.recorded.lock().unwrap().mode, None);
}

#[tokio::test]
async fn cancelling_a_turn_reaches_the_agent() {
    let mut session = Session::start(None);
    session.ready().await;

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
        command: "/nonexistent/otto-agents-test-agent".into(),
        ..AgentConfig::claude()
    };
    let backend = AcpBackend::new(vec![agent], Vec::new());
    let (_commands, command_rx) = mpsc::unbounded_channel();
    let (event_tx, mut events) = mpsc::unbounded_channel();
    let spec = SessionSpec {
        provider: "claude".into(),
        cwd: std::env::temp_dir(),
        resume: None,
        attached: Vec::new(),
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
    let root = std::env::temp_dir().join(format!("otto-agents-acp-skills-{}", std::process::id()));
    let skill = root.join("otto/skills/configure-otto");
    std::fs::create_dir_all(&skill).expect("a fixture plugin");
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: configure-otto\ndescription: Change settings\n---\n",
    )
    .expect("a fixture skill");
    let plugins = otto_agents::skills::discover_in(std::slice::from_ref(&root));
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
