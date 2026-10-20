//! What the machine looks like now: which harnesses are installed, what
//! `agents.toml` lists, the PATH the service runs with, and the unit's state.
//!
//! Reading is kept apart from deciding ([`crate::plan`]) so the plan can be
//! tested against made-up machines.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use otto_agents_client::harness::{self, Harness};
use otto_foundations::xdg;
use toml_edit::{DocumentMut, Item};

/// The systemd user unit that runs the agents.
pub const SERVICE: &str = "otto-agents.service";

/// The programs setup always looks for: every harness's own, and what runs
/// the Claude Code and Codex adapters. The commands the agent list names are
/// looked for too.
pub const PROGRAMS: [&str; 8] = [
    "npx",
    "node",
    "claude",
    "codex",
    "opencode",
    "hermes",
    "pi-acp",
    "otto-agents",
];

/// The service unit as systemd reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// systemd knows the unit.
    Loaded { enabled: bool, active: bool },
    /// No such unit: otto-agents is not installed.
    Missing,
    /// No user manager to ask: a session without systemd.
    Unmanaged,
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
    /// Where each program setup looks for is on the shell's PATH.
    pub found: BTreeMap<String, PathBuf>,
    /// The PATH the service starts with: the unit's own when it sets one,
    /// else the user manager's; `None` without a user manager.
    pub service_path: Option<Vec<PathBuf>>,
    /// Where the service finds each of those programs on that PATH.
    pub service_found: BTreeMap<String, PathBuf>,
    /// The drop-in setup writes for the PATH, as it stands.
    pub dropin_path: PathBuf,
    /// Its text, when there is one.
    pub dropin: Option<String>,
    /// The unit's state.
    pub unit: Unit,
    /// Whether the Hermes profile Otto's agent runs in exists.
    pub hermes_profile: bool,
    /// Whether Otto's help skill is linked where agents look for skills,
    /// the sign `otto-agents plugins install` has run.
    pub skills_linked: bool,
}

impl Facts {
    /// Reads the machine.
    pub fn read() -> Self {
        let home = xdg::home().unwrap_or_default();
        let config_home = xdg::config_home().unwrap_or_else(|| home.join(".config"));
        let config_path = config_home.join("otto").join("agents.toml");
        let user_config = std::fs::read_to_string(&config_path).unwrap_or_default();
        let system_config = std::fs::read_to_string("/etc/otto/agents.toml").ok();
        let (agents, config_error) = configured(&user_config, system_config.as_deref());
        let (configured, commands): (Vec<Harness>, Vec<String>) = agents.into_iter().unzip();
        let programs: Vec<&str> = PROGRAMS
            .into_iter()
            .chain(commands.iter().map(String::as_str))
            .filter(|program| !program.contains('/'))
            .collect();
        let shell_path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default();
        let found = found_in(&programs, &shell_path);
        let service_path = service_path();
        let service_found = service_path
            .as_deref()
            .map(|dirs| found_in(&programs, dirs))
            .unwrap_or_default();
        // A name of its own, so a drop-in the user wrote (`systemctl edit`
        // writes `override.conf`) is never overwritten.
        let dropin_path = config_home
            .join("systemd/user")
            .join(format!("{SERVICE}.d"))
            .join("otto-setup.conf");
        let dropin = std::fs::read_to_string(&dropin_path).ok();
        let hermes_profile = home.join(".hermes/profiles").join(harness::OTTO).is_dir();
        let skills_linked = home.join(".agents/skills/otto-help").exists();
        Self {
            home,
            config_path,
            user_config,
            system_config,
            config_error,
            configured,
            commands,
            found,
            service_path,
            service_found,
            dropin_path,
            dropin,
            unit: unit(),
            hermes_profile,
            skills_linked,
        }
    }

    /// A path as setup shows it, with the home folder as `~`.
    pub fn shown(&self, path: &Path) -> String {
        xdg::tilde_in(path, Some(&self.home))
    }
}

/// The harness and command of each listed agent, read the way the service
/// reads the list: the user's when it has one, else the system's. With no
/// list at all the service runs Claude Code, but nothing is configured.
fn configured(user: &str, system: Option<&str>) -> (Vec<(Harness, String)>, Option<String>) {
    let mut error = None;
    let mut list = |text: &str, name: &str| -> Option<Vec<(Harness, String)>> {
        let doc = match text.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(err) => {
                error.get_or_insert_with(|| format!("{name}: {err}"));
                return None;
            }
        };
        let agents = doc.get("agents").and_then(Item::as_array_of_tables)?;
        let harnesses: Vec<(Harness, String)> = agents
            .iter()
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
        (!harnesses.is_empty()).then_some(harnesses)
    };
    let found = list(user, "agents.toml")
        .or_else(|| system.and_then(|text| list(text, "/etc/otto/agents.toml")))
        .unwrap_or_default();
    (found, error)
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

fn service_path() -> Option<Vec<PathBuf>> {
    let unit = Command::new("systemctl")
        .args(["--user", "show", "-p", "Environment", SERVICE])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    if let Some(path) = parse_unit_path(&String::from_utf8_lossy(&unit.stdout)) {
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

/// The unit's state from `systemctl show -p LoadState,ActiveState,UnitFileState`.
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

fn unit() -> Unit {
    match Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState,ActiveState,UnitFileState",
            SERVICE,
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
[[agents]]
id = "c"
name = "Claude"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
"#;
        assert_eq!(
            configured(user, Some(system)).0,
            [(Harness::OpenCode, "opencode".to_owned())]
        );
        assert_eq!(
            configured("", Some(system)).0,
            [(Harness::Claude, "npx".to_owned())]
        );
        assert_eq!(configured("idle_timeout = 0", None).0, []);
    }

    #[test]
    fn a_broken_file_is_reported() {
        let (list, error) = configured("[[agents]\n", None);
        assert!(list.is_empty());
        assert!(error.is_some_and(|e| e.starts_with("agents.toml")));
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
        assert_eq!(
            parse_unit("LoadState=loaded\nActiveState=inactive\nUnitFileState=disabled\n"),
            Unit::Loaded {
                enabled: false,
                active: false
            }
        );
    }
}
