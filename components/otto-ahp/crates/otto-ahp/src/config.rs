//! Agent configuration: `[[agents]]` entries in Otto's `agents.toml`.
//!
//! Files load in order, `/etc/otto/agents.toml` then
//! `$XDG_CONFIG_HOME/otto/agents.toml`; the last file that lists agents wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub permissions: PermissionPolicy,
    /// The command that enters one of the agent's sessions in a terminal, in
    /// the agent's own interface, with `{session}` standing for the agent's id
    /// for it and `{cwd}` for its folder, such as
    /// `["claude", "--resume", "{session}"]`. Empty when the agent has none.
    #[serde(default)]
    pub enter: Vec<String>,
}

/// Everything `agents.toml` configures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub agents: Vec<AgentConfig>,
    /// The terminal a session is entered in, with the agent's `enter` command
    /// appended, such as `["ghostty", "--working-directory={cwd}", "-e"]`.
    /// Empty when none is configured.
    pub terminal: Vec<String>,
    /// The agent a session gets when none is asked for. [`load`] lists it
    /// first, which is how clients and `createSession` tell the default.
    pub default_agent: Option<String>,
}

/// The command that opens `agent_session` of `agent` in `terminal`, in `cwd`,
/// when both a terminal and the agent's `enter` command are configured.
pub fn terminal_command(
    terminal: &[String],
    agent: &AgentConfig,
    agent_session: &str,
    cwd: &Path,
) -> Option<Vec<String>> {
    if terminal.is_empty() || agent.enter.is_empty() {
        return None;
    }
    let cwd = cwd.to_string_lossy();
    Some(
        terminal
            .iter()
            .chain(&agent.enter)
            .map(|arg| {
                arg.replace("{session}", agent_session)
                    .replace("{cwd}", &cwd)
            })
            .collect(),
    )
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
                "@agentclientprotocol/claude-agent-acp@latest".into(),
            ],
            env: BTreeMap::new(),
            model: None,
            permissions: PermissionPolicy::Deny,
            enter: vec!["claude".into(), "--resume".into(), "{session}".into()],
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
    })
}

fn default_paths() -> Vec<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let mut paths = vec![PathBuf::from("/etc/otto/agents.toml")];
    paths.extend(config_home.map(|dir| dir.join("otto/agents.toml")));
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
            terminal_command(&config.terminal, &config.agents[0], "abc", cwd),
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
            terminal_command(&config.terminal, &config.agents[1], "abc", cwd),
            None,
            "an agent without a resume command has nothing to open"
        );
        assert_eq!(
            terminal_command(&[], &config.agents[0], "abc", cwd),
            None,
            "nor does a missing terminal"
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
        let dir = std::env::temp_dir().join(format!("otto-ahp-config-{}", uuid::Uuid::new_v4()));
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
        let dir = std::env::temp_dir().join(format!("otto-ahp-config-{}", uuid::Uuid::new_v4()));
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
}
