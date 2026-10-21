//! What the machine looks like now: which programs are installed and where,
//! what `agents.toml` lists, the PATH each unit runs with, and their state.
//!
//! Reading is kept apart from deciding ([`crate::plan`]) so the plan can be
//! tested against made-up machines.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use otto_agents_client::harness::Harness;
use otto_agents_client::instructions;
use otto_foundations::xdg;

use crate::plugins;
use toml_edit::{DocumentMut, Item};

/// The systemd user unit that runs the agents.
pub const SERVICE: &str = "otto-agents.service";

/// The systemd user unit that runs the chat bridge.
pub const BRIDGE_SERVICE: &str = "otto-agents-bridge.service";

/// The programs setup always looks for: Node, every harness's own program
/// and adapter, cc-connect and otto-agents. The commands the agent list
/// names are looked for too.
pub const PROGRAMS: [&str; 13] = [
    "node",
    "npm",
    "npx",
    "claude",
    "claude-agent-acp",
    "codex",
    "codex-acp",
    "opencode",
    "hermes",
    "pi",
    "pi-acp",
    "cc-connect",
    "otto-agents",
];

/// A unit as systemd reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// systemd knows the unit.
    Loaded { enabled: bool, active: bool },
    /// No such unit: otto-agents is not installed.
    Missing,
    /// No user manager to ask: a session without systemd.
    Unmanaged,
}

/// One systemd unit and the PATH it runs with.
#[derive(Debug, Clone)]
pub struct UnitFacts {
    /// The unit's name.
    pub name: &'static str,
    /// Its state.
    pub state: Unit,
    /// The PATH it starts with: its own when it sets one, else the user
    /// manager's; `None` without a user manager.
    pub path: Option<Vec<PathBuf>>,
    /// Where it finds each program setup looks for, on that PATH.
    pub found: BTreeMap<String, PathBuf>,
    /// The drop-in setup writes for its PATH.
    pub dropin: PathBuf,
}

/// Everything setup decides from.
#[derive(Debug, Clone)]
pub struct Facts {
    /// The home folder.
    pub home: PathBuf,
    /// The user's `agents.toml`, the file setup writes.
    pub config_path: PathBuf,
    /// The user's `agents.toml` as it stands; empty when there is none.
    pub user_config: String,
    /// `/etc/otto/agents.toml`, when there is one.
    pub system_config: Option<String>,
    /// A file that could not be read as TOML, with why. Setup writes nothing
    /// over a file it cannot parse.
    pub config_error: Option<String>,
    /// The harnesses the agent list runs on, in its order.
    pub configured: Vec<Harness>,
    /// The program each listed agent starts, as written.
    pub commands: Vec<String>,
    /// The chat bridge's command, when `[bridge]` names one.
    pub bridge_command: Option<String>,
    /// Where each program setup looks for is on the shell's PATH.
    pub found: BTreeMap<String, PathBuf>,
    /// `node --version`, when Node is installed.
    pub node_version: Option<String>,
    /// The system's package manager, for the instructions.
    pub package_manager: Option<&'static str>,
    /// The instructions agents can run as, by name.
    pub instructions: Vec<String>,
    /// The agent service.
    pub agents_unit: UnitFacts,
    /// The chat bridge.
    pub bridge_unit: UnitFacts,
    /// The Hermes profiles there are. Hermes runs an agent in the profile
    /// named after its instructions.
    pub hermes_profiles: Vec<String>,
    /// What `otto-agents plugins status` says of Otto's skills and each
    /// harness's rendering of the plugin agents; `None` when it cannot run.
    pub plugins: Option<plugins::Status>,
    /// cc-connect's configuration.
    pub cc_config: PathBuf,
    /// Whether it exists.
    pub cc_config_exists: bool,
    /// Whether the cc-connect found is a Node script, installed with npm,
    /// and so needs Node on the bridge's PATH.
    pub cc_connect_node: bool,
}

impl Facts {
    /// Reads the machine.
    pub fn read() -> Self {
        let home = xdg::home().unwrap_or_default();
        let config_home = xdg::config_home().unwrap_or_else(|| home.join(".config"));
        let config_path = config_home.join("otto").join("agents.toml");
        let user_config = std::fs::read_to_string(&config_path).unwrap_or_default();
        let system_config = std::fs::read_to_string("/etc/otto/agents.toml").ok();
        let lists = configured(&user_config, system_config.as_deref());
        let (configured, commands): (Vec<Harness>, Vec<String>) = lists.agents.into_iter().unzip();
        let programs: Vec<&str> = PROGRAMS
            .into_iter()
            .chain(commands.iter().map(String::as_str))
            .filter(|program| !program.contains('/'))
            .collect();
        let shell_path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default();
        let found = found_in(&programs, &shell_path);
        let node_version = found.get("node").and_then(|node| {
            let output = Command::new(node).arg("--version").output().ok()?;
            let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            (!text.is_empty()).then_some(text)
        });
        let package_manager = ["pacman", "apt", "dnf", "zypper"]
            .into_iter()
            .find(|name| find(name, &shell_path).is_some());
        let unit_facts = |name: &'static str| {
            let path = service_path(name);
            let found = path
                .as_deref()
                .map(|dirs| found_in(&programs, dirs))
                .unwrap_or_default();
            // A name of its own, so a drop-in the user wrote (`systemctl
            // edit` writes `override.conf`) is never overwritten.
            let dropin = config_home
                .join("systemd/user")
                .join(format!("{name}.d"))
                .join("otto-setup.conf");
            UnitFacts {
                name,
                state: unit(name),
                path,
                found,
                dropin,
            }
        };
        let cc_config = home.join(".cc-connect/config.toml");
        let cc_connect_node = found
            .get("cc-connect")
            .and_then(|path| std::fs::canonicalize(path).ok())
            .is_some_and(|path| path.components().any(|c| c.as_os_str() == "node_modules"));
        let agents_unit = unit_facts(SERVICE);
        let bridge_unit = unit_facts(BRIDGE_SERVICE);
        Self {
            config_path,
            user_config,
            system_config,
            config_error: lists.error,
            configured,
            commands,
            bridge_command: lists.bridge,
            found,
            node_version,
            package_manager,
            instructions: instructions::discover()
                .into_iter()
                .map(|set| set.name)
                .collect(),
            agents_unit,
            bridge_unit,
            hermes_profiles: std::fs::read_dir(home.join(".hermes/profiles"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect(),
            plugins: plugins::Status::read(),
            cc_config_exists: cc_config.is_file(),
            cc_config,
            cc_connect_node,
            home,
        }
    }

    /// A path as setup shows it, with the home folder as `~`.
    pub fn shown(&self, path: &Path) -> String {
        xdg::tilde_in(path, Some(&self.home))
    }
}

/// What the agent files hold.
#[derive(Debug, Default)]
struct Lists {
    /// Each listed agent's harness and program.
    agents: Vec<(Harness, String)>,
    /// The chat bridge's command.
    bridge: Option<String>,
    /// A file that would not parse.
    error: Option<String>,
}

/// The agents and the bridge, read the way the service reads them: the
/// agent list from the user's file when it has one, else the system's; the
/// bridge from the last file that sets it. With no list at all the service
/// runs Claude Code, but nothing is configured.
fn configured(user: &str, system: Option<&str>) -> Lists {
    let mut lists = Lists::default();
    let mut docs = Vec::new();
    for (text, name) in [
        (system, "/etc/otto/agents.toml"),
        (Some(user), "agents.toml"),
    ] {
        let Some(text) = text else { continue };
        match text.parse::<DocumentMut>() {
            Ok(doc) => docs.push(doc),
            Err(err) => {
                lists.error.get_or_insert_with(|| format!("{name}: {err}"));
            }
        }
    }
    for doc in &docs {
        let agents: Vec<(Harness, String)> = doc
            .get("agents")
            .and_then(Item::as_array_of_tables)
            .into_iter()
            .flatten()
            .map(|table| {
                let command = table.get("command").and_then(Item::as_str);
                let args = table
                    .get("args")
                    .and_then(Item::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|arg| arg.as_str());
                let words: Vec<String> =
                    command.into_iter().chain(args).map(str::to_owned).collect();
                let program = words.first().cloned().unwrap_or_default();
                (Harness::detect(&words), program)
            })
            .collect();
        if !agents.is_empty() {
            lists.agents = agents;
        }
        if let Some(command) = doc
            .get("bridge")
            .and_then(|bridge| bridge.get("command"))
            .and_then(Item::as_str)
        {
            lists.bridge = Some(command.to_owned());
        }
    }
    lists
}

/// Where each of `programs` is in `dirs`.
fn found_in(programs: &[&str], dirs: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    programs
        .iter()
        .filter_map(|name| Some(((*name).to_owned(), find(name, dirs)?)))
        .collect()
}

/// Where `name` is in `dirs`, the first match as a shell would find it.
fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The PATH in `systemctl --user show-environment` output.
pub fn parse_environment_path(environment: &str) -> Option<Vec<PathBuf>> {
    let value = environment
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))?;
    // systemd quotes a value with spaces in it as $'…'.
    let value = value.trim_start_matches("$'").trim_end_matches('\'');
    Some(std::env::split_paths(value).collect())
}

/// The PATH a unit sets for itself, from `systemctl show -p Environment`:
/// space-separated assignments, quoted when one holds a space.
pub fn parse_unit_path(show: &str) -> Option<Vec<PathBuf>> {
    let assignments = show
        .lines()
        .find_map(|line| line.strip_prefix("Environment="))?;
    let value = assignments
        .split(' ')
        .map(|word| word.trim_matches('"'))
        .find_map(|word| word.strip_prefix("PATH="))?;
    Some(std::env::split_paths(value).collect())
}

fn service_path(unit: &str) -> Option<Vec<PathBuf>> {
    let own = Command::new("systemctl")
        .args(["--user", "show", "-p", "Environment", unit])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    if let Some(path) = parse_unit_path(&String::from_utf8_lossy(&own.stdout)) {
        return Some(path);
    }
    let output = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    // A user manager with no PATH starts units with its built-in default.
    parse_environment_path(&String::from_utf8_lossy(&output.stdout)).or_else(|| {
        Some(
            ["/usr/local/bin", "/usr/bin"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        )
    })
}

/// A unit's state from `systemctl show -p LoadState,ActiveState,UnitFileState`.
pub fn parse_unit(show: &str) -> Unit {
    let value = |key: &str| {
        show.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_default()
    };
    if value("LoadState") == "not-found" {
        return Unit::Missing;
    }
    Unit::Loaded {
        enabled: value("UnitFileState").starts_with("enabled"),
        active: matches!(value("ActiveState"), "active" | "reloading" | "activating"),
    }
}

fn unit(name: &str) -> Unit {
    match Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState,ActiveState,UnitFileState",
            name,
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            parse_unit(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Unit::Unmanaged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_list_wins_over_the_system_list() {
        let user = r#"
[[agents]]
id = "o"
name = "OpenCode"
command = "opencode"
args = ["acp"]
"#;
        let system = r#"
[bridge]
command = "/usr/bin/cc-connect"

[[agents]]
id = "c"
name = "Claude"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
"#;
        let lists = configured(user, Some(system));
        assert_eq!(lists.agents, [(Harness::OpenCode, "opencode".to_owned())]);
        assert_eq!(lists.bridge.as_deref(), Some("/usr/bin/cc-connect"));
        assert_eq!(
            configured("", Some(system)).agents,
            [(Harness::Claude, "npx".to_owned())]
        );
        assert_eq!(configured("idle_timeout = 0", None).agents, []);
    }

    #[test]
    fn a_broken_file_is_reported() {
        let lists = configured("[[agents]\n", None);
        assert!(lists.agents.is_empty());
        assert!(lists.error.is_some_and(|e| e.starts_with("agents.toml")));
    }

    #[test]
    fn reads_the_managers_path() {
        let env = "HOME=/home/u\nPATH=/usr/local/bin:/usr/bin\nLANG=C\n";
        assert_eq!(
            parse_environment_path(env),
            Some(vec![
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin")
            ])
        );
        assert_eq!(parse_environment_path("HOME=/home/u\n"), None);
    }

    #[test]
    fn the_units_own_path_is_read() {
        let show = "Environment=LANG=C PATH=/home/u/.local/bin:/usr/bin\n";
        assert_eq!(
            parse_unit_path(show),
            Some(vec![
                PathBuf::from("/home/u/.local/bin"),
                PathBuf::from("/usr/bin")
            ])
        );
        assert_eq!(parse_unit_path("Environment=\n"), None);
    }

    #[test]
    fn reads_the_units_state() {
        assert_eq!(
            parse_unit("LoadState=not-found\nActiveState=inactive\n"),
            Unit::Missing
        );
        assert_eq!(
            parse_unit("LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n"),
            Unit::Loaded {
                enabled: true,
                active: true
            }
        );
    }
}
