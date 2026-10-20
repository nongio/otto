//! What setup would do: from the [`Facts`] and the harnesses picked, the
//! steps to run and, for each, exactly what it writes or runs.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;

use otto_agents_client::harness::{self, Harness};
use toml_edit::{value, DocumentMut, Item, Table};

use crate::probe::{Facts, Unit, SERVICE};

/// The harnesses setup offers, in the order it lists them.
pub const HARNESSES: [Harness; 5] = [
    Harness::Claude,
    Harness::Codex,
    Harness::OpenCode,
    Harness::Hermes,
    Harness::Pi,
];

/// Where a harness stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Already in the agent list.
    Configured,
    /// Installed, with what it was found as.
    Installed(PathBuf),
    /// Something it needs is not on the PATH.
    Missing(&'static str),
}

/// The programs a harness needs on the PATH, its own last.
pub fn needs(harness: Harness) -> Vec<&'static str> {
    let adapter = match harness {
        // The adapters are npm packages `npx` runs.
        Harness::Claude | Harness::Codex => Some("npx"),
        _ => None,
    };
    adapter.into_iter().chain(harness.program()).collect()
}

/// Where `harness` stands on this machine.
pub fn readiness(facts: &Facts, harness: Harness) -> Readiness {
    if facts.configured.contains(&harness) {
        return Readiness::Configured;
    }
    let needs = needs(harness);
    if let Some(missing) = needs.iter().find(|name| !facts.found.contains_key(**name)) {
        return Readiness::Missing(missing);
    }
    let own = needs
        .last()
        .and_then(|name| facts.found.get(*name))
        .cloned();
    Readiness::Installed(own.unwrap_or_default())
}

/// The harnesses ticked to begin with: every installed one, when the list is
/// empty and so Otto has nothing of the user's yet; none otherwise, since the
/// user has already chosen.
pub fn initial_picks(facts: &Facts) -> Vec<Harness> {
    if !facts.configured.is_empty() {
        return Vec::new();
    }
    HARNESSES
        .into_iter()
        .filter(|h| matches!(readiness(facts, *h), Readiness::Installed(_)))
        .collect()
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Write `text` to `path`, replacing it whole.
    Write { path: PathBuf, text: String },
    /// Run a command; `args[0]` is the program.
    Run { args: Vec<String> },
}

impl Action {
    fn run(args: &[&str]) -> Self {
        Action::Run {
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }
}

/// Where a step stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing to do.
    Done,
    /// Will run on Apply.
    Todo,
    /// Cannot run; the user has to do something first.
    Blocked,
}

/// One thing setup does, or would do.
#[derive(Debug, Clone)]
pub struct Step {
    /// What it is, in a few words.
    pub title: &'static str,
    /// Where it stands.
    pub state: State,
    /// One line on why, or what it will change.
    pub summary: String,
    /// Several lines on exactly what it writes or runs, for the preview.
    pub detail: String,
    /// What Apply does, in order.
    pub actions: Vec<Action>,
}

/// The steps for the harnesses `picks` (those already configured are skipped).
pub fn steps(facts: &Facts, picks: &[Harness]) -> Vec<Step> {
    let adding: Vec<Harness> = picks
        .iter()
        .copied()
        .filter(|h| !facts.configured.contains(h))
        .collect();
    let mut steps = vec![config_step(facts, &adding)];
    steps.push(path_step(facts, &adding));
    if adding.contains(&Harness::Hermes) || facts.configured.contains(&Harness::Hermes) {
        steps.push(hermes_step(facts));
    }
    // A change the service reads at startup; the plugins are read by the
    // agents themselves, so they need no restart.
    let changed = steps.iter().any(|step| step.state == State::Todo);
    steps.push(plugins_step(facts, changed));
    steps.push(service_step(facts, changed));
    steps
}

fn config_step(facts: &Facts, adding: &[Harness]) -> Step {
    let shown = facts.shown(&facts.config_path);
    if let Some(error) = &facts.config_error {
        return Step {
            title: "Agent list",
            state: State::Blocked,
            summary: format!("cannot read it: fix {shown} first"),
            detail: error.clone(),
            actions: Vec::new(),
        };
    }
    if adding.is_empty() {
        let summary = if facts.configured.is_empty() {
            "nothing picked: tick an agent above".to_owned()
        } else {
            match facts.configured.len() {
                1 => format!("1 agent in {shown}"),
                n => format!("{n} agents in {shown}"),
            }
        };
        return Step {
            title: "Agent list",
            state: if facts.configured.is_empty() {
                State::Blocked
            } else {
                State::Done
            },
            summary,
            detail: facts.user_config.clone(),
            actions: Vec::new(),
        };
    }
    match render_config(facts, adding) {
        Ok(text) => Step {
            title: "Agent list",
            state: State::Todo,
            summary: format!("add {} to {shown}", names(adding)),
            detail: text.clone(),
            actions: vec![Action::Write {
                path: facts.config_path.clone(),
                text,
            }],
        },
        Err(error) => Step {
            title: "Agent list",
            state: State::Blocked,
            summary: format!("cannot update {shown}"),
            detail: error,
            actions: Vec::new(),
        },
    }
}

/// The user's `agents.toml` with an agent for each of `adding` appended.
///
/// Edited with `toml_edit`, so comments and keys setup does not know stay as
/// they were. When the agents came from the system file, its list is copied
/// in first: a user file that lists agents replaces the system's whole.
pub fn render_config(facts: &Facts, adding: &[Harness]) -> Result<String, String> {
    let mut doc = facts
        .user_config
        .parse::<DocumentMut>()
        .map_err(|err| err.to_string())?;
    if facts.user_config.trim().is_empty() {
        doc.decor_mut().set_prefix(
            "# The agents Otto runs, one [[agents]] block each. Settings › Agents\n\
             # edits them too; docs/user/agents.md has every key.\n\n",
        );
    }
    let listed = doc
        .get("agents")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|list| !list.is_empty());
    if !listed {
        let copied = facts
            .system_config
            .as_deref()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .and_then(|system| {
                system
                    .get("agents")
                    .and_then(Item::as_array_of_tables)
                    .cloned()
            })
            .unwrap_or_default();
        doc.insert("agents", Item::ArrayOfTables(copied));
    }
    let list = doc
        .get_mut("agents")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or("`agents` is not a list of tables")?;
    let home = facts.home.display().to_string();
    for &harness in adding {
        let taken: Vec<String> = list
            .iter()
            .filter_map(|table| table.get("id").and_then(Item::as_str).map(str::to_owned))
            .collect();
        let id = unique(harness.id(), &taken);
        let mut table = Table::new();
        table.insert("id", value(&id));
        table.insert("name", value(harness.name().unwrap_or(harness.id())));
        let command = harness.command(harness::OTTO);
        if let Some((program, args)) = command.split_first() {
            table.insert("command", value(program));
            harness::set_list(&mut table, "args", args);
        }
        // The same as an agent added in Settings: one the user is about to
        // try should ask rather than refuse.
        table.insert("permissions", value("ask"));
        harness.write(&mut table, harness::OTTO, &home);
        list.push(table);
    }
    Ok(doc.to_string())
}

/// `base`, or `base-2`, `base-3`… whichever is not in `taken`.
fn unique(base: &str, taken: &[String]) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                base.to_owned()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|id| !taken.contains(id))
        .unwrap_or_else(|| base.to_owned())
}

fn path_step(facts: &Facts, adding: &[Harness]) -> Step {
    let title = "Service PATH";
    let Some(service_path) = &facts.service_path else {
        return Step {
            title,
            state: State::Done,
            summary: "no systemd user manager: agents run with your session's PATH".into(),
            detail: String::new(),
            actions: Vec::new(),
        };
    };
    // What the service starts: each agent's command, as listed and as about
    // to be added. A harness's own CLI (`claude --resume`) runs in the user's
    // terminal, not here. A program the shell has not got either is the
    // harness row's problem, not this step's; a full path needs no PATH.
    let new_commands: Vec<String> = adding
        .iter()
        .filter_map(|h| h.command(harness::OTTO).into_iter().next())
        .collect();
    let mut programs: Vec<&str> = facts
        .commands
        .iter()
        .chain(&new_commands)
        .map(String::as_str)
        .filter(|program| !program.contains('/'))
        .collect();
    if programs.contains(&"npx") {
        // npx is a Node script: `#!/usr/bin/env node`.
        programs.push("node");
    }
    programs.sort_unstable();
    programs.dedup();
    let mut missing_dirs: Vec<PathBuf> = Vec::new();
    let mut missing_programs: Vec<&str> = Vec::new();
    for program in programs {
        let Some(found) = facts.found.get(program) else {
            continue;
        };
        if facts.service_found.contains_key(program) {
            continue;
        }
        missing_programs.push(program);
        if let Some(dir) = found.parent() {
            if !missing_dirs.iter().any(|d| d == dir) {
                missing_dirs.push(dir.to_owned());
            }
        }
    }
    let dropin = facts.shown(&facts.dropin_path);
    if missing_dirs.is_empty() {
        return Step {
            title,
            state: State::Done,
            summary: "the service finds every agent".into(),
            detail: facts.dropin.clone().unwrap_or_default(),
            actions: Vec::new(),
        };
    }
    let full: Vec<PathBuf> = missing_dirs.iter().chain(service_path).cloned().collect();
    let joined = std::env::join_paths(&full)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = format!(
        "# Written by otto-setup. The service does not get your login shell's\n\
         # PATH, so the folders the agents live in are added here.\n\
         [Service]\n\
         Environment=\"PATH={joined}\"\n"
    );
    let shown_dirs: Vec<String> = missing_dirs.iter().map(|d| facts.shown(d)).collect();
    Step {
        title,
        state: State::Todo,
        summary: format!(
            "{} not found by the service: add {}",
            missing_programs.join(", "),
            shown_dirs.join(", ")
        ),
        detail: format!("{dropin}\n\n{text}\nthen: systemctl --user daemon-reload"),
        actions: vec![
            Action::Write {
                path: facts.dropin_path.clone(),
                text,
            },
            Action::run(&["systemctl", "--user", "daemon-reload"]),
        ],
    }
}

fn hermes_step(facts: &Facts) -> Step {
    let title = "Hermes profile";
    if facts.hermes_profile {
        return Step {
            title,
            state: State::Done,
            summary: format!("~/.hermes/profiles/{} exists", harness::OTTO),
            detail: String::new(),
            actions: Vec::new(),
        };
    }
    Step {
        title,
        state: State::Todo,
        summary: format!("create the {} profile Hermes runs Otto in", harness::OTTO),
        detail: format!("hermes profile create {}", harness::OTTO),
        actions: vec![Action::run(&["hermes", "profile", "create", harness::OTTO])],
    }
}

fn plugins_step(facts: &Facts, changed: bool) -> Step {
    let title = "Otto's knowledge";
    if !facts.found.contains_key("otto-agents") {
        return Step {
            title,
            state: State::Blocked,
            summary: "otto-agents is not installed".into(),
            detail: String::new(),
            actions: Vec::new(),
        };
    }
    // Run again with any change: a new agent needs its rendering.
    if facts.skills_linked && !changed {
        return Step {
            title,
            state: State::Done,
            summary: "skills linked; `otto-agents plugins status` has the details".into(),
            detail: String::new(),
            actions: Vec::new(),
        };
    }
    Step {
        title,
        state: State::Todo,
        summary: "link the skills and write Otto's instructions for each agent".into(),
        detail: "otto-agents plugins install\n\n\
                 Links the desktop's skills into ~/.agents/skills and writes the\n\
                 Otto instructions in each harness's dialect. Safe to run again:\n\
                 it only writes what changed."
            .into(),
        actions: vec![Action::run(&["otto-agents", "plugins", "install"])],
    }
}

fn service_step(facts: &Facts, changed: bool) -> Step {
    let title = "Agent service";
    match facts.unit {
        Unit::Unmanaged => Step {
            title,
            state: State::Blocked,
            summary: "no systemd user manager: run `otto-agents serve` in your session".into(),
            detail: String::new(),
            actions: Vec::new(),
        },
        Unit::Missing => Step {
            title,
            state: State::Blocked,
            summary: format!("{SERVICE} is not installed"),
            detail: String::new(),
            actions: Vec::new(),
        },
        Unit::Loaded {
            enabled: true,
            active: true,
        } if !changed => Step {
            title,
            state: State::Done,
            summary: "running, and starts at login".into(),
            detail: String::new(),
            actions: Vec::new(),
        },
        Unit::Loaded { enabled, active } => {
            let mut actions = Vec::new();
            let mut what = Vec::new();
            if !enabled {
                what.push("start at login");
            }
            if active {
                what.push("restart to read the changes");
                actions.push(Action::run(&["systemctl", "--user", "restart", SERVICE]));
            } else {
                what.push("start now");
            }
            if !enabled || !active {
                actions.insert(
                    0,
                    Action::run(&["systemctl", "--user", "enable", "--now", SERVICE]),
                );
            }
            let detail = actions
                .iter()
                .filter_map(|action| match action {
                    Action::Run { args } => Some(args.join(" ")),
                    Action::Write { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Step {
                title,
                state: State::Todo,
                summary: what.join(", "),
                detail,
                actions,
            }
        }
    }
}

/// Harness names joined for a sentence.
fn names(harnesses: &[Harness]) -> String {
    harnesses
        .iter()
        .map(|h| h.name().unwrap_or(h.id()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Unit;
    use std::collections::BTreeMap;

    fn facts() -> Facts {
        let nvm = PathBuf::from("/home/u/.nvm/bin");
        let local = PathBuf::from("/home/u/.local/bin");
        let mut found = BTreeMap::new();
        found.insert("npx".to_owned(), nvm.join("npx"));
        found.insert("node".to_owned(), nvm.join("node"));
        found.insert("claude".to_owned(), local.join("claude"));
        found.insert("opencode".to_owned(), local.join("opencode"));
        found.insert(
            "otto-agents".to_owned(),
            PathBuf::from("/usr/bin/otto-agents"),
        );
        Facts {
            home: PathBuf::from("/home/u"),
            config_path: PathBuf::from("/home/u/.config/otto/agents.toml"),
            user_config: String::new(),
            system_config: None,
            config_error: None,
            configured: Vec::new(),
            commands: Vec::new(),
            found,
            service_path: Some(vec![PathBuf::from("/usr/bin")]),
            service_found: BTreeMap::new(),
            dropin_path: PathBuf::from(
                "/home/u/.config/systemd/user/otto-agents.service.d/path.conf",
            ),
            dropin: None,
            unit: Unit::Loaded {
                enabled: false,
                active: false,
            },
            hermes_profile: false,
            skills_linked: false,
        }
    }

    #[test]
    fn a_fresh_machine_picks_what_is_installed() {
        let facts = facts();
        assert_eq!(initial_picks(&facts), [Harness::Claude, Harness::OpenCode]);
        assert_eq!(
            readiness(&facts, Harness::Codex),
            Readiness::Missing("codex")
        );
        let mut configured = facts.clone();
        configured.configured = vec![Harness::OpenCode];
        assert!(initial_picks(&configured).is_empty());
    }

    #[test]
    fn adds_an_agent_the_way_settings_does() {
        let mut facts = facts();
        facts.user_config = "# mine\nidle_timeout = 0\n".into();
        let text = render_config(&facts, &[Harness::Claude]).unwrap();
        assert!(text.starts_with("# mine\nidle_timeout = 0\n"));
        let doc: DocumentMut = text.parse().unwrap();
        let agent = &doc["agents"][0];
        assert_eq!(agent["id"].as_str(), Some("claude"));
        assert_eq!(agent["command"].as_str(), Some("npx"));
        assert_eq!(agent["agent"].as_str(), Some("otto"));
        assert_eq!(agent["skills"].as_str(), Some("claude"));
        assert_eq!(agent["permissions"].as_str(), Some("ask"));
    }

    #[test]
    fn keeps_the_system_list_and_ids_unique() {
        let mut facts = facts();
        facts.system_config = Some(
            "[[agents]]\nid = \"claude\"\nname = \"Work\"\ncommand = \"/opt/claude-acp\"\n".into(),
        );
        let text = render_config(&facts, &[Harness::Claude]).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        let ids: Vec<_> = doc["agents"]
            .as_array_of_tables()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["claude", "claude-2"]);
    }

    #[test]
    fn adds_the_folders_the_service_cannot_see() {
        let facts = facts();
        let steps = steps(&facts, &[Harness::Claude]);
        let path = steps.iter().find(|s| s.title == "Service PATH").unwrap();
        assert_eq!(path.state, State::Todo);
        let Action::Write { text, .. } = &path.actions[0] else {
            panic!("expected a write");
        };
        assert!(text.contains("Environment=\"PATH=/home/u/.nvm/bin:/usr/bin\""));
        let service = steps.last().unwrap();
        assert_eq!(
            service.actions,
            [Action::run(&[
                "systemctl",
                "--user",
                "enable",
                "--now",
                SERVICE
            ])]
        );
    }

    #[test]
    fn a_running_service_restarts_only_for_a_change() {
        let mut facts = facts();
        facts.configured = vec![Harness::Claude, Harness::OpenCode];
        facts.commands = vec!["npx".into(), "opencode".into()];
        facts.service_found = facts.found.clone();
        facts.skills_linked = true;
        facts.unit = Unit::Loaded {
            enabled: true,
            active: true,
        };
        let steps = steps(&facts, &[]);
        assert!(steps.iter().all(|s| s.state != State::Todo), "{steps:#?}");
    }
}
