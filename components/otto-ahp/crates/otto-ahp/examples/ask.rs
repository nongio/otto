//! Sends one prompt to a running otto-ahp server.
//!
//! By default it creates a session, waits for it to be ready, starts a turn on
//! its chat, and streams the answer. With `--queue` it hands the prompt off the
//! way Otto's launcher does: it queues the prompt on the new session and exits
//! at once, and the server starts it when the agent is ready.
//!
//! ```sh
//! cargo run -p otto-ahp --example ask -- --folder ~/dev/otto-ahp "summarise the README"
//! cargo run -p otto-ahp --example ask -- --queue "summarise the README"
//! ```

use std::io::Write;
use std::path::PathBuf;

use ahp::reducers::apply_action_to_session;
use ahp::{Client, ClientConfig, SessionSubscription, SubscriptionEvent};
use ahp_types::actions::{
    ActionEnvelope, ChatPendingMessageSetAction, ChatTurnStartedAction, StateAction,
};
use ahp_types::state::{
    Message, MessageKind, MessageOrigin, PendingMessageKind, ResponsePart, SessionLifecycle,
    SnapshotState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use anyhow::{Context, bail};
use clap::Parser;
use otto_ahp::{host, uri};
use serde_json::{Value, json};

#[derive(Parser)]
struct Args {
    /// WebSocket URL of the server.
    #[arg(long, default_value = "ws://127.0.0.1:4800")]
    url: String,
    /// Agent to ask. Defaults to the server's first agent.
    #[arg(long)]
    agent: Option<String>,
    /// Working directory for the session. Defaults to the current directory.
    #[arg(long)]
    folder: Option<PathBuf>,
    /// Queue the prompt and exit without waiting, like Otto's launcher.
    #[arg(long)]
    queue: bool,
    prompt: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let folder = match args.folder {
        Some(folder) => folder,
        None => std::env::current_dir()?,
    };
    let folder = folder
        .canonicalize()
        .with_context(|| format!("no such folder: {}", folder.display()))?;

    let transport = ahp_ws::WebSocketTransport::connect(&args.url)
        .await
        .with_context(|| format!("could not connect to {}", args.url))?;
    let client = Client::connect(transport, ClientConfig::default()).await?;
    client
        .initialize(
            "otto-ahp-ask".into(),
            vec![PROTOCOL_VERSION.into()],
            vec![ROOT_RESOURCE_URI.into()],
        )
        .await?;

    let session_uri = format!("ahp-session:/{}", uuid::Uuid::new_v4());
    let mut params = json!({
        "channel": session_uri,
        "workingDirectories": [uri::from_path(&folder)],
    });
    if let Some(agent) = &args.agent {
        params["provider"] = json!(agent);
    }
    client.request::<_, Value>("createSession", params).await?;

    let (result, mut session_events) = client.subscribe(session_uri.clone()).await?;
    let Some(SnapshotState::Session(session)) = result.snapshot.map(|s| s.state) else {
        bail!("the server sent no session snapshot");
    };
    let mut session = *session;
    let message = Message {
        text: args.prompt,
        origin: MessageOrigin {
            kind: MessageKind::User,
        },
        attachments: None,
        model: None,
        agent: None,
        meta: None,
    };

    if args.queue {
        let chat = session.default_chat.context("the session has no chat")?;
        let queued = StateAction::ChatPendingMessageSet(ChatPendingMessageSetAction {
            kind: PendingMessageKind::Queued,
            id: uuid::Uuid::new_v4().to_string(),
            message,
        });
        client.dispatch(chat, queued).await?;
        // A dispatched action gets no reply. The server handles a connection's
        // messages in order, so the ping coming back means it has the prompt.
        client.ping().await?;
        client.shutdown().await;
        println!("{session_uri}");
        return Ok(());
    }

    eprintln!("starting {}…", session.provider);
    while session.lifecycle == SessionLifecycle::Creating {
        let envelope = next_envelope(&mut session_events).await?;
        apply_action_to_session(&mut session, &envelope.action);
    }
    if let Some(error) = session.creation_error {
        bail!("the agent could not start: {}", error.message);
    }

    let chat = session.default_chat.context("the session has no chat")?;
    let (_, mut chat_events) = client.subscribe(chat.clone()).await?;
    let turn_id = uuid::Uuid::new_v4().to_string();
    let started = StateAction::ChatTurnStarted(ChatTurnStartedAction {
        turn_id: turn_id.clone(),
        started_at: host::now(),
        message,
        queued_message_id: None,
        meta: None,
    });
    client.dispatch(chat, started).await?;

    let mut stdout = std::io::stdout();
    loop {
        let envelope = next_envelope(&mut chat_events).await?;
        if let Some(reason) = envelope.rejection_reason {
            bail!("the server rejected the prompt: {reason}");
        }
        match envelope.action {
            StateAction::ChatResponsePart(part) => {
                if let ResponsePart::Markdown(markdown) = part.part {
                    write!(stdout, "{}", markdown.content)?;
                }
            }
            StateAction::ChatDelta(delta) => write!(stdout, "{}", delta.content)?,
            StateAction::ChatTurnComplete(end) if end.turn_id == turn_id => break,
            StateAction::ChatTurnCancelled(end) if end.turn_id == turn_id => bail!("cancelled"),
            StateAction::ChatError(error) if error.turn_id == turn_id => {
                bail!("the agent failed: {}", error.part.error.message)
            }
            _ => {}
        }
        stdout.flush()?;
    }
    writeln!(stdout)?;
    client.shutdown().await;
    Ok(())
}

async fn next_envelope(events: &mut SessionSubscription) -> anyhow::Result<ActionEnvelope> {
    loop {
        match events.recv().await {
            Some(SubscriptionEvent::Action(envelope)) => return Ok(envelope),
            Some(_) => {}
            None => bail!("the server closed the connection"),
        }
    }
}
