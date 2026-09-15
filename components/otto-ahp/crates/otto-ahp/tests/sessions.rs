//! End-to-end session tests: the official AHP client drives a real server
//! backed by the echo agent, and reduces what it receives with the same
//! reducers the server uses.

use std::sync::Arc;
use std::time::Duration;

use ahp::reducers::{apply_action_to_chat, apply_action_to_session};
use ahp::{Client, ClientConfig, ClientError, SessionSubscription, SubscriptionEvent};
use ahp_types::actions::{
    ActionEnvelope, ChatPendingMessageSetAction, ChatTurnCancelledAction, ChatTurnStartedAction,
    StateAction,
};
use ahp_types::commands::ListSessionsResult;
use ahp_types::errors::{ahp_error_codes, json_rpc_error_codes};
use ahp_types::state::{
    ChatState, Message, MessageKind, MessageOrigin, PendingMessageKind, ResponsePart,
    SessionLifecycle, SessionState, SnapshotState, TurnState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_ahp::Server;
use otto_ahp::agent::EchoBackend;
use otto_ahp::{host, uri};
use serde_json::{Value, json};
use tokio::time::timeout;

async fn connect() -> Client {
    let server = Server::bind("127.0.0.1:0", Arc::new(EchoBackend))
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

fn new_session_uri() -> String {
    format!("ahp-session:/{}", uuid::Uuid::new_v4())
}

fn temp_dir_uri() -> String {
    uri::from_path(&std::env::temp_dir())
}

async fn create_session(client: &Client, uri: &str) -> Result<Value, ClientError> {
    client
        .request(
            "createSession",
            json!({ "channel": uri, "provider": "echo", "workingDirectories": [temp_dir_uri()] }),
        )
        .await
}

async fn next_event(subscription: &mut SessionSubscription) -> SubscriptionEvent {
    timeout(Duration::from_secs(5), subscription.recv())
        .await
        .expect("timed out waiting for an event")
        .expect("subscription closed")
}

async fn next_envelope(subscription: &mut SessionSubscription) -> ActionEnvelope {
    loop {
        if let SubscriptionEvent::Action(envelope) = next_event(subscription).await {
            return envelope;
        }
    }
}

/// Creates a session, waits until it is ready, and subscribes to its chat.
async fn ready_session(client: &Client) -> (String, ChatState, SessionSubscription) {
    let uri = new_session_uri();
    create_session(client, &uri).await.expect("createSession");

    let (result, mut session_events) = client.subscribe(uri.clone()).await.expect("subscribe");
    let Some(SnapshotState::Session(session)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a session snapshot");
    };
    let mut session: SessionState = *session;
    while session.lifecycle != SessionLifecycle::Ready {
        let envelope = next_envelope(&mut session_events).await;
        apply_action_to_session(&mut session, &envelope.action);
    }

    let chat_uri = session.default_chat.expect("a default chat");
    let (result, chat_events) = client.subscribe(chat_uri).await.expect("subscribe to chat");
    let Some(SnapshotState::Chat(chat)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a chat snapshot");
    };
    (uri, *chat, chat_events)
}

fn turn_started(turn_id: &str, text: &str) -> StateAction {
    StateAction::ChatTurnStarted(ChatTurnStartedAction {
        turn_id: turn_id.into(),
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
    })
}

#[tokio::test]
async fn a_prompt_streams_back_and_completes() {
    let client = connect().await;
    let (session_uri, mut chat, mut chat_events) = ready_session(&client).await;

    client
        .dispatch(
            chat.resource.clone(),
            turn_started("t1", "hello from the test"),
        )
        .await
        .expect("dispatch");
    while chat.turns.is_empty() {
        let envelope = next_envelope(&mut chat_events).await;
        assert_eq!(envelope.rejection_reason, None);
        apply_action_to_chat(&mut chat, &envelope.action);
    }

    let turn = &chat.turns[0];
    assert!(matches!(turn.state, TurnState::Complete));
    let text: String = turn
        .response_parts
        .iter()
        .map(|part| match part {
            ResponsePart::Markdown(markdown) => markdown.content.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(text, "hello from the test");

    let sessions: ListSessionsResult = client
        .request("listSessions", json!({ "channel": ROOT_RESOURCE_URI }))
        .await
        .expect("listSessions");
    let listed = sessions
        .items
        .iter()
        .find(|s| s.resource == session_uri)
        .expect("the session is listed");
    assert_eq!(listed.provider, "echo");
    assert_eq!(listed.title, "hello from the test");
}

#[tokio::test]
async fn root_subscribers_hear_about_new_sessions() {
    let client = connect().await;
    let mut root_events = client.attach_subscription(ROOT_RESOURCE_URI).await;
    let uri = new_session_uri();
    create_session(&client, &uri).await.expect("createSession");

    loop {
        if let SubscriptionEvent::SessionAdded(added) = next_event(&mut root_events).await {
            assert_eq!(added.summary.resource, uri);
            break;
        }
    }
}

#[tokio::test]
async fn invalid_client_actions_are_echoed_with_a_reason() {
    let client = connect().await;
    let (_, chat, mut chat_events) = ready_session(&client).await;

    let cancel = StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
        turn_id: "no-such-turn".into(),
        duration: 0,
        meta: None,
    });
    client
        .dispatch(chat.resource, cancel)
        .await
        .expect("dispatch");

    let envelope = next_envelope(&mut chat_events).await;
    assert!(envelope.rejection_reason.is_some(), "{envelope:?}");
    assert!(envelope.origin.is_some());
}

#[tokio::test]
async fn create_session_validates_its_params() {
    let client = connect().await;
    let uri = new_session_uri();
    create_session(&client, &uri).await.expect("createSession");

    let code = |err: ClientError| match err {
        ClientError::Rpc(err) => err.code,
        other => panic!("expected a JSON-RPC error, got: {other}"),
    };

    let duplicate = create_session(&client, &uri).await.unwrap_err();
    assert_eq!(code(duplicate), ahp_error_codes::SESSION_ALREADY_EXISTS);

    let unknown_agent = client
        .request::<_, Value>(
            "createSession",
            json!({ "channel": new_session_uri(), "provider": "nope" }),
        )
        .await
        .unwrap_err();
    assert_eq!(code(unknown_agent), ahp_error_codes::PROVIDER_NOT_FOUND);

    let missing_folder = client
        .request::<_, Value>(
            "createSession",
            json!({
                "channel": new_session_uri(),
                "provider": "echo",
                "workingDirectories": ["file:///definitely/not/here"],
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(code(missing_folder), json_rpc_error_codes::INVALID_PARAMS);
}

/// The launcher's hand-off: queue a request right after creating the session,
/// without waiting for it to be ready, and expect it to run once it is.
#[tokio::test]
async fn a_queued_message_runs_once_the_session_can_take_it() {
    let client = connect().await;
    let uri = new_session_uri();
    create_session(&client, &uri).await.expect("createSession");

    let (result, _session_events) = client.subscribe(uri).await.expect("subscribe");
    let Some(SnapshotState::Session(session)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a session snapshot");
    };
    let chat_uri = session.default_chat.expect("a default chat");
    let (result, mut chat_events) = client
        .subscribe(chat_uri.clone())
        .await
        .expect("subscribe to chat");
    let Some(SnapshotState::Chat(chat)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a chat snapshot");
    };
    let mut chat = *chat;

    let queued = StateAction::ChatPendingMessageSet(ChatPendingMessageSetAction {
        kind: PendingMessageKind::Queued,
        id: "q1".into(),
        message: Message {
            text: "queued hello".into(),
            origin: MessageOrigin {
                kind: MessageKind::User,
            },
            attachments: None,
            model: None,
            agent: None,
            meta: None,
        },
    });
    client.dispatch(chat_uri, queued).await.expect("dispatch");

    while chat.turns.is_empty() {
        let envelope = next_envelope(&mut chat_events).await;
        assert_eq!(envelope.rejection_reason, None);
        apply_action_to_chat(&mut chat, &envelope.action);
    }
    let turn = &chat.turns[0];
    assert!(matches!(turn.state, TurnState::Complete));
    assert_eq!(turn.message.text, "queued hello");
    assert_eq!(chat.queued_messages, None);
}
