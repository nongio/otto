//! Questions: an agent's permission request opens in the chat as a tool call
//! waiting for confirmation, and its questions as an input request. A client
//! watching the chat answers there; when nobody is watching, or the watcher
//! leaves, the host asks through its prompter instead.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::PermissionOptionKind;
use ahp::reducers::{apply_action_to_chat, apply_action_to_session};
use ahp::{Client, SessionSubscription, SubscriptionEvent};
use ahp_types::actions::{
    ActionEnvelope, ChatInputCompletedAction, ChatToolCallConfirmedAction, ChatTurnCancelledAction,
    ChatTurnStartedAction, StateAction,
};
use ahp_types::state::{
    ChatInputAnswer, ChatInputAnswerValue, ChatInputAnswered, ChatInputOption, ChatInputQuestion,
    ChatInputRequest, ChatInputResponseKind, ChatInputSelectedAnswerValue,
    ChatInputSingleSelectQuestion, ChatInputTextAnswerValue, ChatInputTextQuestion, ChatState,
    Message, MessageKind, MessageOrigin, ResponsePart, SessionInputRequest, SessionLifecycle,
    SessionState, SessionStatus, SnapshotState, ToolCallPendingConfirmationState, ToolCallState,
    ToolInput,
};
use otto_agents::agent::{
    Backend, Decision, InputAnswer, Question, QuestionOption, SessionCommand, SessionEvent,
    SessionSpec, TurnOutcome, agent_info,
};
use otto_agents::dialog::{Prompt, Prompter, Reply};
use otto_agents::{host, uri};

mod common;
use serde_json::json;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::timeout;

/// Two agents. `asker` asks to run `cargo test` before every answer, and
/// answers with the option it was given. `questioner` asks a question —
/// which database, for a prompt of `pick`, or a name, for anything else —
/// and answers with what it was told, which it also keeps in `answers`.
struct AskingBackend {
    answers: Arc<Mutex<Vec<String>>>,
}

impl Backend for AskingBackend {
    fn agents(&self) -> Vec<ahp_types::state::AgentInfo> {
        vec![
            agent_info("asker", "Asker", "Asks before it answers"),
            agent_info("questioner", "Questioner", "Asks what it should do"),
        ]
    }

    fn start(
        &self,
        spec: SessionSpec,
        mut commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) {
        let answers = Arc::clone(&self.answers);
        tokio::spawn(async move {
            let _ = events.send(SessionEvent::Ready {
                agent_session: None,
            });
            while let Some(command) = commands.recv().await {
                let SessionCommand::Prompt { turn_id, text, .. } = command else {
                    continue;
                };
                let text = if spec.provider == "questioner" {
                    let (reply, answer) = oneshot::channel();
                    let _ = events.send(SessionEvent::InputRequested {
                        turn_id: turn_id.clone(),
                        request: input_request(&format!("req-{turn_id}"), &text),
                        reply,
                    });
                    let said = summary(answer.await.ok());
                    answers.lock().unwrap().push(said.clone());
                    said
                } else {
                    let said = permission(&events, &turn_id).await;
                    answers.lock().unwrap().push(said.clone());
                    said
                };
                let _ = events.send(SessionEvent::MessageChunk {
                    turn_id: turn_id.clone(),
                    text,
                });
                let _ = events.send(SessionEvent::TurnEnded {
                    turn_id,
                    outcome: TurnOutcome::Complete,
                });
            }
        });
    }
}

async fn permission(events: &mpsc::UnboundedSender<SessionEvent>, turn_id: &str) -> String {
    let tool_call_id = format!("call-{turn_id}");
    let (reply, decision) = oneshot::channel();
    let _ = events.send(SessionEvent::PermissionRequested {
        turn_id: turn_id.to_owned(),
        question: Box::new(question(&tool_call_id)),
        reply,
    });
    let decision = decision.await.unwrap_or_else(|_| Decision::deny());
    if decision.approved() {
        let _ = events.send(SessionEvent::ToolCallFinished {
            turn_id: turn_id.to_owned(),
            tool_call_id,
            success: true,
        });
    }
    match decision {
        Decision::Cancelled => "cancelled".into(),
        decision => decision.option_id().unwrap_or("none").to_owned(),
    }
}

fn question(tool_call_id: &str) -> Question {
    let option = |id: &str, label: &str, kind| QuestionOption {
        id: id.into(),
        label: label.into(),
        kind,
    };
    Question {
        tool_call_id: tool_call_id.into(),
        tool_name: "execute".into(),
        prompt: Prompt {
            title: "Asker wants to run a command".into(),
            subtitle: "cargo test".into(),
            body: "in /tmp".into(),
            grant: "Allow".into(),
            deny: "Reject".into(),
            ..Prompt::default()
        },
        options: vec![
            option(
                "allow_always",
                "Always Allow",
                PermissionOptionKind::AllowAlways,
            ),
            option("allow", "Allow", PermissionOptionKind::AllowOnce),
            option("reject", "Reject", PermissionOptionKind::RejectOnce),
        ],
        default_option_id: Some("allow".into()),
        tool_input: Some(json!({ "rawInput": { "command": "cargo test" } })),
        edits: vec![json!({ "path": "/tmp/a.rs", "oldText": "a", "newText": "b" })],
    }
}

fn input_request(id: &str, prompt: &str) -> ChatInputRequest {
    let question = if prompt == "pick" {
        let option = |id: &str| ChatInputOption {
            id: id.into(),
            label: id.into(),
            description: None,
            recommended: None,
        };
        ChatInputQuestion::SingleSelect(ChatInputSingleSelectQuestion {
            id: "question_0".into(),
            title: None,
            message: "Which database?".into(),
            required: Some(true),
            options: vec![option("Postgres"), option("SQLite")],
            allow_freeform_input: Some(true),
        })
    } else {
        ChatInputQuestion::Text(ChatInputTextQuestion {
            id: "name".into(),
            title: None,
            message: "What is it called?".into(),
            required: None,
            format: None,
            min: None,
            max: None,
            default_value: None,
        })
    };
    ChatInputRequest {
        id: id.into(),
        message: Some("Questioner needs to know".into()),
        url: None,
        questions: Some(vec![question]),
        answers: None,
    }
}

/// An answer as `response key=value…`, keys in order.
fn summary(answer: Option<InputAnswer>) -> String {
    let Some(answer) = answer else {
        return "dropped".into();
    };
    let mut pairs: Vec<String> = answer
        .answers
        .iter()
        .map(|(key, answer)| {
            let value = match answer {
                ChatInputAnswer::Submitted(answered) => match &answered.value {
                    ChatInputAnswerValue::Selected(selected) => selected.value.clone(),
                    ChatInputAnswerValue::Text(text) => text.value.clone(),
                    other => format!("{other:?}"),
                },
                other => format!("{other:?}"),
            };
            format!("{key}={value}")
        })
        .collect();
    pairs.sort();
    let response = match answer.response {
        ChatInputResponseKind::Accept => "accept",
        ChatInputResponseKind::Decline => "decline",
        ChatInputResponseKind::Cancel => "cancel",
    };
    std::iter::once(response.to_owned())
        .chain(pairs)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A dialog that answers every prompt the same way, and says when it is asked
/// and when it opens Ask.
struct FakeDialog {
    reply: Reply,
    asked: Mutex<Vec<Prompt>>,
    shown: Notify,
    opened: Mutex<Vec<String>>,
    open: Notify,
    /// The cookies the host asked to take down.
    withdrawn: Mutex<Vec<String>>,
    taken_down: Notify,
}

impl FakeDialog {
    fn answering(grant: bool) -> Arc<Self> {
        Self::replying(if grant {
            Reply::Granted(Vec::new())
        } else {
            Reply::Denied
        })
    }

    fn replying(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            asked: Mutex::default(),
            shown: Notify::new(),
            opened: Mutex::default(),
            open: Notify::new(),
            withdrawn: Mutex::default(),
            taken_down: Notify::new(),
        })
    }

    fn asked(&self) -> Vec<Prompt> {
        self.asked.lock().unwrap().clone()
    }

    async fn wait_shown(&self) {
        timeout(Duration::from_secs(5), self.shown.notified())
            .await
            .expect("the dialog was never shown");
    }

    fn withdrawn(&self) -> Vec<String> {
        self.withdrawn.lock().unwrap().clone()
    }

    async fn wait_withdrawn(&self) {
        timeout(Duration::from_secs(5), self.taken_down.notified())
            .await
            .expect("the dialog was never taken down");
    }
}

impl Prompter for FakeDialog {
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = Reply> + Send + '_>> {
        self.asked.lock().unwrap().push(prompt);
        self.shown.notify_one();
        Box::pin(std::future::ready(self.reply.clone()))
    }

    fn open(&self, session_uri: &str) {
        self.opened.lock().unwrap().push(session_uri.to_owned());
        self.open.notify_one();
    }

    fn withdraw(&self, cookie: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.withdrawn.lock().unwrap().push(cookie.to_owned());
        self.taken_down.notify_one();
        Box::pin(std::future::ready(()))
    }
}

struct Harness {
    client: Client,
    answers: Arc<Mutex<Vec<String>>>,
    host: Arc<host::Host>,
}

async fn connect(dialog: Arc<FakeDialog>) -> Harness {
    let answers = Arc::default();
    let backend = AskingBackend {
        answers: Arc::clone(&answers),
    };
    let common::Harness { client, host } = common::with_prompter(Arc::new(backend), dialog).await;
    Harness {
        client,
        answers,
        host,
    }
}

async fn next_envelope(subscription: &mut SessionSubscription) -> ActionEnvelope {
    loop {
        let event = timeout(Duration::from_secs(5), subscription.recv())
            .await
            .expect("timed out waiting for an event")
            .expect("subscription closed");
        if let SubscriptionEvent::Action(envelope) = event {
            return envelope;
        }
    }
}

/// Creates a session of `provider`, waits until it is ready, and returns its
/// URI and its chat's. The client is left subscribed to the session, not to
/// the chat.
async fn ready_session(client: &Client, provider: &str) -> (String, String) {
    let session_uri = format!("ahp-session:/{}", uuid::Uuid::new_v4());
    let params = json!({
        "channel": session_uri,
        "provider": provider,
        "workingDirectories": [uri::from_path(&std::env::temp_dir())],
    });
    client
        .request::<_, serde_json::Value>("createSession", params)
        .await
        .expect("createSession");
    let (result, mut events) = client
        .subscribe(session_uri.clone())
        .await
        .expect("subscribe");
    let Some(SnapshotState::Session(session)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a session snapshot");
    };
    let mut session: SessionState = *session;
    while session.lifecycle != SessionLifecycle::Ready {
        let envelope = next_envelope(&mut events).await;
        apply_action_to_session(&mut session, &envelope.action);
    }
    let chat = session.default_chat.expect("a default chat");
    (session_uri, chat)
}

async fn ready_chat(client: &Client) -> String {
    ready_session(client, "asker").await.1
}

async fn session_state(client: &Client, session_uri: &str) -> SessionState {
    let (result, _) = client
        .subscribe(session_uri.to_owned())
        .await
        .expect("subscribe to session");
    match result.snapshot.map(|s| s.state) {
        Some(SnapshotState::Session(session)) => *session,
        _ => panic!("expected a session snapshot"),
    }
}

async fn watch(client: &Client, chat_uri: &str) -> (ChatState, SessionSubscription) {
    let (result, events) = client
        .subscribe(chat_uri.to_owned())
        .await
        .expect("subscribe to chat");
    let Some(SnapshotState::Chat(chat)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a chat snapshot");
    };
    (*chat, events)
}

async fn start_turn(client: &Client, chat_uri: &str, text: &str) {
    let started = StateAction::ChatTurnStarted(ChatTurnStartedAction {
        turn_id: "t1".into(),
        started_at: host::now(),
        message: Message {
            text: text.into(),
            origin: MessageOrigin {
                kind: MessageKind::User,
            },
            attachments: None,
            model: None,
            agent: None,
            meta: None,
        },
        queued_message_id: None,
        meta: None,
    });
    client
        .dispatch(chat_uri.to_owned(), started)
        .await
        .expect("dispatch");
}

/// The tool call waiting for confirmation, with its options' ids.
fn pending(chat: &ChatState) -> Option<(String, Vec<String>)> {
    let turn = chat.active_turn.as_ref()?;
    turn.response_parts.iter().find_map(|part| match part {
        ResponsePart::ToolCall(tool) => match &tool.tool_call {
            ToolCallState::PendingConfirmation(state) => Some((
                state.tool_call_id.clone(),
                state
                    .options
                    .iter()
                    .flatten()
                    .map(|option| option.id.clone())
                    .collect(),
            )),
            _ => None,
        },
        _ => None,
    })
}

/// The tool call waiting for confirmation, whole.
fn pending_state(chat: &ChatState) -> Option<&ToolCallPendingConfirmationState> {
    let turn = chat.active_turn.as_ref()?;
    turn.response_parts.iter().find_map(|part| match part {
        ResponsePart::ToolCall(tool) => match &tool.tool_call {
            ToolCallState::PendingConfirmation(state) => Some(state),
            _ => None,
        },
        _ => None,
    })
}

/// The input request waiting for an answer in the active turn.
fn pending_input(chat: &ChatState) -> Option<&ChatInputRequest> {
    let turn = chat.active_turn.as_ref()?;
    turn.response_parts.iter().find_map(|part| match part {
        ResponsePart::InputRequest(input) if input.response.is_none() => Some(&input.request),
        _ => None,
    })
}

async fn until(
    chat: &mut ChatState,
    events: &mut SessionSubscription,
    done: impl Fn(&ChatState) -> bool,
) {
    while !done(chat) {
        let envelope = next_envelope(events).await;
        if envelope.rejection_reason.is_none() {
            apply_action_to_chat(chat, &envelope.action);
        }
    }
}

/// The finished turn's answer, and how its tool call ended.
fn outcome(chat: &ChatState) -> (String, &'static str) {
    let turn = chat.turns.first().expect("a finished turn");
    let mut answer = String::new();
    let mut tool = "none";
    for part in &turn.response_parts {
        match part {
            ResponsePart::Markdown(markdown) => answer.push_str(&markdown.content),
            ResponsePart::ToolCall(call) => {
                tool = match &call.tool_call {
                    ToolCallState::Completed(state) if state.success => "completed",
                    ToolCallState::Cancelled(_) => "cancelled",
                    _ => "other",
                }
            }
            _ => {}
        }
    }
    (answer, tool)
}

fn confirmed(tool_call_id: &str, approved: bool, option: &str) -> StateAction {
    StateAction::ChatToolCallConfirmed(ChatToolCallConfirmedAction {
        turn_id: "t1".into(),
        tool_call_id: tool_call_id.into(),
        meta: None,
        approved,
        confirmed: None,
        reason: None,
        edited_tool_input: None,
        user_suggestion: None,
        reason_message: None,
        selected_option_id: Some(option.into()),
    })
}

fn completed(
    request_id: &str,
    response: ChatInputResponseKind,
    answers: &[(&str, ChatInputAnswerValue)],
) -> StateAction {
    let answers: HashMap<String, ChatInputAnswer> = answers
        .iter()
        .map(|(id, value)| {
            let answer = ChatInputAnswer::Submitted(ChatInputAnswered {
                value: value.clone(),
            });
            (id.to_string(), answer)
        })
        .collect();
    StateAction::ChatInputCompleted(ChatInputCompletedAction {
        request_id: request_id.into(),
        response,
        answers: (!answers.is_empty()).then_some(answers),
    })
}

fn pick(option: &str) -> ChatInputAnswerValue {
    ChatInputAnswerValue::Selected(ChatInputSelectedAnswerValue {
        value: option.into(),
        freeform_values: None,
    })
}

#[tokio::test]
async fn a_watching_client_answers_in_the_chat() {
    let dialog = FakeDialog::answering(false);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    let (tool_call_id, options) = pending(&chat).unwrap();
    assert_eq!(options, ["allow_always", "allow", "reject"]);
    // The chat is told which option to start on and what the tool would
    // touch, so a client need not work either out for itself.
    let state = pending_state(&chat).unwrap();
    assert_eq!(
        state.meta.as_ref().and_then(|meta| meta.get("otto")),
        Some(&json!({ "defaultOption": "allow" }))
    );
    assert_eq!(
        state.tool_input,
        Some(ToolInput::Inline(
            json!({ "rawInput": { "command": "cargo test" } }).to_string()
        ))
    );
    assert_eq!(
        state.edits,
        Some(json!([{ "path": "/tmp/a.rs", "oldText": "a", "newText": "b" }]))
    );

    // A confirmation for a tool call that is not waiting is refused.
    client
        .dispatch(chat_uri.clone(), confirmed("no-such-call", true, "allow"))
        .await
        .expect("dispatch");
    let refused = next_envelope(&mut events).await;
    assert!(refused.rejection_reason.is_some(), "{refused:?}");

    client
        .dispatch(
            chat_uri.clone(),
            confirmed(&tool_call_id, true, "allow_always"),
        )
        .await
        .expect("dispatch");
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat), ("allow_always".into(), "completed"));
    assert!(dialog.asked().is_empty(), "nobody should be asked twice");
}

#[tokio::test]
async fn with_nobody_watching_the_dialog_answers() {
    let dialog = FakeDialog::answering(true);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    dialog.wait_shown().await;
    let prompt = &dialog.asked()[0];
    assert_eq!(prompt.title, "Asker wants to run a command");
    assert_eq!(prompt.open, "Open in Ask");

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    // A plain yes picks the narrowest allowing option.
    assert_eq!(outcome(&chat), ("allow".into(), "completed"));
}

#[tokio::test]
async fn a_client_that_stops_watching_hands_the_question_to_the_dialog() {
    let dialog = FakeDialog::answering(false);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    assert!(dialog.asked().is_empty());

    client
        .unsubscribe(chat_uri.clone())
        .await
        .expect("unsubscribe");
    dialog.wait_shown().await;

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat), ("reject".into(), "cancelled"));
}

#[tokio::test]
async fn a_watched_question_left_unanswered_reaches_the_dialog() {
    let dialog = FakeDialog::answering(false);
    let Harness { client, host, .. } = connect(Arc::clone(&dialog)).await;
    host.set_watched_grace(Duration::from_millis(200));
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    // Watched, so the question waits for the client first.
    assert!(dialog.asked().is_empty());

    // The client shows nothing and answers nothing, and the dialog takes over.
    dialog.wait_shown().await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat), ("reject".into(), "cancelled"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(dialog.asked().len(), 1, "a question escalates once");
}

#[tokio::test]
async fn opening_a_permission_question_in_ask_keeps_it_waiting() {
    let dialog = FakeDialog::replying(Reply::Open);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let (session_uri, chat_uri) = ready_session(&client, "asker").await;

    start_turn(&client, &chat_uri, "run the tests").await;
    timeout(Duration::from_secs(5), dialog.open.notified())
        .await
        .expect("Ask was never opened");
    assert_eq!(*dialog.opened.lock().unwrap(), [session_uri]);

    // Ask comes up and answers it.
    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    let (tool_call_id, _) = pending(&chat).unwrap();
    client
        .dispatch(chat_uri.clone(), confirmed(&tool_call_id, true, "allow"))
        .await
        .expect("dispatch");
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat), ("allow".into(), "completed"));

    // Leaving again asks again: the question was handed back when Ask opened.
    assert_eq!(dialog.asked().len(), 1);
}

/// A dialog that is up when the question is settled somewhere else comes down.
/// Otherwise the panel sits in front of the person asking about something
/// already decided, and the answer they give is dropped.
#[tokio::test]
async fn a_dialog_is_taken_down_when_the_question_is_answered_elsewhere() {
    // A question the dialog could not show stays open in the chat, escalated —
    // the same state a dialog someone is still looking at would be in.
    let dialog = FakeDialog::replying(Reply::Unavailable);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let (session_uri, chat_uri) = ready_session(&client, "questioner").await;

    start_turn(&client, &chat_uri, "pick").await;
    dialog.wait_shown().await;
    assert!(
        dialog.withdrawn().is_empty(),
        "nothing settled yet: {:?}",
        dialog.withdrawn()
    );

    // A client answers it in the chat instead.
    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending_input(chat).is_some()).await;
    let request_id = pending_input(&chat).expect("a question").id.clone();
    let picked = ChatInputAnswerValue::Selected(ChatInputSelectedAnswerValue {
        value: "Postgres".into(),
        freeform_values: None,
    });
    client
        .dispatch(
            chat_uri.clone(),
            completed(
                &request_id,
                ChatInputResponseKind::Accept,
                &[("question_0", picked)],
            ),
        )
        .await
        .expect("dispatch");

    dialog.wait_withdrawn().await;
    assert_eq!(dialog.withdrawn(), [format!("{session_uri}#{request_id}")]);
}

/// A question nobody ever escalated has no dialog to take down.
#[tokio::test]
async fn a_question_answered_before_any_dialog_withdraws_nothing() {
    let dialog = FakeDialog::answering(true);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let (_session_uri, chat_uri) = ready_session(&client, "asker").await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    let (tool_call_id, _) = pending(&chat).unwrap();
    client
        .dispatch(chat_uri.clone(), confirmed(&tool_call_id, true, "allow"))
        .await
        .expect("dispatch");
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;

    assert!(dialog.asked().is_empty(), "a watching client answered it");
    assert!(dialog.withdrawn().is_empty(), "so there was nothing up");
}

/// A dialog that answered the question itself has already closed: telling it
/// to take itself down would be a call for nothing.
#[tokio::test]
async fn a_dialog_that_answered_is_not_asked_to_withdraw() {
    let dialog = FakeDialog::answering(false);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let (_session_uri, chat_uri) = ready_session(&client, "asker").await;

    start_turn(&client, &chat_uri, "run the tests").await;
    dialog.wait_shown().await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;

    assert_eq!(outcome(&chat), ("reject".into(), "cancelled"));
    assert!(dialog.withdrawn().is_empty(), "{:?}", dialog.withdrawn());
}

#[tokio::test]
async fn a_watching_client_answers_an_agents_question() {
    let dialog = FakeDialog::answering(false);
    let Harness {
        client, answers, ..
    } = connect(Arc::clone(&dialog)).await;
    let (session_uri, chat_uri) = ready_session(&client, "questioner").await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "pick").await;
    until(&mut chat, &mut events, |chat| pending_input(chat).is_some()).await;
    let request = pending_input(&chat).unwrap().clone();
    assert_eq!(request.id, "req-t1");
    assert_ne!(chat.status & SessionStatus::InputNeeded.bits(), 0);

    // The session carries it too, for clients that do not watch the chat.
    let session = session_state(&client, &session_uri).await;
    let [SessionInputRequest::ChatInput(mirrored)] = session.input_needed.as_deref().unwrap()
    else {
        panic!(
            "expected the question in inputNeeded: {:?}",
            session.input_needed
        );
    };
    assert_eq!(mirrored.chat, chat_uri);
    assert_eq!(mirrored.request, request);

    // Accepting without the required answer is refused, and so is an answer
    // of the wrong kind.
    let accept = ChatInputResponseKind::Accept;
    client
        .dispatch(chat_uri.clone(), completed("req-t1", accept, &[]))
        .await
        .expect("dispatch");
    assert!(next_envelope(&mut events).await.rejection_reason.is_some());
    let text = ChatInputAnswerValue::Text(ChatInputTextAnswerValue {
        value: "SQLite".into(),
    });
    client
        .dispatch(
            chat_uri.clone(),
            completed("req-t1", accept, &[("question_0", text)]),
        )
        .await
        .expect("dispatch");
    assert!(next_envelope(&mut events).await.rejection_reason.is_some());

    client
        .dispatch(
            chat_uri.clone(),
            completed("req-t1", accept, &[("question_0", pick("SQLite"))]),
        )
        .await
        .expect("dispatch");
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat).0, "accept question_0=SQLite");
    assert_eq!(*answers.lock().unwrap(), ["accept question_0=SQLite"]);
    assert!(dialog.asked().is_empty());
    assert_eq!(
        session_state(&client, &session_uri).await.input_needed,
        None
    );
}

#[tokio::test]
async fn with_nobody_watching_a_select_question_is_answered_in_the_dialog() {
    let reply = Reply::Granted(vec![("question_0".into(), "Postgres".into())]);
    let dialog = FakeDialog::replying(reply);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_session(&client, "questioner").await.1;

    start_turn(&client, &chat_uri, "pick").await;
    dialog.wait_shown().await;
    let prompt = dialog.asked().remove(0);
    // Who is asking, as a handle; the dialog owns the rest of the words.
    assert_eq!(prompt.title, "@questioner");
    assert!(prompt.handle_title);
    assert_eq!(prompt.subtitle, "Questioner needs to know");
    assert_eq!(
        (
            prompt.grant.as_str(),
            prompt.deny.as_str(),
            prompt.open.as_str()
        ),
        ("", "", "Open in Ask")
    );
    let [choice] = &prompt.choices[..] else {
        panic!("expected one choice group: {:?}", prompt.choices);
    };
    assert_eq!(choice.id, "question_0");
    assert_eq!(choice.label, "Which database?");
    assert_eq!(
        choice.options,
        [
            ("Postgres".into(), "Postgres".into()),
            ("SQLite".into(), "SQLite".into())
        ]
    );

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat).0, "accept question_0=Postgres");
}

#[tokio::test]
async fn skipping_in_the_dialog_declines() {
    let dialog = FakeDialog::replying(Reply::Denied);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_session(&client, "questioner").await.1;

    start_turn(&client, &chat_uri, "pick").await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat).0, "decline");
}

#[tokio::test]
async fn a_question_the_dialog_cannot_ask_waits_in_the_chat() {
    let dialog = FakeDialog::replying(Reply::Unavailable);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_session(&client, "questioner").await.1;

    start_turn(&client, &chat_uri, "name it").await;
    dialog.wait_shown().await;
    let prompt = dialog.asked().remove(0);
    // Free text cannot be typed into the dialog: it only offers to open Ask.
    assert!(prompt.choices.is_empty());
    assert_eq!(prompt.grant, "");
    assert_eq!(prompt.open, "Open in Ask");
    assert_eq!(prompt.body, "What is it called?");

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending_input(chat).is_some()).await;
    let name = ChatInputAnswerValue::Text(ChatInputTextAnswerValue {
        value: "Otto".into(),
    });
    client
        .dispatch(
            chat_uri.clone(),
            completed("req-t1", ChatInputResponseKind::Accept, &[("name", name)]),
        )
        .await
        .expect("dispatch");
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat).0, "accept name=Otto");
}

#[tokio::test]
async fn opening_an_agents_question_in_ask_keeps_it_waiting() {
    let dialog = FakeDialog::replying(Reply::Open);
    let Harness { client, .. } = connect(Arc::clone(&dialog)).await;
    let (session_uri, chat_uri) = ready_session(&client, "questioner").await;

    start_turn(&client, &chat_uri, "pick").await;
    timeout(Duration::from_secs(5), dialog.open.notified())
        .await
        .expect("Ask was never opened");
    assert_eq!(
        *dialog.opened.lock().unwrap(),
        std::slice::from_ref(&session_uri)
    );

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending_input(chat).is_some()).await;
    // Handed back: leaving without answering asks in a dialog again.
    client
        .unsubscribe(chat_uri.clone())
        .await
        .expect("unsubscribe");
    timeout(Duration::from_secs(5), async {
        while dialog.asked().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the question was not asked again");
}

#[tokio::test]
async fn cancelling_the_turn_cancels_the_question() {
    let dialog = FakeDialog::answering(false);
    let Harness {
        client, answers, ..
    } = connect(Arc::clone(&dialog)).await;
    let (session_uri, chat_uri) = ready_session(&client, "questioner").await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "pick").await;
    until(&mut chat, &mut events, |chat| pending_input(chat).is_some()).await;
    let cancel = StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
        turn_id: "t1".into(),
        duration: 0,
        meta: None,
    });
    client
        .dispatch(chat_uri.clone(), cancel)
        .await
        .expect("dispatch");
    timeout(Duration::from_secs(5), async {
        while answers.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the agent never heard back");
    assert_eq!(*answers.lock().unwrap(), ["cancel"]);
    assert_eq!(
        session_state(&client, &session_uri).await.input_needed,
        None
    );
    assert!(dialog.asked().is_empty());
}

#[tokio::test]
async fn cancelling_the_turn_cancels_the_permission_request() {
    let dialog = FakeDialog::answering(false);
    let Harness {
        client, answers, ..
    } = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri, "run the tests").await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    let cancel = StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
        turn_id: "t1".into(),
        duration: 0,
        meta: None,
    });
    client
        .dispatch(chat_uri.clone(), cancel)
        .await
        .expect("dispatch");
    timeout(Duration::from_secs(5), async {
        while answers.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the agent never heard back");
    // Cancelled, not refused: the agent is not told the person said no.
    assert_eq!(*answers.lock().unwrap(), ["cancelled"]);
    assert!(dialog.asked().is_empty());
}
