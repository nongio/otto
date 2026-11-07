//! What setup would do: from the [`Facts`] and the user's [`Choices`], the
//! steps to run and, for each, exactly what it writes or runs.
//!
//! Setup does the simple, local part of setting up: it writes Otto's own
//! files and turns Otto's units on. Installing software, signing in and
//! making bot accounts are the user's, and setup says how (`crate::guide`).
//! What is already installed is used as it is.

// Rust guideline compliant 2026-02-21

use std::path::{Path, PathBuf};

use otto_agents_client::harness::{self, Harness};
use toml_edit::{value, DocumentMut, Item, Table};

#[cfg(test)]
use crate::plugins;
use crate::plugins::{Render, Skill};
use crate::probe::{Facts, Unit, UnitFacts};

/// The harnesses setup offers, in the order it lists them.
pub const HARNESSES: [Harness; 5] = [
    Harness::Claude,
    Harness::Codex,
    Harness::OpenCode,
    Harness::Hermes,
    Harness::Pi,
];

/// What the user picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// Harnesses to add an agent for.
    pub picks: Vec<Harness>,
    /// Whether the agents added answer as Otto, rather than as the plain
    /// harness.
    pub otto: bool,
    /// Whether Studio's chat should work: it needs a Claude Code agent.
    pub studio: bool,
    /// Whether to turn the chat bridge on.
    pub bridge: bool,
}

impl Choices {
    /// What is ticked to begin with: every ready harness when the agent
    /// list is empty, since Otto has nothing of the user's yet, and none
    /// otherwise, since the user has already chosen; Otto, and Studio when
    /// it can work; the bridge off.
    pub fn initial(facts: &Facts) -> Self {
        let picks = if facts.configured.is_empty() {
            HARNESSES
                .into_iter()
                .filter(|h| matches!(readiness(facts, *h), Readiness::Ready(_)))
                .collect()
        } else {
            Vec::new()
        };
        let mut choices = Self {
            picks,
            otto: facts.instructions.iter().any(|n| n == harness::OTTO),
            studio: false,
            bridge: false,
        };
        choices.studio = studio(facts, &choices) == Studio::Ready;
        choices
    }

    /// The plugin agent new agents run as: Otto's, or none for the plain
    /// harness.
    pub fn persona(&self) -> &'static str {
        if self.otto {
            harness::OTTO
        } else {
            ""
        }
    }
}

/// The plugin agent Studio's chat runs as, in Otto's plugin.
pub const STUDIO: &str = "studio";

/// Where Studio's chat stands. Studio asks for its plugin agent on every
/// session it opens, and only Claude Code runs a plugin agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Studio {
    /// Otto's plugin has no Studio agent.
    Unavailable,
    /// A Claude Code agent is in the list, or ticked.
    Ready,
    /// No Claude Code agent: it has to be ticked, or installed first.
    NeedsClaude,
}

/// Where Studio's chat stands with `choices`.
pub fn studio(facts: &Facts, choices: &Choices) -> Studio {
    if !facts.instructions.iter().any(|n| n == STUDIO) {
        Studio::Unavailable
    } else if facts.configured.contains(&Harness::Claude)
        || choices.picks.contains(&Harness::Claude)
    {
        Studio::Ready
    } else {
        Studio::NeedsClaude
    }
}

/// Where a harness stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Already in the agent list.
    Configured,
    /// Everything it needs is installed; its own program is here.
    Ready(PathBuf),
    /// These programs are missing; the guide says how to get them.
    Missing(Vec<&'static str>),
}

/// The programs `harness` is missing on the user's PATH: its own, and the
/// adapter or `npx` to run the adapter with.
pub fn missing(facts: &Facts, harness: Harness) -> Vec<&'static str> {
    let has = |name: &str| facts.found.contains_key(name);
    let mut missing: Vec<&'static str> =
        harness.program().filter(|p| !has(p)).into_iter().collect();
    match (harness, harness.adapter()) {
        // pi only runs through its adapter.
        (Harness::Pi, Some(adapter)) if !has(adapter.bin) => missing.push(adapter.bin),
        (_, Some(adapter)) if !has(adapter.bin) && !has("npx") => missing.push("npx"),
        _ => {}
    }
    missing
}

/// Where `harness` stands on this machine.
pub fn readiness(facts: &Facts, harness: Harness) -> Readiness {
    if facts.configured.contains(&harness) {
        return Readiness::Configured;
    }
    let missing = missing(facts, harness);
    if !missing.is_empty() {
        return Readiness::Missing(missing);
    }
    let own = harness
        .program()
        .and_then(|p| facts.found.get(p))
        .cloned()
        .unwrap_or_default();
    Readiness::Ready(own)
}

/// The command a new agent on `harness` starts: its installed adapter when
/// there is one, so nothing is fetched at every start, else the preset.
pub fn command(facts: &Facts, harness: Harness, persona: &str) -> Vec<String> {
    match harness.adapter() {
        Some(adapter) if facts.found.contains_key(adapter.bin) => vec![adapter.bin.to_owned()],
        _ => harness.command(persona),
    }
}

/// How an agent reaches its ACP adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    /// Installed: it starts at once.
    Installed,
    /// Fetched by `npx` at every start.
    Npx,
    /// Neither: the agent cannot start.
    Missing,
}

/// The ACP adapter of each agent there is or is being added, for the
/// harnesses that need one, in the list's order.
pub fn adapters(facts: &Facts, choices: &Choices) -> Vec<(Harness, Adapter)> {
    selected(facts, choices)
        .into_iter()
        .filter_map(|harness| {
            let adapter = harness.adapter()?;
            let state = if facts.found.contains_key(adapter.bin) {
                Adapter::Installed
            } else if harness != Harness::Pi && facts.found.contains_key("npx") {
                Adapter::Npx
            } else {
                Adapter::Missing
            };
            Some((harness, state))
        })
        .collect()
}

/// The harnesses there are or are being added, in the list's order.
pub fn selected(facts: &Facts, choices: &Choices) -> Vec<Harness> {
    HARNESSES
        .into_iter()
        .filter(|h| facts.configured.contains(h) || choices.picks.contains(h))
        .collect()
}

/// Otto's skills, as `otto-agents plugins status` reports them.
pub fn skills(facts: &Facts) -> &[Skill] {
    facts.plugins.as_ref().map_or(&[], |status| &status.skills)
}

/// Skills only the chat bridge uses: sending to the phone goes through
/// cc-connect.
pub const BRIDGE_SKILLS: [&str; 1] = ["send-to-phone"];

/// Whether `skill` matters with these choices: every skill, but the
/// bridge's only when the bridge runs or is being turned on.
pub fn skill_needed(facts: &Facts, choices: &Choices, skill: &Skill) -> bool {
    !BRIDGE_SKILLS.contains(&skill.name.as_str()) || bridge_on(facts, choices)
}

/// The skills that matter with these choices.
pub fn needed_skills<'a>(facts: &'a Facts, choices: &Choices) -> Vec<&'a Skill> {
    skills(facts)
        .iter()
        .filter(|s| skill_needed(facts, choices, s))
        .collect()
}

/// Whether the chat bridge runs, or will once applied.
pub fn bridge_on(facts: &Facts, choices: &Choices) -> bool {
    let state = bridge(facts);
    state == Bridge::Running || choices.bridge && matches!(state, Bridge::Ready | Bridge::Off)
}

/// The renderings of Otto's agent for the selected harnesses. Claude Code
/// reads the plugin itself and has none. Empty when the agents are plain.
pub fn profiles<'a>(facts: &'a Facts, choices: &Choices) -> Vec<&'a Render> {
    let Some(status) = &facts.plugins else {
        return Vec::new();
    };
    if !choices.otto {
        return Vec::new();
    }
    selected(facts, choices)
        .into_iter()
        .filter(|h| *h != Harness::Claude)
        .flat_map(|h| status.renders_of(h.id(), harness::OTTO).collect::<Vec<_>>())
        .collect()
}

/// Whether `otto-agents plugins install` has something to do for these
/// choices: a skill to link, or a rendering to write. A skill something
/// else is in the way of is left to the user.
pub fn plugins_behind(facts: &Facts, choices: &Choices) -> bool {
    needed_skills(facts, choices)
        .iter()
        .any(|s| !s.linked() && !s.in_the_way())
        || profiles(facts, choices)
            .iter()
            .any(|r| !r.current() && r.state != "skipped")
}

/// The oldest Node.js the adapters run on: the Claude Code adapter asks
/// for 22, the others for less.
pub const NODE_MIN: u32 = 22;

/// Where Node.js stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// Not installed.
    Missing,
    /// Installed, but this major version is below [`NODE_MIN`].
    TooOld(u32),
    /// New enough.
    Ok,
}

/// Where Node.js stands, from `node --version` (`v22.1.0`). A version
/// that does not parse counts as new enough: the user can see it.
pub fn node(facts: &Facts) -> Node {
    let Some(version) = &facts.node_version else {
        return Node::Missing;
    };
    let major = version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    match major {
        Some(major) if major < NODE_MIN => Node::TooOld(major),
        _ => Node::Ok,
    }
}

/// Where the chat bridge stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bridge {
    /// No bridge unit to run it with.
    Unavailable(&'static str),
    /// `[bridge]` is set and its unit runs.
    Running,
    /// `[bridge]` is set but its unit is off.
    Off,
    /// cc-connect and its configuration are there; setup can turn it on.
    Ready,
    /// Something the user sets up is missing; the guide says how.
    Missing(Vec<&'static str>),
}

/// Where the chat bridge stands on this machine.
pub fn bridge(facts: &Facts) -> Bridge {
    match facts.bridge_unit.state {
        Unit::Unmanaged => return Bridge::Unavailable("no systemd user manager"),
        Unit::Missing => return Bridge::Unavailable("otto-agents-bridge.service is not installed"),
        Unit::Loaded { active, .. } if facts.bridge_command.is_some() => {
            return if active { Bridge::Running } else { Bridge::Off };
        }
        Unit::Loaded { .. } => {}
    }
    let mut missing = Vec::new();
    if !facts.found.contains_key("cc-connect") {
        missing.push("cc-connect");
    }
    if !facts.cc_config_exists {
        missing.push("its configuration");
    }
    if missing.is_empty() {
        Bridge::Ready
    } else {
        Bridge::Missing(missing)
    }
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
    pub(crate) fn run(args: &[&str]) -> Self {
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

impl Step {
    pub(crate) fn done(title: &'static str, summary: impl Into<String>) -> Self {
        Self {
            title,
            state: State::Done,
            summary: summary.into(),
            detail: String::new(),
            actions: Vec::new(),
        }
    }

    pub(crate) fn blocked(title: &'static str, summary: impl Into<String>) -> Self {
        Self {
            state: State::Blocked,
            ..Self::done(title, summary)
        }
    }
}

/// The steps for `choices`.
pub fn steps(facts: &Facts, choices: &Choices) -> Vec<Step> {
    let adding: Vec<Harness> = choices
        .picks
        .iter()
        .copied()
        .filter(|h| matches!(readiness(facts, *h), Readiness::Ready(_)))
        .collect();
    let bridge_state = bridge(facts);
    let set_bridge = choices.bridge && bridge_state == Bridge::Ready;
    let bridge_on = bridge_on(facts, choices);

    let mut steps = vec![config_step(facts, choices, &adding, set_bridge)];

    // What the agent service starts: each agent's command, as listed and as
    // about to be added. A harness's own CLI (`claude --resume`) runs in the
    // user's terminal, not here.
    let new_commands: Vec<String> = adding
        .iter()
        .filter_map(|h| command(facts, *h, choices.persona()).into_iter().next())
        .collect();
    let commands: Vec<&str> = facts
        .commands
        .iter()
        .chain(&new_commands)
        .map(String::as_str)
        .collect();
    let mut programs: Vec<&str> = commands
        .iter()
        .copied()
        .filter(|p| !p.contains('/'))
        .collect();
    // npx and the ACP adapters are Node scripts: `#!/usr/bin/env node`.
    if commands.iter().any(|p| is_node_script(p)) {
        programs.push("node");
    }
    steps.push(path_step(
        "Program paths",
        facts,
        &facts.agents_unit,
        programs,
    ));
    if bridge_on {
        let mut programs = Vec::new();
        if facts.cc_connect_node {
            programs.push("node");
        }
        steps.push(path_step(
            "Bridge paths",
            facts,
            &facts.bridge_unit,
            programs,
        ));
    }

    // Hermes runs an agent in the profile named after its instructions; one
    // that runs as itself uses its default profile.
    if adding.contains(&Harness::Hermes) && choices.otto {
        steps.push(hermes_step(facts, choices.persona()));
    }
    // A change the services read at startup; the plugins are read by the
    // agents themselves, so they need no restart.
    let changed = steps.iter().any(|step| step.state == State::Todo);
    steps.push(plugins_step(
        facts,
        changed || plugins_behind(facts, choices),
    ));
    steps.push(unit_step(
        "Agent service",
        &facts.agents_unit,
        true,
        changed,
    ));
    if bridge_on {
        steps.push(unit_step("Chat bridge", &facts.bridge_unit, true, changed));
    }
    steps
}

/// Whether `program` is one of the Node scripts agents run through.
fn is_node_script(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    name == "npx" || name.ends_with("-acp")
}

fn config_step(facts: &Facts, choices: &Choices, adding: &[Harness], set_bridge: bool) -> Step {
    let title = "Agents";
    let shown = facts.shown(&facts.config_path);
    if let Some(error) = &facts.config_error {
        return Step {
            detail: error.clone(),
            ..Step::blocked(title, format!("{shown} cannot be read: fix it first"))
        };
    }
    if adding.is_empty() && !set_bridge {
        return match facts.configured.len() {
            0 => Step::blocked(title, "no agent ticked: tick one on the Agents page"),
            1 => Step::done(title, "1 agent added".to_owned()),
            n => Step::done(title, format!("{n} agents added")),
        };
    }
    let mut what = Vec::new();
    if !adding.is_empty() {
        what.push(format!("add {}", names(adding)));
    }
    if set_bridge {
        what.push("turn on the chat bridge".to_owned());
    }
    match render_config(facts, choices, adding, set_bridge) {
        Ok(text) => Step {
            title,
            state: State::Todo,
            summary: what.join(", "),
            detail: format!(
                "Writes {shown}:\n{}",
                text.lines()
                    .map(|line| format!("│ {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            actions: vec![Action::Write {
                path: facts.config_path.clone(),
                text,
            }],
        },
        Err(error) => Step {
            detail: error,
            ..Step::blocked(title, format!("{shown} cannot be updated"))
        },
    }
}

/// The user's `agents.toml` with an agent for each of `adding` appended,
/// and the bridge set when `set_bridge`.
///
/// Edited with `toml_edit`, so comments and keys setup does not know stay as
/// they were. When the agents came from the system file, its list is copied
/// in first: a user file that lists agents replaces the system's whole.
pub fn render_config(
    facts: &Facts,
    choices: &Choices,
    adding: &[Harness],
    set_bridge: bool,
) -> Result<String, String> {
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
    if set_bridge {
        let cc_connect = facts
            .found
            .get("cc-connect")
            .ok_or("cc-connect is not installed")?;
        let mut bridge = Table::new();
        // Full paths: the unit does not get the shell's PATH.
        bridge.insert("command", value(cc_connect.display().to_string()));
        harness::set_list(
            &mut bridge,
            "args",
            &["--config".to_owned(), facts.cc_config.display().to_string()],
        );
        doc.insert("bridge", Item::Table(bridge));
    }
    if adding.is_empty() {
        return Ok(doc.to_string());
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
        let command = command(facts, harness, choices.persona());
        if let Some((program, args)) = command.split_first() {
            table.insert("command", value(program));
            harness::set_list(&mut table, "args", args);
        }
        // The same as an agent added in Settings: one the user is about to
        // try should ask rather than refuse.
        table.insert("permissions", value("ask"));
        harness.write(&mut table, choices.persona(), &home);
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

/// Adds to `unit`'s PATH the folders it needs for `programs` that the
/// shell finds but the unit does not. A program the shell has not got
/// either is for the guide, not this step.
fn path_step(
    title: &'static str,
    facts: &Facts,
    unit: &UnitFacts,
    mut programs: Vec<&str>,
) -> Step {
    let noun = if unit.name == crate::probe::BRIDGE_SERVICE {
        "bridge"
    } else {
        "service"
    };
    let Some(service_path) = &unit.path else {
        return Step::done(title, "no systemd user session: uses your session's paths");
    };
    programs.sort_unstable();
    programs.dedup();
    let mut missing_dirs: Vec<PathBuf> = Vec::new();
    let mut missing_programs: Vec<&str> = Vec::new();
    for program in programs {
        let Some(found) = facts.found.get(program) else {
            continue;
        };
        if unit.found.contains_key(program) {
            continue;
        }
        missing_programs.push(program);
        if let Some(dir) = found.parent() {
            if !missing_dirs.iter().any(|d| d == dir) {
                missing_dirs.push(dir.to_owned());
            }
        }
    }
    if missing_dirs.is_empty() {
        return Step::done(title, format!("the {noun} finds every program"));
    }
    let full: Vec<PathBuf> = missing_dirs.iter().chain(service_path).cloned().collect();
    let joined = std::env::join_paths(&full)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = format!(
        "# Written by otto-setup. The unit does not get your login shell's\n\
         # PATH, so the folders the programs it runs live in are added here.\n\
         [Service]\n\
         Environment=\"PATH={joined}\"\n"
    );
    let shown_dirs: Vec<String> = missing_dirs.iter().map(|d| facts.shown(d)).collect();
    Step {
        title,
        state: State::Todo,
        summary: format!(
            "{noun} cannot find {}: add {}",
            missing_programs.join(", "),
            shown_dirs.join(", ")
        ),
        detail: format!(
            "Writes {}:\n{}\nThen:\n$ systemctl --user daemon-reload",
            facts.shown(&unit.dropin),
            text.lines()
                .map(|line| format!("│ {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        actions: vec![
            Action::Write {
                path: unit.dropin.clone(),
                text,
            },
            Action::run(&["systemctl", "--user", "daemon-reload"]),
        ],
    }
}

fn hermes_step(facts: &Facts, persona: &str) -> Step {
    let title = "Hermes profile";
    if facts.hermes_profiles.iter().any(|p| p == persona) {
        return Step::done(title, "already created".to_owned());
    }
    Step {
        title,
        state: State::Todo,
        summary: format!("create the {persona} profile in Hermes"),
        detail: format!("$ hermes profile create {persona}"),
        actions: vec![Action::run(&["hermes", "profile", "create", persona])],
    }
}

/// Runs `otto-agents plugins install` when it has something to do: a skill
/// or rendering behind, or any other change, since a new agent needs its
/// rendering.
fn plugins_step(facts: &Facts, needed: bool) -> Step {
    let title = "Teach the agents";
    if !facts.found.contains_key("otto-agents") {
        return Step::blocked(title, "otto-agents is not installed");
    }
    if !needed {
        return Step::done(title, "up to date");
    }
    Step {
        title,
        state: State::Todo,
        summary: "link Otto's skills and write each agent's copy".into(),
        detail: "$ otto-agents plugins install

\
                 Links Otto's skills into ~/.agents/skills and writes Otto's
\
                 instructions in the format each agent reads. Safe to run again:
\
                 it only writes what changed."
            .into(),
        actions: vec![Action::run(&["otto-agents", "plugins", "install"])],
    }
}

/// Starts `unit` at login and now, or restarts it to read a change.
fn unit_step(title: &'static str, unit: &UnitFacts, wanted: bool, changed: bool) -> Step {
    match unit.state {
        Unit::Unmanaged => {
            Step::blocked(title, "no systemd user session: start otto-agents yourself")
        }
        Unit::Missing => Step::blocked(title, format!("{} is not installed", unit.name)),
        Unit::Loaded {
            enabled: true,
            active: true,
        } if !changed => Step::done(title, "running, and starts at login"),
        Unit::Loaded { .. } if !wanted => Step::done(title, "off"),
        Unit::Loaded { enabled, active } => {
            let mut actions = Vec::new();
            let mut what = Vec::new();
            if !enabled {
                what.push("start at login");
            }
            if active {
                what.push("restart to pick up the changes");
                actions.push(Action::run(&["systemctl", "--user", "restart", unit.name]));
            } else {
                what.push("start now");
            }
            if !enabled || !active {
                actions.insert(
                    0,
                    Action::run(&["systemctl", "--user", "enable", "--now", unit.name]),
                );
            }
            let detail = actions
                .iter()
                .filter_map(|action| match action {
                    Action::Run { args } => Some(format!("$ {}", args.join(" "))),
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
pub(crate) mod tests {
    use super::*;
    use crate::probe::{Unit, UnitFacts};
    use std::collections::BTreeMap;

    fn unit(name: &'static str) -> UnitFacts {
        UnitFacts {
            name,
            state: Unit::Loaded {
                enabled: false,
                active: false,
            },
            path: Some(vec![PathBuf::from("/usr/bin")]),
            found: BTreeMap::new(),
            dropin: PathBuf::from(format!(
                "/home/u/.config/systemd/user/{name}.d/otto-setup.conf"
            )),
        }
    }

    /// A machine with Node under nvm, Claude Code and OpenCode installed.
    pub(crate) fn facts() -> Facts {
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
            bridge_command: None,
            found,
            node_version: Some("v24.7.0".into()),
            package_manager: Some("pacman"),
            instructions: vec!["otto".into(), "reviewer".into()],
            agents_unit: unit("otto-agents.service"),
            bridge_unit: unit("otto-agents-bridge.service"),
            hermes_profiles: Vec::new(),
            plugins: Some(plugins::Status::default()),
            cc_config: PathBuf::from("/home/u/.cc-connect/config.toml"),
            cc_config_exists: false,
            cc_connect_node: false,
        }
    }

    #[test]
    fn a_fresh_machine_picks_what_is_ready() {
        let facts = facts();
        let choices = Choices::initial(&facts);
        assert_eq!(choices.picks, [Harness::Claude, Harness::OpenCode]);
        assert_eq!(choices.persona(), "otto");
        assert!(!choices.studio, "no Studio in the made-up plugin");
        assert_eq!(
            readiness(&facts, Harness::Pi),
            Readiness::Missing(vec!["pi", "pi-acp"])
        );
        let mut configured = facts.clone();
        configured.configured = vec![Harness::OpenCode];
        assert!(Choices::initial(&configured).picks.is_empty());
    }

    #[test]
    fn stale_profiles_of_the_selected_harnesses_are_written() {
        let mut facts = facts();
        facts.configured = vec![Harness::Claude];
        facts.commands = vec!["npx".into()];
        facts.agents_unit.found = facts.found.clone();
        facts.agents_unit.state = Unit::Loaded {
            enabled: true,
            active: true,
        };
        facts.plugins = Some(plugins::parse(
            "opencode  stale      ~/.config/opencode/agents/otto.md\n\
             codex     not there  ~/.local/share/otto/agents/codex/otto.md\n",
        ));
        let mut choices = Choices::initial(&facts);
        // Codex is not selected, OpenCode is not either: nothing to write.
        assert!(!plugins_behind(&facts, &choices));
        choices.picks = vec![Harness::OpenCode];
        assert!(plugins_behind(&facts, &choices));
        let steps = steps(&facts, &choices);
        let knowledge = steps
            .iter()
            .find(|s| s.title == "Teach the agents")
            .unwrap();
        assert_eq!(knowledge.state, State::Todo);
    }

    #[test]
    fn send_to_phone_matters_only_with_the_bridge() {
        let mut facts = facts();
        facts.plugins = Some(plugins::parse(
            "otto 1.0.0  /usr/share/otto/plugins/otto\n  \
             otto-help                linked      ~/.agents/skills/otto-help\n  \
             send-to-phone            not linked  ~/.agents/skills/send-to-phone\n",
        ));
        let mut choices = Choices::initial(&facts);
        assert!(!plugins_behind(&facts, &choices));
        facts
            .found
            .insert("cc-connect".into(), PathBuf::from("/usr/bin/cc-connect"));
        facts.cc_config_exists = true;
        choices.bridge = true;
        assert!(plugins_behind(&facts, &choices));
    }

    #[test]
    fn an_old_node_is_flagged() {
        let mut facts = facts();
        assert_eq!(node(&facts), Node::Ok);
        facts.node_version = Some("v18.19.1".into());
        assert_eq!(node(&facts), Node::TooOld(18));
        facts.node_version = None;
        assert_eq!(node(&facts), Node::Missing);
    }

    #[test]
    fn studio_needs_a_claude_code_agent() {
        let mut facts = facts();
        facts.instructions.push(STUDIO.into());
        let mut choices = Choices::initial(&facts);
        assert!(choices.studio);
        choices.picks.retain(|h| *h != Harness::Claude);
        assert_eq!(studio(&facts, &choices), Studio::NeedsClaude);
        facts.configured = vec![Harness::Claude];
        assert_eq!(studio(&facts, &choices), Studio::Ready);
    }

    #[test]
    fn npx_is_only_needed_without_the_adapter() {
        let mut facts = facts();
        facts.found.remove("npx");
        assert_eq!(missing(&facts, Harness::Claude), ["npx"]);
        facts.found.insert(
            "claude-agent-acp".into(),
            PathBuf::from("/home/u/.local/bin/claude-agent-acp"),
        );
        assert!(missing(&facts, Harness::Claude).is_empty());
        assert_eq!(
            command(&facts, Harness::Claude, "otto"),
            ["claude-agent-acp"]
        );
    }

    #[test]
    fn adds_an_agent_the_way_settings_does() {
        let mut facts = facts();
        facts.user_config = "# mine\nidle_timeout = 0\n".into();
        let choices = Choices::initial(&facts);
        let text = render_config(&facts, &choices, &[Harness::Claude], false).unwrap();
        assert!(text.starts_with("# mine\nidle_timeout = 0\n"));
        let doc: DocumentMut = text.parse().unwrap();
        let agent = &doc["agents"][0];
        assert_eq!(agent["id"].as_str(), Some("claude"));
        assert_eq!(agent["command"].as_str(), Some("npx"));
        assert_eq!(agent["agent"].as_str(), Some("otto"));
        assert_eq!(agent["permissions"].as_str(), Some("ask"));
    }

    #[test]
    fn the_harness_can_run_as_itself() {
        let facts = facts();
        let choices = Choices {
            otto: false,
            ..Choices::initial(&facts)
        };
        let text = render_config(&facts, &choices, &[Harness::Claude], false).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert!(doc["agents"][0].get("agent").is_none());
    }

    #[test]
    fn keeps_the_system_list_and_ids_unique() {
        let mut facts = facts();
        facts.system_config = Some(
            "[[agents]]\nid = \"claude\"\nname = \"Work\"\ncommand = \"/opt/claude-acp\"\n".into(),
        );
        let choices = Choices::initial(&facts);
        let text = render_config(&facts, &choices, &[Harness::Claude], false).unwrap();
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
        let steps = steps(&facts, &Choices::initial(&facts));
        let path = steps.iter().find(|s| s.title == "Program paths").unwrap();
        assert_eq!(path.state, State::Todo);
        let Action::Write { text, .. } = &path.actions[0] else {
            panic!("expected a write");
        };
        assert!(text.contains("Environment=\"PATH=/home/u/.nvm/bin:/home/u/.local/bin:/usr/bin\""));
        let service = steps.iter().find(|s| s.title == "Agent service").unwrap();
        assert_eq!(
            service.actions,
            [Action::run(&[
                "systemctl",
                "--user",
                "enable",
                "--now",
                "otto-agents.service"
            ])]
        );
        assert!(steps.iter().all(|s| s.title != "Chat bridge"));
    }

    #[test]
    fn a_set_up_machine_has_nothing_to_do() {
        let mut facts = facts();
        facts.configured = vec![Harness::Claude, Harness::OpenCode];
        facts.commands = vec!["npx".into(), "opencode".into()];
        facts.agents_unit.found = facts.found.clone();
        facts.agents_unit.state = Unit::Loaded {
            enabled: true,
            active: true,
        };
        let steps = steps(&facts, &Choices::initial(&facts));
        assert!(steps.iter().all(|s| s.state != State::Todo), "{steps:#?}");
    }

    #[test]
    fn the_bridge_is_turned_on_only_when_its_parts_are_there() {
        let mut facts = facts();
        assert_eq!(
            bridge(&facts),
            Bridge::Missing(vec!["cc-connect", "its configuration"])
        );
        facts.found.insert(
            "cc-connect".into(),
            PathBuf::from("/home/u/.local/bin/cc-connect"),
        );
        facts.cc_config_exists = true;
        facts.cc_connect_node = true;
        assert_eq!(bridge(&facts), Bridge::Ready);
        let choices = Choices {
            bridge: true,
            picks: Vec::new(),
            ..Choices::initial(&facts)
        };
        let steps = steps(&facts, &choices);
        let config = &steps[0];
        let Action::Write { text, .. } = &config.actions[0] else {
            panic!("expected a write");
        };
        assert!(text.contains("command = \"/home/u/.local/bin/cc-connect\""));
        assert!(text.contains("\"--config\", \"/home/u/.cc-connect/config.toml\""));
        let path = steps.iter().find(|s| s.title == "Bridge paths").unwrap();
        assert_eq!(path.state, State::Todo);
        let service = steps.iter().find(|s| s.title == "Chat bridge").unwrap();
        assert_eq!(service.state, State::Todo);
    }
}
