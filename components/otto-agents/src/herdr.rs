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

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use ahp_types::state::{AgentInfo, ChatInputResponseKind};
use elicitation_content::to_labels;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Backend, InputAnswer, SessionCommand, SessionEvent, SessionSpec, TurnOutcome, agent_info,
};
use crate::elicitation;
use crate::skills;

/// The provider id Ask sees.
pub const PROVIDER: &str = "claude";

pub struct HerdrBackend {
    /// Where new terminals open in herdr-app: `herdr.space` and `herdr.tab`.
    place: Option<(String, String)>,
    /// The `ask` MCP server's command line.
    ask_mcp: Vec<String>,
    model: Option<String>,
    /// Claude's `--agent`, such as `otto:otto`.
    run_as: Option<String>,
}

impl HerdrBackend {
    /// Configured from the environment while this is a prototype:
    /// `OTTO_HERDR_SPACE`/`OTTO_HERDR_TAB`, `OTTO_HERDR_ASK_MCP` (a command
    /// line), `OTTO_HERDR_MODEL` and `OTTO_HERDR_AGENT`.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let place = var("OTTO_HERDR_SPACE").zip(var("OTTO_HERDR_TAB"));
        let ask_mcp = var("OTTO_HERDR_ASK_MCP")
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        Self {
            place,
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
        let place = self.place.clone();
        tokio::spawn(async move {
            let terminal = match find(&session).await {
                Some(terminal) => terminal,
                None => match run(&session, &spec.cwd, place, &argv).await {
                    Ok(terminal) => terminal,
                    Err(error) => {
                        let _ = events.send(SessionEvent::CreationFailed(error));
                        return;
                    }
                },
            };
            tracing::info!(%session, %terminal, "herdr session");
            let _ = events.send(SessionEvent::Ready {
                agent_session: Some(session.clone()),
            });
            let transcript = transcript_path(&spec.cwd, &session);
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
    place: Option<(String, String)>,
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
    if let Some((space, tab)) = place {
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
    let watch_transcript = tokio::spawn(follow_transcript(transcript, seen_tx));

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
            () = &mut patience, if !ready => {
                ready = true;
                for prompt in held.drain(..) {
                    send(&terminal, &prompt).await;
                }
            }
            Some(item) = seen.recv() => match item {
                Seen::State(state) => {
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
                        if let Some(turn_id) = turn.take() {
                            let _ = events.send(SessionEvent::TurnEnded {
                                turn_id,
                                outcome: TurnOutcome::Complete,
                            });
                        }
                        worked = false;
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
        for item in event
            .get("terminals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
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

/// The agent's text, as Claude appends it to the transcript. Only what is
/// written after the session is opened here: earlier turns are history.
async fn follow_transcript(path: PathBuf, seen: mpsc::UnboundedSender<Seen>) {
    let mut offset = tokio::fs::metadata(&path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let mut pending = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(mut file) = tokio::fs::File::open(&path).await else {
            continue;
        };
        let Ok(len) = file.metadata().await.map(|m| m.len()) else {
            continue;
        };
        if len <= offset {
            continue;
        }
        if file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
            continue;
        }
        let mut added = String::new();
        if file.read_to_string(&mut added).await.is_err() {
            continue;
        }
        offset = len;
        pending.push_str(&added);
        while let Some(end) = pending.find('\n') {
            let line: String = pending.drain(..=end).collect();
            for text in assistant_text(&line) {
                if seen.send(Seen::Text(text)).is_err() {
                    return;
                }
            }
        }
    }
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
