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

use ahp_types::state::{AgentInfo, ChatInputResponseKind, SessionStatus};
use elicitation_content::to_labels;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{
    Backend, FoundSession, HistoryPart, HistoryTurn, InputAnswer, SessionChange, SessionCommand,
    SessionEvent, SessionSpec, TurnOutcome, agent_info,
};
use crate::elicitation;
use crate::skills;

/// The provider id Ask sees.
pub const PROVIDER: &str = "claude";

/// The agent program a herdr session runs, in its own interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Harness {
    Claude,
    Pi,
}

impl Harness {
    /// From an agent's `command` in `agents.toml`: `pi` (or `pi-acp`, the
    /// same agent over ACP) is pi, anything else Claude Code.
    fn of(command: &str) -> Self {
        let name = Path::new(command)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == "pi" || name.starts_with("pi-") {
            Self::Pi
        } else {
            Self::Claude
        }
    }
}

/// An agent Ask offers: Claude Code or pi, opening its sessions in a herdr
/// space.
#[derive(Debug, Clone)]
struct HerdrAgent {
    harness: Harness,
    id: String,
    name: String,
    description: String,
    /// The herdr-app space (`herdr.space`) its sessions open in, each in a
    /// tab of its own: a space's name or id.
    space: Option<String>,
    model: Option<String>,
    /// The plugin agent its sessions run as: its prompt, tools and skills.
    profile: Option<skills::AgentFile>,
    folder: Option<PathBuf>,
    colour: Option<&'static str>,
}

pub struct HerdrBackend {
    /// The first is the one a session of an unknown provider runs as.
    agents: Vec<HerdrAgent>,
    /// The `ask` MCP server's command line.
    ask_mcp: Vec<String>,
    /// The `PermissionRequest` hook's command line.
    permission_hook: Option<String>,
    /// Claude's `--permission-mode`, such as `manual`; unset keeps the
    /// person's own default.
    permission_mode: Option<String>,
    /// The sessions this service started: Otto has them already, so the
    /// watcher does not take them up again.
    started: Started,
}

/// Agent session ids, shared between [`HerdrBackend::start`] and the watcher.
type Started = std::sync::Arc<std::sync::Mutex<HashSet<String>>>;

impl HerdrBackend {
    /// The agents are `agents.toml`'s that name a herdr `space`, after the
    /// environment's own (while this is a prototype): `OTTO_HERDR_SPACE`,
    /// `OTTO_HERDR_MODEL` and `OTTO_HERDR_AGENT`, the plugin agent it runs
    /// as (`otto` unless set, `none` for plain Claude). An agent of the file with
    /// that one's id, `claude`, takes its place. Also from the environment:
    /// `OTTO_HERDR_ASK_MCP` and `OTTO_HERDR_PERMISSION_HOOK` (command lines),
    /// and `OTTO_HERDR_PERMISSION_MODE`.
    pub fn new(config: &crate::config::Config) -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let plugins = skills::discover();
        let profile = |name: &str| {
            let name = name.rsplit(':').next().unwrap_or(name);
            let found = skills::agent_file(&plugins, name).map(|(_, agent)| agent.clone());
            if found.is_none() && name != "none" {
                tracing::warn!(agent = name, "no plugin agent by that name");
            }
            found
        };
        let mut agents = vec![HerdrAgent {
            harness: Harness::Claude,
            id: PROVIDER.to_owned(),
            name: "Otto".to_owned(),
            description: String::new(),
            space: var("OTTO_HERDR_SPACE"),
            model: var("OTTO_HERDR_MODEL"),
            profile: profile(&var("OTTO_HERDR_AGENT").unwrap_or_else(|| "otto".to_owned())),
            folder: None,
            colour: None,
        }];
        for agent in config.agents.iter().filter(|agent| agent.space.is_some()) {
            let herdr = HerdrAgent {
                harness: Harness::of(&agent.command),
                id: agent.id.clone(),
                name: agent.name.clone(),
                description: agent.description.clone(),
                space: agent.space.clone(),
                model: agent.model.clone(),
                profile: agent.agent.as_deref().and_then(profile),
                folder: agent.folder.clone(),
                colour: agent.colour.map(|colour| colour.name()),
            };
            tracing::info!(id = %herdr.id, space = ?herdr.space, "herdr agent configured");
            match agents.iter_mut().find(|known| known.id == herdr.id) {
                Some(known) => *known = herdr,
                None => agents.push(herdr),
            }
        }
        // Without a description of its own, an agent is described by the
        // model it runs: agents of one name differ in model and folder.
        for agent in &mut agents {
            if agent.description.is_empty() {
                agent.description = model_name(agent.model.as_deref());
            }
        }
        let ask_mcp = var("OTTO_HERDR_ASK_MCP")
            .map(|line| line.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        Self {
            agents,
            ask_mcp,
            permission_hook: var("OTTO_HERDR_PERMISSION_HOOK"),
            permission_mode: var("OTTO_HERDR_PERMISSION_MODE"),
            started: Started::default(),
        }
    }

    /// The agent a Claude Code session started by hand is taken up as: plain
    /// Claude, the one whose folder holds the session's, else the first.
    fn adopt_as(&self) -> Vec<(String, Option<PathBuf>)> {
        self.agents
            .iter()
            .filter(|agent| agent.harness == Harness::Claude && agent.profile.is_none())
            .map(|agent| (agent.id.clone(), agent.folder.clone()))
            .collect()
    }

    fn agent(&self, provider: &str) -> &HerdrAgent {
        self.agents
            .iter()
            .find(|agent| agent.id == provider)
            .unwrap_or(&self.agents[0])
    }

    fn argv(&self, agent: &HerdrAgent, session: &str, resume: bool) -> Vec<String> {
        if agent.harness == Harness::Pi {
            return pi_argv(agent, session);
        }
        let mut argv = vec!["claude".to_owned()];
        argv.extend([
            if resume { "--resume" } else { "--session-id" }.to_owned(),
            session.to_owned(),
        ]);
        if let Some(mode) = &self.permission_mode {
            argv.extend(["--permission-mode".to_owned(), mode.clone()]);
        }
        if let Some(model) = &agent.model {
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
        if let Some(hook) = &self.permission_hook {
            // Long enough for a person to answer: the hook gives up first,
            // and Claude then asks in the terminal.
            let settings = json!({ "hooks": { "PermissionRequest": [
                { "matcher": "*", "hooks": [{ "type": "command", "command": hook, "timeout": 600 }] }
            ] } });
            argv.extend(["--settings".to_owned(), settings.to_string()]);
        }
        argv.extend(skills::claude_cli_args(&skills::discover(), None));
        if let Some(profile) = &agent.profile {
            let definition = agent_definition(profile, !self.ask_mcp.is_empty());
            argv.extend([
                "--agents".to_owned(),
                json!({ profile.name.clone(): definition }).to_string(),
                "--agent".to_owned(),
                profile.name.clone(),
            ]);
        }
        argv
    }
}

impl Backend for HerdrBackend {
    fn agents(&self) -> Vec<AgentInfo> {
        self.agents
            .iter()
            .map(|agent| agent_info(&agent.id, &agent.name, &agent.description))
            .collect()
    }

    fn colour(&self, provider: &str) -> Option<&'static str> {
        self.agent(provider).colour
    }

    fn folder(&self, provider: &str) -> Option<&Path> {
        self.agent(provider).folder.as_deref()
    }

    fn reports_running(&self) -> bool {
        true
    }

    fn live(&self, _provider: &str, agent_session: &str) -> bool {
        terminals_blocking(agent_session).is_some_and(|terminals| {
            terminals
                .iter()
                .any(|terminal| terminal.get("state").and_then(Value::as_str) == Some("live"))
        })
    }

    /// `otto-agents herdr-show`: herdr-app shows the session's tab.
    fn terminal(
        &self,
        _provider: &str,
        agent_session: &str,
        _cwd: &Path,
        _written: bool,
    ) -> Option<Vec<String>> {
        let exe = std::env::current_exe().ok()?;
        Some(vec![
            exe.to_string_lossy().into_owned(),
            "herdr-show".to_owned(),
            agent_session.to_owned(),
        ])
    }

    fn enter(
        &self,
        provider: &str,
        agent_session: &str,
        cwd: &Path,
        written: bool,
    ) -> Option<Vec<String>> {
        self.terminal(provider, agent_session, cwd, written)
    }

    fn window(&self, _provider: &str) -> Option<String> {
        Some("herdr-app".to_owned())
    }

    /// herdr keeps a session as long as it keeps one of its terminals,
    /// running or finished.
    fn exists(&self, _provider: &str, agent_session: &str) -> Option<bool> {
        terminals_blocking(agent_session).map(|terminals| !terminals.is_empty())
    }

    /// Ends the session's terminals and removes them from herdr.
    fn forget(&self, _provider: &str, agent_session: &str) {
        let session = agent_session.to_owned();
        tokio::spawn(async move {
            for terminal in terminals(&session).await.unwrap_or_default() {
                let Some(address) = terminal.get("address").and_then(Value::as_str) else {
                    continue;
                };
                if terminal.get("state").and_then(Value::as_str) == Some("live") {
                    let _ = hc().args(["kill", address]).output().await;
                }
                remove(address).await;
            }
        });
    }

    /// Follows every herdr terminal: each Otto session in one, running or
    /// finished, and each Claude Code started in herdr by hand, is a session;
    /// when the last terminal of one is removed, the session is gone.
    fn watch_sessions(&self, changes: mpsc::UnboundedSender<SessionChange>) {
        let adopt_as = self.adopt_as();
        let started = self.started.clone();
        tokio::spawn(async move {
            while !changes.is_closed() {
                follow_sessions(&changes, &adopt_as, &started).await;
                // The server restarted, or hc is not there: try again shortly.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
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
        self.started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.clone());
        let agent = self.agent(&spec.provider);
        let argv = self.argv(agent, &session, resume.is_some());
        let space = agent.space.clone();
        let harness = agent.harness;
        let labels = vec![
            format!("otto.agent={}", agent.id),
            format!(
                "otto.harness={}",
                match harness {
                    Harness::Claude => "claude",
                    Harness::Pi => "pi",
                }
            ),
        ];
        tokio::spawn(async move {
            let terminal = match find(&session).await {
                Some(terminal) => terminal,
                None => {
                    let placement = place(space, &spec.cwd).await;
                    let started = run(&session, &labels, &spec.cwd, placement.space, &argv).await;
                    if let (Ok(_), Some(tab)) = (&started, &placement.first_tab) {
                        close_tab(tab).await;
                    }
                    match started {
                        Ok(terminal) => {
                            // One terminal per session: the finished ones of
                            // earlier runs go once the new one is there.
                            prune(&session, &terminal).await;
                            terminal
                        }
                        Err(error) => {
                            let _ = events.send(SessionEvent::CreationFailed(error));
                            return;
                        }
                    }
                }
            };
            tracing::info!(%session, %terminal, "herdr session");
            let transcript = match harness {
                Harness::Claude => Transcript::Claude(transcript_path(&spec.cwd, &session)),
                Harness::Pi => Transcript::Pi(session.clone()),
            };
            // Before the session is ready, so a queued turn starts against a
            // chat that already holds what was said, here or in the terminal.
            if resume.is_some() {
                let turns = history(&transcript.read().await);
                tracing::info!(turns = turns.len(), "history read from the transcript");
                let _ = events.send(SessionEvent::HistoryLoaded { turns });
            }
            let _ = events.send(SessionEvent::Ready {
                agent_session: Some(session.clone()),
            });
            let _ = events.send(SessionEvent::Running(true));
            drive(terminal, transcript, spec.cwd.clone(), commands, events).await;
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

/// A model as a person reads it: `opus` as "Opus"; Claude's own default
/// when none is set.
fn model_name(model: Option<&str>) -> String {
    let Some(model) = model.filter(|model| !model.is_empty()) else {
        return "Claude Code's default model".to_owned();
    };
    let mut chars = model.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A plugin agent as Claude's `--agents` takes one. The plugin's own copy
/// would do, but for its tools: with the `ask` server, its questions go
/// through `mcp__otto__ask`, which an agent limited to its own list of tools
/// could not call.
fn agent_definition(profile: &skills::AgentFile, ask: bool) -> Value {
    let mut definition = json!({
        "description": profile.description,
        "prompt": profile.body,
    });
    if !profile.tools.is_empty() {
        let mut tools: Vec<&str> = profile.tools.iter().map(String::as_str).collect();
        if ask {
            tools.retain(|tool| *tool != "AskUserQuestion");
            tools.push("mcp__otto__ask");
        }
        definition["tools"] = json!(tools);
    }
    if !profile.skills.is_empty() {
        definition["skills"] = json!(profile.skills);
    }
    if let Some(model) = &profile.model {
        definition["model"] = json!(model);
    }
    definition
}

/// Where a new terminal goes in herdr-app.
#[derive(Debug, Default)]
struct Placement {
    /// The space's id; `None` only when herdr-app has no spaces to name.
    space: Option<String>,
    /// The shell tab herdr-app opens in a space it has just made, closed once
    /// the session's own tab is there.
    first_tab: Option<String>,
}

/// The herdr-app space named `space`, by its id, its name, or for a space
/// without one, the name of its folder, as herdr-app shows it. The spaces are
/// in herdr-app's layout, a document herdr-core keeps. A name no space has
/// is made, starting in `folder`. With no space set, or one gone by its id,
/// the first space.
async fn place(space: Option<String>, folder: &Path) -> Placement {
    // One session at a time, so two sessions of one agent make one space.
    static PLACING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _placing = PLACING.lock().await;
    let wanted = space.unwrap_or_default();
    let layout = doc("herdr-app").await;
    let spaces = layout
        .as_ref()
        .and_then(|layout| layout.pointer("/value/spaces"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(space) = find_space(&spaces, &wanted) {
        return Placement {
            space: Some(space),
            first_tab: None,
        };
    }
    if !wanted.is_empty() && !is_space_id(&wanted) {
        if let Some(made) = new_space(&wanted, folder).await {
            return made;
        }
    }
    // A terminal in no space has no tab to be seen in: the first space
    // takes it.
    tracing::warn!(space = %wanted, "no herdr space by that name; using the first");
    Placement {
        space: spaces
            .first()
            .and_then(|space| space.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        first_tab: None,
    }
}

/// Whether `space` is a space's id, such as `sp5321…`, rather than a name.
fn is_space_id(space: &str) -> bool {
    space.len() > 20
        && space
            .strip_prefix("sp")
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Makes a space called `name`, without showing it. herdr-app makes spaces,
/// so it has to be running.
async fn new_space(name: &str, folder: &Path) -> Option<Placement> {
    let output = Command::new("herdr-app")
        .args([
            "do",
            "space.new",
            "--name",
            name,
            "--focus",
            "false",
            "--folder",
        ])
        .arg(folder)
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let made: Value = serde_json::from_slice(&output.stdout).ok()?;
    let text = |key: &str| made.get(key).and_then(Value::as_str).map(str::to_owned);
    tracing::info!(space = %name, "made a herdr space");
    Some(Placement {
        space: Some(text("space")?),
        first_tab: text("tab"),
    })
}

/// Closes the shell tab a new space opened with.
async fn close_tab(tab: &str) {
    let _ = Command::new("herdr-app")
        .args(["do", "tab.close", "--id", tab, "--confirmed", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

fn find_space(spaces: &[Value], wanted: &str) -> Option<String> {
    let text = |space: &Value, key: &str| {
        space
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let shown = |space: &Value| match text(space, "name") {
        name if !name.is_empty() => name,
        _ => Path::new(&text(space, "folder"))
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    let by = |matches: &dyn Fn(&Value) -> bool| {
        spaces
            .iter()
            .find(|space| matches(space))
            .map(|space| text(space, "id"))
    };
    by(&|space| text(space, "id") == wanted)
        .or_else(|| by(&|space| shown(space) == wanted))
        .or_else(|| by(&|space| shown(space).eq_ignore_ascii_case(wanted)))
}

/// A document herdr-core keeps, read through its API: `hc connect` takes
/// JSON-RPC on stdin.
async fn doc(name: &str) -> Option<Value> {
    use tokio::io::AsyncWriteExt;
    let mut child = hc()
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let request =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "doc.get", "params": { "name": name } });
    let mut stdin = child.stdin.take()?;
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .ok()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply.get("id") == Some(&json!(1)) {
                return reply.get("result").cloned();
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    drop(stdin);
    reply
}

/// Shows `session`'s running terminal in herdr-app, in its space and tab.
pub async fn show(session: &str) -> anyhow::Result<()> {
    let Some(terminal) = find(session).await else {
        anyhow::bail!("no running herdr terminal has the session {session}");
    };
    // With its window closed, herdr-app is opened first: it takes commands
    // once its window is up.
    let running = || async {
        Command::new("herdr-app")
            .arg("status")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success())
    };
    if !running().await {
        use std::os::unix::process::CommandExt;
        std::process::Command::new("herdr-app")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while !running().await {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "herdr-app did not open"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    // A terminal started in no space, or whose tab was closed, has no pane
    // to show: it gets a tab of its own. One with a pane keeps it.
    let status = Command::new("herdr-app")
        .args([
            "do",
            "terminal.keep",
            "--terminal",
            &terminal,
            "--place",
            "tab",
        ])
        .stdout(Stdio::null())
        .status()
        .await?;
    anyhow::ensure!(
        status.success(),
        "herdr-app could not give {terminal} a tab"
    );
    let status = Command::new("herdr-app")
        .args(["do", "agent.focus", "--terminal", &terminal])
        .stdout(Stdio::null())
        .status()
        .await?;
    anyhow::ensure!(status.success(), "herdr-app could not show {terminal}");
    Ok(())
}

/// Every terminal herdr keeps for `session`, running or finished; `None`
/// when hc cannot be asked.
async fn terminals(session: &str) -> Option<Vec<Value>> {
    let filter = format!("otto.session={session}");
    let list = hc_json(&["ls", "--all", "--label", &filter]).await?;
    list.get("terminals")?.as_array().cloned()
}

/// [`terminals`], for the backend's synchronous questions.
fn terminals_blocking(session: &str) -> Option<Vec<Value>> {
    let filter = format!("otto.session={session}");
    let output = std::process::Command::new("hc")
        .env_remove("HC_TERMINAL_ID")
        .env_remove("HC_SCOPE")
        .args(["ls", "--all", "--json", "--label", &filter])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let list: Value = serde_json::from_slice(&output.stdout).ok()?;
    list.get("terminals")?.as_array().cloned()
}

/// Removes a terminal, waiting a moment for one just killed to finish.
async fn remove(address: &str) {
    for _ in 0..20 {
        let finished = hc_json(&["get", address])
            .await
            .is_none_or(|item| item.get("state").and_then(Value::as_str) != Some("live"));
        if finished {
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let _ = hc().args(["rm", address]).output().await;
}

/// Removes `session`'s finished terminals other than `keep`.
async fn prune(session: &str, keep: &str) {
    for terminal in terminals(session).await.unwrap_or_default() {
        let address = terminal.get("address").and_then(Value::as_str);
        let finished = terminal.get("state").and_then(Value::as_str) == Some("finished");
        if let Some(address) = address.filter(|address| finished && *address != keep) {
            let _ = hc().args(["rm", address]).output().await;
        }
    }
}

/// One run of `hc events` over every terminal, reporting the sessions in
/// them as they are found and removed. Returns when the stream ends.
async fn follow_sessions(
    changes: &mpsc::UnboundedSender<SessionChange>,
    adopt_as: &[(String, Option<PathBuf>)],
    started: &Started,
) {
    let Ok(mut child) = hc()
        .args(["events", "--json"])
        .stdout(Stdio::piped())
        .spawn()
    else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut lines = BufReader::new(stdout).lines();
    // Each terminal's session, from the snapshot and the terminals seen since.
    let mut sessions: std::collections::HashMap<String, String> = Default::default();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(snapshot) = event.get("terminals").and_then(Value::as_array) {
            for item in snapshot {
                consider(item, &mut sessions, changes, adopt_as, started).await;
            }
            continue;
        }
        let Some(params) = event.get("params") else {
            continue;
        };
        let Some(address) = params.get("terminal").and_then(Value::as_str) else {
            continue;
        };
        match params.get("type").and_then(Value::as_str) {
            Some("removed") => {
                let Some(session) = sessions.remove(address) else {
                    continue;
                };
                if !sessions.values().any(|other| *other == session) {
                    let _ = changes.send(SessionChange::Removed(session));
                }
            }
            // A terminal new, with another program in front, or reporting
            // an agent's status: it may hold a session now. Claude notes its
            // session a moment after it starts, so its status is the cue
            // that finds it.
            Some("created" | "foreground" | "labels" | "status")
                if !sessions.contains_key(address) =>
            {
                if let Some(item) = hc_json(&["get", address]).await {
                    consider(&item, &mut sessions, changes, adopt_as, started).await;
                }
            }
            _ => {}
        }
    }
}

/// Reports the session a terminal holds: an Otto session by its label, or a
/// Claude Code started by hand, which is labelled as Otto's from then on.
async fn consider(
    item: &Value,
    sessions: &mut std::collections::HashMap<String, String>,
    changes: &mpsc::UnboundedSender<SessionChange>,
    adopt_as: &[(String, Option<PathBuf>)],
    started: &Started,
) {
    let Some(address) = item.get("address").and_then(Value::as_str) else {
        return;
    };
    let text = |pointer: &str| item.pointer(pointer).and_then(Value::as_str);
    let running = text("/state") == Some("live");
    let cwd = PathBuf::from(text("/cwd").unwrap_or_default());
    let (session, provider, cwd) = match text("/labels/otto.session") {
        Some(session) => {
            let provider = text("/labels/otto.agent").unwrap_or(PROVIDER).to_owned();
            (session.to_owned(), provider, cwd)
        }
        None if running => {
            let Some((session, cwd)) = claude_in(item).await else {
                return;
            };
            let provider = adopted_agent(adopt_as, &cwd);
            let labels = [
                "otto=agent".to_owned(),
                format!("otto.session={session}"),
                format!("otto.agent={provider}"),
            ];
            let _ = hc().arg("label").arg(address).args(&labels).output().await;
            tracing::info!(%address, %session, "a Claude Code started in herdr");
            (session, provider, cwd)
        }
        None => return,
    };
    sessions.insert(address.to_owned(), session.clone());
    let ours = started
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&session);
    if ours {
        return;
    }
    let transcript = match text("/labels/otto.harness") {
        Some("pi") => Transcript::Pi(session.clone()),
        _ if provider_is_pi(item) => Transcript::Pi(session.clone()),
        _ => Transcript::Claude(transcript_path(&cwd, &session)),
    };
    let title = history(&transcript.read().await)
        .into_iter()
        .next()
        .map(|turn| turn.prompt);
    let _ = changes.send(SessionChange::Found(FoundSession {
        provider,
        agent_session: session,
        cwd,
        title,
        running,
    }));
}

/// Whether the terminal runs pi, as herdr names its agent or its command.
fn provider_is_pi(item: &Value) -> bool {
    item.get("agent").and_then(Value::as_str) == Some("pi")
        || item
            .pointer("/argv/0")
            .and_then(Value::as_str)
            .is_some_and(|program| Harness::of(program) == Harness::Pi)
}

/// The session of the Claude Code in front in a terminal, and its folder:
/// Claude notes both in `~/.claude/sessions/<pid>.json`.
async fn claude_in(item: &Value) -> Option<(String, PathBuf)> {
    let foreground = item.get("foreground")?;
    if foreground.get("name").and_then(Value::as_str) != Some("claude") {
        return None;
    }
    let pid = foreground.get("pid")?.as_u64()?;
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::xdg::home().map(|home| home.join(".claude")))?;
    let note = tokio::fs::read_to_string(base.join(format!("sessions/{pid}.json")))
        .await
        .ok()?;
    let note: Value = serde_json::from_str(&note).ok()?;
    let session = note.get("sessionId")?.as_str()?.to_owned();
    let cwd = note
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| foreground.get("cwd")?.as_str().map(PathBuf::from))?;
    Some((session, cwd))
}

/// The agent a session in `cwd` started by hand is taken up as: the one whose
/// folder holds `cwd` most closely, else the first, else the default.
fn adopted_agent(adopt_as: &[(String, Option<PathBuf>)], cwd: &Path) -> String {
    adopt_as
        .iter()
        .filter_map(|(id, folder)| {
            let folder = folder.as_deref()?;
            cwd.starts_with(folder)
                .then(|| (folder.components().count(), id))
        })
        .max()
        .map(|(_, id)| id.clone())
        .or_else(|| adopt_as.first().map(|(id, _)| id.clone()))
        .unwrap_or_else(|| PROVIDER.to_owned())
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
    labels: &[String],
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
    for label in labels {
        command.args(["--label", label]);
    }
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

/// Where a session's transcript is, by harness.
#[derive(Debug, Clone)]
enum Transcript {
    /// Claude's file, which is known from the start.
    Claude(PathBuf),
    /// pi's session id: its file, named for the time it was made, is found
    /// once pi has written it.
    Pi(String),
}

impl Transcript {
    async fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Claude(path) => Some(path.clone()),
            Self::Pi(session) => pi_transcript(session).await,
        }
    }

    /// The transcript's lines, in Claude's shape whatever the harness: what
    /// the rest of the backend reads.
    async fn read(&self) -> String {
        let Some(path) = self.path().await else {
            return String::new();
        };
        let text = tokio::fs::read_to_string(path).await.unwrap_or_default();
        match self {
            Self::Claude(_) => text,
            Self::Pi(_) => pi_as_claude(&text),
        }
    }
}

/// pi in a herdr terminal: the session by its id, made when it is new, with
/// the agent's model and the profile's prompt.
fn pi_argv(agent: &HerdrAgent, session: &str) -> Vec<String> {
    let mut argv = vec![
        "pi".to_owned(),
        "--session-id".to_owned(),
        session.to_owned(),
    ];
    if let Some(model) = &agent.model {
        argv.extend(["--model".to_owned(), model.clone()]);
    }
    if let Some(profile) = &agent.profile {
        argv.extend(["--append-system-prompt".to_owned(), profile.body.clone()]);
    }
    argv
}

/// pi keeps each session as `<time>_<id>.jsonl` in a folder per working
/// folder, under `~/.pi/agent/sessions`.
async fn pi_transcript(session: &str) -> Option<PathBuf> {
    let root = crate::xdg::home()?.join(".pi/agent/sessions");
    let suffix = format!("_{session}.jsonl");
    let mut folders = tokio::fs::read_dir(root).await.ok()?;
    while let Ok(Some(folder)) = folders.next_entry().await {
        let Ok(mut files) = tokio::fs::read_dir(folder.path()).await else {
            continue;
        };
        while let Ok(Some(file)) = files.next_entry().await {
            if file.file_name().to_string_lossy().ends_with(&suffix) {
                return Some(file.path());
            }
        }
    }
    None
}

/// A pi transcript as Claude's: each message a `user` or `assistant` entry
/// keyed by `uuid`, a tool's result a `tool_result` block, a tool call a
/// `tool_use` block, and the end of a turn `end_turn`. Lines that are not
/// messages go. An unfinished last line stays unfinished.
fn pi_as_claude(text: &str) -> String {
    let complete = text.rfind('\n').map_or("", |end| &text[..end]);
    let mut out = String::new();
    for line in complete.lines() {
        if let Some(entry) = pi_entry(line) {
            out.push_str(&entry.to_string());
            out.push('\n');
        }
    }
    out
}

fn pi_entry(line: &str) -> Option<Value> {
    let entry: Value = serde_json::from_str(line).ok()?;
    if entry.get("type")?.as_str()? != "message" {
        return None;
    }
    let id = entry.get("id")?.as_str()?;
    let message = entry.get("message")?;
    let blocks = message
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(match message.get("role")?.as_str()? {
        "user" => json!({ "type": "user", "uuid": id, "message": { "content": blocks } }),
        "toolResult" => json!({ "type": "user", "uuid": id, "message": { "content": [{
            "type": "tool_result",
            "tool_use_id": message.get("toolCallId"),
            "is_error": message.get("isError"),
        }] } }),
        "assistant" => {
            let mut content: Vec<Value> = blocks
                .iter()
                .map(|block| match block.get("type").and_then(Value::as_str) {
                    Some("toolCall") => json!({
                        "type": "tool_use",
                        "id": block.get("id"),
                        "name": block.get("name"),
                        "input": block.get("arguments"),
                    }),
                    _ => block.clone(),
                })
                .collect();
            // A failed request ends the turn with what went wrong.
            if let Some(error) = message.get("errorMessage").and_then(Value::as_str) {
                content.push(json!({ "type": "text", "text": error }));
            }
            let stop = match message.get("stopReason").and_then(Value::as_str) {
                Some("toolUse") | None => "tool_use",
                Some(_) => "end_turn",
            };
            json!({ "type": "assistant", "uuid": id, "message": { "content": content, "stop_reason": stop } })
        }
        _ => return None,
    })
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
    /// The `PermissionRequest` hook's request, or `None` once it is settled.
    Permission(Option<Value>),
    Text(String),
    /// The transcript says the agent finished its turn.
    Ended,
    /// A prompt in the transcript: Otto's own, or typed in the terminal.
    Prompt(String),
    Gone,
}

async fn drive(
    terminal: String,
    transcript: Transcript,
    cwd: PathBuf,
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
    let mut permitting: Option<String> = None;
    let (decided_tx, mut decided) = mpsc::unbounded_channel::<(String, bool)>();
    // A prompt typed into an agent still starting up is dropped: hold prompts
    // until herdr has seen it ready, or long enough has passed.
    let mut ready = false;
    let mut held: Vec<String> = Vec::new();
    // herdr reports no state for pi: there, a short wait for it to start.
    let patience = tokio::time::sleep(match transcript {
        Transcript::Claude(_) => Duration::from_secs(20),
        Transcript::Pi(_) => Duration::from_secs(3),
    });
    tokio::pin!(patience);
    // herdr says `done` a little before the transcript has the last words:
    // the transcript ends the turn, herdr's word only after a grace period.
    let mut fallback: Option<tokio::time::Instant> = None;
    // Prompts Otto typed in and has not yet seen in the transcript: any
    // other prompt there was typed in the terminal.
    let mut sent: Vec<String> = Vec::new();
    // herdr's latest word on the agent, and the status last reported from it.
    let mut herdr_state = String::new();
    let mut reported: u32 = 0;
    let (answer_tx, mut answers) =
        mpsc::unbounded_channel::<(String, Option<InputAnswer>, Value)>();

    loop {
        let now = status_bits(&herdr_state, asked.is_some() || permitting.is_some());
        if now != reported {
            reported = now;
            let _ = events.send(SessionEvent::Outside(now));
        }
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
                if asked.is_none() && permitting.is_none() {
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
                    herdr_state = state.clone();
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
                    } else if worked && matches!(state.as_str(), "done" | "idle") && (asked.is_none() && permitting.is_none()) {
                        fallback = Some(tokio::time::Instant::now() + Duration::from_secs(3));
                        worked = false;
                    }
                }
                Seen::Prompt(prompt) => {
                    match sent.iter().position(|ours| *ours == prompt.trim()) {
                        Some(index) => {
                            sent.remove(index);
                        }
                        // Typed in the terminal: it gets a turn in the chat
                        // like one sent from Otto.
                        None if turn.is_none() => {
                            let turn_id = uuid::Uuid::new_v4().to_string();
                            let _ = events.send(SessionEvent::TurnStarted {
                                turn_id: turn_id.clone(),
                                prompt,
                            });
                            let _ = events.send(SessionEvent::Active);
                            turn = Some(turn_id);
                            worked = false;
                        }
                        None => {}
                    }
                }
                Seen::Ended => {
                    if asked.is_none() && permitting.is_none() {
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
                Seen::Permission(Some(request)) => {
                    let Some(id) = request.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        continue;
                    };
                    if permitting.as_deref() == Some(&id) {
                        continue;
                    }
                    permitting = Some(id.clone());
                    let question = permission_question(&id, &request, &cwd);
                    let (reply, wait) = oneshot::channel();
                    let _ = events.send(SessionEvent::PermissionRequested {
                        turn_id: turn.clone().unwrap_or_default(),
                        question: Box::new(question),
                        reply,
                    });
                    let decided_tx = decided_tx.clone();
                    tokio::spawn(async move {
                        let allow = wait.await.is_ok_and(|decision| decision.approved());
                        let _ = decided_tx.send((id, allow));
                    });
                }
                Seen::Permission(None) => permitting = None,
                Seen::Gone => {
                    let _ = events.send(SessionEvent::Running(false));
                    if let Some(turn_id) = turn.take() {
                        let _ = events.send(SessionEvent::TurnEnded {
                            turn_id,
                            outcome: TurnOutcome::Failed("the terminal ended".into()),
                        });
                    }
                    break;
                }
            },
            Some((id, allow)) = decided.recv() => {
                let reply = json!({ "id": id, "allow": allow });
                let label = format!("otto.permission.answer={reply}");
                let _ = hc().args(["label", &terminal, &label]).output().await;
            }
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
    if reported != 0 {
        let _ = events.send(SessionEvent::Outside(0));
    }
    watch_events.abort();
    watch_transcript.abort();
}

/// The hook's request, `{tool_name, tool_input}`, as a permission question:
/// built as the ACP request Claude's adapter would send, so the card shows
/// the command, or the file with its diff, as it does for ACP sessions.
fn permission_question(id: &str, request: &Value, cwd: &Path) -> crate::agent::Question {
    use agent_client_protocol::schema::v1::{
        Diff, PermissionOption, PermissionOptionKind, RequestPermissionRequest, ToolCallContent,
        ToolCallLocation, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    };
    let tool = request
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = request.get("tool_input").cloned().unwrap_or(Value::Null);
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let mut fields = ToolCallUpdateFields::new();
    fields.kind = Some(match tool {
        "Bash" => ToolKind::Execute,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => ToolKind::Edit,
        "Read" | "Glob" | "Grep" => ToolKind::Read,
        "WebFetch" | "WebSearch" => ToolKind::Fetch,
        _ => ToolKind::Other,
    });
    let path = text("file_path").or_else(|| text("notebook_path"));
    fields.title = Some(match (tool, text("command"), path) {
        (_, Some(command), _) => command.to_owned(),
        (_, None, Some(path)) => format!("{tool} {path}"),
        _ => tool.to_owned(),
    });
    if let Some(path) = path {
        fields.locations = Some(vec![ToolCallLocation::new(path)]);
        let diff = match tool {
            "Write" => text("content").map(|new| Diff::new(path, new)),
            "Edit" => text("new_string")
                .map(|new| Diff::new(path, new).old_text(text("old_string").map(str::to_owned))),
            _ => None,
        };
        if let Some(diff) = diff {
            fields.content = Some(vec![ToolCallContent::from(diff)]);
        }
    }
    fields.raw_input = Some(input);
    let options = vec![
        PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
        PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
    ];
    let request = RequestPermissionRequest::new(
        id.to_owned(),
        ToolCallUpdate::new(id.to_owned(), fields),
        options,
    );
    crate::dialog::question_for("Claude", None, cwd, &request)
}

/// What herdr says the agent is doing, as session status bits.
fn status_bits(state: &str, asking: bool) -> u32 {
    if asking || state == "blocked" {
        SessionStatus::InputNeeded.bits()
    } else if state == "working" {
        SessionStatus::InProgress.bits()
    } else {
        0
    }
}

fn end(turn: &mut Option<String>, events: &mpsc::UnboundedSender<SessionEvent>) {
    if let Some(turn_id) = turn.take() {
        let _ = events.send(SessionEvent::TurnEnded {
            turn_id,
            outcome: TurnOutcome::Complete,
        });
    }
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
///
/// Claude writes each block of its last message as an entry of its own, all
/// marked `end_turn`, thinking first: only a block that is not thinking ends
/// the turn, or the text written after it would be lost.
fn ends_turn(line: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|entry| {
        let said = entry
            .pointer("/message/content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks.iter().any(|block| {
                    !matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("thinking" | "redacted_thinking")
                    )
                })
            });
        entry.get("type").and_then(Value::as_str) == Some("assistant")
            && entry
                .pointer("/message/stop_reason")
                .and_then(Value::as_str)
                == Some("end_turn")
            && said
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
    let mut permission: Option<String> = None;
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
            let label = item
                .pointer("/labels/otto.permission")
                .and_then(Value::as_str)
                .filter(|raw| !raw.is_empty())
                .map(str::to_owned);
            if label != permission {
                permission = label.clone();
                let request = label.and_then(|raw| serde_json::from_str(&raw).ok());
                let _ = seen.send(Seen::Permission(request));
            }
        }
    }
    let _ = seen.send(Seen::Gone);
}

/// The agent's text, as Claude writes it to the transcript. Only entries
/// written after the session is opened here: earlier ones are history.
/// Entries are told apart by their `uuid`, not their place in the file, as a
/// resumed Claude rewrites the file rather than only appending to it.
async fn follow_transcript(transcript: Transcript, seen: mpsc::UnboundedSender<Seen>) {
    let mut known: HashSet<String> = entries(&transcript.read().await)
        .map(|(key, _)| key)
        .collect();
    let mut last_len = None;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Some(path) = transcript.path().await else {
            continue;
        };
        let Ok(len) = tokio::fs::metadata(&path).await.map(|m| m.len()) else {
            continue;
        };
        if last_len == Some(len) {
            continue;
        }
        last_len = Some(len);
        let text = transcript.read().await;
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
    fn a_pi_turn_reads_as_claude() {
        let pi = [
            r#"{"type":"session","id":"s1"}"#,
            r#"{"type":"message","id":"a","message":{"role":"user","content":[{"type":"text","text":"Run echo hi"}]}}"#,
            r#"{"type":"message","id":"b","message":{"role":"assistant","content":[{"type":"thinking","thinking":"ok"},{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"echo hi"}}],"stopReason":"toolUse"}}"#,
            r#"{"type":"message","id":"c","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[{"type":"text","text":"hi"}],"isError":false}}"#,
            r#"{"type":"message","id":"d","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stopReason":"stop"}}"#,
            "",
        ]
        .join("\n");
        let claude = pi_as_claude(&pi);
        let lines: Vec<&str> = claude.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(prompt_of(lines[0]).as_deref(), Some("Run echo hi"));
        assert!(!ends_turn(lines[1]));
        assert_eq!(prompt_of(lines[2]), None, "a tool's result is not a prompt");
        assert!(ends_turn(lines[3]));
        assert_eq!(assistant_text(lines[3]), vec!["done\n\n".to_owned()]);
        let turns = history(&claude);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].prompt, "Run echo hi");
        assert_eq!(turns[0].parts.len(), 2, "the tool call and the answer");
    }

    #[test]
    fn a_session_started_by_hand_goes_to_the_agent_of_its_folder() {
        let agents = vec![
            ("claude".to_owned(), None),
            ("dev".to_owned(), Some(PathBuf::from("/home/me/dev"))),
            ("otto".to_owned(), Some(PathBuf::from("/home/me/dev/otto"))),
        ];
        assert_eq!(
            adopted_agent(&agents, Path::new("/home/me/dev/otto/src")),
            "otto"
        );
        assert_eq!(adopted_agent(&agents, Path::new("/home/me/dev/x")), "dev");
        assert_eq!(adopted_agent(&agents, Path::new("/tmp")), "claude");
        assert_eq!(adopted_agent(&[], Path::new("/tmp")), PROVIDER);
    }

    #[test]
    fn a_space_id_is_not_a_name() {
        assert!(is_space_id("speca9fea29607a6c98182a4d76bb22054"));
        assert!(!is_space_id("Speaker-Studio"));
        assert!(!is_space_id("spaces"));
    }

    #[test]
    fn pi_is_told_by_its_command() {
        assert_eq!(Harness::of("pi"), Harness::Pi);
        assert_eq!(Harness::of("/usr/bin/pi-acp"), Harness::Pi);
        assert_eq!(Harness::of("claude"), Harness::Claude);
        assert_eq!(Harness::of("pipx"), Harness::Claude);
    }

    #[test]
    fn a_profile_asks_through_otto() {
        let profile = skills::AgentFile {
            name: "otto".into(),
            description: "Otto's own helper".into(),
            tools: vec!["Read".into(), "AskUserQuestion".into()],
            model: None,
            skills: vec!["otto-help".into()],
            session_only: false,
            body: "You are Otto.".into(),
            path: PathBuf::new(),
        };
        let definition = agent_definition(&profile, true);
        assert_eq!(definition["tools"], json!(["Read", "mcp__otto__ask"]));
        assert_eq!(definition["prompt"], "You are Otto.");
        assert_eq!(definition["skills"], json!(["otto-help"]));
        let plain = agent_definition(&profile, false);
        assert_eq!(plain["tools"], json!(["Read", "AskUserQuestion"]));
    }

    #[test]
    fn spaces_answer_to_their_id_name_or_folder() {
        let spaces = vec![
            json!({ "id": "sp1", "name": "Otto", "folder": "/home/me" }),
            json!({ "id": "sp2", "name": "", "folder": "/home/me/dev/speaker-studio" }),
        ];
        assert_eq!(find_space(&spaces, "sp2").as_deref(), Some("sp2"));
        assert_eq!(find_space(&spaces, "Otto").as_deref(), Some("sp1"));
        assert_eq!(find_space(&spaces, "otto").as_deref(), Some("sp1"));
        assert_eq!(
            find_space(&spaces, "speaker-studio").as_deref(),
            Some("sp2")
        );
        assert_eq!(find_space(&spaces, "elsewhere"), None);
    }

    #[test]
    fn a_write_asks_with_its_diff() {
        let request = json!({
            "id": "p1",
            "tool_name": "Write",
            "tool_input": { "file_path": "/tmp/test.txt", "content": "test" }
        });
        let question = permission_question("p1", &request, Path::new("/home/me"));
        assert_eq!(question.edits.len(), 1);
        assert_eq!(question.edits[0]["newText"], "test");
        assert!(question.options.iter().any(|option| option.allow()));
        assert!(question.options.iter().any(|option| !option.allow()));
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
            r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done."}]}}"#
        ));
        // The thinking comes first, as an entry of its own, already marked
        // as the end: the text is still to come.
        assert!(!ends_turn(
            r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"thinking","thinking":""}]}}"#
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
