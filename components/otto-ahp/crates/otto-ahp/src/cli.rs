//! Terminal views of the host: `otto-ahp sessions` and `otto-ahp show`.

use std::io::Write;
use std::path::Path;

use ahp::reducers::apply_action_to_chat;
use ahp::{Client, ClientConfig, SubscriptionEvent};
use ahp_types::actions::StateAction;
use ahp_types::commands::ListSessionsResult;
use ahp_types::state::{
    ChatState, ResponsePart, SessionStatus, SessionSummary, SnapshotState, TurnState,
};
use ahp_types::{PROTOCOL_VERSION, ROOT_RESOURCE_URI};
use anyhow::{Context, bail};
use serde_json::json;

use crate::uri;

const SESSION_SCHEME: &str = "ahp-session:/";
const SHORT_ID_LEN: usize = 8;

pub async fn list_sessions(url: &str) -> anyhow::Result<()> {
    let client = connect(url).await?;
    let sessions = fetch_sessions(&client).await?;
    client.shutdown().await;

    let home = std::env::var_os("HOME");
    print!(
        "{}",
        render(
            &sessions,
            jiff::Timestamp::now(),
            home.as_deref().map(Path::new)
        )
    );
    Ok(())
}

/// Prints a session's transcript. With `follow`, keeps printing as the agent
/// works, until it has nothing left to do.
pub async fn show_session(url: &str, session: Option<&str>, follow: bool) -> anyhow::Result<()> {
    let client = connect(url).await?;
    let sessions = fetch_sessions(&client).await?;
    let summary = find_session(&sessions, session)?;

    let (subscribed, _session_events) = client.subscribe(summary.resource.clone()).await?;
    let Some(SnapshotState::Session(state)) = subscribed.snapshot.map(|s| s.state) else {
        bail!("the server sent no session snapshot");
    };
    let chat_uri = state.default_chat.context("the session has no chat")?;
    let (subscribed, mut chat_events) = client.subscribe(chat_uri).await?;
    let Some(SnapshotState::Chat(chat)) = subscribed.snapshot.map(|s| s.state) else {
        bail!("the server sent no chat snapshot");
    };
    let mut chat = *chat;

    let home = std::env::var_os("HOME");
    let mut out = std::io::stdout();
    writeln!(
        out,
        "{} · {} · {}\n",
        title(summary),
        summary.provider,
        folder(summary, home.as_deref().map(Path::new))
    )?;
    write!(out, "{}", render_chat(&chat))?;

    if follow && busy(&chat) {
        loop {
            out.flush()?;
            let Some(event) = chat_events.recv().await else {
                bail!("the server closed the connection");
            };
            let SubscriptionEvent::Action(envelope) = event else {
                continue;
            };
            if envelope.rejection_reason.is_some() {
                continue;
            }
            write!(out, "{}", follow_text(&envelope.action))?;
            apply_action_to_chat(&mut chat, &envelope.action);
            if done_after(&chat, &envelope.action) {
                break;
            }
        }
    } else if busy(&chat) {
        writeln!(out, "\n(still working; use --follow to watch)")?;
    }
    out.flush()?;
    client.shutdown().await;
    Ok(())
}

async fn connect(url: &str) -> anyhow::Result<Client> {
    let transport = ahp_ws::WebSocketTransport::connect(url)
        .await
        .with_context(|| format!("could not connect to {url}; is `otto-ahp serve` running?"))?;
    let client = Client::connect(transport, ClientConfig::default()).await?;
    client
        .initialize("otto-ahp-cli".into(), vec![PROTOCOL_VERSION.into()], vec![])
        .await?;
    Ok(client)
}

/// The host's sessions, most recently modified first.
async fn fetch_sessions(client: &Client) -> anyhow::Result<Vec<SessionSummary>> {
    let result: ListSessionsResult = client
        .request("listSessions", json!({ "channel": ROOT_RESOURCE_URI }))
        .await?;
    Ok(result.items)
}

/// Picks the session `query` names — a URI, an id, or a prefix of an id — or
/// the most recent one when there is no query.
pub fn find_session<'a>(
    sessions: &'a [SessionSummary],
    query: Option<&str>,
) -> anyhow::Result<&'a SessionSummary> {
    let Some(query) = query else {
        return sessions.first().context("no sessions");
    };
    let id = query.strip_prefix(SESSION_SCHEME).unwrap_or(query);
    let matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|session| session_id(session).starts_with(id))
        .collect();
    match matches.as_slice() {
        [session] => Ok(session),
        [] => bail!("no session matches `{query}`"),
        many => bail!(
            "`{query}` matches {} sessions; give more of the id",
            many.len()
        ),
    }
}

fn session_id(session: &SessionSummary) -> &str {
    session
        .resource
        .strip_prefix(SESSION_SCHEME)
        .unwrap_or(&session.resource)
}

/// Whether the agent still has work in hand: a running turn, or queued ones.
fn busy(chat: &ChatState) -> bool {
    chat.active_turn.is_some() || chat.queued_messages.as_ref().is_some_and(|q| !q.is_empty())
}

/// Whether following can stop once `action` has been applied to `chat`.
///
/// The host starts a queued message in two steps, `chat/pendingMessageRemoved`
/// then `chat/turnStarted`, and between them the chat looks idle. So the
/// removal never ends following; the turn it starts does.
fn done_after(chat: &ChatState, action: &StateAction) -> bool {
    !matches!(action, StateAction::ChatPendingMessageRemoved(_)) && !busy(chat)
}

/// Formats sessions as a table, most recently modified first.
pub fn render(sessions: &[SessionSummary], now: jiff::Timestamp, home: Option<&Path>) -> String {
    if sessions.is_empty() {
        return "No sessions.\n".to_owned();
    }
    let mut out = format!(
        "{:<8} {:<12} {:<10} {:>4}  {:<32} {}\n",
        "ID", "STATUS", "AGENT", "AGE", "FOLDER", "TITLE"
    );
    for session in sessions {
        let id: String = session_id(session).chars().take(SHORT_ID_LEN).collect();
        out.push_str(&format!(
            "{:<8} {:<12} {:<10} {:>4}  {:<32} {}\n",
            id,
            status_label(session.status),
            session.provider,
            age(&session.modified_at, now),
            folder(session, home),
            title(session),
        ));
    }
    out
}

/// Formats a chat as a transcript: each prompt quoted, then the answer.
pub fn render_chat(chat: &ChatState) -> String {
    let mut out = String::new();
    for turn in &chat.turns {
        push_exchange(&mut out, &turn.message.text, &turn.response_parts);
        out.push_str(match turn.state {
            TurnState::Complete | TurnState::Error => "\n\n",
            TurnState::Cancelled => "\n[cancelled]\n\n",
        });
    }
    if let Some(turn) = &chat.active_turn {
        push_exchange(&mut out, &turn.message.text, &turn.response_parts);
    }
    for queued in chat.queued_messages.iter().flatten() {
        out.push_str(&format!("[queued] {}\n", queued.message.text));
    }
    out
}

fn push_exchange(out: &mut String, prompt: &str, parts: &[ResponsePart]) {
    out.push_str(&quote(prompt));
    for part in parts {
        match part {
            ResponsePart::Markdown(markdown) => out.push_str(&markdown.content),
            ResponsePart::Error(error) => {
                out.push_str(&format!("\n[error] {}", error.error.message))
            }
            _ => {}
        }
    }
}

fn quote(prompt: &str) -> String {
    let quoted: Vec<String> = prompt.lines().map(|line| format!("> {line}")).collect();
    format!("{}\n\n", quoted.join("\n"))
}

/// What a live action adds to a transcript already on screen.
fn follow_text(action: &StateAction) -> String {
    match action {
        StateAction::ChatTurnStarted(started) => quote(&started.message.text),
        StateAction::ChatResponsePart(part) => match &part.part {
            ResponsePart::Markdown(markdown) => markdown.content.clone(),
            _ => String::new(),
        },
        StateAction::ChatDelta(delta) => delta.content.clone(),
        StateAction::ChatTurnComplete(_) => "\n\n".to_owned(),
        StateAction::ChatTurnCancelled(_) => "\n[cancelled]\n\n".to_owned(),
        StateAction::ChatError(error) => format!("\n[error] {}\n\n", error.part.error.message),
        _ => String::new(),
    }
}

fn title(session: &SessionSummary) -> &str {
    if session.title.is_empty() {
        "(untitled)"
    } else {
        &session.title
    }
}

fn status_label(bits: u32) -> &'static str {
    let status = SessionStatus::from_bits(bits);
    if status.contains(SessionStatus::InputNeeded) {
        "needs input"
    } else if status.contains(SessionStatus::InProgress) {
        "working"
    } else if status.contains(SessionStatus::Error) {
        "error"
    } else {
        "idle"
    }
}

fn age(timestamp: &str, now: jiff::Timestamp) -> String {
    let Ok(then) = timestamp.parse::<jiff::Timestamp>() else {
        return "?".to_owned();
    };
    match (now.as_second() - then.as_second()).max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn folder(session: &SessionSummary, home: Option<&Path>) -> String {
    let Some(path) = session
        .working_directories
        .as_ref()
        .and_then(|dirs| dirs.first())
        .and_then(|dir| uri::to_path(dir))
    else {
        return "-".to_owned();
    };
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ahp_types::state::{
        ActiveTurn, MarkdownResponsePart, Message, MessageKind, MessageOrigin, PendingMessage, Turn,
    };

    use super::*;

    fn summary(
        id: &str,
        status: SessionStatus,
        modified_at: &str,
        dir: &str,
        title: &str,
    ) -> SessionSummary {
        SessionSummary {
            provider: "claude".into(),
            title: title.into(),
            status: status.bits(),
            activity: None,
            origin: None,
            project: None,
            working_directories: Some(vec![dir.into()]),
            annotations: None,
            resource: format!("{SESSION_SCHEME}{id}"),
            created_at: modified_at.into(),
            modified_at: modified_at.into(),
            changes: None,
            meta: None,
        }
    }

    fn message(text: &str) -> Message {
        Message {
            text: text.into(),
            origin: MessageOrigin {
                kind: MessageKind::User,
            },
            attachments: None,
            model: None,
            agent: None,
            meta: None,
        }
    }

    fn markdown(content: &str) -> ResponsePart {
        ResponsePart::Markdown(MarkdownResponsePart {
            id: "p".into(),
            content: content.into(),
        })
    }

    fn chat() -> ChatState {
        ChatState {
            resource: "ahp-chat:/c".into(),
            title: String::new(),
            status: SessionStatus::Idle.bits(),
            activity: None,
            modified_at: "2026-09-15T12:00:00.000Z".into(),
            origin: None,
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

    #[test]
    fn renders_a_row_per_session() {
        let now: jiff::Timestamp = "2026-09-15T12:00:00Z".parse().unwrap();
        let sessions = [
            summary(
                "0123456789abcdef",
                SessionStatus::InputNeeded,
                "2026-09-15T11:59:30.000Z",
                "file:///home/me/dev/otto",
                "fix the dock",
            ),
            summary(
                "fedcba9876543210",
                SessionStatus::Idle,
                "2026-09-13T12:00:00.000Z",
                "file:///srv/data",
                "",
            ),
        ];
        let table = render(&sessions, now, Some(Path::new("/home/me")));
        let rows: Vec<&str> = table.lines().collect();
        assert_eq!(rows.len(), 3, "{table}");
        assert!(
            rows[1].starts_with("01234567 needs input  claude      30s  ~/dev/otto"),
            "{table}"
        );
        assert!(rows[1].ends_with("fix the dock"), "{table}");
        assert!(
            rows[2].starts_with("fedcba98 idle         claude       2d  /srv/data"),
            "{table}"
        );
        assert!(rows[2].ends_with("(untitled)"), "{table}");
    }

    #[test]
    fn says_so_when_there_are_no_sessions() {
        assert_eq!(render(&[], jiff::Timestamp::now(), None), "No sessions.\n");
    }

    #[test]
    fn finds_sessions_by_id_prefix_or_takes_the_newest() {
        let at = "2026-09-15T12:00:00.000Z";
        let sessions = [
            summary("abc123", SessionStatus::Idle, at, "file:///", "newest"),
            summary("abd456", SessionStatus::Idle, at, "file:///", "older"),
        ];
        assert_eq!(find_session(&sessions, None).unwrap().title, "newest");
        assert_eq!(find_session(&sessions, Some("abd")).unwrap().title, "older");
        assert_eq!(
            find_session(&sessions, Some("ahp-session:/abc123"))
                .unwrap()
                .title,
            "newest"
        );
        assert!(find_session(&sessions, Some("ab")).is_err(), "ambiguous");
        assert!(find_session(&sessions, Some("zzz")).is_err(), "no match");
        assert!(find_session(&[], None).is_err());
    }

    #[test]
    fn renders_a_chat_as_a_transcript() {
        let mut chat = chat();
        chat.turns.push(Turn {
            id: "t1".into(),
            started_at: None,
            duration: None,
            message: message("hello\nthere"),
            response_parts: vec![markdown("General "), markdown("Kenobi")],
            usage: None,
            state: TurnState::Complete,
        });
        chat.active_turn = Some(ActiveTurn {
            id: "t2".into(),
            started_at: "2026-09-15T12:00:00.000Z".into(),
            message: message("and now?"),
            response_parts: vec![markdown("Working on")],
            usage: None,
        });
        chat.queued_messages = Some(vec![PendingMessage {
            id: "q".into(),
            message: message("later"),
        }]);

        assert_eq!(
            render_chat(&chat),
            "> hello\n> there\n\nGeneral Kenobi\n\n> and now?\n\nWorking on[queued] later\n"
        );
        assert!(busy(&chat));
    }

    #[test]
    fn following_survives_the_gap_before_a_queued_turn_starts() {
        use ahp_types::actions::{ChatPendingMessageRemovedAction, ChatTurnCompleteAction};
        use ahp_types::state::PendingMessageKind;

        // Right after the queued message is removed and before its turn
        // starts, the chat has nothing queued and nothing running.
        let idle = chat();
        let removed = StateAction::ChatPendingMessageRemoved(ChatPendingMessageRemovedAction {
            kind: PendingMessageKind::Queued,
            id: "q".into(),
        });
        assert!(!done_after(&idle, &removed));

        let completed = StateAction::ChatTurnComplete(ChatTurnCompleteAction {
            turn_id: "t".into(),
            duration: 1,
            meta: None,
        });
        assert!(done_after(&idle, &completed));
    }
}
