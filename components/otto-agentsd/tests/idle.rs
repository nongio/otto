//! Idle agents: a session's agent stops after the idle timeout, and its next
//! request starts it again, taking up the agent's own session.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ahp::reducers::{apply_action_to_chat, apply_action_to_session};
use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::actions::{ChatTurnStartedAction, StateAction};
use ahp_types::state::{
    AgentInfo, Message, MessageKind, MessageOrigin, SessionLifecycle, SessionState, SnapshotState,
    TurnState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use otto_agentsd::Server;
use otto_agentsd::agent::{
    Backend, SessionCommand, SessionEvent, SessionSpec, TurnOutcome, agent_info,
};
use otto_agentsd::{host, uri};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::timeout;

const IDLE_TIMEOUT: Duration = Duration::from_millis(300);

/// Echoes prompts like the echo agent, reporting an agent session id, and
/// records each start and each stop.
struct Recording {
    /// The `resume` each start was given.
    starts: Arc<Mutex<Vec<Option<String>>>>,
    stopped: mpsc::UnboundedSender<()>,
}

impl Backend for Recording {
    fn agents(&self) -> Vec<AgentInfo> {
        vec![agent_info("echo", "Echo", "Repeats every prompt back")]
    }

    fn start(
        &self,
        spec: SessionSpec,
        mut commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) {
        self.starts.lock().unwrap().push(spec.resume);
        let stopped = self.stopped.clone();
        tokio::spawn(async move {
            let _ = events.send(SessionEvent::Ready {
                agent_session: Some("agent-1".into()),
            });
            while let Some(command) = commands.recv().await {
                if let SessionCommand::Prompt { turn_id, text, .. } = command {
                    let _ = events.send(SessionEvent::MessageChunk {
                        turn_id: turn_id.clone(),
                        text,
                    });
                    let _ = events.send(SessionEvent::TurnEnded {
                        turn_id,
                        outcome: TurnOutcome::Complete,
                    });
                }
            }
            let _ = stopped.send(());
        });
    }
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
async fn an_idle_agent_stops_and_its_next_request_starts_it_again() {
    let starts = Arc::default();
    let (stopped, mut stops) = mpsc::unbounded_channel();
    let backend = Recording {
        starts: Arc::clone(&starts),
        stopped,
    };
    let server = Server::bind("127.0.0.1:0", Arc::new(backend))
        .await
        .expect("bind");
    server.host().set_idle_timeout(Some(IDLE_TIMEOUT));
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

    let session_uri = format!("ahp-session:/{}", uuid::Uuid::new_v4());
    client
        .request::<_, serde_json::Value>(
            "createSession",
            json!({
                "channel": session_uri,
                "provider": "echo",
                "workingDirectories": [uri::from_path(&std::env::temp_dir())],
            }),
        )
        .await
        .expect("createSession");
    let (result, mut session_events) = client
        .subscribe(session_uri.clone())
        .await
        .expect("subscribe");
    let Some(SnapshotState::Session(session)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a session snapshot");
    };
    let mut session: SessionState = *session;
    while session.lifecycle != SessionLifecycle::Ready {
        let event = timeout(Duration::from_secs(5), session_events.recv())
            .await
            .expect("timed out waiting for the session")
            .expect("subscription closed");
        if let SubscriptionEvent::Action(envelope) = event {
            apply_action_to_session(&mut session, &envelope.action);
        }
    }
    let chat_uri = session.default_chat.expect("a default chat");
    // Being watched doesn't keep the agent running.
    let (result, mut chat_events) = client
        .subscribe(chat_uri.clone())
        .await
        .expect("subscribe to chat");
    let Some(SnapshotState::Chat(chat)) = result.snapshot.map(|s| s.state) else {
        panic!("expected a chat snapshot");
    };
    let mut chat = *chat;

    let mut run_turn = async |turn_id: &str, text: &str, chat: &mut _| {
        client
            .dispatch(chat_uri.clone(), turn_started(turn_id, text))
            .await
            .expect("dispatch");
        let done = |chat: &ahp_types::state::ChatState| {
            chat.turns
                .iter()
                .any(|turn| turn.id == turn_id && matches!(turn.state, TurnState::Complete))
        };
        while !done(chat) {
            let event = timeout(Duration::from_secs(5), chat_events.recv())
                .await
                .expect("timed out waiting for the turn")
                .expect("subscription closed");
            if let SubscriptionEvent::Action(envelope) = event {
                assert_eq!(envelope.rejection_reason, None);
                apply_action_to_chat(chat, &envelope.action);
            }
        }
    };

    run_turn("t1", "first", &mut chat).await;
    timeout(IDLE_TIMEOUT * 10, stops.recv())
        .await
        .expect("the idle agent was not stopped")
        .expect("backend gone");

    run_turn("t2", "second", &mut chat).await;
    assert_eq!(
        *starts.lock().unwrap(),
        [None, Some("agent-1".to_owned())],
        "the next request starts the agent again, resuming its session"
    );
}
