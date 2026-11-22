//! herdr backend (plan 0016, prototype): each session is Claude Code's own
//! interface in a herdr terminal. herdr-core owns the process; this backend
//! only renders what herdr and the transcript say, and sends what Otto does
//! back to the terminal.
//!
//! - State comes from `hc events` for the session's terminal.
//! - Text comes from Claude's transcript, `~/.claude/projects/<cwd>/<id>.jsonl`.
//! - Questions come from the `ask` MCP tool, which waits on the terminal's
//!   `otto.ask` label; the answer goes back as `otto.answer`.
//! - Prompts are typed in with `hc send`, Stop is `hc interrupt`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use ahp_types::state::{AgentInfo, ChatInputResponseKind};
use elicitation_content::to_labels;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Backend, HistoryPart, HistoryTurn, InputAnswer, SessionCommand, SessionEvent, SessionSpec,
    TurnOutcome, agent_info,
};
use crate::elicitation;
use crate::skills;

/// The provider id Ask sees.
pub const PROVIDER: &str = "claude";

pub struct HerdrBackend {
    /// The herdr-app workspace (`herdr.space`) sessions open in, each in a
    /// tab of its own.
    space: Option<String>,
    /// The `ask` MCP server's command line.
    ask_mcp: Vec<String>,
    model: Option<String>,
    /// Claude's `--agent`, such as `otto:otto`.
    run_as: Option<String>,
}

impl HerdrBackend {
    /// Configured from the environment while this is a prototype:
    /// `OTTO_HERDR_SPACE`, `OTTO_HERDR_ASK_MCP` (a command
    /// line), `OTTO_HERDR_MODEL` and `OTTO_HERDR_AGENT`.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let space = var("OTTO_HERDR_SPACE");
        let ask_mcp = var("OTTO_HERDR_ASK_MCP")
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        Self {
            space,
            ask_mcp,
            model: var("OTTO_HERDR_MODEL"),
            run_as: var("OTTO_HERDR_AGENT"),
        }
    }

    fn argv(&self, session: &str, resume: bool) -> Vec<String> {
        let mut argv = vec!["claude".to_owned()];
        argv.extend([
            if resume { "--resume" } else { "--session-id" }.to_owned(),
            session.to_owned(),
        ]);
        if let Some(model) = &self.model {
            argv.extend(["--model".to_owned(), model.clone()]);
        }
        if let Some((command, args)) = self.ask_mcp.split_first() {
            let config = json!({ "mcpServers": { "otto": { "command": command, "args": args } } });
            argv.extend([
                "--mcp-config".to_owned(),
                config.to_string(),
                "--disallowedTools".to_owned(),
                "AskUserQuestion".to_owned(),
                "--allowedTools".to_owned(),
                "mcp__otto__ask".to_owned(),
                "--append-system-prompt".to_owned(),
                "To ask the person anything, use the mcp__otto__ask tool, never a plain text question."
                    .to_owned(),
            ]);
        }
        argv.extend(skills::claude_cli_args(
            &skills::discover(),
            self.run_as.as_deref(),
        ));
        argv
    }
}

impl Backend for HerdrBackend {
    fn agents(&self) -> Vec<AgentInfo> {
        vec![agent_info(
            PROVIDER,
            "Otto",
            "Claude Code in a herdr terminal",
        )]
    }

    fn start(
        &self,
        spec: SessionSpec,
        commands: mpsc::UnboundedReceiver<SessionCommand>,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) {
        let resume = spec.resume.clone();
        let session = resume
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let argv = self.argv(&session, resume.is_some());
        let space = self.space.clone();
        tokio::spawn(async move {
            let terminal = match find(&session).await {
                Some(terminal) => terminal,
                None => match run(&session, &spec.cwd, space, &argv).await {
                    Ok(terminal) => terminal,
                    Err(error) => {
                        let _ = events.send(SessionEvent::CreationFailed(error));
                        return;
                    }
                },
            };
            tracing::info!(%session, %terminal, "herdr session");
            let transcript = transcript_path(&spec.cwd, &session);
            // Before the session is ready, so a queued turn starts against a
            // chat that already holds what was said, here or in the terminal.
            if resume.is_some() {
                let turns = match tokio::fs::read_to_string(&transcript).await {
                    Ok(text) => history(&text),
                    Err(_) => Vec::new(),
                };
                tracing::info!(turns = turns.len(), "history read from the transcript");
                let _ = events.send(SessionEvent::HistoryLoaded { turns });
            }
            let _ = events.send(SessionEvent::Ready {
                agent_session: Some(session.clone()),
            });
            drive(terminal, transcript, commands, events).await;
        });
    }
}

/// `hc`, with this process's own hc identity removed: sends from inside an hc
/// terminal are wrapped in `<hc from=…>` tags, sends from outside arrive plain.
fn hc() -> Command {
    let mut command = Command::new("hc");
    command
        .env_remove("HC_TERMINAL_ID")
        .env_remove("HC_SCOPE")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn hc_json(args: &[&str]) -> Option<Value> {
    let output = hc().args(args).arg("--json").output().await.ok()?;
    serde_json::from_slice(&output.stdout).ok()
}

/// The live terminal already running `session`, as after an otto-agents
/// restart: herdr kept it.
async fn find(session: &str) -> Option<String> {
    let filter = format!("otto.session={session}");
    let list = hc_json(&["ls", "--label", &filter]).await?;
    list.get("terminals")?
        .as_array()?
        .iter()
        .find(|terminal| terminal.get("state").and_then(Value::as_str) == Some("live"))?
        .get("address")?
        .as_str()
        .map(str::to_owned)
}

async fn run(
    session: &str,
    cwd: &Path,
    space: Option<String>,
    argv: &[String],
) -> Result<String, String> {
    let mut command = hc();
    command.args([
        "run",
        "--bg",
        "--keep",
        "--no-parent",
        "--no-notify",
        "--json",
    ]);
    command.args([
        "--name",
        &format!("otto-{}", &session[..8.min(session.len())]),
    ]);
    command.arg("--cwd").arg(cwd);
    command.args([
        "--label",
        "otto=agent",
        "--label",
        &format!("otto.session={session}"),
    ]);
    // herdr-app opens a terminal whose tab id it has not seen in a new tab.
    if let Some(space) = space {
        let tab = format!("tb{}", uuid::Uuid::new_v4().simple());
        command.args(["--label", &format!("herdr.space={space}")]);
        command.args(["--label", &format!("herdr.tab={tab}")]);
    }
    command.arg("--").args(argv);
    let output = command
        .output()
        .await
        .map_err(|error| format!("could not run hc: {error}"))?;
    let started: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        String::from_utf8_lossy(&output.stderr).trim().to_owned()
            + &String::from_utf8_lossy(&output.stdout)
    })?;
    started
        .get("address")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| started.to_string())
}

/// Claude keeps a session's transcript under its folder's path with every
/// character that is not a letter or digit made a dash.
fn transcript_path(cwd: &Path, session: &str) -> PathBuf {
    let folder: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::xdg::home().map(|home| home.join(".claude")))
        .unwrap_or_default();
    base.join("projects")
        .join(folder)
        .join(format!("{session}.jsonl"))
}

/// What the terminal tells us, merged from herdr's events and the transcript.
enum Seen {
    State(String),
    Ask(Option<Value>),
    Text(String),
    /// The transcript says the agent finished its turn.
    Ended,
    /// A prompt in the transcript: Otto's own, or typed in the terminal.
    Prompt(String),
    Gone,
}

async fn drive(
    terminal: String,
    transcript: PathBuf,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    events: mpsc::UnboundedSender<SessionEvent>,
) {
    let (seen_tx, mut seen) = mpsc::unbounded_channel();
    let watch_events = tokio::spawn(follow_events(terminal.clone(), seen_tx.clone()));
    let watch_transcript = tokio::spawn(follow_transcript(transcript.clone(), seen_tx));

    // The turn Ask started, and whether herdr has seen the agent work on it.
    let mut turn: Option<String> = None;
    let mut worked = false;
    let mut asked: Option<String> = None;
    // A prompt typed into an agent still starting up is dropped: hold prompts
    // until herdr has seen it ready, or long enough has passed.
    let mut ready = false;
    let mut held: Vec<String> = Vec::new();
    let patience = tokio::time::sleep(Duration::from_secs(20));
    tokio::pin!(patience);
    // herdr says `done` a little before the transcript has the last words:
    // the transcript ends the turn, herdr's word only after a grace period.
    let mut fallback: Option<tokio::time::Instant> = None;
    // Prompts Otto typed in and has not yet seen in the transcript, and
    // whether the agent is answering one typed in the terminal instead. Those
    // turns have no turn in Ask, so the chat is rebuilt from the transcript.
    let mut sent: Vec<String> = Vec::new();
    let mut outside = false;
    let (answer_tx, mut answers) =
        mpsc::unbounded_channel::<(String, Option<InputAnswer>, Value)>();

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(SessionCommand::Prompt { turn_id, text, attachments, .. }) => {
                    let mut prompt = String::new();
                    for attachment in &attachments {
                        if let Some(path) = crate::uri::to_path(&attachment.uri) {
                            prompt.push_str(&format!("@{} ", path.display()));
                        }
                    }
                    prompt.push_str(&text);
                    sent.push(prompt.trim().to_owned());
                    turn = Some(turn_id);
                    worked = false;
                    if ready {
                        send(&terminal, &prompt).await;
                    } else {
                        held.push(prompt);
                    }
                }
                Some(SessionCommand::Cancel { .. }) => {
                    let _ = hc().args(["interrupt", &terminal]).output().await;
                }
                Some(SessionCommand::SetMode { .. }) => {}
                // The session is closed in Otto; herdr keeps the terminal.
                None => break,
            },
            () = async { tokio::time::sleep_until(fallback.unwrap_or_else(tokio::time::Instant::now)).await }, if fallback.is_some() => {
                fallback = None;
                if asked.is_none() {
                    end(&mut turn, &events);
                }
            }
            () = &mut patience, if !ready => {
                ready = true;
                for prompt in held.drain(..) {
                    send(&terminal, &prompt).await;
                }
            }
            Some(item) = seen.recv() => match item {
                Seen::State(state) => {
                    tracing::debug!(%terminal, %state, ready, worked, turn = ?turn, asked = ?asked, "herdr state");
                    if !ready && matches!(state.as_str(), "idle" | "done") {
                        ready = true;
                        for prompt in held.drain(..) {
                            send(&terminal, &prompt).await;
                        }
                        continue;
                    }
                    if state == "working" {
                        worked = true;
                    } else if worked && matches!(state.as_str(), "done" | "idle") && asked.is_none() {
                        fallback = Some(tokio::time::Instant::now() + Duration::from_secs(3));
                        worked = false;
                    }
                }
                Seen::Prompt(prompt) => {
                    match sent.iter().position(|ours| *ours == prompt.trim()) {
                        Some(index) => {
                            sent.remove(index);
                        }
                        None if turn.is_none() => {
                            outside = true;
                            rebuild(&transcript, &events).await;
                            let _ = events.send(SessionEvent::Active);
                        }
                        None => {}
                    }
                }
                Seen::Ended if outside && turn.is_none() => {
                    outside = false;
                    rebuild(&transcript, &events).await;
                    let _ = events.send(SessionEvent::Active);
                }
                Seen::Text(_) if outside && turn.is_none() => {
                    rebuild(&transcript, &events).await;
                }
                Seen::Ended => {
                    if asked.is_none() {
                        fallback = None;
                        worked = false;
                        end(&mut turn, &events);
                    }
                }
                Seen::Text(text) => {
                    if let Some(turn_id) = &turn {
                        let _ = events.send(SessionEvent::MessageChunk {
                            turn_id: turn_id.clone(),
                            text,
                        });
                    }
                }
                Seen::Ask(Some(request)) => {
                    let Some(id) = request.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        continue;
                    };
                    if asked.as_deref() == Some(&id) {
                        continue;
                    }
                    asked = Some(id.clone());
                    let questions = request.get("questions").cloned().unwrap_or(Value::Null);
                    let schema = schema(&questions);
                    let form = elicitation::form(&id, "", &schema);
                    let (reply, wait) = oneshot::channel();
                    let turn_id = turn.clone().unwrap_or_default();
                    let _ = events.send(SessionEvent::InputRequested {
                        turn_id,
                        request: form.request.clone(),
                        reply,
                    });
                    let answer_tx = answer_tx.clone();
                    tokio::spawn(async move {
                        let answer = wait.await.ok();
                        let labels = match &answer {
                            Some(InputAnswer { response: ChatInputResponseKind::Accept, answers }) => {
                                to_labels(&questions, &elicitation::content(&form, answers))
                            }
                            _ => Value::Null,
                        };
                        let _ = answer_tx.send((id, answer, labels));
                    });
                }
                Seen::Ask(None) => asked = None,
                Seen::Gone => {
                    if let Some(turn_id) = turn.take() {
                        let _ = events.send(SessionEvent::TurnEnded {
                            turn_id,
                            outcome: TurnOutcome::Failed("the terminal ended".into()),
                        });
                    }
                    break;
                }
            },
            Some((id, answer, labels)) = answers.recv() => {
                let answered = matches!(&answer, Some(InputAnswer { response: ChatInputResponseKind::Accept, .. }));
                let reply = if answered {
                    json!({ "id": id, "answers": labels })
                } else {
                    json!({ "id": id, "answers": {}, "declined": true })
                };
                let label = format!("otto.answer={reply}");
                let _ = hc().args(["label", &terminal, &label]).output().await;
            }
        }
    }
    watch_events.abort();
    watch_transcript.abort();
}

fn end(turn: &mut Option<String>, events: &mpsc::UnboundedSender<SessionEvent>) {
    if let Some(turn_id) = turn.take() {
        let _ = events.send(SessionEvent::TurnEnded {
            turn_id,
            outcome: TurnOutcome::Complete,
        });
    }
}

/// The chat rebuilt from the transcript, for turns Ask did not start.
async fn rebuild(transcript: &Path, events: &mpsc::UnboundedSender<SessionEvent>) {
    let text = tokio::fs::read_to_string(transcript)
        .await
        .unwrap_or_default();
    let _ = events.send(SessionEvent::HistoryLoaded {
        turns: history(&text),
    });
}

/// The prompt a transcript line holds, when it is one you wrote: not a tool's
/// result, a slash command or the harness's own note.
fn prompt_of(line: &str) -> Option<String> {
    let entry = serde_json::from_str::<Value>(line).ok()?;
    let flag = |name: &str| entry.get(name).and_then(Value::as_bool) == Some(true);
    if entry.get("type").and_then(Value::as_str) != Some("user")
        || flag("isMeta")
        || flag("isSidechain")
    {
        return None;
    }
    let text = match entry.pointer("/message/content")? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_owned())
}

/// Whether a transcript line is the assistant finishing its turn.
fn ends_turn(line: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|entry| {
        entry.get("type").and_then(Value::as_str) == Some("assistant")
            && entry
                .pointer("/message/stop_reason")
                .and_then(Value::as_str)
                == Some("end_turn")
    })
}

async fn send(terminal: &str, prompt: &str) {
    let _ = hc().args(["send", terminal, prompt]).output().await;
}

/// herdr's view of the terminal: its status and its `otto.ask` label.
async fn follow_events(terminal: String, seen: mpsc::UnboundedSender<Seen>) {
    let Ok(mut child) = hc()
        .args(["events", "--json", "--terminal", &terminal])
        .stdout(Stdio::piped())
        .spawn()
    else {
        let _ = seen.send(Seen::Gone);
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut state = String::new();
    let mut ask: Option<String> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // The first line is a snapshot of the terminal; later ones are
        // notifications carrying only the fact that changed. Each is a cue to
        // read the terminal whole, so no event type needs its own parsing.
        let items = match event.get("terminals").and_then(Value::as_array) {
            Some(terminals) => terminals.clone(),
            None => match hc_json(&["get", &terminal]).await {
                Some(item) => vec![item],
                None => continue,
            },
        };
        for item in &items {
            if item.get("address").and_then(Value::as_str) != Some(terminal.as_str()) {
                continue;
            }
            if item.get("state").and_then(Value::as_str) == Some("finished") {
                let _ = seen.send(Seen::Gone);
                return;
            }
            let now = item
                .pointer("/status/state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if now != state {
                state = now.clone();
                let _ = seen.send(Seen::State(now));
            }
            let label = item
                .pointer("/labels/otto.ask")
                .and_then(Value::as_str)
                .filter(|raw| !raw.is_empty())
                .map(str::to_owned);
            if label != ask {
                ask = label.clone();
                let request = label.and_then(|raw| serde_json::from_str(&raw).ok());
                let _ = seen.send(Seen::Ask(request));
            }
        }
    }
    let _ = seen.send(Seen::Gone);
}

/// The agent's text, as Claude writes it to the transcript. Only entries
/// written after the session is opened here: earlier ones are history.
/// Entries are told apart by their `uuid`, not their place in the file, as a
/// resumed Claude rewrites the file rather than only appending to it.
async fn follow_transcript(path: PathBuf, seen: mpsc::UnboundedSender<Seen>) {
    let read =
        |path: PathBuf| async move { tokio::fs::read_to_string(path).await.unwrap_or_default() };
    let mut known: HashSet<String> = entries(&read(path.clone()).await)
        .map(|(key, _)| key)
        .collect();
    let mut last_len = None;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(len) = tokio::fs::metadata(&path).await.map(|m| m.len()) else {
            continue;
        };
        if last_len == Some(len) {
            continue;
        }
        last_len = Some(len);
        let text = read(path.clone()).await;
        for (key, line) in entries(&text) {
            if !known.insert(key) {
                continue;
            }
            for text in assistant_text(line) {
                if seen.send(Seen::Text(text)).is_err() {
                    return;
                }
            }
            if let Some(prompt) = prompt_of(line) {
                if seen.send(Seen::Prompt(prompt)).is_err() {
                    return;
                }
            }
            if ends_turn(line) && seen.send(Seen::Ended).is_err() {
                return;
            }
        }
    }
}

/// The complete lines of a transcript, each with what identifies it: its
/// `uuid`, or the line itself for the few entries without one.
fn entries(text: &str) -> impl Iterator<Item = (String, &str)> {
    let complete = text.rfind('\n').map_or("", |end| &text[..end]);
    complete
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let key = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|entry| entry.get("uuid")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| line.to_owned());
            (key, line)
        })
}

/// The conversation in a transcript, as turns: each prompt you wrote, and
/// what the agent said and which tools it ran in answer.
fn history(transcript: &str) -> Vec<HistoryTurn> {
    let mut turns: Vec<HistoryTurn> = Vec::new();
    for line in transcript.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let flag = |name: &str| entry.get(name).and_then(Value::as_bool) == Some(true);
        if flag("isMeta") || flag("isSidechain") {
            continue;
        }
        let content = entry.pointer("/message/content");
        match entry.get("type").and_then(Value::as_str) {
            Some("user") => {
                let prompt = match content {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Array(blocks)) => {
                        // A tool's result comes back as a user entry: it
                        // settles the call, it is not a prompt.
                        for block in blocks {
                            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                                continue;
                            }
                            let id = block.get("tool_use_id").and_then(Value::as_str);
                            let failed =
                                block.get("is_error").and_then(Value::as_bool) == Some(true);
                            if let (Some(id), Some(turn)) = (id, turns.last_mut()) {
                                for part in &mut turn.parts {
                                    if let HistoryPart::ToolCall {
                                        tool_call_id,
                                        success,
                                        ..
                                    } = part
                                    {
                                        if tool_call_id == id {
                                            *success = !failed;
                                        }
                                    }
                                }
                            }
                        }
                        let text: Vec<&str> = blocks
                            .iter()
                            .filter(|block| {
                                block.get("type").and_then(Value::as_str) == Some("text")
                            })
                            .filter_map(|block| block.get("text").and_then(Value::as_str))
                            .collect();
                        (!text.is_empty()).then(|| text.join("\n"))
                    }
                    _ => None,
                };
                // Slash commands and the harness's own notes are not prompts.
                if let Some(prompt) = prompt.filter(|text| !text.trim_start().starts_with('<')) {
                    turns.push(HistoryTurn {
                        prompt,
                        ..HistoryTurn::default()
                    });
                }
            }
            Some("assistant") => {
                let Some(turn) = turns.last_mut() else {
                    continue;
                };
                for block in content.and_then(Value::as_array).into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                turn.parts.push(HistoryPart::Message(format!("{text}\n\n")));
                            }
                        }
                        Some("tool_use") => {
                            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            turn.parts.push(HistoryPart::ToolCall {
                                tool_call_id: id.to_owned(),
                                title: name.to_owned(),
                                success: true,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    turns
}

/// The text blocks of one transcript line, when it is the assistant's.
fn assistant_text(line: &str) -> Vec<String> {
    let Ok(entry) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    if entry.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    entry
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(|text| format!("{text}\n\n"))
        .collect()
}

/// The `ask` tool's questions as the elicitation schema claude-agent-acp sends
/// for AskUserQuestion, so the existing form mapping applies.
fn schema(questions: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    for (index, question) in questions.as_array().into_iter().flatten().enumerate() {
        let id = format!("question_{index}");
        let text = question
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = question
            .get("header")
            .and_then(Value::as_str)
            .unwrap_or(text);
        let options: Vec<Value> = question
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                let label = option.get("label")?.as_str()?;
                let mut choice = json!({ "const": label, "title": label });
                if let Some(description) = option.get("description").and_then(Value::as_str) {
                    choice["description"] = json!(description);
                }
                Some(choice)
            })
            .collect();
        let field = if question.get("multiSelect").and_then(Value::as_bool) == Some(true) {
            json!({ "type": "array", "title": title, "description": text, "items": { "anyOf": options } })
        } else {
            json!({ "type": "string", "title": title, "description": text, "oneOf": options })
        };
        properties.insert(id.clone(), field);
        properties.insert(
            format!("{id}_custom"),
            json!({
                "type": "string",
                "title": "Other",
                "_meta": { "_askUserQuestionCustomAnswer": { "questionId": id, "isCustomAnswer": true } },
            }),
        );
    }
    json!({ "type": "object", "properties": properties })
}

mod elicitation_content {
    use std::collections::BTreeMap;

    use agent_client_protocol::schema::v1::ElicitationContentValue;
    use serde_json::{Value, json};

    /// The form's field values back as `{question text: answer}`, what the
    /// `ask` tool returns to the agent.
    pub fn to_labels(
        questions: &Value,
        content: &BTreeMap<String, ElicitationContentValue>,
    ) -> Value {
        let mut answers = serde_json::Map::new();
        for (index, question) in questions.as_array().into_iter().flatten().enumerate() {
            let text = question
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let id = format!("question_{index}");
            let mut parts: Vec<String> = match content.get(&id) {
                Some(ElicitationContentValue::String(value)) => vec![value.clone()],
                Some(ElicitationContentValue::StringArray(values)) => values.clone(),
                _ => Vec::new(),
            };
            if let Some(ElicitationContentValue::String(other)) =
                content.get(&format!("{id}_custom"))
            {
                parts.push(other.clone());
            }
            answers.insert(text.to_owned(), json!(parts.join(", ")));
        }
        Value::Object(answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_live_under_the_folder_with_dashes() {
        let path = transcript_path(Path::new("/home/me/dev/otto/.claude/x"), "abc");
        assert!(path.ends_with("projects/-home-me-dev-otto--claude-x/abc.jsonl"));
    }

    #[test]
    fn history_is_prompts_and_what_answered_them() {
        let transcript = [
            r#"{"type":"user","message":{"content":"what time is it?"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"11:15"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"caveat"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"thanks"}]}}"#,
        ]
        .join("\n");
        let turns = history(&transcript);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].prompt, "what time is it?");
        assert_eq!(
            turns[0].parts,
            vec![
                HistoryPart::ToolCall {
                    tool_call_id: "t1".into(),
                    title: "Bash".into(),
                    success: false,
                },
                HistoryPart::Message("11:15\n\n".into()),
            ]
        );
        assert_eq!(turns[1].prompt, "thanks");
        assert!(turns[1].parts.is_empty());
    }

    #[test]
    fn prompts_are_what_you_wrote() {
        assert_eq!(
            prompt_of(r#"{"type":"user","message":{"content":" hi "}}"#).as_deref(),
            Some("hi")
        );
        assert_eq!(
            prompt_of(r#"{"type":"user","message":{"content":[{"type":"text","text":"yo"}]}}"#)
                .as_deref(),
            Some("yo")
        );
        assert_eq!(
            prompt_of(
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t"}]}}"#
            ),
            None
        );
        assert_eq!(
            prompt_of(r#"{"type":"user","isMeta":true,"message":{"content":"x"}}"#),
            None
        );
        assert_eq!(
            prompt_of(r#"{"type":"user","message":{"content":"<command-name>/x</command-name>"}}"#),
            None
        );
        assert_eq!(
            prompt_of(r#"{"type":"assistant","message":{"content":"x"}}"#),
            None
        );
    }

    #[test]
    fn the_transcript_says_when_a_turn_ends() {
        assert!(ends_turn(
            r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[]}}"#
        ));
        assert!(!ends_turn(
            r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[]}}"#
        ));
        assert!(!ends_turn(r#"{"type":"user","message":{"content":"hi"}}"#));
    }

    #[test]
    fn only_assistant_text_is_taken() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"},{"type":"tool_use","name":"x"}]}}"#;
        assert_eq!(assistant_text(line), vec!["Hi\n\n".to_owned()]);
        assert!(assistant_text(r#"{"type":"user","message":{"content":"hey"}}"#).is_empty());
    }

    #[test]
    fn ask_questions_become_a_form_and_answers_come_back_by_question() {
        let questions = json!([{
            "question": "Favourite colour?",
            "options": [{ "label": "Red" }, { "label": "Blue" }]
        }]);
        let form = elicitation::form("r1", "", &schema(&questions));
        let asked = form.request.questions.as_ref().unwrap();
        assert_eq!(asked.len(), 1);

        let mut content = std::collections::BTreeMap::new();
        content.insert(
            "question_0".to_owned(),
            agent_client_protocol::schema::v1::ElicitationContentValue::String("Blue".into()),
        );
        assert_eq!(
            to_labels(&questions, &content),
            json!({ "Favourite colour?": "Blue" })
        );
    }
}
