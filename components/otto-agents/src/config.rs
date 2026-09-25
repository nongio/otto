//! Agent configuration: `[[agents]]` entries in Otto's `agents.toml`.
//!
//! Files load in order, `/etc/otto/agents.toml` then
//! `$XDG_CONFIG_HOME/otto/agents.toml`; the last file that lists agents wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Published as `AgentInfo.provider`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Executable speaking ACP over stdio.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra, non-secret environment for the agent process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Model each new session switches to, as the agent names it. Claude takes
    /// aliases such as `haiku`, `sonnet` and `opus`. Unset keeps the agent's own
    /// default.
    #[serde(default)]
    pub model: Option<String>,
    /// The agent's mode id every new session starts in, such as `acceptEdits`
    /// for Claude or `agent` for Codex. Modes are the agent's own permission
    /// and sandboxing presets, and its own list is what counts: an id it does
    /// not offer is logged and ignored. A session taken up again keeps the
    /// mode it was in. Unset keeps the agent's default.
    #[serde(default)]
    pub mode: Option<String>,
    /// Session configuration options the agent takes by its own ids, set on
    /// every session after the model. They are how a harness reaches settings
    /// ACP has no field for: Codex only offers its question tool in the `plan`
    /// collaboration mode, so `config = { collaboration_mode = "plan" }` is
    /// what lets it ask. An option the agent refuses is logged and the
    /// session carries on.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// The plugin agent this agent runs as, by the `name` in its agent file,
    /// such as `otto` for the desktop's own helper: Claude starts with
    /// `--agent plugin:name` and honours the file's instructions, tools,
    /// skills and model itself. Rides the Claude route only, so it needs
    /// `skills = "claude"`; a name no plugin has is logged and ignored.
    /// Unset runs the agent as itself. Other harnesses are pointed at the
    /// rendering `otto-agents plugins install` wrote for them through
    /// `args` or `env` instead — see [`crate::vendors`].
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub permissions: PermissionPolicy,
    /// The command that enters one of the agent's sessions in a terminal, in
    /// the agent's own interface, with `{session}` standing for the agent's id
    /// for it and `{cwd}` for its folder, such as
    /// `["claude", "--resume", "{session}"]`. Empty when the agent has none.
    #[serde(default)]
    pub enter: Vec<String>,
    /// The same, for a session the agent has not written a history for yet:
    /// `otto-agents new` creates the session and enters it before anything is
    /// said in it, and a harness that resumes by id refuses one it has never
    /// seen. Claude takes `["claude", "--session-id", "{session}"]` here and
    /// `--resume` in `enter`; the two are exclusive, so the service offers
    /// this one only until the session has a turn. Empty falls back to
    /// [`AgentConfig::enter`].
    #[serde(default)]
    pub enter_new: Vec<String>,
    /// How the desktop's skills reach this agent — see [`crate::skills`]. Off
    /// by default. `skills = "claude"` is for Claude, which loads them as its
    /// own plugin; every other agent finds them in `~/.agents/skills` after
    /// `otto-agents plugins install`, and needs nothing from the session.
    #[serde(default)]
    pub skills: SkillDelivery,
    /// The frosted material the agent's surfaces wear — the Ask card, for one
    /// — as `colour` (or `color`) in `agents.toml`: one of the names in
    /// [`Colour::NAMES`], such as `"teal"`. Published in the root state's
    /// `_meta` as `otto.colours.<id>`. Unset leaves the surfaces on the
    /// desktop's plain material.
    #[serde(default, alias = "color")]
    pub colour: Option<Colour>,
    /// The folder this agent's sessions start in when nobody picks one, as
    /// `folder` in `agents.toml`. `~` stands for the home folder.
    ///
    /// A folder is the reach the agent is given: everything under it is
    /// something the agent can read, and a permission policy only covers what
    /// the agent thinks to ask about. So this is worth setting narrowly —
    /// `folder = "~/dev"` for an agent that works on code, and nothing at all
    /// for one that only changes desktop settings, which need no folder.
    /// Published in the root state's `_meta` as `otto.folders.<id>`. Unset
    /// leaves the client to choose; Ask then uses a scratch folder of its own.
    #[serde(default, deserialize_with = "tilde_path")]
    pub folder: Option<PathBuf>,
}

/// A frosted material's name, as an agent's `colour` takes it. The names are
/// otto-kit's `Frosted` materials; that crate holds the colours, and this
/// list has to agree with its (otto-kit's tests check that it does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Teal,
    Cyan,
    Blue,
    Indigo,
    Violet,
    Magenta,
}

impl Colour {
    /// Every name the config accepts, in the order the materials go round
    /// the wheel.
    pub const NAMES: [&str; 12] = [
        "red", "orange", "amber", "yellow", "lime", "green", "teal", "cyan", "blue", "indigo",
        "violet", "magenta",
    ];

    const ALL: [Colour; 12] = [
        Colour::Red,
        Colour::Orange,
        Colour::Amber,
        Colour::Yellow,
        Colour::Lime,
        Colour::Green,
        Colour::Teal,
        Colour::Cyan,
        Colour::Blue,
        Colour::Indigo,
        Colour::Violet,
        Colour::Magenta,
    ];

    /// The name as `agents.toml` writes it and `_meta` carries it.
    pub fn name(self) -> &'static str {
        Self::NAMES[Self::ALL.iter().position(|c| *c == self).unwrap_or(0)]
    }
}

impl std::str::FromStr for Colour {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::NAMES
            .iter()
            .position(|known| *known == name)
            .map(|index| Self::ALL[index])
            .ok_or_else(|| {
                format!(
                    "colour = {name:?}: expected one of {}",
                    Self::NAMES
                        .iter()
                        .map(|name| format!("{name:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// How an agent is given the desktop's skills: `false` or `"claude"` in
/// `agents.toml`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkillDelivery {
    /// Not at all. The default: an agent that finds skills on its own reads
    /// what `otto-agents plugins install` linked into `~/.agents/skills`, and
    /// needs nothing from the session.
    #[default]
    Off,
    /// Loaded as Claude Code plugins, through claude-agent-acp's session
    /// options, with a short Otto preamble appended to Claude's own system
    /// prompt. Claude then knows them as its own skills: `/name` invokes one,
    /// and a skill's `allowed-tools` is Claude's to honour.
    Claude,
}

impl SkillDelivery {
    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

impl<'de> Deserialize<'de> for SkillDelivery {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Name(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Flag(false) => Ok(Self::Off),
            Raw::Name(name) if name == "claude" => Ok(Self::Claude),
            Raw::Flag(true) => Err(serde::de::Error::custom(
                "skills = true: expected false or \"claude\"",
            )),
            Raw::Name(name) => Err(serde::de::Error::custom(format!(
                "skills = {name:?}: expected false or \"claude\""
            ))),
        }
    }
}

/// Everything `agents.toml` configures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub agents: Vec<AgentConfig>,
    /// The terminal a session is entered in, with the agent's `enter` command
    /// appended, such as `["ghostty", "--working-directory={cwd}", "-e"]`.
    /// `{session}` and `{cwd}` stand here as they do in `enter`: a window
    /// class or title holding `{session}` is how the launcher finds a
    /// session's terminal again. `{title}` is filled in by the launcher with
    /// what the window is called — the agent and what the session is about.
    /// Empty when none is configured.
    pub terminal: Vec<String>,
    /// The agent a session gets when none is asked for. [`load`] lists it
    /// first, which is how clients and `createSession` tell the default.
    pub default_agent: Option<String>,
    /// Seconds a session's agent may sit with nothing to do before it is
    /// stopped, as `agents.toml` sets it. Read it through
    /// [`Config::idle_timeout`].
    pub idle_timeout: Option<u64>,
}

/// How long an agent stays up with nothing to do when `idle_timeout` is unset.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

impl Config {
    /// How long a session's agent may sit with nothing to do before it is
    /// stopped. `None` keeps agents running, which `idle_timeout = 0` asks for.
    pub fn idle_timeout(&self) -> Option<Duration> {
        match self.idle_timeout {
            None => Some(DEFAULT_IDLE_TIMEOUT),
            Some(0) => None,
            Some(seconds) => Some(Duration::from_secs(seconds)),
        }
    }
}

/// The command that enters `agent_session` of `agent` in the terminal it is
/// already in, when the agent has one. `written` says whether the agent has a
/// history for the session: an unwritten one takes
/// [`AgentConfig::enter_new`], since resuming by an id the harness has never
/// seen fails. `extra` goes straight after the program: the flags that give
/// the harness in the terminal what the service gave its own copy, such as
/// the plugins Claude loads ([`crate::skills::claude_cli_args`]).
pub fn enter_command(
    agent: &AgentConfig,
    agent_session: &str,
    cwd: &Path,
    written: bool,
    extra: &[String],
) -> Option<Vec<String>> {
    let enter = match (written, agent.enter_new.is_empty()) {
        (false, false) => &agent.enter_new,
        _ => &agent.enter,
    };
    if enter.is_empty() {
        return None;
    }
    let cwd = cwd.to_string_lossy();
    let mut words: Vec<String> = enter
        .iter()
        .map(|arg| {
            arg.replace("{session}", agent_session)
                .replace("{cwd}", &cwd)
        })
        .collect();
    words.splice(1..1, extra.iter().cloned());
    Some(words)
}

/// The command that opens `agent_session` of `agent` in `terminal`, in `cwd`,
/// when both a terminal and the agent's enter command are configured. `extra`
/// is as [`enter_command`] takes it.
pub fn terminal_command(
    terminal: &[String],
    agent: &AgentConfig,
    agent_session: &str,
    cwd: &Path,
    written: bool,
    extra: &[String],
) -> Option<Vec<String>> {
    if terminal.is_empty() {
        return None;
    }
    let enter = enter_command(agent, agent_session, cwd, written, extra)?;
    let cwd = cwd.to_string_lossy();
    Some(
        terminal
            .iter()
            .map(|arg| {
                arg.replace("{session}", agent_session)
                    .replace("{cwd}", &cwd)
            })
            .chain(enter)
            .collect(),
    )
}

/// The agent's `args` as the process gets them: every `{file:<path>}` is
/// replaced by that file's contents, `~` standing for the home directory. It
/// is how an argument carries a rendered instructions file to a harness that
/// takes text and not a path — Codex's `-c developer_instructions=...`. A
/// file that cannot be read is logged and stands for nothing, so the harness
/// gets an empty setting rather than a path where text was due.
pub fn expand_args(args: &[String]) -> Vec<String> {
    args.iter().map(|arg| expand_arg(arg)).collect()
}

/// The same for the environment an agent is started with: a value naming a
/// file carries the file, which is how an adapter that takes its whole
/// configuration in one variable is handed a long instruction.
pub fn expand_env(env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    env.iter()
        .map(|(key, value)| (key.clone(), expand_arg(value)))
        .collect()
}

fn expand_arg(arg: &str) -> String {
    const OPEN: &str = "{file:";
    let mut out = String::new();
    let mut rest = arg;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let path = expand_tilde(&after[..end]);
        match std::fs::read_to_string(&path) {
            Ok(text) => out.push_str(text.trim_end()),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "an argument names a file that cannot be read; passing nothing in its place")
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A path as `agents.toml` writes one, with a leading `~/` expanded.
fn tilde_path<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.map(|path| expand_tilde(&path)))
}

fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

/// How the service answers an agent's permission requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionPolicy {
    #[default]
    Deny,
    Allow,
    /// Ask the user in an Otto dialog (otto-islands), and deny when the dialog
    /// can't be shown.
    Ask,
}

impl AgentConfig {
    /// Claude Code through the ACP adapter, used when no agents are configured.
    pub fn claude() -> Self {
        Self {
            id: "claude".into(),
            name: "Claude".into(),
            description: "Claude Code through claude-agent-acp".into(),
            command: "npx".into(),
            args: vec![
                "-y".into(),
                "@agentclientprotocol/claude-agent-acp@0.79.0".into(),
            ],
            env: BTreeMap::new(),
            model: None,
            mode: None,
            config: BTreeMap::new(),
            agent: None,
            permissions: PermissionPolicy::Deny,
            enter: vec!["claude".into(), "--resume".into(), "{session}".into()],
            // A session with nothing in it yet has no history to resume, and
            // Claude refuses an id it has never seen; `--session-id` takes
            // the one the service made for it.
            enter_new: vec!["claude".into(), "--session-id".into(), "{session}".into()],
            skills: SkillDelivery::Claude,
            colour: None,
            folder: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    terminal: Vec<String>,
    #[serde(default)]
    default_agent: Option<String>,
    #[serde(default)]
    idle_timeout: Option<u64>,
    #[serde(default)]
    agents: Vec<AgentConfig>,
}

/// Loads the configuration, from `explicit` alone when given. Otherwise the
/// last file that lists agents wins for the agents, and the last that names a
/// terminal for the terminal.
pub fn load(explicit: Option<&Path>) -> anyhow::Result<Config> {
    let mut config = Config::default();
    match explicit {
        Some(path) => config = read(path)?,
        None => {
            for path in default_paths().iter().filter(|path| path.is_file()) {
                let file = read(path)?;
                if !file.agents.is_empty() {
                    config.agents = file.agents;
                }
                if !file.terminal.is_empty() {
                    config.terminal = file.terminal;
                }
                if file.default_agent.is_some() {
                    config.default_agent = file.default_agent;
                }
                if file.idle_timeout.is_some() {
                    config.idle_timeout = file.idle_timeout;
                }
            }
        }
    }
    let agents = &mut config.agents;
    if agents.is_empty() {
        agents.push(AgentConfig::claude());
    }
    for (i, agent) in agents.iter().enumerate() {
        if agents[..i].iter().any(|other| other.id == agent.id) {
            anyhow::bail!("agent id `{}` is configured more than once", agent.id);
        }
    }
    // The default goes first: `RootState.agents` keeps this order, and a
    // session created without an agent gets the first.
    if let Some(default) = &config.default_agent {
        let Some(index) = agents.iter().position(|agent| &agent.id == default) else {
            anyhow::bail!("default_agent `{default}` is not a configured agent");
        };
        let agent = agents.remove(index);
        agents.insert(0, agent);
    }
    Ok(config)
}

fn read(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse(&text).with_context(|| format!("invalid agent configuration in {}", path.display()))
}

fn parse(text: &str) -> anyhow::Result<Config> {
    let file = toml::from_str::<ConfigFile>(text)?;
    Ok(Config {
        agents: file.agents,
        terminal: file.terminal,
        default_agent: file.default_agent,
        idle_timeout: file.idle_timeout,
    })
}

fn default_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/etc/otto/agents.toml")];
    paths.extend(crate::xdg::config_home().map(|dir| dir.join("otto/agents.toml")));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_entered_in_the_terminal_with_the_agents_enter_command() {
        let config = parse(
            r#"
            terminal = ["ghostty", "--working-directory={cwd}", "-e"]

            [[agents]]
            id = "claude"
            name = "Claude"
            command = "claude-agent-acp"
            enter = ["claude", "--resume", "{session}"]

            [[agents]]
            id = "other"
            name = "Other"
            command = "other-acp"
            "#,
        )
        .unwrap();
        let cwd = Path::new("/home/me");
        assert_eq!(
            terminal_command(&config.terminal, &config.agents[0], "abc", cwd, true, &[]),
            Some(
                [
                    "ghostty",
                    "--working-directory=/home/me",
                    "-e",
                    "claude",
                    "--resume",
                    "abc"
                ]
                .map(String::from)
                .to_vec()
            )
        );
        assert_eq!(
            terminal_command(&config.terminal, &config.agents[1], "abc", cwd, true, &[]),
            None,
            "an agent without a resume command has nothing to open"
        );
        assert_eq!(
            terminal_command(&[], &config.agents[0], "abc", cwd, true, &[]),
            None,
            "nor does a missing terminal"
        );
    }

    #[test]
    fn a_session_with_no_history_is_entered_with_the_agents_new_command() {
        let config = parse(
            r#"
            terminal = ["ghostty", "-e"]

            [[agents]]
            id = "claude"
            name = "Claude"
            command = "claude-agent-acp"
            enter = ["claude", "--resume", "{session}"]
            enter_new = ["claude", "--session-id", "{session}"]

            [[agents]]
            id = "plain"
            name = "Plain"
            command = "plain-acp"
            enter = ["plain", "--resume", "{session}"]
            "#,
        )
        .unwrap();
        let cwd = Path::new("/home/me");
        assert_eq!(
            enter_command(&config.agents[0], "abc", cwd, false, &[]),
            Some(["claude", "--session-id", "abc"].map(String::from).to_vec()),
            "an unwritten session cannot be resumed by id"
        );
        assert_eq!(
            enter_command(&config.agents[0], "abc", cwd, true, &[]),
            Some(["claude", "--resume", "abc"].map(String::from).to_vec())
        );
        assert_eq!(
            enter_command(&config.agents[1], "abc", cwd, false, &[]),
            Some(["plain", "--resume", "abc"].map(String::from).to_vec()),
            "without one of its own, an agent is entered the one way it has"
        );
        assert_eq!(
            enter_command(
                &config.agents[0],
                "abc",
                cwd,
                true,
                &["--agent".into(), "otto:otto".into()]
            ),
            Some(
                ["claude", "--agent", "otto:otto", "--resume", "abc"]
                    .map(String::from)
                    .to_vec()
            ),
            "the extra flags go straight after the program"
        );
        assert_eq!(
            terminal_command(&config.terminal, &config.agents[0], "abc", cwd, false, &[]),
            Some(
                ["ghostty", "-e", "claude", "--session-id", "abc"]
                    .map(String::from)
                    .to_vec()
            )
        );
    }

    #[test]
    fn parses_agents() {
        let agents = parse(
            r#"
            [[agents]]
            id = "claude"
            name = "Claude"
            command = "claude-agent-acp"
            model = "haiku"
            permissions = "allow"
            env = { CLAUDE_LOG = "1" }
            "#,
        )
        .unwrap()
        .agents;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].model.as_deref(), Some("haiku"));
        assert_eq!(agents[0].permissions, PermissionPolicy::Allow);
        assert_eq!(
            agents[0].env.get("CLAUDE_LOG").map(String::as_str),
            Some("1")
        );
        assert!(agents[0].args.is_empty());
    }

    #[test]
    fn skills_are_off_or_the_claude_plugin_loader() {
        let skills = |value: &str| {
            parse(&format!(
                "[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n{value}"
            ))
            .map(|config| config.agents[0].skills)
        };
        assert_eq!(skills("").unwrap(), SkillDelivery::Off, "off unless asked");
        assert_eq!(skills("skills = false").unwrap(), SkillDelivery::Off);
        assert_eq!(
            skills("skills = \"claude\"").unwrap(),
            SkillDelivery::Claude
        );
        assert!(skills("skills = \"codex\"").is_err());
    }

    /// `skills` takes `false` or `"claude"`; anything else is said to be
    /// wrong rather than leaving the agent silently without them.
    #[test]
    fn an_unknown_skills_setting_is_a_clear_error() {
        let skills = |value: &str| {
            parse(&format!(
                "[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n{value}"
            ))
            .map(|config| config.agents[0].skills)
        };
        assert_eq!(skills("skills = false").unwrap(), SkillDelivery::Off);
        assert_eq!(
            skills("skills = \"claude\"").unwrap(),
            SkillDelivery::Claude
        );
        for value in ["skills = true", "skills = \"briefing\""] {
            let error = skills(value).expect_err(value).to_string();
            assert!(error.contains("expected false or"), "{value}: {error}");
        }
    }

    #[test]
    fn a_colour_is_one_of_the_named_materials() {
        let colour = |value: &str| {
            parse(&format!(
                "[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n{value}"
            ))
            .map(|config| config.agents[0].colour)
        };
        assert_eq!(colour("").unwrap(), None);
        assert_eq!(colour("colour = \"teal\"").unwrap(), Some(Colour::Teal));
        assert_eq!(
            colour("color = \"violet\"").unwrap(),
            Some(Colour::Violet),
            "either spelling"
        );
        let error = format!("{:#}", colour("colour = \"Teal\"").unwrap_err());
        assert!(error.contains("\"Teal\""), "{error}");
        for name in Colour::NAMES {
            assert!(error.contains(name), "{error} should list {name}");
        }
        for name in Colour::NAMES {
            let parsed = colour(&format!("colour = \"{name}\"")).unwrap().unwrap();
            assert_eq!(parsed.name(), name);
        }
    }

    #[test]
    fn idle_agents_stop_after_five_minutes_unless_configured() {
        let timeout = |text: &str| parse(text).unwrap().idle_timeout();
        assert_eq!(timeout(""), Some(Duration::from_secs(300)));
        assert_eq!(timeout("idle_timeout = 60"), Some(Duration::from_secs(60)));
        assert_eq!(timeout("idle_timeout = 0"), None, "0 keeps agents running");
    }

    #[test]
    fn permissions_default_to_deny() {
        let agents = parse("[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n")
            .unwrap()
            .agents;
        assert_eq!(agents[0].permissions, PermissionPolicy::Deny);
    }

    #[test]
    fn rejects_misspelled_keys() {
        assert!(parse("[[agents]]\nid = \"a\"\nname = \"A\"\ncomand = \"a\"\n").is_err());
        assert!(parse("[[agent]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n").is_err());
    }

    #[test]
    fn falls_back_to_claude_and_rejects_duplicate_ids() {
        let dir = std::env::temp_dir().join(format!("otto-agents-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let empty = dir.join("empty.toml");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(
            load(Some(&empty)).unwrap().agents,
            vec![AgentConfig::claude()]
        );

        let duplicated = dir.join("duplicated.toml");
        let agent = "[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n";
        std::fs::write(&duplicated, format!("{agent}{agent}")).unwrap();
        assert!(load(Some(&duplicated)).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_default_agent_is_listed_first_and_must_exist() {
        let dir = std::env::temp_dir().join(format!("otto-agents-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let agents = "[[agents]]\nid = \"a\"\nname = \"A\"\ncommand = \"a\"\n\
                      [[agents]]\nid = \"b\"\nname = \"B\"\ncommand = \"b\"\n";

        let chosen = dir.join("chosen.toml");
        std::fs::write(&chosen, format!("default_agent = \"b\"\n{agents}")).unwrap();
        let ids: Vec<_> = load(Some(&chosen))
            .unwrap()
            .agents
            .into_iter()
            .map(|agent| agent.id)
            .collect();
        assert_eq!(ids, ["b", "a"]);

        let unset = dir.join("unset.toml");
        std::fs::write(&unset, agents).unwrap();
        assert_eq!(
            load(Some(&unset)).unwrap().agents[0].id,
            "a",
            "first listed otherwise"
        );

        let missing = dir.join("missing.toml");
        std::fs::write(&missing, format!("default_agent = \"c\"\n{agents}")).unwrap();
        assert!(load(Some(&missing)).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_token_in_args_is_replaced_by_the_file() {
        let dir = std::env::temp_dir().join(format!("otto-agents-args-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otto.md");
        std::fs::write(&file, "You are Otto.\n\n").unwrap();
        let token = format!("{{file:{}}}", file.display());

        let args = vec![
            "-c".to_owned(),
            format!("developer_instructions={token}"),
            "plain".to_owned(),
            format!("{{file:{}}}", dir.join("missing.md").display()),
            "{file:unclosed".to_owned(),
        ];
        assert_eq!(
            expand_args(&args),
            [
                "-c",
                "developer_instructions=You are Otto.",
                "plain",
                "",
                "{file:unclosed",
            ],
            "the text trimmed at the end; a missing file stands for nothing; a token that never closes is left as written"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }
}
