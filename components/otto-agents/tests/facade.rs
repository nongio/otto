//! `otto-agents acp`, driven the way a chat bridge drives it: the binary is
//! started as an ACP agent and talks to a server over its socket, and an ACP
//! client on the other end creates a session, prompts it, answers a
//! permission request, and takes the session up again from a new process.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, CloseSessionRequest, ContentBlock, InitializeRequest, ListSessionsRequest,
    LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionNotification, SessionUpdate,
    StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo};
use otto_agents::Server;
use otto_agents::agent::{
    Backend, Decision, Question, QuestionOption, SessionCommand, SessionEvent, SessionSpec,
    TurnOutcome, agent_info,
};
use otto_agents::dialog::{Prompt, Prompter, Reply};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

/// One agent. A prompt of `ask` asks to run a command first and answers with
/// the option it was given; anything else is echoed back.
struct Asker;

impl Backend for Asker {
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
                let SessionCommand::Prompt {
                    turn_id,
                    text,
                    remote,
                    ..
                } = command
                else {
                    continue;
                };
                let text = if text == "ask" {
                    let (reply, decision) = oneshot::channel();
                    let _ = events.send(SessionEvent::PermissionRequested {
                        turn_id: turn_id.clone(),
                        question: Box::new(question(&format!("call-{turn_id}"))),
                        reply,
                    });
                    let decision = decision.await.unwrap_or_else(|_| Decision::deny());
                    format!("answered {}", decision.option_id().unwrap_or("none"))
                } else if let Some(via) = remote {
                    format!("you said {text} from {via}")
                } else {
                    format!("you said {text}")
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
            ..Prompt::default()
        },
        options: vec![
            option("allow", "Allow", PermissionOptionKind::AllowOnce),
            option("reject", "Reject", PermissionOptionKind::RejectOnce),
        ],
        default_option_id: Some("allow".into()),
        tool_input: None,
        edits: vec![],
    }
}

/// A desktop dialog nobody answers, so every answer here comes over ACP.
struct Unanswered;

impl Prompter for Unanswered {
    fn ask(&self, _prompt: Prompt) -> Pin<Box<dyn Future<Output = Reply> + Send + '_>> {
        Box::pin(std::future::pending())
    }

    fn open(&self, _session_uri: &str) {}
}

async fn serving() -> String {
    otto_agents::i18n::pin_source_locale();
    let server = Server::bind_with_prompter("127.0.0.1:0", Arc::new(Asker), Arc::new(Unanswered))
        .await
        .expect("bind");
    let url = format!("ws://{}", server.local_addr().expect("local addr"));
    tokio::spawn(server.run());
    url
}

/// What the ACP client saw: the text of every update, and the permission
/// requests put to it.
#[derive(Default)]
struct Seen {
    text: String,
    asked: Vec<RequestPermissionRequest>,
}

/// Starts `otto-agents acp` against `url` and runs `body` with a connection to
/// it, with `--permissions client`. Permission requests are answered with
/// `allow`.
async fn with_facade<T: Send + 'static>(
    url: &str,
    seen: Arc<Mutex<Seen>>,
    body: impl AsyncFnOnce(ConnectionTo<Agent>) -> Result<T, agent_client_protocol::Error>
    + Send
    + 'static,
) -> T {
    with_facade_args(url, &["--permissions", "client"], seen, body).await
}

/// As [`with_facade`], with more arguments for `otto-agents acp`.
async fn with_facade_args<T: Send + 'static>(
    url: &str,
    extra: &[&str],
    seen: Arc<Mutex<Seen>>,
    body: impl AsyncFnOnce(ConnectionTo<Agent>) -> Result<T, agent_client_protocol::Error>
    + Send
    + 'static,
) -> T {
    let mut args = vec!["acp".to_owned(), "--url".to_owned(), url.to_owned()];
    args.extend(extra.iter().map(|arg| arg.to_string()));
    let config = AcpAgentConfig::new(env!("CARGO_BIN_EXE_otto-agents")).args(args);
    let run = Client
        .builder()
        .on_receive_notification(
            {
                let seen = Arc::clone(&seen);
                async move |notification: SessionNotification, _connection| {
                    let chunk = match notification.update {
                        SessionUpdate::UserMessageChunk(chunk)
                        | SessionUpdate::AgentMessageChunk(chunk) => chunk,
                        _ => return Ok(()),
                    };
                    if let ContentBlock::Text(text) = chunk.content {
                        seen.lock().unwrap().text.push_str(&text.text);
                        seen.lock().unwrap().text.push('\n');
                    }
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let seen = Arc::clone(&seen);
                async move |request: RequestPermissionRequest, responder, _connection| {
                    seen.lock().unwrap().asked.push(request);
                    responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new("allow")),
                    ))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            AcpAgent::new(config),
            async move |connection: ConnectionTo<Agent>| {
                connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                body(connection).await
            },
        );
    timeout(Duration::from_secs(30), run)
        .await
        .expect("the facade took too long")
        .expect("the ACP conversation failed")
}

fn prompt(session: &agent_client_protocol::schema::v1::SessionId, text: &str) -> PromptRequest {
    PromptRequest::new(
        session.clone(),
        vec![ContentBlock::Text(TextContent::new(text))],
    )
}

#[tokio::test]
async fn a_chat_bridge_drives_a_desktop_session() {
    let url = serving().await;
    let folder = tempfile::tempdir().expect("tempdir");
    let cwd = folder.path().to_path_buf();

    // A new session, prompted twice: once plainly, once with a permission
    // request that only the ACP client answers.
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (session, stops) = with_facade(&url, Arc::clone(&seen), async move |connection| {
        let opened = connection
            .send_request(NewSessionRequest::new(cwd))
            .block_task()
            .await?;
        let plain = connection
            .send_request(prompt(&opened.session_id, "hello"))
            .block_task()
            .await?;
        let asked = connection
            .send_request(prompt(&opened.session_id, "ask"))
            .block_task()
            .await?;
        Ok((opened.session_id, [plain.stop_reason, asked.stop_reason]))
    })
    .await;

    assert_eq!(stops, [StopReason::EndTurn, StopReason::EndTurn]);
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert!(seen.text.contains("you said hello"), "{}", seen.text);
    assert!(seen.text.contains("answered allow"), "{}", seen.text);
    assert_eq!(seen.asked.len(), 1, "one permission request");
    let options: Vec<_> = seen.asked[0]
        .options
        .iter()
        .map(|option| (option.option_id.0.to_string(), option.kind))
        .collect();
    assert_eq!(
        options,
        [
            ("allow".to_owned(), PermissionOptionKind::AllowOnce),
            ("reject".to_owned(), PermissionOptionKind::RejectOnce),
        ]
    );

    // Another process takes the same session up by the start of its id, is
    // told what was said so far, and carries on.
    let short = session.0[..8].to_owned();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let cwd = folder.path().to_path_buf();
    let stop = with_facade(&url, Arc::clone(&seen), async move |connection| {
        let id = agent_client_protocol::schema::v1::SessionId::new(short);
        connection
            .send_request(LoadSessionRequest::new(id.clone(), cwd))
            .block_task()
            .await?;
        let again = connection
            .send_request(prompt(&id, "again"))
            .block_task()
            .await?;
        Ok(again.stop_reason)
    })
    .await;
    assert_eq!(stop, StopReason::EndTurn);
    let text = seen.lock().unwrap().text.clone();
    assert!(
        text.contains("hello"),
        "the replay has the first prompt: {text}"
    );
    assert!(text.contains("you said again"), "{text}");

    // By default (`--permissions desktop`, also for a client that answers
    // by a fixed policy) permission requests stay on the desktop. The
    // client resumes the session without a replay, is never asked, and
    // closes the session when done.
    let id = session.0.to_string();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (asked_stop, after_close) =
        with_facade_args(&url, &[], Arc::clone(&seen), async move |connection| {
            let id = agent_client_protocol::schema::v1::SessionId::new(id);
            let cwd = std::env::temp_dir();
            connection
                .send_request(ResumeSessionRequest::new(id.clone(), cwd))
                .block_task()
                .await?;
            // The question waits for the desktop, which never answers here;
            // the client gives up on the turn.
            let asked = connection.send_request(prompt(&id, "ask"));
            tokio::time::sleep(Duration::from_millis(500)).await;
            connection.send_notification(CancelNotification::new(id.clone()))?;
            let asked = asked.block_task().await?;
            connection
                .send_request(CloseSessionRequest::new(id.clone()))
                .block_task()
                .await?;
            let after_close = connection
                .send_request(prompt(&id, "after close"))
                .block_task()
                .await;
            Ok((asked.stop_reason, after_close.is_err()))
        })
        .await;
    assert_eq!(asked_stop, StopReason::Cancelled);
    assert!(after_close, "a closed session takes no more prompts");
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert!(
        seen.asked.is_empty(),
        "the client was asked: {:?}",
        seen.asked
    );
    assert!(
        !seen.text.contains("hello"),
        "resume replays nothing: {}",
        seen.text
    );
}

#[tokio::test]
async fn a_bridge_says_its_messages_come_from_the_phone() {
    let url = serving().await;
    let folder = tempfile::tempdir().expect("tempdir");
    let cwd = folder.path().to_path_buf();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let stop = with_facade_args(
        &url,
        &["--remote", "Telegram"],
        Arc::clone(&seen),
        async move |connection| {
            let opened = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            let said = connection
                .send_request(prompt(&opened.session_id, "hi"))
                .block_task()
                .await?;
            Ok(said.stop_reason)
        },
    )
    .await;
    assert_eq!(stop, StopReason::EndTurn);
    let text = seen.lock().unwrap().text.clone();
    assert!(text.contains("you said hi from Telegram"), "{text}");
}

#[tokio::test]
async fn a_bridge_lists_the_sessions_in_its_folder() {
    let url = serving().await;
    let folder = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let cwd = folder.path().to_path_buf();
    let other = elsewhere.path().to_path_buf();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (created, mine, all) = with_facade(&url, seen, async move |connection| {
        let opened = connection
            .send_request(NewSessionRequest::new(cwd.clone()))
            .block_task()
            .await?;
        connection
            .send_request(NewSessionRequest::new(other))
            .block_task()
            .await?;
        let mine = connection
            .send_request(ListSessionsRequest::new().cwd(cwd))
            .block_task()
            .await?;
        let all = connection
            .send_request(ListSessionsRequest::new())
            .block_task()
            .await?;
        Ok((opened.session_id, mine.sessions, all.sessions))
    })
    .await;
    let ids: Vec<_> = mine.iter().map(|info| info.session_id.clone()).collect();
    assert_eq!(ids, [created], "only the session in the folder asked about");
    assert_eq!(mine[0].cwd, folder.path());
    assert_eq!(all.len(), 2, "no folder, every session");
}
