//! `otto-agents acp`: the service, seen from outside as one ACP agent.
//!
//! Tools that drive ACP agents — chat bridges such as OpenClaw's acpx, or an
//! editor — start `otto-agents acp` as if it were an agent and speak ACP to it
//! over stdio. Every ACP session is one of the desktop's sessions:
//! `session/new` creates one, `session/load` takes up one that is already
//! there by its id (or the start of it), and a prompt is queued on the
//! session's chat the way the launcher queues one. So a conversation begun
//! from a chat app shows up in Sessions, and one begun at the desk can be
//! carried on from the phone.
//!
//! Permission requests reach the ACP client as `session/request_permission`
//! while the desktop gets them as it always does; whichever answers first
//! wins, and the host ignores the late one.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Stdio};
use ahp::reducers::apply_action_to_session;
use ahp::{SessionSubscription, SubscriptionEvent};
use ahp_types::actions::{
    ActionEnvelope, ChatPendingMessageSetAction, ChatToolCallConfirmedAction,
    ChatToolCallReadyAction, ChatTurnCancelledAction, StateAction,
};
use ahp_types::common::StringOrMarkdown;
use ahp_types::state::{
    ChatState, ConfirmationOption, ConfirmationOptionKind, Message, MessageAttachment, MessageKind,
    MessageOrigin, MessageResourceAttachment, PendingMessageKind, ResponsePart, SessionLifecycle,
    SessionState, SnapshotState, ToolCallConfirmationReason,
};
use anyhow::{Context, bail};
use otto_agents_client::session::{self, SESSION_SCHEME};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::cli;
use crate::uri;

/// How long a new session may take to start its agent before `session/new`
/// gives up: long enough for a harness fetched with `npx` on a cold cache.
const START_TIMEOUT: Duration = Duration::from_secs(120);

/// Serves ACP on stdin and stdout until the client goes away. `agent` is the
/// service's agent new sessions run, by id or name; the default agent without
/// one.
pub async fn serve(url: &str, agent: Option<&str>) -> anyhow::Result<()> {
    let service = cli::connect(url).await?;
    let provider = cli::resolve_agent(&service, agent).await?;
    let sessions: Sessions = Arc::default();

    Agent
        .builder()
        .name("otto-agents")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                let capabilities = AgentCapabilities::new().load_session(true);
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(capabilities)
                        .agent_info(Implementation::new(
                            "otto-agents",
                            env!("CARGO_PKG_VERSION"),
                        )),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let service = service.clone();
                let sessions = Arc::clone(&sessions);
                async move |request: NewSessionRequest,
                            responder,
                            connection: ConnectionTo<Client>| {
                    let service = service.clone();
                    let sessions = Arc::clone(&sessions);
                    let provider = provider.clone();
                    // Starting the agent can take a minute; the handler must
                    // not hold up every other message meanwhile.
                    connection.clone().spawn(async move {
                        match new_session(&service, provider.as_deref(), &request.cwd).await {
                            Ok(opened) => {
                                let id = opened.id.clone();
                                open(&sessions, &service, &connection, opened);
                                responder.respond(NewSessionResponse::new(id))
                            }
                            Err(err) => responder.respond_with_error(internal(err)),
                        }
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let service = service.clone();
                let sessions = Arc::clone(&sessions);
                async move |request: LoadSessionRequest,
                            responder,
                            connection: ConnectionTo<Client>| {
                    let service = service.clone();
                    let sessions = Arc::clone(&sessions);
                    connection.clone().spawn(async move {
                        match load_session(&service, &request.session_id.0).await {
                            Ok((mut opened, chat)) => {
                                // The client goes on using the id it asked
                                // for, which may be only the start of one.
                                opened.id = request.session_id.0.to_string();
                                // The client rebuilds the conversation from
                                // the replay before the response comes.
                                replay(&connection, &opened.id, &chat);
                                open(&sessions, &service, &connection, opened);
                                responder.respond(LoadSessionResponse::new())
                            }
                            Err(err) => responder.respond_with_error(internal(err)),
                        }
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let sessions = Arc::clone(&sessions);
                async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                    let Some(commands) = lock(&sessions).get(&*request.session_id.0).cloned()
                    else {
                        return responder.respond_with_error(internal(anyhow::anyhow!(
                            "no such session: {}",
                            request.session_id.0
                        )));
                    };
                    let (done, outcome) = oneshot::channel();
                    let message = message(request.prompt);
                    if commands
                        .send(Command::Prompt {
                            message: Box::new(message),
                            done,
                        })
                        .is_err()
                    {
                        return responder.respond_with_error(internal(anyhow::anyhow!(
                            "the session has closed"
                        )));
                    }
                    // A turn runs for as long as the agent works; answered in
                    // a task of its own so cancels and answers still arrive.
                    connection.spawn(async move {
                        match outcome.await {
                            Ok(Ok(reason)) => responder.respond(PromptResponse::new(reason)),
                            Ok(Err(err)) => responder.respond_with_error(internal(err)),
                            Err(_) => responder.respond_with_error(internal(anyhow::anyhow!(
                                "the service closed the session"
                            ))),
                        }
                    })
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let sessions = Arc::clone(&sessions);
                async move |notification: CancelNotification, _connection| {
                    if let Some(commands) = lock(&sessions).get(&*notification.session_id.0) {
                        let _ = commands.send(Command::Cancel);
                    }
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
        .map_err(|err| anyhow::anyhow!("ACP connection: {err}"))
}

/// The ACP sessions this connection has open, by ACP session id, and the
/// channel to each one's [`pump`].
type Sessions = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Command>>>>;

enum Command {
    /// Queue `message` on the chat and report how its turn ended.
    Prompt {
        message: Box<Message>,
        done: oneshot::Sender<anyhow::Result<StopReason>>,
    },
    /// Cancel the prompt in hand, started or still queued.
    Cancel,
}

/// A desktop session taken up by this connection.
struct Opened {
    /// The ACP session id: the session's id without its scheme, as
    /// `otto-agents sessions` lists it.
    id: String,
    chat: String,
    events: SessionSubscription,
}

/// Starts following `opened` and makes it answer prompts.
fn open(
    sessions: &Sessions,
    service: &ahp::Client,
    connection: &ConnectionTo<Client>,
    opened: Opened,
) {
    let (commands, receiver) = mpsc::unbounded_channel();
    lock(sessions).insert(opened.id.clone(), commands);
    let pump = Pump {
        service: service.clone(),
        acp: connection.clone(),
        session: SessionId::new(opened.id.as_str()),
        chat: opened.chat,
        waiting: None,
        turn: None,
        cancel_when_started: false,
    };
    tokio::spawn(pump.run(opened.events, receiver));
}

async fn new_session(
    service: &ahp::Client,
    provider: Option<&str>,
    cwd: &Path,
) -> anyhow::Result<Opened> {
    let id = uuid::Uuid::new_v4().to_string();
    let resource = format!("{SESSION_SCHEME}{id}");
    let mut params = json!({
        "channel": resource,
        "workingDirectories": [uri::from_path(cwd)],
    });
    if let Some(provider) = provider {
        params["provider"] = json!(provider);
    }
    service
        .request::<_, Value>("createSession", params)
        .await
        .context("the service would not create the session")?;
    let (chat, _) = follow(service, &resource).await?;
    Ok(Opened {
        id,
        chat: chat.0,
        events: chat.1,
    })
}

async fn load_session(service: &ahp::Client, query: &str) -> anyhow::Result<(Opened, ChatState)> {
    let sessions = cli::fetch_sessions(service).await?;
    let found = session::find(&sessions, Some(query))?;
    let id = session::session_id(found).to_owned();
    let ((chat, events), state) = follow(service, &found.resource).await?;
    Ok((Opened { id, chat, events }, state))
}

/// Waits for `resource` to have its agent started, then subscribes to its
/// chat. Returns the chat's URI, its events and the chat as it stands.
async fn follow(
    service: &ahp::Client,
    resource: &str,
) -> anyhow::Result<((String, SessionSubscription), ChatState)> {
    let (subscribed, mut events) = service.subscribe(resource.to_owned()).await?;
    let Some(SnapshotState::Session(state)) = subscribed.snapshot.map(|s| s.state) else {
        bail!("the service sent no session snapshot for {resource}");
    };
    let state = started(*state, &mut events).await?;
    let chat = state.default_chat.context("the session has no chat")?;
    let (subscribed, chat_events) = service.subscribe(chat.clone()).await?;
    let Some(SnapshotState::Chat(snapshot)) = subscribed.snapshot.map(|s| s.state) else {
        bail!("the service sent no chat snapshot for {chat}");
    };
    service.unsubscribe(resource.to_owned()).await.ok();
    Ok(((chat, chat_events), *snapshot))
}

async fn started(
    mut state: SessionState,
    events: &mut SessionSubscription,
) -> anyhow::Result<SessionState> {
    let wait = async {
        while state.lifecycle == SessionLifecycle::Creating {
            let envelope = next_action(events).await?;
            apply_action_to_session(&mut state, &envelope.action);
        }
        anyhow::Ok(())
    };
    tokio::time::timeout(START_TIMEOUT, wait)
        .await
        .context("the agent took too long to start")??;
    if let Some(error) = &state.creation_error {
        bail!("the agent could not start: {}", error.message);
    }
    Ok(state)
}

/// Sends the chat's earlier turns to the client, as `session/load` asks.
fn replay(connection: &ConnectionTo<Client>, id: &str, chat: &ChatState) {
    for turn in &chat.turns {
        let asked = SessionUpdate::UserMessageChunk(text_chunk(&turn.message.text));
        send(connection, id, asked);
        let answer: String = turn
            .response_parts
            .iter()
            .filter_map(|part| match part {
                ResponsePart::Markdown(markdown) => Some(markdown.content.as_str()),
                _ => None,
            })
            .collect();
        if !answer.is_empty() {
            send(
                connection,
                id,
                SessionUpdate::AgentMessageChunk(text_chunk(&answer)),
            );
        }
    }
}

/// Follows one session's chat for this connection: queues its prompts,
/// streams their turns back as `session/update`, and puts the permission
/// requests to the client.
struct Pump {
    service: ahp::Client,
    acp: ConnectionTo<Client>,
    session: SessionId,
    chat: String,
    /// The prompt queued and not started yet: its pending message id.
    waiting: Option<(String, oneshot::Sender<anyhow::Result<StopReason>>)>,
    /// The prompt whose turn is running: its turn id.
    turn: Option<(String, oneshot::Sender<anyhow::Result<StopReason>>)>,
    /// A cancel came before the queued prompt started.
    cancel_when_started: bool,
}

impl Pump {
    async fn run(
        mut self,
        mut events: SessionSubscription,
        mut commands: mpsc::UnboundedReceiver<Command>,
    ) {
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Prompt { message, done }) => self.queue(*message, done).await,
                    Some(Command::Cancel) => self.cancel().await,
                    None => break,
                },
                event = events.recv() => match event {
                    Some(SubscriptionEvent::Action(envelope)) => self.on_action(envelope).await,
                    Some(_) => {}
                    None => break,
                },
            }
        }
        let gone = || Err(anyhow::anyhow!("the service closed the session"));
        if let Some((_, done)) = self.waiting.take() {
            let _ = done.send(gone());
        }
        if let Some((_, done)) = self.turn.take() {
            let _ = done.send(gone());
        }
    }

    async fn queue(&mut self, message: Message, done: oneshot::Sender<anyhow::Result<StopReason>>) {
        if self.waiting.is_some() || self.turn.is_some() {
            let _ = done.send(Err(anyhow::anyhow!("a prompt is already running")));
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let queued = StateAction::ChatPendingMessageSet(ChatPendingMessageSetAction {
            kind: PendingMessageKind::Queued,
            id: id.clone(),
            message,
        });
        if let Err(err) = self.service.dispatch(self.chat.clone(), queued).await {
            let _ = done.send(Err(err.into()));
            return;
        }
        self.cancel_when_started = false;
        self.waiting = Some((id, done));
    }

    async fn cancel(&mut self) {
        match &self.turn {
            Some((turn_id, _)) => self.cancel_turn(turn_id.clone()).await,
            // The host takes no removal of a queued message from clients;
            // the turn is cancelled as soon as it starts instead.
            None if self.waiting.is_some() => self.cancel_when_started = true,
            None => {}
        }
    }

    async fn cancel_turn(&self, turn_id: String) {
        let cancelled = StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
            turn_id,
            duration: 0,
            meta: None,
        });
        if let Err(err) = self.service.dispatch(self.chat.clone(), cancelled).await {
            tracing::warn!(%err, "could not cancel the turn");
        }
    }

    fn ours(&self, turn_id: &str) -> bool {
        self.turn.as_ref().is_some_and(|(id, _)| id == turn_id)
    }

    fn finish(&mut self, outcome: anyhow::Result<StopReason>) {
        if let Some((_, done)) = self.turn.take() {
            let _ = done.send(outcome);
        }
    }

    async fn on_action(&mut self, envelope: ActionEnvelope) {
        if let Some(reason) = envelope.rejection_reason {
            // Only our own queued prompt matters: someone else's rejected
            // action is theirs to handle.
            if let StateAction::ChatPendingMessageSet(set) = &envelope.action
                && self.waiting.as_ref().is_some_and(|(id, _)| *id == set.id)
                && let Some((_, done)) = self.waiting.take()
            {
                let _ = done.send(Err(anyhow::anyhow!(
                    "the service refused the prompt: {reason}"
                )));
            }
            return;
        }
        match envelope.action {
            StateAction::ChatTurnStarted(started) => {
                let queued = started.queued_message_id.as_deref();
                if queued.is_some() && self.waiting.as_ref().map(|(id, _)| id.as_str()) == queued {
                    let (_, done) = self.waiting.take().expect("checked");
                    self.turn = Some((started.turn_id.clone(), done));
                    if std::mem::take(&mut self.cancel_when_started) {
                        self.cancel_turn(started.turn_id).await;
                    }
                }
            }
            StateAction::ChatResponsePart(part) if self.ours(&part.turn_id) => {
                if let ResponsePart::Markdown(markdown) = part.part
                    && !markdown.content.is_empty()
                {
                    self.update(SessionUpdate::AgentMessageChunk(text_chunk(
                        &markdown.content,
                    )));
                }
            }
            StateAction::ChatDelta(delta) if self.ours(&delta.turn_id) => {
                self.update(SessionUpdate::AgentMessageChunk(text_chunk(&delta.content)));
            }
            StateAction::ChatToolCallStart(start) if self.ours(&start.turn_id) => {
                let call = ToolCall::new(start.tool_call_id, start.display_name)
                    .status(ToolCallStatus::InProgress);
                self.update(SessionUpdate::ToolCall(call));
            }
            StateAction::ChatToolCallComplete(complete) if self.ours(&complete.turn_id) => {
                let status = if complete.result.success {
                    ToolCallStatus::Completed
                } else {
                    ToolCallStatus::Failed
                };
                let fields = ToolCallUpdateFields::new().status(status);
                let update = ToolCallUpdate::new(complete.tool_call_id, fields);
                self.update(SessionUpdate::ToolCallUpdate(update));
            }
            StateAction::ChatToolCallReady(ready) if self.ours(&ready.turn_id) => {
                self.ask(ready);
            }
            StateAction::ChatInputRequested(_) => {
                // Forms have no ACP counterpart a client is sure to show.
                self.update(SessionUpdate::AgentMessageChunk(text_chunk(
                    "\n\n(The agent is asking a question; answer it on the desktop.)\n\n",
                )));
            }
            StateAction::ChatTurnComplete(end) if self.ours(&end.turn_id) => {
                self.finish(Ok(StopReason::EndTurn));
            }
            StateAction::ChatTurnCancelled(end) if self.ours(&end.turn_id) => {
                self.finish(Ok(StopReason::Cancelled));
            }
            StateAction::ChatError(error) if self.ours(&error.turn_id) => {
                let message = error.part.error.message;
                self.finish(Err(anyhow::anyhow!("the agent failed: {message}")));
            }
            _ => {}
        }
    }

    /// Puts a tool call awaiting confirmation to the client. The desktop is
    /// asked too; the first answer wins, and the host turns the later one
    /// away.
    fn ask(&self, ready: ChatToolCallReadyAction) {
        let (Some(options), None) = (ready.options, ready.confirmed) else {
            return;
        };
        let title = ready
            .confirmation_title
            .map(text_of)
            .unwrap_or_else(|| text_of(ready.invocation_message));
        let acp_options = options
            .iter()
            .map(|option| {
                PermissionOption::new(option.id.clone(), option.label.clone(), kind(option))
            })
            .collect();
        let fields = ToolCallUpdateFields::new().title(title);
        let request = RequestPermissionRequest::new(
            self.session.clone(),
            ToolCallUpdate::new(ready.tool_call_id.clone(), fields),
            acp_options,
        );
        let acp = self.acp.clone();
        let service = self.service.clone();
        let chat = self.chat.clone();
        let (turn_id, tool_call_id) = (ready.turn_id, ready.tool_call_id);
        tokio::spawn(async move {
            let answer = match acp.send_request(request).block_task().await {
                Ok(answer) => answer,
                Err(err) => {
                    tracing::warn!(%err, "the client did not answer the permission request");
                    return;
                }
            };
            let RequestPermissionOutcome::Selected(selected) = answer.outcome else {
                return;
            };
            let chosen = &*selected.option_id.0;
            let Some(option) = options.iter().find(|option| option.id == chosen) else {
                return;
            };
            let approved = option.kind == ConfirmationOptionKind::Approve;
            let confirmed = StateAction::ChatToolCallConfirmed(ChatToolCallConfirmedAction {
                turn_id,
                tool_call_id,
                meta: None,
                approved,
                confirmed: approved.then_some(ToolCallConfirmationReason::UserAction),
                reason: None,
                edited_tool_input: None,
                user_suggestion: None,
                reason_message: None,
                selected_option_id: Some(option.id.clone()),
            });
            if let Err(err) = service.dispatch(chat, confirmed).await {
                tracing::warn!(%err, "could not pass the answer on");
            }
        });
    }

    fn update(&self, update: SessionUpdate) {
        send(&self.acp, &self.session.0, update);
    }
}

fn kind(option: &ConfirmationOption) -> PermissionOptionKind {
    match option.kind {
        ConfirmationOptionKind::Approve => PermissionOptionKind::AllowOnce,
        _ => PermissionOptionKind::RejectOnce,
    }
}

/// An ACP prompt as a chat message. Text is kept as written; links become
/// attachments, which the agent may read without asking; embedded text
/// resources are inlined. Pictures and audio have nowhere to go yet.
fn message(prompt: Vec<ContentBlock>) -> Message {
    let mut text = Vec::new();
    let mut attachments = Vec::new();
    for block in prompt {
        match block {
            ContentBlock::Text(content) => text.push(content.text),
            ContentBlock::ResourceLink(link) => {
                attachments.push(MessageAttachment::Resource(MessageResourceAttachment {
                    label: link.title.clone().unwrap_or(link.name.clone()),
                    range: None,
                    display_kind: None,
                    meta: None,
                    uri: link.uri,
                    size_hint: None,
                    content_type: link.mime_type,
                    nonce: None,
                    selection: None,
                }))
            }
            ContentBlock::Resource(resource) => {
                let value = serde_json::to_value(&resource.resource).unwrap_or_default();
                if let (Some(uri), Some(body)) = (value["uri"].as_str(), value["text"].as_str()) {
                    text.push(format!("{uri}:\n```\n{body}\n```"));
                }
            }
            other => tracing::warn!(?other, "dropping a prompt part the chat cannot carry"),
        }
    }
    Message {
        text: text.join("\n\n"),
        origin: MessageOrigin {
            kind: MessageKind::User,
        },
        attachments: (!attachments.is_empty()).then_some(attachments),
        model: None,
        agent: None,
        meta: None,
    }
}

fn text_chunk(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

fn text_of(text: StringOrMarkdown) -> String {
    match text {
        StringOrMarkdown::Plain(text) => text,
        StringOrMarkdown::Markdown { markdown } => markdown,
    }
}

fn send(connection: &ConnectionTo<Client>, id: &str, update: SessionUpdate) {
    let notification = SessionNotification::new(SessionId::new(id), update);
    if let Err(err) = connection.send_notification(notification) {
        tracing::warn!(%err, "could not send a session update");
    }
}

async fn next_action(events: &mut SessionSubscription) -> anyhow::Result<ActionEnvelope> {
    loop {
        match events.recv().await {
            Some(SubscriptionEvent::Action(envelope)) => return Ok(envelope),
            Some(_) => {}
            None => bail!("the service closed the connection"),
        }
    }
}

fn internal(err: anyhow::Error) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(format!("{err:#}"))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
