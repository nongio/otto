//! Permission questions: an agent's question opens in the chat as a tool call
//! waiting for confirmation. A client watching the chat answers it there; when
//! nobody is watching, or the watcher leaves, the host asks through its
//! prompter instead.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ahp::reducers::{apply_action_to_chat, apply_action_to_session};
use ahp::{Client, ClientConfig, SessionSubscription, SubscriptionEvent};
use ahp_types::actions::{
    ActionEnvelope, ChatToolCallConfirmedAction, ChatTurnStartedAction, StateAction,
};
use ahp_types::state::{
    ChatState, Message, MessageKind, MessageOrigin, ResponsePart, SessionLifecycle, SessionState,
    SnapshotState, ToolCallState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_ahp::agent::{
    Backend, Decision, Question, QuestionOption, SessionCommand, SessionEvent, SessionSpec,
    TurnOutcome, agent_info,
};
use otto_ahp::dialog::{Prompt, Prompter};
use otto_ahp::{Server, host, uri};
use serde_json::json;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::timeout;

/// An agent that asks to run `cargo test` before every answer, and answers
/// with the option it was given.
struct AskingBackend;

impl Backend for AskingBackend {
    fn agents(&self) -> Vec<ahp_types::state::AgentInfo> {
        vec![agent_info("asker", "Asker", "Asks before it answers")]
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
                let SessionCommand::Prompt { turn_id, .. } = command else {
                    continue;
                };
                let tool_call_id = format!("call-{turn_id}");
                let (reply, decision) = oneshot::channel();
                let _ = events.send(SessionEvent::PermissionRequested {
                    turn_id: turn_id.clone(),
                    question: question(&tool_call_id),
                    reply,
                });
                let decision = decision.await.unwrap_or_else(|_| Decision::deny());
                if decision.approved {
                    let _ = events.send(SessionEvent::ToolCallFinished {
                        turn_id: turn_id.clone(),
                        tool_call_id,
                        success: true,
                    });
                }
                let text = decision.option_id.unwrap_or_else(|| "none".into());
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

fn question(tool_call_id: &str) -> Question {
    let option = |id: &str, label: &str, allow| QuestionOption {
        id: id.into(),
        label: label.into(),
        allow,
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
        },
        options: vec![
            option("allow_always", "Always Allow", true),
            option("allow", "Allow", true),
            option("reject", "Reject", false),
        ],
    }
}

/// A dialog that answers every prompt the same way, and says when it is asked.
struct FakeDialog {
    grant: bool,
    asked: Mutex<Vec<Prompt>>,
    shown: Notify,
}

impl FakeDialog {
    fn answering(grant: bool) -> Arc<Self> {
        Arc::new(Self {
            grant,
            asked: Mutex::default(),
            shown: Notify::new(),
        })
    }

    fn asked(&self) -> Vec<Prompt> {
        self.asked.lock().unwrap().clone()
    }
}

impl Prompter for FakeDialog {
    fn ask(&self, prompt: Prompt) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        self.asked.lock().unwrap().push(prompt);
        self.shown.notify_one();
        Box::pin(std::future::ready(self.grant))
    }
}

async fn connect(dialog: Arc<FakeDialog>) -> Client {
    let server = Server::bind_with_prompter("127.0.0.1:0", Arc::new(AskingBackend), dialog)
        .await
        .expect("bind");
    let url = format!("ws://{}", server.local_addr().expect("local addr"));
    tokio::spawn(server.run());
    let transport = ahp_ws::WebSocketTransport::connect(&url)
        .await
        .expect("connect");
    let client = Client::connect(transport, ClientConfig::default())
        .await
        .expect("client");
    client
        .initialize(
            "test-client".into(),
            vec![PROTOCOL_VERSION.into()],
            vec![ROOT_RESOURCE_URI.into()],
        )
        .await
        .expect("initialize");
    client
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

/// Creates a session, waits until it is ready, and returns its chat's URI.
/// The client is left subscribed to the session, not to the chat.
async fn ready_chat(client: &Client) -> String {
    let session_uri = format!("ahp-session:/{}", uuid::Uuid::new_v4());
    let params = json!({
        "channel": session_uri,
        "provider": "asker",
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
    session.default_chat.expect("a default chat")
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

async fn start_turn(client: &Client, chat_uri: &str) {
    let started = StateAction::ChatTurnStarted(ChatTurnStartedAction {
        turn_id: "t1".into(),
        started_at: host::now(),
        message: Message {
            text: "run the tests".into(),
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

#[tokio::test]
async fn a_watching_client_answers_in_the_chat() {
    let dialog = FakeDialog::answering(false);
    let client = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    let (tool_call_id, options) = pending(&chat).unwrap();
    assert_eq!(options, ["allow_always", "allow", "reject"]);

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
    let client = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;

    start_turn(&client, &chat_uri).await;
    timeout(Duration::from_secs(5), dialog.shown.notified())
        .await
        .expect("the dialog was never shown");
    assert_eq!(dialog.asked()[0].title, "Asker wants to run a command");

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    // A plain yes picks the narrowest allowing option.
    assert_eq!(outcome(&chat), ("allow".into(), "completed"));
}

#[tokio::test]
async fn a_client_that_stops_watching_hands_the_question_to_the_dialog() {
    let dialog = FakeDialog::answering(false);
    let client = connect(Arc::clone(&dialog)).await;
    let chat_uri = ready_chat(&client).await;
    let (mut chat, mut events) = watch(&client, &chat_uri).await;

    start_turn(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| pending(chat).is_some()).await;
    assert!(dialog.asked().is_empty());

    client
        .unsubscribe(chat_uri.clone())
        .await
        .expect("unsubscribe");
    timeout(Duration::from_secs(5), dialog.shown.notified())
        .await
        .expect("the dialog was never shown");

    let (mut chat, mut events) = watch(&client, &chat_uri).await;
    until(&mut chat, &mut events, |chat| !chat.turns.is_empty()).await;
    assert_eq!(outcome(&chat), ("reject".into(), "cancelled"));
}
