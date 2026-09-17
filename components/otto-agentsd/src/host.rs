//! Authoritative host state: sessions, chats and subscriptions.
//!
//! Every mutation happens under one lock, which reduces the action with the
//! same `ahp::reducers` clients use, assigns its `serverSeq`, and queues the
//! resulting messages. Responses to requests are queued under the same lock,
//! so each connection sees snapshots, responses and envelopes in a consistent
//! order.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use ahp::reducers::{
    ReduceOutcome, apply_action_to_chat, apply_action_to_root, apply_action_to_session,
};
use ahp_types::actions::{
    ActionEnvelope, ActionOrigin, ChatDeltaAction, ChatErrorAction, ChatInputAnswerChangedAction,
    ChatInputCompletedAction, ChatInputRequestedAction, ChatPendingMessageRemovedAction,
    ChatPendingMessageSetAction, ChatReasoningAction, ChatResponsePartAction,
    ChatToolCallCompleteAction, ChatToolCallConfirmedAction, ChatToolCallReadyAction,
    ChatToolCallStartAction, ChatTurnCancelledAction, ChatTurnCompleteAction,
    ChatTurnStartedAction, PartialChatSummary, RootActiveSessionsChangedAction,
    SessionChatAddedAction, SessionChatUpdatedAction, SessionCreationFailedAction,
    SessionDefaultChatChangedAction, SessionInputNeededRemovedAction, SessionInputNeededSetAction,
    SessionMetaChangedAction, SessionReadyAction, SessionTitleChangedAction, StateAction,
};
use ahp_types::commands::{
    CreateSessionParams, DispatchActionParams, Implementation, InitializeParams, InitializeResult,
    ListSessionsResult, SubscribeParams, SubscribeResult, UnsubscribeParams,
};
use ahp_types::common::{JsonObject, StringOrMarkdown};
use ahp_types::errors::ahp_error_codes;
use ahp_types::notifications::SessionAddedParams;
use ahp_types::state::{
    ChatInputAnswer, ChatInputAnswerValue, ChatInputAnswered, ChatInputOption, ChatInputQuestion,
    ChatInputRequest, ChatInputResponseKind, ChatInputSelectedAnswerValue, ChatOrigin, ChatState,
    ChatSummary, ConfirmationOption, ConfirmationOptionKind, ErrorInfo, ErrorResponsePart,
    MarkdownResponsePart, Message, MessageAttachment, PendingMessageKind, ReasoningResponsePart,
    ResponsePart, RootState, SessionChatInputRequest, SessionInputRequest, SessionLifecycle,
    SessionState, SessionStatus, SessionSummary, Snapshot, SnapshotState,
    ToolCallCancellationReason, ToolCallConfirmationReason, ToolCallResult,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Attachment, Backend, Decision, InputAnswer, Question, QuestionOption, SessionCommand,
    SessionEvent, SessionSpec, TurnOutcome,
};
use crate::dialog::{self, Choice, Prompt, Prompter, Reply};
use crate::elicitation;
use crate::rpc::{self, RpcError};
use crate::store::{SessionRecord, Store};
use crate::uri;

/// Protocol versions this server implements, most preferred first.
///
/// Only the version of the pinned `ahp-types` crate is listed: the wire types
/// are generated for exactly that version, so offering older ones would
/// promise behaviour this server does not implement.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[PROTOCOL_VERSION];

const SESSION_SCHEME: &str = "ahp-session:/";
const TITLE_MAX_CHARS: usize = 80;

pub type ConnId = u64;

/// A message queued for one connection's writer.
#[derive(Debug)]
pub enum Outgoing {
    Message(Value),
    Close,
}

/// Protocol state for one client connection, owned by its connection task.
pub struct Connection {
    id: ConnId,
    client_id: Option<String>,
    close_requested: bool,
}

impl Connection {
    /// Whether the transport must be closed once queued messages are sent.
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }
}

pub struct Host {
    /// Asks the user the questions no client is watching the chat to answer.
    prompter: Arc<dyn Prompter>,
    /// Where sessions are kept between runs, when they are.
    store: Option<Store>,
    /// Held while saving, so two saves never write out of order.
    saving: Mutex<()>,
    state: Mutex<HostState>,
    next_conn: AtomicU64,
}

struct HostState {
    /// The host itself, for the tasks that forward a session's events to it.
    host: Weak<Host>,
    backend: Arc<dyn Backend>,
    /// Sessions changed since they were last saved. `None` when sessions are
    /// not kept.
    unsaved: Option<HashSet<String>>,
    /// `serverSeq` of the last action applied to any channel.
    server_seq: u64,
    root: RootState,
    sessions: BTreeMap<String, Session>,
    chats: HashMap<String, ChatState>,
    /// Chat URI to the URI of the session that owns it.
    chat_sessions: HashMap<String, String>,
    peers: HashMap<ConnId, Peer>,
    /// How long a session's agent may sit with nothing to do before it is
    /// stopped. `None` keeps agents running.
    idle_timeout: Option<Duration>,
}

struct Peer {
    outbox: mpsc::UnboundedSender<Outgoing>,
    subscriptions: HashSet<String>,
}

struct Session {
    state: SessionState,
    created_at: String,
    chat: String,
    /// The running agent's commands. `None` until the agent starts, which for
    /// a session restored from the store is when it is next asked something.
    commands: Option<mpsc::UnboundedSender<SessionCommand>>,
    /// The agent's own id for the session, so a new agent process can take
    /// it up again.
    agent_session: Option<String>,
    /// The response part currently receiving streamed chunks.
    open_part: Option<OpenPart>,
    /// Status and title last announced with `root/sessionSummaryChanged`.
    announced: (u32, String),
    /// The agent's permission questions waiting for an answer, by tool call id.
    questions: HashMap<String, Pending>,
    /// The agent's input requests waiting for an answer, by request id.
    inputs: HashMap<String, PendingInput>,
    /// Tool calls that were allowed and have not finished, with the text that
    /// names them.
    tools: HashMap<String, String>,
    /// Counts the session's events and commands, so an idle countdown can
    /// tell whether anything happened while it ran.
    activity: u64,
}

/// A permission question, open in the chat as a tool call waiting for
/// confirmation.
struct Pending {
    turn_id: String,
    reply: oneshot::Sender<Decision>,
    prompt: Prompt,
    /// How the tool call is named in the chat.
    display: String,
    options: Vec<QuestionOption>,
    /// Whether it has also been put to the user in a dialog.
    escalated: bool,
}

/// An agent's question, open in the chat as an input request and mirrored in
/// the session's `inputNeeded`.
struct PendingInput {
    turn_id: String,
    reply: oneshot::Sender<InputAnswer>,
    /// Whether it has also been put to the user in a dialog.
    escalated: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum PartKind {
    Markdown,
    Reasoning,
}

struct OpenPart {
    kind: PartKind,
    id: String,
}

impl Host {
    pub fn new(backend: Arc<dyn Backend>, prompter: Arc<dyn Prompter>) -> Arc<Self> {
        Self::with_store(backend, prompter, None)
    }

    /// Like [`Host::new`], keeping sessions in `store` between runs. The
    /// sessions stored there are served again from the start, and changes are
    /// written back by [`Host::save`].
    pub fn with_store(
        backend: Arc<dyn Backend>,
        prompter: Arc<dyn Prompter>,
        store: Option<Store>,
    ) -> Arc<Self> {
        let root = RootState {
            agents: backend.agents(),
            active_sessions: Some(0),
            terminals: None,
            config: None,
            meta: None,
        };
        let records = store.as_ref().map(Store::load).unwrap_or_default();
        let unsaved = store.as_ref().map(|_| HashSet::new());
        let host = Arc::new_cyclic(|host| Self {
            prompter,
            store,
            saving: Mutex::new(()),
            state: Mutex::new(HostState {
                host: host.clone(),
                backend,
                unsaved,
                server_seq: 0,
                root,
                sessions: BTreeMap::new(),
                chats: HashMap::new(),
                chat_sessions: HashMap::new(),
                peers: HashMap::new(),
                idle_timeout: None,
            }),
            next_conn: AtomicU64::new(1),
        });
        host.lock().restore(records);
        host
    }

    /// Stops a session's agent once it has had nothing to do for `timeout`;
    /// its next request starts it again. `None`, the default, keeps agents
    /// running until the host stops.
    pub fn set_idle_timeout(&self, timeout: Option<Duration>) {
        let mut state = self.lock();
        state.idle_timeout = timeout;
        let uris: Vec<String> = state.sessions.keys().cloned().collect();
        for uri in uris {
            state.touch(&uri);
        }
    }

    /// Writes the sessions changed since the last save to the store.
    pub fn save(&self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let _saving = self.saving.lock().unwrap_or_else(PoisonError::into_inner);
        let records = self.lock().take_unsaved();
        for record in records {
            if let Err(err) = store.save(&record) {
                tracing::warn!(session = %record.resource, %err, "could not store the session");
            }
        }
    }

    /// Saves every `period` in the background, for as long as the host lives.
    pub fn save_periodically(self: &Arc<Self>, period: Duration) {
        if self.store.is_none() {
            return;
        }
        let host = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(period);
            loop {
                ticks.tick().await;
                let Some(host) = host.upgrade() else { break };
                // Writing files blocks, so it happens off the runtime's threads.
                if tokio::task::spawn_blocking(move || host.save())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    fn lock(&self) -> MutexGuard<'_, HostState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn connect(&self, outbox: mpsc::UnboundedSender<Outgoing>) -> Connection {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let peer = Peer {
            outbox,
            subscriptions: HashSet::new(),
        };
        self.lock().peers.insert(id, peer);
        Connection {
            id,
            client_id: None,
            close_requested: false,
        }
    }

    /// Forgets a connection. Its sessions keep running, and questions it was
    /// the only one watching go to the user in a dialog.
    pub fn disconnect(self: &Arc<Self>, conn: &Connection) {
        let mut state = self.lock();
        state.peers.remove(&conn.id);
        self.escalate_unwatched(&mut state);
    }

    pub fn handle_request(
        self: &Arc<Self>,
        conn: &mut Connection,
        id: Value,
        method: &str,
        params: Value,
    ) {
        let mut state = self.lock();
        let result = match method {
            // The spec requires answering `ping` even before `initialize`.
            "ping" => Ok(Value::Null),
            "initialize" => parse_params(params).and_then(|p| state.initialize(conn, p)),
            _ if conn.client_id.is_none() => Err(RpcError::invalid_request(
                "initialize must be the first request",
            )),
            "subscribe" => parse_params(params).and_then(|p| state.subscribe(conn, p)),
            "listSessions" => to_result(&state.list_sessions()),
            "createSession" => {
                parse_params(params).and_then(|p| self.create_session(&mut state, p))
            }
            _ => Err(RpcError::method_not_found(method)),
        };
        let response = match result {
            Ok(result) => rpc::success_response(id, result),
            Err(err) => rpc::error_response(id, &err),
        };
        state.send(conn.id, Outgoing::Message(response));
    }

    pub fn handle_notification(self: &Arc<Self>, conn: &Connection, method: &str, params: Value) {
        match method {
            "unsubscribe" => match parse_params::<UnsubscribeParams>(params) {
                Ok(params) => {
                    let mut state = self.lock();
                    if let Some(peer) = state.peers.get_mut(&conn.id) {
                        peer.subscriptions.remove(&params.channel);
                    }
                    self.escalate_unwatched(&mut state);
                }
                Err(err) => tracing::debug!("ignoring malformed unsubscribe: {}", err.message),
            },
            "dispatchAction" => match parse_params::<DispatchActionParams>(params) {
                Ok(params) => self.dispatch_client_action(conn, params),
                Err(err) => tracing::debug!("ignoring malformed dispatchAction: {}", err.message),
            },
            _ => tracing::debug!(method, "ignoring unknown notification"),
        }
    }

    fn create_session(
        self: &Arc<Self>,
        state: &mut HostState,
        params: CreateSessionParams,
    ) -> Result<Value, RpcError> {
        let uri = params.channel;
        if uri.len() <= SESSION_SCHEME.len() || !uri.starts_with(SESSION_SCHEME) {
            return Err(RpcError::invalid_params(format!(
                "session URIs look like {SESSION_SCHEME}<id>"
            )));
        }
        if state.sessions.contains_key(&uri) {
            return Err(RpcError::new(
                ahp_error_codes::SESSION_ALREADY_EXISTS,
                format!("session already exists: {uri}"),
            ));
        }
        let provider = match params.provider {
            Some(provider) => provider,
            None => state
                .root
                .agents
                .first()
                .map(|agent| agent.provider.clone())
                .ok_or_else(|| {
                    RpcError::new(
                        ahp_error_codes::PROVIDER_NOT_FOUND,
                        "no agents are configured",
                    )
                })?,
        };
        if !state
            .root
            .agents
            .iter()
            .any(|agent| agent.provider == provider)
        {
            return Err(RpcError::new(
                ahp_error_codes::PROVIDER_NOT_FOUND,
                format!("unknown agent: {provider}"),
            ));
        }
        let cwd = working_directory(params.working_directories)?;
        // What the agent is given, repeated on the session so a client sees it
        // without having to join the two lists itself. Read-only: these are the
        // desktop's own skills, and this host has no way to write into them.
        let customizations = state
            .root
            .agents
            .iter()
            .find(|agent| agent.provider == provider)
            .and_then(|agent| agent.customizations.clone());

        let now = now();
        let chat_uri = format!("ahp-chat:/{}", uuid::Uuid::new_v4());
        let session = Session {
            state: SessionState {
                customizations,
                ..new_session_state(&provider, &uri::from_path(&cwd))
            },
            created_at: now.clone(),
            chat: chat_uri.clone(),
            commands: None,
            agent_session: None,
            open_part: None,
            announced: (SessionStatus::Idle.bits(), String::new()),
            questions: HashMap::new(),
            inputs: HashMap::new(),
            tools: HashMap::new(),
            activity: 0,
        };
        state.sessions.insert(uri.clone(), session);
        state
            .chats
            .insert(chat_uri.clone(), new_chat_state(&chat_uri, &now));
        state.chat_sessions.insert(chat_uri.clone(), uri.clone());

        let summary = ChatSummary {
            resource: chat_uri.clone(),
            title: String::new(),
            status: SessionStatus::Idle.bits(),
            activity: None,
            modified_at: now,
            origin: Some(ChatOrigin::User),
            interactivity: None,
            working_directories: None,
        };
        state.apply(
            &uri,
            StateAction::SessionChatAdded(SessionChatAddedAction { summary }),
            None,
        );
        state.apply(
            &uri,
            StateAction::SessionDefaultChatChanged(SessionDefaultChatChangedAction {
                default_chat: Some(chat_uri),
            }),
            None,
        );
        let active_sessions = state.sessions.len() as i64;
        state.apply(
            ROOT_RESOURCE_URI,
            StateAction::RootActiveSessionsChanged(RootActiveSessionsChangedAction {
                active_sessions,
            }),
            None,
        );
        if let Some(session) = state.sessions.get(&uri) {
            let added = SessionAddedParams {
                channel: ROOT_RESOURCE_URI.to_owned(),
                summary: state.summary(&uri, session),
            };
            state.broadcast(ROOT_RESOURCE_URI, "root/sessionAdded", &added);
        }

        state.launch(&uri);
        Ok(Value::Null)
    }

    fn dispatch_client_action(&self, conn: &Connection, params: DispatchActionParams) {
        let Some(client_id) = conn.client_id.clone() else {
            return;
        };
        let origin = ActionOrigin {
            client_id,
            client_seq: params.client_seq,
        };
        let channel = params.channel;
        let mut state = self.lock();

        let Some(session_uri) = state.chat_sessions.get(&channel).cloned() else {
            if channel == ROOT_RESOURCE_URI || state.sessions.contains_key(&channel) {
                let reason = "this host does not accept client actions on this channel yet";
                state.reject(conn.id, channel, params.action, origin, reason);
            }
            // Actions on unknown channels are ignored silently, as the spec requires.
            return;
        };

        let accepted = match &params.action {
            StateAction::ChatTurnStarted(_) => state.check_turn_start(&session_uri, &channel),
            StateAction::ChatTurnCancelled(action) => {
                state.check_turn_cancel(&channel, &action.turn_id)
            }
            StateAction::ChatPendingMessageSet(action) => HostState::check_queued(action),
            StateAction::ChatToolCallConfirmed(action) => {
                state.check_confirmation(&session_uri, action)
            }
            StateAction::ChatInputAnswerChanged(action) => {
                state.check_answer(&session_uri, &channel, action)
            }
            StateAction::ChatInputCompleted(action) => {
                state.check_completion(&session_uri, &channel, action)
            }
            _ => Err("this host does not support this action yet"),
        };
        if let Err(reason) = accepted {
            state.reject(conn.id, channel, params.action, origin, reason);
            return;
        }

        match params.action {
            StateAction::ChatTurnStarted(action) => {
                state.start_turn(&session_uri, action, Some(origin))
            }
            StateAction::ChatTurnCancelled(action) => {
                let turn_id = action.turn_id.clone();
                let cancelled = StateAction::ChatTurnCancelled(action);
                state.apply(&channel, cancelled, Some(origin));
                state.drop_turn_requests(&session_uri, &turn_id);
                state.send_command(&session_uri, SessionCommand::Cancel { turn_id });
            }
            StateAction::ChatToolCallConfirmed(action) => {
                state.confirm(&session_uri, &channel, action, Some(origin))
            }
            StateAction::ChatInputAnswerChanged(action) => {
                let request_id = action.request_id.clone();
                let changed = StateAction::ChatInputAnswerChanged(action);
                state.apply(&channel, changed, Some(origin));
                state.mirror_input(&session_uri, &channel, &request_id);
            }
            StateAction::ChatInputCompleted(action) => {
                state.complete_input(&session_uri, &channel, action, Some(origin))
            }
            queued => {
                state.apply(&channel, queued, Some(origin));
                state.start_next_queued(&session_uri);
            }
        }
        state.sync_summaries(&session_uri);
        state.touch(&session_uri);
    }

    fn on_session_event(self: &Arc<Self>, session_uri: &str, event: SessionEvent) {
        let mut state = self.lock();
        let Some(chat_uri) = state.sessions.get(session_uri).map(|s| s.chat.clone()) else {
            return;
        };
        match event {
            SessionEvent::Ready { agent_session } => {
                state.agent_ready(session_uri, agent_session);
                state.start_next_queued(session_uri);
            }
            SessionEvent::CreationFailed(message) => {
                state.agent_failed(session_uri, &chat_uri, message)
            }
            SessionEvent::MessageChunk { turn_id, text } => {
                state.stream(session_uri, &chat_uri, turn_id, PartKind::Markdown, text)
            }
            SessionEvent::ThoughtChunk { turn_id, text } => {
                state.stream(session_uri, &chat_uri, turn_id, PartKind::Reasoning, text)
            }
            SessionEvent::TurnEnded { turn_id, outcome } => {
                state.end_turn(session_uri, &chat_uri, turn_id, outcome);
                // The agent is free again, so the next queued message can run.
                state.start_next_queued(session_uri);
            }
            SessionEvent::PermissionRequested {
                turn_id,
                question,
                reply,
            } => {
                let tool_call_id = question.tool_call_id.clone();
                let opened = state.open_question(session_uri, &chat_uri, turn_id, question, reply);
                // A client watching the chat can answer it there; otherwise
                // the user is asked in a dialog.
                if opened && !state.watched(&chat_uri) {
                    self.escalate(&mut state, session_uri, &tool_call_id);
                }
            }
            SessionEvent::InputRequested {
                turn_id,
                request,
                reply,
            } => {
                let request_id = request.id.clone();
                let opened = state.open_input(session_uri, &chat_uri, turn_id, request, reply);
                if opened && !state.watched(&chat_uri) {
                    self.escalate_input(&mut state, session_uri, &request_id);
                }
            }
            SessionEvent::ToolCallFinished {
                turn_id,
                tool_call_id,
                success,
            } => state.finish_tool(session_uri, &chat_uri, turn_id, &tool_call_id, success),
        }
        state.sync_summaries(session_uri);
        state.touch(session_uri);
    }

    /// Puts a pending question to the user through the prompter, and answers
    /// it with what they say there, unless a client answers it first.
    fn escalate(self: &Arc<Self>, state: &mut HostState, session_uri: &str, tool_call_id: &str) {
        let Some(pending) = state
            .sessions
            .get_mut(session_uri)
            .and_then(|session| session.questions.get_mut(tool_call_id))
        else {
            return;
        };
        if pending.escalated {
            return;
        }
        pending.escalated = true;
        tracing::info!(
            session_uri,
            tool_call_id,
            "no client is watching; asking in a dialog"
        );
        let prompt = Prompt {
            open: dialog::OPEN_IN_ASK.into(),
            ..pending.prompt.clone()
        };
        let prompter = Arc::clone(&self.prompter);
        let host = Arc::downgrade(self);
        let (session_uri, tool_call_id) = (session_uri.to_owned(), tool_call_id.to_owned());
        tokio::spawn(async move {
            let reply = prompter.ask(prompt).await;
            let Some(host) = host.upgrade() else {
                return;
            };
            match reply {
                Reply::Open => {
                    host.lock().unescalate(&session_uri, &tool_call_id);
                    prompter.open(&session_uri);
                }
                reply => {
                    let granted = matches!(reply, Reply::Granted(_));
                    host.lock()
                        .answer_from_dialog(&session_uri, &tool_call_id, granted);
                }
            }
        });
    }

    /// Puts a pending input request to the user through the prompter, and
    /// answers it with what they say there, unless a client answers it first.
    /// Select questions are asked in the dialog itself; anything else can only
    /// be answered in Ask, which the dialog offers to open.
    fn escalate_input(
        self: &Arc<Self>,
        state: &mut HostState,
        session_uri: &str,
        request_id: &str,
    ) {
        let Some(session) = state.sessions.get(session_uri) else {
            return;
        };
        let Some(request) = state.input_request(&session.chat, request_id).cloned() else {
            return;
        };
        let agent = state
            .root
            .agents
            .iter()
            .find(|agent| agent.provider == session.state.provider)
            .map_or_else(
                || "The agent".to_owned(),
                |agent| agent.display_name.clone(),
            );
        let cwd = session_cwd(&session.state);
        let Some(pending) = state
            .sessions
            .get_mut(session_uri)
            .and_then(|session| session.inputs.get_mut(request_id))
        else {
            return;
        };
        if pending.escalated {
            return;
        }
        pending.escalated = true;
        tracing::info!(
            session_uri,
            request_id,
            "no client is watching; asking in a dialog"
        );
        let prompt = question_prompt(&agent, &cwd, &request);
        let prompter = Arc::clone(&self.prompter);
        let host = Arc::downgrade(self);
        let (session_uri, request_id) = (session_uri.to_owned(), request_id.to_owned());
        tokio::spawn(async move {
            let reply = prompter.ask(prompt).await;
            let Some(host) = host.upgrade() else {
                return;
            };
            let (response, answers) = match reply {
                Reply::Granted(selections) => (ChatInputResponseKind::Accept, selected(selections)),
                // Skipped, or dismissed: the agent carries on without an
                // answer, as it does when a permission dialog goes away.
                Reply::Denied | Reply::Ended => (ChatInputResponseKind::Decline, HashMap::new()),
                Reply::Open => {
                    host.lock().unescalate_input(&session_uri, &request_id);
                    prompter.open(&session_uri);
                    return;
                }
                // The question stays in the chat, for a client to answer.
                Reply::Unavailable => return,
            };
            let mut state = host.lock();
            let Some(chat_uri) = state.sessions.get(&session_uri).map(|s| s.chat.clone()) else {
                return;
            };
            let action = ChatInputCompletedAction {
                request_id,
                response,
                answers: (!answers.is_empty()).then_some(answers),
            };
            state.complete_input(&session_uri, &chat_uri, action, None);
            state.sync_summaries(&session_uri);
            state.touch(&session_uri);
        });
    }

    /// Escalates every pending question that no client is watching any more.
    fn escalate_unwatched(self: &Arc<Self>, state: &mut HostState) {
        let mut questions = Vec::new();
        let mut inputs = Vec::new();
        for (uri, session) in &state.sessions {
            if state.watched(&session.chat) {
                continue;
            }
            questions.extend(
                session
                    .questions
                    .iter()
                    .filter(|(_, pending)| !pending.escalated)
                    .map(|(id, _)| (uri.clone(), id.clone())),
            );
            inputs.extend(
                session
                    .inputs
                    .iter()
                    .filter(|(_, pending)| !pending.escalated)
                    .map(|(id, _)| (uri.clone(), id.clone())),
            );
        }
        for (session_uri, tool_call_id) in questions {
            self.escalate(state, &session_uri, &tool_call_id);
        }
        for (session_uri, request_id) in inputs {
            self.escalate_input(state, &session_uri, &request_id);
        }
    }
}

impl HostState {
    /// Serves the sessions an earlier run stored. Their agents start when they
    /// are next asked something. A turn that was running when that run ended
    /// never finished, so it ends in an error; requests queued behind it start.
    fn restore(&mut self, records: Vec<SessionRecord>) {
        let mut restored = Vec::new();
        for record in records {
            let SessionRecord {
                resource,
                created_at,
                agent_session,
                session: mut state,
                chat,
                ..
            } = record;
            if self.sessions.contains_key(&resource) || self.chats.contains_key(&chat.resource) {
                continue;
            }
            // A session whose agent never started gets another go with its
            // next request.
            if !matches!(state.lifecycle, SessionLifecycle::Ready) {
                state.lifecycle = SessionLifecycle::Ready;
                state.creation_error = None;
            }
            // The questions an earlier run was waiting on went with its agent.
            for request in state.input_needed.clone().into_iter().flatten() {
                if let SessionInputRequest::ChatInput(request) = request {
                    let removed =
                        StateAction::SessionInputNeededRemoved(SessionInputNeededRemovedAction {
                            id: request.id,
                        });
                    apply_action_to_session(&mut state, &removed);
                }
            }
            // From today's configuration, which may name another terminal
            // than the one the session was saved with.
            state.meta = terminal_meta(self.backend.as_ref(), &state, agent_session.as_deref());
            let chat_uri = chat.resource.clone();
            let session = Session {
                announced: (chat.status, state.title.clone()),
                state,
                created_at,
                chat: chat_uri.clone(),
                commands: None,
                agent_session,
                open_part: None,
                questions: HashMap::new(),
                inputs: HashMap::new(),
                tools: HashMap::new(),
                activity: 0,
            };
            self.sessions.insert(resource.clone(), session);
            self.chats.insert(chat_uri.clone(), chat);
            self.chat_sessions
                .insert(chat_uri.clone(), resource.clone());
            restored.push((resource, chat_uri));
        }
        if restored.is_empty() {
            return;
        }
        tracing::info!(count = restored.len(), "sessions restored");
        self.root.active_sessions = Some(self.sessions.len() as i64);
        for (session_uri, chat_uri) in restored {
            let interrupted = self
                .chats
                .get(&chat_uri)
                .and_then(|chat| chat.active_turn.as_ref())
                .map(|turn| turn.id.clone());
            if let Some(turn_id) = interrupted {
                let outcome =
                    TurnOutcome::Failed("otto-agentsd stopped before the turn finished".into());
                self.end_turn(&session_uri, &chat_uri, turn_id, outcome);
            }
            self.start_next_queued(&session_uri);
            self.sync_summaries(&session_uri);
        }
    }

    /// The sessions changed since the last save, as records to store.
    fn take_unsaved(&mut self) -> Vec<SessionRecord> {
        let Some(unsaved) = self.unsaved.as_mut() else {
            return Vec::new();
        };
        let uris: Vec<String> = unsaved.drain().collect();
        uris.into_iter()
            .filter_map(|uri| {
                let session = self.sessions.get(&uri)?;
                let chat = self.chats.get(&session.chat)?;
                Some(SessionRecord::new(
                    uri,
                    session.created_at.clone(),
                    session.agent_session.clone(),
                    session.state.clone(),
                    chat.clone(),
                ))
            })
            .collect()
    }

    fn mark_unsaved(&mut self, session_uri: &str) {
        if let Some(unsaved) = self.unsaved.as_mut() {
            unsaved.insert(session_uri.to_owned());
        }
    }

    /// Starts the session's agent, taking up the agent's own session again
    /// when there is one, and forwards what the agent reports to the host.
    fn launch(&mut self, session_uri: &str) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        let cwd = session_cwd(&session.state);
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, mut event_rx) = mpsc::unbounded_channel();
        session.commands = Some(commands);
        let spec = SessionSpec {
            provider: session.state.provider.clone(),
            cwd,
            resume: session.agent_session.clone(),
        };
        self.backend.start(spec, command_rx, events);
        let host = self.host.clone();
        let session_uri = session_uri.to_owned();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let Some(host) = host.upgrade() else { break };
                host.on_session_event(&session_uri, event);
            }
        });
    }

    /// The agent is up. A new session becomes ready; one carried on after a
    /// restart already is, and only learns the agent's id.
    fn agent_ready(&mut self, session_uri: &str, agent_session: Option<String>) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        let ready = matches!(session.state.lifecycle, SessionLifecycle::Ready);
        if agent_session.is_some() && session.agent_session != agent_session {
            session.agent_session = agent_session;
            // Knowing the agent's id is what makes the session openable in a
            // terminal; clients read the command from the session's `_meta`.
            let meta = terminal_meta(
                self.backend.as_ref(),
                &session.state,
                session.agent_session.as_deref(),
            );
            if meta != session.state.meta {
                let changed = StateAction::SessionMetaChanged(SessionMetaChangedAction { meta });
                self.apply(session_uri, changed, None);
            }
            self.mark_unsaved(session_uri);
        }
        if !ready {
            let ready = StateAction::SessionReady(SessionReadyAction {});
            self.apply(session_uri, ready, None);
        }
    }

    /// The agent did not start. A new session fails to be created. A session
    /// already under way fails the turn the agent was started for, and tries
    /// the agent again with its next request.
    fn agent_failed(&mut self, session_uri: &str, chat_uri: &str, message: String) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        session.commands = None;
        if matches!(session.state.lifecycle, SessionLifecycle::Creating) {
            let failed = StateAction::SessionCreationFailed(SessionCreationFailedAction {
                error: error_info("agentStartFailed", message),
            });
            self.apply(session_uri, failed, None);
            return;
        }
        let turn_id = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .map(|turn| turn.id.clone());
        match turn_id {
            Some(turn_id) => {
                self.end_turn(session_uri, chat_uri, turn_id, TurnOutcome::Failed(message))
            }
            None => tracing::warn!(session_uri, %message, "the agent could not be started"),
        }
    }

    /// Whether the session's agent has nothing to do: no turn running, nothing
    /// queued, no question waiting and no allowed tool call still running.
    fn idle(&self, session_uri: &str) -> bool {
        let Some(session) = self.sessions.get(session_uri) else {
            return false;
        };
        let chat_busy = self.chats.get(&session.chat).is_some_and(|chat| {
            chat.active_turn.is_some()
                || chat
                    .queued_messages
                    .as_ref()
                    .is_some_and(|queue| !queue.is_empty())
        });
        !chat_busy
            && session.questions.is_empty()
            && session.inputs.is_empty()
            && session.tools.is_empty()
    }

    /// Records activity on the session, which restarts its idle countdown,
    /// and starts a new countdown when its running agent has nothing to do.
    fn touch(&mut self, session_uri: &str) {
        let Some(timeout) = self.idle_timeout else {
            return;
        };
        let idle = self.idle(session_uri);
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        session.activity += 1;
        if !idle || session.commands.is_none() {
            return;
        }
        let activity = session.activity;
        let host = self.host.clone();
        let session_uri = session_uri.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if let Some(host) = host.upgrade() {
                host.lock().stop_idle(&session_uri, activity);
            }
        });
    }

    /// Stops the session's agent when nothing happened since the countdown
    /// that ends here began. Closing its commands ends the agent's session,
    /// and the process with it; that exit is expected, and reports nothing.
    /// The session's state stays, and its next request starts the agent again.
    fn stop_idle(&mut self, session_uri: &str, activity: u64) {
        let idle = self.idle(session_uri);
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        if session.activity != activity || !idle || session.commands.is_none() {
            return;
        }
        tracing::info!(session_uri, "stopping the idle agent");
        session.commands = None;
    }

    /// Whether any client is subscribed to `chat_uri`, and so can answer the
    /// questions asked in it.
    fn watched(&self, chat_uri: &str) -> bool {
        self.peers
            .values()
            .any(|peer| peer.subscriptions.contains(chat_uri))
    }

    /// Opens `question` in the chat as a tool call waiting for confirmation,
    /// with the agent's options. Returns whether it was opened: a question
    /// from outside the active turn is denied at once.
    fn open_question(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        turn_id: String,
        question: Question,
        reply: oneshot::Sender<Decision>,
    ) -> bool {
        let in_turn = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .is_some_and(|turn| turn.id == turn_id);
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return false;
        };
        if !in_turn || session.questions.contains_key(&question.tool_call_id) {
            let _ = reply.send(Decision::deny());
            return false;
        }
        // Text the agent writes after the question goes below it.
        session.open_part = None;
        let Question {
            tool_call_id,
            tool_name,
            prompt,
            options,
        } = question;
        let display = if prompt.subtitle.is_empty() {
            prompt.title.clone()
        } else {
            prompt.subtitle.clone()
        };
        let confirmation_options = options
            .iter()
            .map(|option| ConfirmationOption {
                id: option.id.clone(),
                label: option.label.clone(),
                kind: if option.allow {
                    ConfirmationOptionKind::Approve
                } else {
                    ConfirmationOptionKind::Deny
                },
                group: None,
            })
            .collect();
        let title = prompt.title.clone();
        session.questions.insert(
            tool_call_id.clone(),
            Pending {
                turn_id: turn_id.clone(),
                reply,
                prompt,
                display: display.clone(),
                options,
                escalated: false,
            },
        );

        let start = StateAction::ChatToolCallStart(ChatToolCallStartAction {
            turn_id: turn_id.clone(),
            tool_call_id: tool_call_id.clone(),
            meta: None,
            tool_name,
            display_name: display.clone(),
            intention: None,
            contributor: None,
        });
        self.apply(chat_uri, start, None);
        let ready = StateAction::ChatToolCallReady(ChatToolCallReadyAction {
            turn_id,
            tool_call_id,
            meta: None,
            contributor: None,
            intention: None,
            invocation_message: StringOrMarkdown::Plain(display),
            tool_input: None,
            confirmation_title: Some(StringOrMarkdown::Plain(title)),
            risk_assessment: None,
            edits: None,
            editable: None,
            confirmed: None,
            options: Some(confirmation_options),
        });
        self.apply(chat_uri, ready, None);
        true
    }

    fn check_confirmation(
        &self,
        session_uri: &str,
        action: &ChatToolCallConfirmedAction,
    ) -> Result<(), &'static str> {
        let pending = self
            .sessions
            .get(session_uri)
            .and_then(|session| session.questions.get(&action.tool_call_id));
        match pending {
            Some(pending) if pending.turn_id == action.turn_id => Ok(()),
            _ => Err("the tool call is not pending confirmation"),
        }
    }

    /// Applies a confirmation and hands the decision to the agent. The first
    /// answer wins: a question already answered is left alone.
    fn confirm(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        action: ChatToolCallConfirmedAction,
        origin: Option<ActionOrigin>,
    ) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        let Some(pending) = session.questions.remove(&action.tool_call_id) else {
            return;
        };
        let option_id = action.selected_option_id.clone().filter(|id| {
            pending
                .options
                .iter()
                .any(|option| option.id == *id && option.allow == action.approved)
        });
        if action.approved {
            session
                .tools
                .insert(action.tool_call_id.clone(), pending.display.clone());
        }
        tracing::info!(
            tool_call = %action.tool_call_id,
            approved = action.approved,
            from_client = origin.is_some(),
            "permission question answered"
        );
        let decision = Decision {
            approved: action.approved,
            option_id,
        };
        self.apply(chat_uri, StateAction::ChatToolCallConfirmed(action), origin);
        let _ = pending.reply.send(decision);
    }

    /// Answers a question with what the user said in a dialog, unless it has
    /// been answered already.
    fn answer_from_dialog(&mut self, session_uri: &str, tool_call_id: &str, granted: bool) {
        let Some(session) = self.sessions.get(session_uri) else {
            return;
        };
        let Some(pending) = session.questions.get(tool_call_id) else {
            return;
        };
        let chat_uri = session.chat.clone();
        let action = ChatToolCallConfirmedAction {
            turn_id: pending.turn_id.clone(),
            tool_call_id: tool_call_id.to_owned(),
            meta: None,
            approved: granted,
            confirmed: granted.then_some(ToolCallConfirmationReason::UserAction),
            reason: (!granted).then_some(ToolCallCancellationReason::Denied),
            edited_tool_input: None,
            user_suggestion: None,
            reason_message: None,
            selected_option_id: dialog::narrowest(&pending.options, granted)
                .map(|option| option.id.clone()),
        };
        self.confirm(session_uri, &chat_uri, action, None);
        self.sync_summaries(session_uri);
        self.touch(session_uri);
    }

    /// Marks a permission question as no longer put to the user in a dialog,
    /// so it is again when nobody watches it.
    fn unescalate(&mut self, session_uri: &str, tool_call_id: &str) {
        if let Some(pending) = self
            .sessions
            .get_mut(session_uri)
            .and_then(|session| session.questions.get_mut(tool_call_id))
        {
            pending.escalated = false;
        }
    }

    /// Like [`HostState::unescalate`], for an input request.
    fn unescalate_input(&mut self, session_uri: &str, request_id: &str) {
        if let Some(pending) = self
            .sessions
            .get_mut(session_uri)
            .and_then(|session| session.inputs.get_mut(request_id))
        {
            pending.escalated = false;
        }
    }

    /// The unresolved input request `request_id` in the chat's active turn,
    /// with the answers given so far.
    fn input_request(&self, chat_uri: &str, request_id: &str) -> Option<&ChatInputRequest> {
        let turn = self.chats.get(chat_uri)?.active_turn.as_ref()?;
        turn.response_parts.iter().find_map(|part| match part {
            ResponsePart::InputRequest(input)
                if input.response.is_none() && input.request.id == request_id =>
            {
                Some(&input.request)
            }
            _ => None,
        })
    }

    /// Opens `request` in the chat's active turn and in the session's
    /// `inputNeeded`. Returns whether it was opened: a request from outside
    /// the active turn is declined at once.
    fn open_input(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        turn_id: String,
        request: ChatInputRequest,
        reply: oneshot::Sender<InputAnswer>,
    ) -> bool {
        let in_turn = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .is_some_and(|turn| turn.id == turn_id);
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return false;
        };
        if !in_turn || session.inputs.contains_key(&request.id) {
            let _ = reply.send(InputAnswer::decline());
            return false;
        }
        // Text the agent writes after the question goes below it.
        session.open_part = None;
        tracing::info!(
            session_uri,
            request_id = %request.id,
            questions = request.questions.as_ref().map_or(0, Vec::len),
            "the agent asks a question"
        );
        let request_id = request.id.clone();
        session.inputs.insert(
            request_id.clone(),
            PendingInput {
                turn_id,
                reply,
                escalated: false,
            },
        );
        let requested = StateAction::ChatInputRequested(ChatInputRequestedAction { request });
        self.apply(chat_uri, requested, None);
        self.mirror_input(session_uri, chat_uri, &request_id);
        true
    }

    /// Copies the input request, as the chat has it now, into the session's
    /// `inputNeeded`, so a client watching only the session sees it too.
    fn mirror_input(&mut self, session_uri: &str, chat_uri: &str, request_id: &str) {
        let Some(request) = self.input_request(chat_uri, request_id).cloned() else {
            return;
        };
        let set = StateAction::SessionInputNeededSet(Box::new(SessionInputNeededSetAction {
            request: SessionInputRequest::ChatInput(SessionChatInputRequest {
                id: request_id.to_owned(),
                chat: chat_uri.to_owned(),
                request,
            }),
        }));
        self.apply(session_uri, set, None);
    }

    fn check_answer(
        &self,
        session_uri: &str,
        chat_uri: &str,
        action: &ChatInputAnswerChangedAction,
    ) -> Result<(), &'static str> {
        let request = self.pending_input(session_uri, chat_uri, &action.request_id)?;
        let question = request
            .questions
            .iter()
            .flatten()
            .find(|question| elicitation::question_id(question) == Some(&action.question_id))
            .ok_or("the input request has no such question")?;
        match &action.answer {
            Some(answer) if !elicitation::answer_fits(question, answer) => {
                Err("the answer does not fit the question")
            }
            _ => Ok(()),
        }
    }

    fn check_completion(
        &self,
        session_uri: &str,
        chat_uri: &str,
        action: &ChatInputCompletedAction,
    ) -> Result<(), &'static str> {
        let request = self.pending_input(session_uri, chat_uri, &action.request_id)?;
        let answers = merged_answers(request, action.answers.as_ref());
        for question in request.questions.iter().flatten() {
            let Some(id) = elicitation::question_id(question) else {
                continue;
            };
            if let Some(answer) = answers.get(id)
                && !elicitation::answer_fits(question, answer)
            {
                return Err("an answer does not fit its question");
            }
            let submitted = matches!(answers.get(id), Some(ChatInputAnswer::Submitted(_)));
            if action.response == ChatInputResponseKind::Accept
                && elicitation::is_required(question)
                && !submitted
            {
                return Err("a required question has no submitted answer");
            }
        }
        Ok(())
    }

    /// The unresolved input request `request_id`, when the host is waiting on
    /// it in the chat's active turn.
    fn pending_input(
        &self,
        session_uri: &str,
        chat_uri: &str,
        request_id: &str,
    ) -> Result<&ChatInputRequest, &'static str> {
        let waiting = self
            .sessions
            .get(session_uri)
            .is_some_and(|session| session.inputs.contains_key(request_id));
        match self.input_request(chat_uri, request_id) {
            Some(request) if waiting => Ok(request),
            _ => Err("no input request with this id is waiting for an answer"),
        }
    }

    /// Applies a completion and hands the answer to the agent. The first
    /// answer wins: a request already answered is left alone.
    fn complete_input(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        action: ChatInputCompletedAction,
        origin: Option<ActionOrigin>,
    ) {
        let request_id = action.request_id.clone();
        let Some(pending) = self
            .sessions
            .get_mut(session_uri)
            .and_then(|session| session.inputs.remove(&request_id))
        else {
            return;
        };
        let answers = self
            .input_request(chat_uri, &request_id)
            .map(|request| merged_answers(request, action.answers.as_ref()))
            .unwrap_or_default();
        tracing::info!(
            request_id,
            response = ?action.response,
            from_client = origin.is_some(),
            "input request answered"
        );
        let answer = InputAnswer {
            response: action.response,
            answers,
        };
        self.apply(chat_uri, StateAction::ChatInputCompleted(action), origin);
        let removed = StateAction::SessionInputNeededRemoved(SessionInputNeededRemovedAction {
            id: request_id,
        });
        self.apply(session_uri, removed, None);
        let _ = pending.reply.send(answer);
    }

    /// Lets go of the questions `turn_id` left unanswered: permission requests
    /// are denied as their replies drop, and input requests cancelled.
    fn drop_turn_requests(&mut self, session_uri: &str, turn_id: &str) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        session
            .questions
            .retain(|_, pending| pending.turn_id != turn_id);
        let dropped: Vec<String> = session
            .inputs
            .iter()
            .filter(|(_, pending)| pending.turn_id == turn_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in dropped {
            if let Some(pending) = self
                .sessions
                .get_mut(session_uri)
                .and_then(|session| session.inputs.remove(&id))
            {
                let _ = pending.reply.send(InputAnswer {
                    response: ChatInputResponseKind::Cancel,
                    answers: HashMap::new(),
                });
            }
            let removed =
                StateAction::SessionInputNeededRemoved(SessionInputNeededRemovedAction { id });
            self.apply(session_uri, removed, None);
        }
    }

    /// Completes a tool call that was allowed, once the agent says it is done.
    fn finish_tool(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        turn_id: String,
        tool_call_id: &str,
        success: bool,
    ) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        let Some(display) = session.tools.remove(tool_call_id) else {
            return;
        };
        session.open_part = None;
        let complete = StateAction::ChatToolCallComplete(ChatToolCallCompleteAction {
            turn_id,
            tool_call_id: tool_call_id.to_owned(),
            meta: None,
            result: ToolCallResult {
                success,
                past_tense_message: StringOrMarkdown::Plain(display),
                content: None,
                structured_content: None,
                error: None,
            },
            requires_result_confirmation: None,
        });
        self.apply(chat_uri, complete, None);
    }

    fn send(&self, conn: ConnId, message: Outgoing) {
        if let Some(peer) = self.peers.get(&conn) {
            let _ = peer.outbox.send(message);
        }
    }

    fn broadcast(&self, channel: &str, method: &str, params: &impl Serialize) {
        let params = match serde_json::to_value(params) {
            Ok(params) => params,
            Err(err) => return tracing::error!(method, "could not serialize params: {err}"),
        };
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        for peer in self.peers.values() {
            if peer.subscriptions.contains(channel) {
                let _ = peer.outbox.send(Outgoing::Message(message.clone()));
            }
        }
    }

    /// Reduces `action` into its channel's state and broadcasts it.
    fn apply(&mut self, channel: &str, action: StateAction, origin: Option<ActionOrigin>) {
        let outcome = if channel == ROOT_RESOURCE_URI {
            apply_action_to_root(&mut self.root, &action)
        } else if let Some(session) = self.sessions.get_mut(channel) {
            apply_action_to_session(&mut session.state, &action)
        } else if let Some(chat) = self.chats.get_mut(channel) {
            apply_action_to_chat(chat, &action)
        } else {
            return tracing::warn!(channel, "dropping an action for an unknown channel");
        };
        if !matches!(outcome, ReduceOutcome::Applied | ReduceOutcome::NoOp) {
            return tracing::error!(
                channel,
                ?outcome,
                "the host produced an action its reducer refuses"
            );
        }
        self.server_seq += 1;
        if self.unsaved.is_some() {
            let session_uri = if self.sessions.contains_key(channel) {
                Some(channel.to_owned())
            } else {
                self.chat_sessions.get(channel).cloned()
            };
            if let Some(session_uri) = session_uri {
                self.mark_unsaved(&session_uri);
            }
        }
        let envelope = ActionEnvelope {
            channel: channel.to_owned(),
            action,
            server_seq: self.server_seq,
            origin,
            rejection_reason: None,
        };
        self.broadcast(channel, "action", &envelope);
    }

    /// Echoes a client action back to its sender, unapplied.
    fn reject(
        &self,
        conn: ConnId,
        channel: String,
        action: StateAction,
        origin: ActionOrigin,
        reason: &str,
    ) {
        let envelope = ActionEnvelope {
            channel,
            action,
            server_seq: self.server_seq,
            origin: Some(origin),
            rejection_reason: Some(reason.to_owned()),
        };
        let message = json!({ "jsonrpc": "2.0", "method": "action", "params": envelope });
        self.send(conn, Outgoing::Message(message));
    }

    /// Hands `command` to the session's agent. With no agent running — a
    /// session restored from the store, or one whose agent has exited — the
    /// agent is started first, so the session carries on.
    fn send_command(&mut self, session_uri: &str, command: SessionCommand) {
        let Some(session) = self.sessions.get(session_uri) else {
            return;
        };
        let command = match &session.commands {
            Some(commands) => match commands.send(command) {
                Ok(()) => return,
                Err(unsent) => unsent.0,
            },
            None => command,
        };
        // With no agent running, there is nothing to cancel.
        if matches!(command, SessionCommand::Cancel { .. }) {
            return;
        }
        tracing::info!(session_uri, "starting the agent to carry the session on");
        self.launch(session_uri);
        if let Some(commands) = self
            .sessions
            .get(session_uri)
            .and_then(|session| session.commands.as_ref())
        {
            let _ = commands.send(command);
        }
    }

    /// Applies `chat/turnStarted` and hands the prompt to the agent.
    fn start_turn(
        &mut self,
        session_uri: &str,
        action: ChatTurnStartedAction,
        origin: Option<ActionOrigin>,
    ) {
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        session.open_part = None;
        let chat_uri = session.chat.clone();
        self.title_from_prompt(session_uri, &action.message.text);
        let prompt = SessionCommand::Prompt {
            turn_id: action.turn_id.clone(),
            text: action.message.text.clone(),
            attachments: attachments(&action.message),
        };
        self.apply(&chat_uri, StateAction::ChatTurnStarted(action), origin);
        self.send_command(session_uri, prompt);
    }

    /// Only queued messages are supported; steering needs an agent that can
    /// take input mid-turn.
    fn check_queued(action: &ChatPendingMessageSetAction) -> Result<(), &'static str> {
        match action.kind {
            PendingMessageKind::Steering => Err("this host does not support steering messages yet"),
            PendingMessageKind::Queued if action.message.text.trim().is_empty() => {
                Err("a queued message needs text")
            }
            PendingMessageKind::Queued => Ok(()),
        }
    }

    /// Starts the first queued message once the chat can take a turn, in the
    /// two steps the chat spec prescribes: `chat/pendingMessageRemoved`, then
    /// `chat/turnStarted` carrying the message's id.
    ///
    /// This is what lets a client hand over a request and leave before the
    /// agent is even up.
    fn start_next_queued(&mut self, session_uri: &str) {
        let Some(chat_uri) = self.sessions.get(session_uri).map(|s| s.chat.clone()) else {
            return;
        };
        if self.check_turn_start(session_uri, &chat_uri).is_err() {
            return;
        }
        let Some(next) = self
            .chats
            .get(&chat_uri)
            .and_then(|chat| chat.queued_messages.as_ref())
            .and_then(|queue| queue.first())
            .cloned()
        else {
            return;
        };
        let removed = StateAction::ChatPendingMessageRemoved(ChatPendingMessageRemovedAction {
            kind: PendingMessageKind::Queued,
            id: next.id.clone(),
        });
        self.apply(&chat_uri, removed, None);
        let started = ChatTurnStartedAction {
            turn_id: uuid::Uuid::new_v4().to_string(),
            started_at: now(),
            message: next.message,
            queued_message_id: Some(next.id),
            meta: None,
        };
        self.start_turn(session_uri, started, None);
    }

    fn initialize(
        &mut self,
        conn: &mut Connection,
        params: InitializeParams,
    ) -> Result<Value, RpcError> {
        if conn.client_id.is_some() {
            return Err(RpcError::invalid_request(
                "connection is already initialized",
            ));
        }
        let Some(protocol_version) = params
            .protocol_versions
            .iter()
            .find(|offered| SUPPORTED_PROTOCOL_VERSIONS.contains(&offered.as_str()))
            .cloned()
        else {
            // The versioning spec requires closing the connection after this error.
            conn.close_requested = true;
            return Err(RpcError::new(
                ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION,
                "no mutually supported protocol version",
            )
            .with_data(json!({ "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS })));
        };
        conn.client_id = Some(params.client_id);

        let mut snapshots = Vec::new();
        for channel in params.initial_subscriptions.unwrap_or_default() {
            match self.snapshot(&channel) {
                Some(snapshot) => {
                    snapshots.push(snapshot);
                    self.add_subscription(conn.id, channel);
                }
                None => tracing::warn!(%channel, "skipping unknown initial subscription"),
            }
        }

        to_result(&InitializeResult {
            protocol_version,
            server_seq: self.server_seq as i64,
            server_info: Some(Implementation {
                name: env!("CARGO_PKG_NAME").into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                title: None,
            }),
            meta: None,
            snapshots,
            default_directory: std::env::var_os("HOME")
                .map(|home| uri::from_path(&PathBuf::from(home))),
            completion_trigger_characters: None,
            terminal_command_prefix: None,
            telemetry: None,
            automations: None,
        })
    }

    fn subscribe(&mut self, conn: &Connection, params: SubscribeParams) -> Result<Value, RpcError> {
        let snapshot = self.snapshot(&params.channel).ok_or_else(|| {
            RpcError::new(
                ahp_error_codes::NOT_FOUND,
                format!("unknown channel: {}", params.channel),
            )
        })?;
        self.add_subscription(conn.id, params.channel);
        to_result(&SubscribeResult {
            snapshot: Some(snapshot),
        })
    }

    fn add_subscription(&mut self, conn: ConnId, channel: String) {
        if let Some(peer) = self.peers.get_mut(&conn) {
            peer.subscriptions.insert(channel);
        }
    }

    fn snapshot(&self, channel: &str) -> Option<Snapshot> {
        let state = if channel == ROOT_RESOURCE_URI {
            SnapshotState::Root(Box::new(self.root.clone()))
        } else if let Some(session) = self.sessions.get(channel) {
            SnapshotState::Session(Box::new(session.state.clone()))
        } else {
            SnapshotState::Chat(Box::new(self.chats.get(channel)?.clone()))
        };
        Some(Snapshot {
            resource: channel.to_owned(),
            state,
            from_seq: self.server_seq as i64,
        })
    }

    fn list_sessions(&self) -> ListSessionsResult {
        let mut items: Vec<SessionSummary> = self
            .sessions
            .iter()
            .map(|(uri, session)| self.summary(uri, session))
            .collect();
        items.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        ListSessionsResult {
            next_cursor: None,
            items,
        }
    }

    /// The session's catalogue entry. Status and modification time come from
    /// its chat, since a single-chat session's aggregates are its chat's values.
    /// `_meta` carries the session's own, so a client listing the sessions can
    /// open one in a terminal without subscribing to it first.
    fn summary(&self, uri: &str, session: &Session) -> SessionSummary {
        let chat = self.chats.get(&session.chat);
        SessionSummary {
            provider: session.state.provider.clone(),
            title: session.state.title.clone(),
            status: chat.map_or(session.state.status, |chat| chat.status),
            activity: None,
            origin: None,
            project: None,
            working_directories: session.state.working_directories.clone(),
            annotations: None,
            resource: uri.to_owned(),
            created_at: session.created_at.clone(),
            modified_at: chat.map_or_else(
                || session.created_at.clone(),
                |chat| chat.modified_at.clone(),
            ),
            changes: None,
            meta: session.state.meta.clone(),
        }
    }

    fn check_turn_start(&self, session_uri: &str, chat_uri: &str) -> Result<(), &'static str> {
        let ready = self
            .sessions
            .get(session_uri)
            .is_some_and(|s| matches!(s.state.lifecycle, SessionLifecycle::Ready));
        if !ready {
            return Err("the session is not ready");
        }
        if self
            .chats
            .get(chat_uri)
            .is_some_and(|c| c.active_turn.is_some())
        {
            return Err("a turn is already in progress");
        }
        Ok(())
    }

    fn check_turn_cancel(&self, chat_uri: &str, turn_id: &str) -> Result<(), &'static str> {
        let active = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .is_some_and(|turn| turn.id == turn_id);
        if active {
            Ok(())
        } else {
            Err("no active turn with this id")
        }
    }

    /// Titles an untitled session after the first line of its first prompt.
    fn title_from_prompt(&mut self, session_uri: &str, prompt: &str) {
        if !self
            .sessions
            .get(session_uri)
            .is_some_and(|s| s.state.title.is_empty())
        {
            return;
        }
        let title: String = prompt
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .chars()
            .take(TITLE_MAX_CHARS)
            .collect();
        if !title.is_empty() {
            let action = StateAction::SessionTitleChanged(SessionTitleChangedAction { title });
            self.apply(session_uri, action, None);
        }
    }

    fn stream(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        turn_id: String,
        kind: PartKind,
        text: String,
    ) {
        let in_turn = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .is_some_and(|turn| turn.id == turn_id);
        let Some(session) = self.sessions.get_mut(session_uri) else {
            return;
        };
        if !in_turn || text.is_empty() {
            return;
        }
        let open = session
            .open_part
            .as_ref()
            .filter(|part| part.kind == kind)
            .map(|part| part.id.clone());
        let action = match (open, kind) {
            (Some(part_id), PartKind::Markdown) => StateAction::ChatDelta(ChatDeltaAction {
                turn_id,
                part_id,
                content: text,
                meta: None,
            }),
            (Some(part_id), PartKind::Reasoning) => {
                StateAction::ChatReasoning(ChatReasoningAction {
                    turn_id,
                    part_id,
                    content: text,
                    meta: None,
                })
            }
            (None, kind) => {
                let id = uuid::Uuid::new_v4().to_string();
                session.open_part = Some(OpenPart {
                    kind,
                    id: id.clone(),
                });
                let part = match kind {
                    PartKind::Markdown => {
                        ResponsePart::Markdown(MarkdownResponsePart { id, content: text })
                    }
                    PartKind::Reasoning => {
                        ResponsePart::Reasoning(ReasoningResponsePart { id, content: text })
                    }
                };
                StateAction::ChatResponsePart(ChatResponsePartAction {
                    turn_id,
                    part,
                    meta: None,
                })
            }
        };
        self.apply(chat_uri, action, None);
    }

    fn end_turn(
        &mut self,
        session_uri: &str,
        chat_uri: &str,
        turn_id: String,
        outcome: TurnOutcome,
    ) {
        let Some(started_at) = self
            .chats
            .get(chat_uri)
            .and_then(|chat| chat.active_turn.as_ref())
            .filter(|turn| turn.id == turn_id)
            .map(|turn| turn.started_at.clone())
        else {
            return;
        };
        if let Some(session) = self.sessions.get_mut(session_uri) {
            session.open_part = None;
            // Its tool calls end with it.
            session.tools.clear();
        }
        // So do the questions it left unanswered.
        self.drop_turn_requests(session_uri, &turn_id);
        let duration = millis_since(&started_at);
        let action = match outcome {
            TurnOutcome::Complete => StateAction::ChatTurnComplete(ChatTurnCompleteAction {
                turn_id,
                duration,
                meta: None,
            }),
            TurnOutcome::Cancelled => StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
                turn_id,
                duration,
                meta: None,
            }),
            TurnOutcome::Failed(message) => StateAction::ChatError(ChatErrorAction {
                turn_id,
                duration,
                part: ErrorResponsePart {
                    error: error_info("agentError", message),
                    resumable: None,
                },
                meta: None,
            }),
        };
        self.apply(chat_uri, action, None);
    }

    /// Keeps the session's chat catalogue and the root catalogue in step with
    /// the chat's own state.
    fn sync_summaries(&mut self, session_uri: &str) {
        let Some(session) = self.sessions.get(session_uri) else {
            return;
        };
        let Some(chat) = self.chats.get(&session.chat) else {
            return;
        };
        let chat_uri = session.chat.clone();
        let (status, modified_at) = (chat.status, chat.modified_at.clone());
        let title = session.state.title.clone();
        let catalogue_stale = session.state.chats.iter().any(|c| {
            c.resource == chat_uri && (c.status != status || c.modified_at != modified_at)
        });
        let announce = session.announced != (status, title.clone());

        if catalogue_stale {
            let updated = StateAction::SessionChatUpdated(SessionChatUpdatedAction {
                chat: chat_uri,
                changes: PartialChatSummary {
                    status: Some(status),
                    modified_at: Some(modified_at.clone()),
                    ..Default::default()
                },
            });
            self.apply(session_uri, updated, None);
        }
        if announce {
            if let Some(session) = self.sessions.get_mut(session_uri) {
                session.announced = (status, title.clone());
            }
            let changed = json!({
                "channel": ROOT_RESOURCE_URI,
                "session": session_uri,
                "changes": { "title": title, "status": status, "modifiedAt": modified_at },
            });
            self.broadcast(ROOT_RESOURCE_URI, "root/sessionSummaryChanged", &changed);
        }
    }
}

fn working_directory(dirs: Option<Vec<String>>) -> Result<PathBuf, RpcError> {
    let dirs = dirs.unwrap_or_default();
    let path = match dirs.as_slice() {
        [] => std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            RpcError::invalid_params("no working directory given and HOME is unset")
        })?,
        [dir] => uri::to_path(dir)
            .ok_or_else(|| RpcError::invalid_params(format!("not an absolute file URI: {dir}")))?,
        _ => {
            return Err(RpcError::invalid_params(
                "this host supports one working directory per session",
            ));
        }
    };
    if !path.is_dir() {
        return Err(RpcError::invalid_params(format!(
            "not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

/// The answers synced on `request`, overlaid with the ones a completion
/// brings.
fn merged_answers(
    request: &ChatInputRequest,
    completed: Option<&HashMap<String, ChatInputAnswer>>,
) -> HashMap<String, ChatInputAnswer> {
    let mut answers = request.answers.clone().unwrap_or_default();
    answers.extend(
        completed
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    answers
}

/// The dialog that asks `request`, made by the agent called `agent` in `cwd`.
/// When every question is a single select, the dialog asks them itself;
/// otherwise it can only say what is asked, and offer to open Ask.
///
/// Every question is shown in full: its own words, and each option's label
/// with its description after a line break, which otto-islands draws under
/// the label.
pub fn question_prompt(agent: &str, cwd: &std::path::Path, request: &ChatInputRequest) -> Prompt {
    let questions: Vec<&ChatInputQuestion> = request.questions.iter().flatten().collect();
    let message = request.message.as_deref().filter(|m| !m.trim().is_empty());
    // What each question asks. Claude's AskUserQuestion puts a lone question's
    // words in the request's message and leaves the field only its short
    // header; the message is the question then.
    let single = questions.len() == 1;
    let asks = |question: &ChatInputQuestion| -> Option<String> {
        let (title, text) = question_text(question)?;
        match message {
            Some(message) if single && title == Some(text) => Some(message.to_owned()),
            _ => Some(text.to_owned()),
        }
    };
    let texts: Vec<Option<String>> = questions.iter().map(|q| asks(q)).collect();
    // The message stands on its own unless a question already says it.
    let subtitle = message
        .filter(|message| !texts.iter().flatten().any(|text| text == message))
        .unwrap_or_default()
        .to_owned();

    let choices: Option<Vec<Choice>> = questions
        .iter()
        .zip(&texts)
        .map(|(question, text)| match question {
            ChatInputQuestion::SingleSelect(select) if !select.options.is_empty() => Some(Choice {
                id: select.id.clone(),
                label: text.clone().unwrap_or_default(),
                options: select
                    .options
                    .iter()
                    .map(|option| (option.id.clone(), option_label(option)))
                    .collect(),
                default: select
                    .options
                    .iter()
                    .find(|option| option.recommended == Some(true))
                    .unwrap_or(&select.options[0])
                    .id
                    .clone(),
            }),
            _ => None,
        })
        .collect();
    let title = if questions.len() > 1 {
        format!("{agent} has some questions")
    } else {
        format!("{agent} has a question")
    };
    match choices.filter(|choices| !choices.is_empty()) {
        Some(choices) => Prompt {
            title,
            subtitle,
            body: format!("in {}", dialog::folder(cwd)),
            grant: "Answer".into(),
            deny: "Skip".into(),
            open: dialog::OPEN_IN_ASK.into(),
            icon: "dialog-question".into(),
            choices,
        },
        None => Prompt {
            title,
            body: questions
                .iter()
                .zip(&texts)
                .filter_map(|(question, text)| {
                    let mut block = text.clone()?;
                    for option in select_options(question) {
                        let label = match option.description.as_deref() {
                            Some(description) if !description.is_empty() => {
                                format!("{} \u{2014} {description}", option.label)
                            }
                            _ => option.label.clone(),
                        };
                        block.push_str("\n\u{2022} ");
                        block.push_str(&label);
                    }
                    Some(block)
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            subtitle,
            grant: String::new(),
            deny: "Skip".into(),
            open: dialog::OPEN_IN_ASK.into(),
            icon: "dialog-question".into(),
            choices: Vec::new(),
        },
    }
}

/// An option as the dialog labels it: its label, and its description after
/// a line break when it has one.
fn option_label(option: &ChatInputOption) -> String {
    match option.description.as_deref().map(str::trim) {
        Some(description) if !description.is_empty() => {
            format!("{}\n{description}", option.label)
        }
        _ => option.label.clone(),
    }
}

/// A question's title, if any, and the words it asks.
fn question_text(question: &ChatInputQuestion) -> Option<(Option<&str>, &str)> {
    let (title, message) = match question {
        ChatInputQuestion::Text(q) => (&q.title, &q.message),
        ChatInputQuestion::Number(q) | ChatInputQuestion::Integer(q) => (&q.title, &q.message),
        ChatInputQuestion::Boolean(q) => (&q.title, &q.message),
        ChatInputQuestion::SingleSelect(q) => (&q.title, &q.message),
        ChatInputQuestion::MultiSelect(q) => (&q.title, &q.message),
        ChatInputQuestion::Unknown(_) => return None,
    };
    Some((title.as_deref(), message))
}

/// The options of a select question; none for any other kind.
fn select_options(question: &ChatInputQuestion) -> &[ChatInputOption] {
    match question {
        ChatInputQuestion::SingleSelect(q) => &q.options,
        ChatInputQuestion::MultiSelect(q) => &q.options,
        _ => &[],
    }
}

/// A dialog's choices as submitted answers, by question id.
fn selected(selections: Vec<(String, String)>) -> HashMap<String, ChatInputAnswer> {
    selections
        .into_iter()
        .map(|(question, option)| {
            let value = ChatInputAnswerValue::Selected(ChatInputSelectedAnswerValue {
                value: option,
                freeform_values: None,
            });
            (
                question,
                ChatInputAnswer::Submitted(ChatInputAnswered { value }),
            )
        })
        .collect()
}

/// The session's folder as a path: its first working directory, or home.
fn session_cwd(state: &SessionState) -> PathBuf {
    state
        .working_directories
        .iter()
        .flatten()
        .next()
        .and_then(|dir| uri::to_path(dir))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The session's `_meta`: `otto.terminal`, the command that opens the agent's
/// own session in a terminal and the folder to run it in, once the agent's id
/// for it is known and the backend has a way to open it. The service owns the
/// whole of `_meta`, so `None` otherwise.
fn terminal_meta(
    backend: &dyn Backend,
    state: &SessionState,
    agent_session: Option<&str>,
) -> Option<JsonObject> {
    let cwd = session_cwd(state);
    let command = backend.terminal(&state.provider, agent_session?, &cwd)?;
    let meta = json!({
        "otto": {
            "terminal": {
                "command": command,
                "cwd": cwd.to_string_lossy(),
            }
        }
    });
    match meta {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

fn new_session_state(provider: &str, folder_uri: &str) -> SessionState {
    SessionState {
        provider: provider.to_owned(),
        title: String::new(),
        status: SessionStatus::Idle.bits(),
        activity: None,
        origin: None,
        project: None,
        working_directories: Some(vec![folder_uri.to_owned()]),
        annotations: None,
        lifecycle: SessionLifecycle::Creating,
        creation_error: None,
        server_tools: None,
        active_clients: Vec::new(),
        chats: Vec::new(),
        default_chat: None,
        config: None,
        customizations: None,
        changesets: None,
        input_needed: None,
        meta: None,
    }
}

fn new_chat_state(uri: &str, now: &str) -> ChatState {
    ChatState {
        resource: uri.to_owned(),
        title: String::new(),
        status: SessionStatus::Idle.bits(),
        activity: None,
        modified_at: now.to_owned(),
        origin: Some(ChatOrigin::User),
        interactivity: None,
        working_directories: None,
        turns: Vec::new(),
        turns_next_cursor: None,
        active_turn: None,
        steering_message: None,
        queued_messages: None,
        draft: None,
        meta: None,
    }
}

fn error_info(error_type: &str, message: String) -> ErrorInfo {
    ErrorInfo {
        error_type: error_type.to_owned(),
        message,
        stack: None,
        meta: None,
    }
}

/// What a message points the agent at. Only resources referenced by URI are
/// passed on; other kinds of attachment are not supported yet, and the text
/// still goes through without them.
fn attachments(message: &Message) -> Vec<Attachment> {
    message
        .attachments
        .iter()
        .flatten()
        .filter_map(|attachment| match attachment {
            MessageAttachment::Resource(resource) => Some(Attachment {
                name: resource.label.clone(),
                uri: resource.uri.clone(),
            }),
            other => {
                tracing::warn!(?other, "dropping an attachment this host cannot pass on");
                None
            }
        })
        .collect()
}

/// The current time as an RFC 3339 timestamp with millisecond precision.
pub fn now() -> String {
    format!("{:.3}", jiff::Timestamp::now())
}

fn millis_since(timestamp: &str) -> i64 {
    timestamp
        .parse::<jiff::Timestamp>()
        .map(|start| (jiff::Timestamp::now().as_millisecond() - start.as_millisecond()).max(0))
        .unwrap_or(0)
}

fn parse_params<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(RpcError::invalid_params)
}

fn to_result<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(RpcError::internal)
}

#[cfg(test)]
mod tests {
    use ahp_types::errors::json_rpc_error_codes;

    use super::*;
    use crate::agent::EchoBackend;

    fn connect() -> (Arc<Host>, Connection, mpsc::UnboundedReceiver<Outgoing>) {
        // The echo agent never asks for permission, so the dialog is never
        // shown.
        let host = Host::new(Arc::new(EchoBackend), Arc::new(dialog::Islands::default()));
        let (outbox, inbox) = mpsc::unbounded_channel();
        let conn = host.connect(outbox);
        (host, conn, inbox)
    }

    fn error_code(inbox: &mut mpsc::UnboundedReceiver<Outgoing>) -> Option<i64> {
        match inbox.try_recv() {
            Ok(Outgoing::Message(message)) => message["error"]["code"].as_i64(),
            _ => None,
        }
    }

    #[test]
    fn requests_before_initialize_are_rejected() {
        let (host, mut conn, mut inbox) = connect();
        host.handle_request(
            &mut conn,
            json!(1),
            "listSessions",
            json!({ "channel": ROOT_RESOURCE_URI }),
        );
        assert_eq!(
            error_code(&mut inbox),
            Some(json_rpc_error_codes::INVALID_REQUEST.into())
        );
    }

    #[test]
    fn unsupported_version_closes_the_connection() {
        let (host, mut conn, mut inbox) = connect();
        let params = json!({ "channel": ROOT_RESOURCE_URI, "clientId": "c1", "protocolVersions": ["0.0.1"] });
        host.handle_request(&mut conn, json!(1), "initialize", params);
        assert_eq!(
            error_code(&mut inbox),
            Some(ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION.into())
        );
        assert!(conn.close_requested());
    }

    #[test]
    fn timestamps_have_millisecond_precision() {
        let timestamp = now();
        assert!(timestamp.parse::<jiff::Timestamp>().is_ok(), "{timestamp}");
        assert_eq!(
            timestamp.split('.').nth(1).map(str::len),
            Some(4),
            "{timestamp}"
        );
    }

    /// What claude-agent-acp sends for AskUserQuestion: a lone question's
    /// words in the message and only its header on the field; several
    /// questions' words on their fields, under a generic message. Options
    /// carry descriptions, and each question has its "Other" field.
    fn ask_user_question(questions: &[(&str, &str, bool)]) -> ChatInputRequest {
        let single = questions.len() == 1;
        let mut properties = serde_json::Map::new();
        for (index, (header, question, multi)) in questions.iter().enumerate() {
            let options = json!([
                { "const": "Keep it", "title": "Keep it",
                  "description": "Clients keep working; the table goes in the next release" },
                { "const": "Drop it", "title": "Drop it",
                  "description": "Older clients have to sign in again" },
                { "const": "Ask later", "title": "Ask later" },
            ]);
            let mut field = if *multi {
                json!({ "type": "array", "title": header, "items": { "anyOf": options } })
            } else {
                json!({ "type": "string", "title": header, "oneOf": options })
            };
            if !single {
                field["description"] = json!(question);
            }
            properties.insert(format!("question_{index}"), field);
            properties.insert(
                format!("question_{index}_custom"),
                json!({
                    "type": "string",
                    "title": "Other",
                    "description": "Type your own answer (optional).",
                    "_meta": { "_askUserQuestionCustomAnswer": {
                        "questionId": format!("question_{index}"), "isCustomAnswer": true } },
                }),
            );
        }
        let message = if single {
            questions[0].1
        } else {
            "Please answer the following questions."
        };
        let schema = json!({ "type": "object", "properties": properties });
        crate::elicitation::form("r1", message, &schema).request
    }

    const LONG: &str = "Should the migration keep the legacy session table until every \
        client has upgraded, or drop it in this release and accept that clients older \
        than three months will have to sign in again?";

    #[test]
    fn a_lone_question_is_asked_in_its_own_words() {
        let request = ask_user_question(&[("Migration", LONG, false)]);
        let prompt = question_prompt("Claude", std::path::Path::new("/srv/app"), &request);
        assert_eq!(prompt.title, "Claude has a question");
        // The question is the group's label, not a header, and not repeated.
        assert_eq!(prompt.subtitle, "");
        assert_eq!(prompt.body, "in /srv/app");
        let [choice] = &prompt.choices[..] else {
            panic!("one choice group: {:?}", prompt.choices);
        };
        assert_eq!(choice.label, LONG);
        assert_eq!(
            choice.options,
            [
                (
                    "Keep it".to_owned(),
                    "Keep it\nClients keep working; the table goes in the next release".to_owned()
                ),
                (
                    "Drop it".to_owned(),
                    "Drop it\nOlder clients have to sign in again".to_owned()
                ),
                ("Ask later".to_owned(), "Ask later".to_owned()),
            ]
        );
        assert_eq!(prompt.grant, "Answer");
    }

    #[test]
    fn every_question_is_asked_when_there_are_several() {
        let request = ask_user_question(&[
            ("Migration", LONG, false),
            ("Rollout", "Which environments should get it first?", false),
        ]);
        let prompt = question_prompt("Claude", std::path::Path::new("/srv/app"), &request);
        assert_eq!(prompt.title, "Claude has some questions");
        assert_eq!(prompt.subtitle, "Please answer the following questions.");
        let labels: Vec<&str> = prompt.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, [LONG, "Which environments should get it first?"]);
        assert!(prompt.choices.iter().all(|c| c.options.len() == 3));
    }

    #[test]
    fn questions_the_dialog_cannot_ask_are_spelled_out_with_their_options() {
        let request = ask_user_question(&[
            ("Migration", LONG, false),
            ("Rollout", "Which environments?", true),
        ]);
        let prompt = question_prompt("Claude", std::path::Path::new("/srv/app"), &request);
        assert!(prompt.choices.is_empty());
        assert_eq!(prompt.grant, "");
        assert_eq!(prompt.open, "Open in Ask");
        let options = "\n\u{2022} Keep it \u{2014} Clients keep working; the table goes in the next release\
            \n\u{2022} Drop it \u{2014} Older clients have to sign in again\
            \n\u{2022} Ask later";
        assert_eq!(
            prompt.body,
            format!("{LONG}{options}\n\nWhich environments?{options}")
        );
    }

    #[test]
    fn a_lone_multi_select_question_keeps_the_message_as_its_words() {
        let request = ask_user_question(&[("Rollout", "Which environments?", true)]);
        let prompt = question_prompt("Claude", std::path::Path::new("/"), &request);
        assert_eq!(prompt.subtitle, "");
        assert!(
            prompt
                .body
                .starts_with("Which environments?\n\u{2022} Keep it")
        );
    }
}
