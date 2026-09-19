//! Terminal views of the host: `otto-agents sessions`, `otto-agents show`,
//! and the `otto-agents plugins` pair that puts the desktop's skills and its
//! agent where each harness looks for them.

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

use crate::skills::{self, AgentFile, Entry, Installed, LinkState, Plugin};
use otto_agents_client::session::{self, session_id};

use crate::uri;
use crate::vendors::{self, Done, Home, State, Target, Vendor, first_sentence};
use crate::xdg::tilde;

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

/// Forgets sessions: the one `session` names, or every one with `all`. The
/// agent's own history is its to keep; this removes what otto-agents stored.
pub async fn forget_sessions(url: &str, session: Option<&str>, all: bool) -> anyhow::Result<()> {
    let client = connect(url).await?;
    let sessions = fetch_sessions(&client).await?;
    let doomed: Vec<String> = if all {
        sessions.iter().map(|s| s.resource.clone()).collect()
    } else {
        vec![find_session(&sessions, session)?.resource.clone()]
    };
    if doomed.is_empty() {
        println!("no sessions");
    }
    for resource in &doomed {
        client
            .request::<_, serde_json::Value>(
                "disposeSession",
                serde_json::json!({ "channel": resource }),
            )
            .await
            .with_context(|| format!("forgetting {resource}"))?;
        println!("forgotten: {resource}");
    }
    client.shutdown().await;
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

/// `otto-agents plugins install`: links every discovered skill into `dir`,
/// writes the plugins' agents for every harness that is set up under `home`,
/// and says what happened to each. `only` names one harness and skips the
/// skills.
pub fn install_plugins(
    dir: Option<&Path>,
    home: Option<&Path>,
    only: Option<&str>,
) -> anyhow::Result<()> {
    let only = vendor(only)?;
    let home = vendors_home(home)?;
    let plugins = skills::discover();
    let mut out = String::new();
    if only.is_none() {
        let dir = skills_dir(dir)?;
        // Before linking: a skill that was renamed upstream leaves a link
        // behind pointing at a path the upgrade removed.
        for stale in skills::prune(&dir)
            .with_context(|| format!("could not tidy {}", dir.display()))?
        {
            out.push_str(&format!(
                "removed {}: the skill it pointed at is gone\n",
                tilde(&stale, Some(&home.root))
            ));
        }
        let installed = skills::install(&plugins, &dir)
            .with_context(|| format!("could not link skills into {}", dir.display()))?;
        out.push_str(&render_installed(
            &plugins,
            &installed,
            &dir,
            Some(&home.root),
        ));
    }
    let agents = agents_of(&plugins);
    let targets =
        vendors::install(&agents, &home, only).context("could not write an agent file")?;
    out.push_str(&render_targets(&plugins, &agents, &targets, &home));
    print!("{out}");
    Ok(())
}

/// `otto-agents plugins status`: the plugins found, their skills and whether
/// each is linked into `dir`, and where each harness's copy of the agents
/// stands under `home`.
pub fn plugins_status(
    dir: Option<&Path>,
    home: Option<&Path>,
    only: Option<&str>,
) -> anyhow::Result<()> {
    let only = vendor(only)?;
    let home = vendors_home(home)?;
    let plugins = skills::discover();
    let mut out = String::new();
    if only.is_none() {
        let dir = skills_dir(dir)?;
        let entries = skills::status(&plugins, &dir);
        out.push_str(&render_skills_status(&plugins, &entries, Some(&home.root)));
    }
    let agents = agents_of(&plugins);
    let targets = vendors::status(&agents, &home, only);
    out.push_str(&render_targets(&plugins, &agents, &targets, &home));
    print!("{out}");
    Ok(())
}

fn vendor(id: Option<&str>) -> anyhow::Result<Option<Vendor>> {
    id.map(|id| {
        Vendor::from_id(id).with_context(|| {
            format!(
                "no harness called {id}; one of {}",
                Vendor::ALL.map(Vendor::id).join(", ")
            )
        })
    })
    .transpose()
}

fn vendors_home(explicit: Option<&Path>) -> anyhow::Result<Home> {
    match explicit {
        Some(root) => Ok(Home::at(root)),
        None => Home::from_env().context("no HOME to place the agents under; give --home"),
    }
}

fn agents_of(plugins: &[Plugin]) -> Vec<&AgentFile> {
    plugins.iter().flat_map(|plugin| &plugin.agents).collect()
}

fn skills_dir(explicit: Option<&Path>) -> anyhow::Result<std::path::PathBuf> {
    match explicit {
        Some(dir) => Ok(dir.to_path_buf()),
        None => skills::default_skills_dir()
            .context("no HOME to find ~/.agents/skills under; give --dir"),
    }
}

fn render_installed(
    plugins: &[Plugin],
    installed: &[Installed],
    dir: &Path,
    home: Option<&Path>,
) -> String {
    if plugins.is_empty() {
        return no_plugins();
    }
    let mut out = String::new();
    for Installed { entry, created } in installed {
        let link = tilde(&entry.link, home);
        out.push_str(&match (entry.state, created) {
            (LinkState::Linked, true) => format!("linked {link} -> {}\n", entry.source.display()),
            (LinkState::Linked, false) => format!("already linked: {link}\n"),
            (LinkState::Taken, _) => format!(
                "left alone: {link} is not a link to {} (yours, or another tool's)\n",
                entry.source.display()
            ),
            (LinkState::Unlinked, _) => format!("not linked: {link}\n"),
        });
    }
    out.push_str(&format!(
        "Agents that read {} will find them from their next session.\n",
        tilde(dir, home)
    ));
    out
}

fn render_skills_status(plugins: &[Plugin], entries: &[Entry], home: Option<&Path>) -> String {
    if plugins.is_empty() {
        return no_plugins();
    }
    let mut out = String::new();
    for plugin in plugins {
        out.push_str(&plugin.name);
        if let Some(version) = &plugin.version {
            out.push_str(&format!(" {version}"));
        }
        out.push_str(&format!("  {}\n", plugin.dir.display()));
        for entry in entries.iter().filter(|entry| entry.plugin == plugin.name) {
            let state = match entry.state {
                LinkState::Linked => "linked",
                LinkState::Unlinked => "not linked",
                LinkState::Taken => "in the way",
            };
            out.push_str(&format!(
                "  {:<24} {:<11} {}\n",
                entry.name,
                state,
                tilde(&entry.link, home)
            ));
        }
        for agent in &plugin.agents {
            out.push_str(&format!(
                "  agent: {} — {}\n",
                agent.name,
                first_sentence(&agent.description)
            ));
        }
    }
    if entries
        .iter()
        .any(|entry| entry.state == LinkState::Unlinked)
    {
        out.push_str("Run `otto-agents plugins install` to link the rest.\n");
    }
    out
}

/// The harnesses' copies of the agents: one line per file, after
/// [`vendors::install`] (what was done) or [`vendors::status`] (where it
/// stands). Claude has no line: it reads the plugin's own file.
fn render_targets(
    plugins: &[Plugin],
    agents: &[&AgentFile],
    targets: &[Target],
    home: &Home,
) -> String {
    if plugins.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if agents.is_empty() {
        out.push_str("No agents in the plugins, so nothing to render for the harnesses.\n");
        return out;
    }
    let names: Vec<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
    out.push_str(&format!(
        "Claude reads {} from the plugin; the other harnesses get a rendering:\n",
        names.join(", ")
    ));
    let mut stale = false;
    for target in targets {
        let path = home.tilde(&target.path);
        let vendor = target.vendor.id();
        let agent = agents.iter().find(|agent| agent.name == target.agent);
        let line = match (target.done, target.state) {
            (_, State::NoHarness) => {
                let hint =
                    agent.map_or_else(String::new, |agent| target.vendor.absent_hint(home, agent));
                format!("skipped {vendor}: {hint}")
            }
            (_, State::Taken) => format!(
                "left alone: {path} is not ours (yours, or {vendor}'s own); move it aside to have it rendered"
            ),
            (Some(Done::Created), _) => format!("wrote {path}"),
            (Some(Done::Updated), _) => format!("updated {path}"),
            (Some(Done::Unchanged), _) => format!("unchanged: {path}"),
            (Some(Done::LeftAlone), _) => format!("left alone: {path}"),
            (None, State::Current) => format!("{vendor:<9} current    {path}"),
            (None, State::Stale) => {
                stale = true;
                format!("{vendor:<9} stale      {path}")
            }
            (None, State::Absent) => {
                stale = true;
                format!("{vendor:<9} not there  {path}")
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    if stale {
        out.push_str("Run `otto-agents plugins install` to write them.\n");
    }
    out
}

fn no_plugins() -> String {
    let mut out = String::from("No plugins found. Looked in:\n");
    for dir in skills::search_paths() {
        out.push_str(&format!("  {}\n", dir.display()));
    }
    out
}

/// `path` with the home directory written as `~`.
async fn connect(url: &str) -> anyhow::Result<Client> {
    let transport = crate::client::connect(url)
        .await
        .with_context(|| format!("could not connect to {url}; is `otto-agents serve` running?"))?;
    let client = Client::connect(transport, ClientConfig::default()).await?;
    client
        .initialize(
            "otto-agents-cli".into(),
            vec![PROTOCOL_VERSION.into()],
            vec![],
        )
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
    session::find(sessions, query).map_err(Into::into)
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
    tilde(&path, home)
}

/// Checks the things that stop Ask working, in the order they fail.
///
/// The four questions the troubleshooting guide asks, answered without
/// reading any code: is the service reachable, does the configuration parse,
/// is each agent's command on the service's `PATH`, and is there a renderer
/// to put a dialog on the screen.
pub async fn doctor(url: &str, config_path: Option<&Path>) -> anyhow::Result<()> {
    let mut sound = true;
    let mut check = |ok: bool, label: &str, detail: String| {
        sound &= ok;
        let mark = if ok { "ok" } else { "problem" };
        println!("[{mark}] {label}: {detail}");
    };

    match connect(url).await {
        Ok(client) => {
            let sessions = fetch_sessions(&client).await;
            client.shutdown().await;
            let detail = match &sessions {
                Ok(sessions) => format!("{url}, {} session(s)", sessions.len()),
                Err(err) => format!("{url}, but listing sessions failed: {err}"),
            };
            check(sessions.is_ok(), "service", detail);
        }
        Err(err) => check(
            false,
            "service",
            format!(
                "cannot reach {url}: {err}. Is otto-agents running? `systemctl --user status otto-agents`"
            ),
        ),
    }

    let config = crate::config::load(config_path);
    match &config {
        Ok(config) => check(
            true,
            "config",
            format!(
                "{} agent(s): {}",
                config.agents.len(),
                config
                    .agents
                    .iter()
                    .map(|agent| agent.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Err(err) => check(false, "config", format!("{err:#}")),
    }

    if let Ok(config) = &config {
        for agent in &config.agents {
            let found = which(&agent.command);
            let detail = match &found {
                Some(path) => format!("{} → {}", agent.command, path.display()),
                None => format!(
                    "{} is not on PATH. The service does not get a login shell's PATH, so name the full path in agents.toml",
                    agent.command
                ),
            };
            check(found.is_some(), &format!("agent {}", agent.id), detail);
        }
    }

    let renderer = crate::dialog::renderer_present().await;
    check(
        renderer,
        "dialogs",
        match renderer {
            true => "otto-islands is on the session bus".to_owned(),
            false => "no otto-islands on the session bus; permission requests will be denied and questions will wait in the chat".to_owned(),
        },
    );

    if sound {
        println!(
            "\nNothing wrong here. Note that the commands were looked up on this \n\
             shell's PATH; the service runs under systemd with its own, so a command \n\
             found here can still be missing there — the journal says which."
        );
        return Ok(());
    }
    bail!("some checks did not pass")
}

/// Where `command` is found on `PATH`, if anywhere. An absolute command is
/// checked where it stands.
fn which(command: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(command))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use ahp_types::state::{
        ActiveTurn, MarkdownResponsePart, Message, MessageKind, MessageOrigin, PendingMessage, Turn,
    };

    use super::*;
    use otto_agents_client::session::SESSION_SCHEME;

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
    fn skills_status_lists_each_plugin_with_its_skills_and_links() {
        let home = Path::new("/home/me");
        let plugin = Plugin {
            name: "otto".into(),
            description: String::new(),
            version: Some("1.0.0".into()),
            dir: "/usr/share/otto/plugins/otto".into(),
            skills: Vec::new(),
            agents: vec![skills::AgentFile {
                name: "otto".into(),
                description: "Otto's own helper. Use it when someone asks about the desktop."
                    .into(),
                tools: vec!["Read".into()],
                model: None,
                skills: vec!["otto".into()],
                body: "You are Otto.".into(),
                path: "/usr/share/otto/plugins/otto/agents/otto.md".into(),
            }],
        };
        let entry = |name: &str, state: LinkState| Entry {
            plugin: "otto".into(),
            name: name.into(),
            source: format!("/usr/share/otto/plugins/otto/skills/{name}").into(),
            link: format!("/home/me/.agents/skills/{name}").into(),
            state,
        };
        let entries = [
            entry("otto", LinkState::Linked),
            entry("files", LinkState::Unlinked),
        ];
        let text = render_skills_status(std::slice::from_ref(&plugin), &entries, Some(home));
        assert_eq!(
            text,
            "otto 1.0.0  /usr/share/otto/plugins/otto\n\
             \x20 otto                     linked      ~/.agents/skills/otto\n\
             \x20 files                    not linked  ~/.agents/skills/files\n\
             \x20 agent: otto — Otto's own helper\n\
             Run `otto-agents plugins install` to link the rest.\n"
        );

        let installed = [
            Installed {
                entry: entry("otto", LinkState::Linked),
                created: true,
            },
            Installed {
                entry: entry("files", LinkState::Taken),
                created: false,
            },
        ];
        let text = render_installed(
            std::slice::from_ref(&plugin),
            &installed,
            Path::new("/home/me/.agents/skills"),
            Some(home),
        );
        assert_eq!(
            text,
            "linked ~/.agents/skills/otto -> /usr/share/otto/plugins/otto/skills/otto\n\
             left alone: ~/.agents/skills/files is not a link to /usr/share/otto/plugins/otto/skills/files (yours, or another tool's)\n\
             Agents that read ~/.agents/skills will find them from their next session.\n"
        );

        assert!(render_skills_status(&[], &[], Some(home)).starts_with("No plugins found."));

        // The harnesses' copies, after an install and as a status.
        let vendors_home = Home::at(home);
        let agents: Vec<&AgentFile> = plugin.agents.iter().collect();
        let target = |vendor: Vendor, path: &str, state: State, done: Option<Done>| Target {
            vendor,
            agent: "otto".into(),
            path: path.into(),
            state,
            done,
        };
        let installed = [
            target(
                Vendor::OpenCode,
                "/home/me/.config/opencode/agents/otto.md",
                State::Current,
                Some(Done::Created),
            ),
            target(
                Vendor::Hermes,
                "/home/me/.hermes/profiles/otto/SOUL.md",
                State::Taken,
                Some(Done::LeftAlone),
            ),
            target(
                Vendor::Codex,
                "/home/me/.local/share/otto/agents/codex/otto.md",
                State::NoHarness,
                Some(Done::LeftAlone),
            ),
            target(
                Vendor::Pi,
                "/home/me/.local/share/otto/agents/pi/otto.md",
                State::Current,
                Some(Done::Updated),
            ),
            target(
                Vendor::Pi,
                "/home/me/.local/share/otto/agents/pi/otto-pi",
                State::Current,
                Some(Done::Unchanged),
            ),
        ];
        let text = render_targets(
            std::slice::from_ref(&plugin),
            &agents,
            &installed,
            &vendors_home,
        );
        assert_eq!(
            text,
            "Claude reads otto from the plugin; the other harnesses get a rendering:\n\
             wrote ~/.config/opencode/agents/otto.md\n\
             left alone: ~/.hermes/profiles/otto/SOUL.md is not ours (yours, or hermes's own); move it aside to have it rendered\n\
             skipped codex: no ~/.codex: the harness is not set up here\n\
             updated ~/.local/share/otto/agents/pi/otto.md\n\
             unchanged: ~/.local/share/otto/agents/pi/otto-pi\n"
        );

        let status = [
            target(
                Vendor::OpenCode,
                "/home/me/.config/opencode/agents/otto.md",
                State::Current,
                None,
            ),
            target(
                Vendor::Codex,
                "/home/me/.local/share/otto/agents/codex/otto.md",
                State::Stale,
                None,
            ),
            target(
                Vendor::Pi,
                "/home/me/.local/share/otto/agents/pi/otto.md",
                State::Absent,
                None,
            ),
        ];
        let text = render_targets(
            std::slice::from_ref(&plugin),
            &agents,
            &status,
            &vendors_home,
        );
        assert_eq!(
            text,
            "Claude reads otto from the plugin; the other harnesses get a rendering:\n\
             opencode  current    ~/.config/opencode/agents/otto.md\n\
             codex     stale      ~/.local/share/otto/agents/codex/otto.md\n\
             pi        not there  ~/.local/share/otto/agents/pi/otto.md\n\
             Run `otto-agents plugins install` to write them.\n"
        );
        assert_eq!(render_targets(&[], &[], &[], &vendors_home), "");
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
