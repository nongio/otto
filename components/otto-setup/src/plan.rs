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
use crate::probe::{Backend, Facts, Unit, UnitFacts};

/// The harnesses setup offers, in the order it lists them.
pub const HARNESSES: [Harness; 5] = [
    Harness::Claude,
    Harness::Codex,
    Harness::OpenCode,
    Harness::Hermes,
    Harness::Pi,
];

/// herdr's programs: `hc`, herdr-core's command, which runs the sessions,
/// and `herdr-app`, which shows them.
pub const HERDR_PROGRAMS: [&str; 2] = ["hc", "herdr-app"];

/// What the user picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// What runs the sessions.
    pub backend: Backend,
    /// Harnesses to add an agent for.
    pub picks: Vec<Harness>,
    /// Whether the agents added answer as Otto, rather than as the plain
    /// harness.
    pub otto: bool,
    /// Whether Studio's chat should work: it needs a Claude Code agent.
    pub studio: bool,
}

impl Choices {
    /// What is ticked to begin with: the backend the service would use;
    /// every ready harness when the agent list is empty, since Otto has
    /// nothing of the user's yet, and none otherwise, since the user has
    /// already chosen; Otto, and Studio when it can work.
    pub fn initial(facts: &Facts) -> Self {
        let mut choices = Self {
            backend: effective(facts),
            picks: Vec::new(),
            otto: facts.instructions.iter().any(|n| n == harness::OTTO),
            studio: false,
        };
        choices.picks = choices.initial_picks(facts);
        choices.studio = studio(facts, &choices) == Studio::Ready;
        choices
    }

    fn initial_picks(&self, facts: &Facts) -> Vec<Harness> {
        if !facts.configured.is_empty() {
            return Vec::new();
        }
        HARNESSES
            .into_iter()
            .filter(|h| matches!(readiness(facts, self.backend, *h), Readiness::Ready(_)))
            .collect()
    }

    /// Switches what runs the sessions. The ticks follow: a harness the new
    /// backend cannot run is unticked, and with no agent list every ready
    /// harness is ticked, as at the start.
    pub fn set_backend(&mut self, facts: &Facts, backend: Backend) {
        self.backend = backend;
        if facts.configured.is_empty() {
            self.picks = self.initial_picks(facts);
        } else {
            let backend = self.backend;
            self.picks
                .retain(|h| matches!(readiness(facts, backend, *h), Readiness::Ready(_)));
        }
        if self.studio && studio(facts, self) != Studio::Ready {
            self.studio = false;
        }
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

/// What runs the sessions when `agents.toml` does not say: herdr when `hc`
/// is installed, else Otto, as `otto-agents` decides.
pub fn automatic(facts: &Facts) -> Backend {
    if facts.found.contains_key("hc") {
        Backend::Herdr
    } else {
        Backend::Acp
    }
}

/// What runs the sessions now: `backend` in `agents.toml`, else
/// [`automatic`].
pub fn effective(facts: &Facts) -> Backend {
    facts.backend.unwrap_or_else(|| automatic(facts))
}

/// The `backend` setup writes: the one chosen, when the files set none and
/// it differs from what [`automatic`] gives. A key the user set is theirs,
/// and is never changed.
pub fn writes_backend(facts: &Facts, choices: &Choices) -> Option<Backend> {
    (facts.backend.is_none() && choices.backend != automatic(facts)).then_some(choices.backend)
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
    /// herdr runs the sessions, and does not run this harness: it runs only
    /// when Otto runs the sessions.
    OttoOnly,
}

/// The programs `harness` is missing on the user's PATH. In herdr: its own
/// program, which herdr starts. In Otto: its own, and the adapter or `npx`
/// to run the adapter with.
pub fn missing(facts: &Facts, backend: Backend, harness: Harness) -> Vec<&'static str> {
    let has = |name: &str| facts.found.contains_key(name);
    if backend == Backend::Herdr {
        return harness
            .herdr_program()
            .filter(|p| !has(p))
            .into_iter()
            .collect();
    }
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

/// Where `harness` stands on this machine, with `backend` running the
/// sessions.
pub fn readiness(facts: &Facts, backend: Backend, harness: Harness) -> Readiness {
    if facts.configured.contains(&harness) {
        return Readiness::Configured;
    }
    if backend == Backend::Herdr && harness.herdr_program().is_none() {
        return Readiness::OttoOnly;
    }
    let missing = missing(facts, backend, harness);
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

/// The command a new agent on `harness` starts. In herdr: the harness's own
/// program, which herdr runs in its own interface. In Otto: its installed
/// adapter when there is one, so nothing is fetched at every start, else the
/// preset.
pub fn command(facts: &Facts, backend: Backend, harness: Harness, persona: &str) -> Vec<String> {
    if backend == Backend::Herdr {
        return harness
            .herdr_program()
            .map(str::to_owned)
            .into_iter()
            .collect();
    }
    match harness.adapter() {
        Some(adapter) if facts.found.contains_key(adapter.bin) => vec![adapter.bin.to_owned()],
        _ => harness.command(persona),
    }
}

/// herdr's programs missing on the user's PATH.
pub fn herdr_missing(facts: &Facts) -> Vec<&'static str> {
    HERDR_PROGRAMS
        .into_iter()
        .filter(|p| !facts.found.contains_key(*p))
        .collect()
}

/// The programs the agents there are or are being added need in herdr, in
/// the list's order: each one's own program, and `sqlite3` for OpenCode,
/// whose sessions the service reads with it. Empty in Otto.
pub fn agent_programs(facts: &Facts, choices: &Choices) -> Vec<&'static str> {
    if choices.backend != Backend::Herdr {
        return Vec::new();
    }
    let selected = selected(facts, choices);
    let mut programs: Vec<&'static str> =
        selected.iter().filter_map(|h| h.herdr_program()).collect();
    if selected.contains(&Harness::OpenCode) {
        programs.push("sqlite3");
    }
    programs
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
/// harnesses that need one, in the list's order. None in herdr, which runs
/// the harnesses' own programs.
pub fn adapters(facts: &Facts, choices: &Choices) -> Vec<(Harness, Adapter)> {
    if choices.backend == Backend::Herdr {
        return Vec::new();
    }
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

/// The renderings of Otto's agent for the selected harnesses. Claude Code
/// reads the plugin itself and has none. Empty when the agents are plain,
/// and in herdr, where the service hands Claude Code and pi the agent on
/// their command line and OpenCode does not take one yet.
pub fn profiles<'a>(facts: &'a Facts, choices: &Choices) -> Vec<&'a Render> {
    let Some(status) = &facts.plugins else {
        return Vec::new();
    };
    if !choices.otto || choices.backend == Backend::Herdr {
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
    skills(facts).iter().any(|s| !s.linked() && !s.in_the_way())
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

impl Step {
    fn done(title: &'static str, summary: impl Into<String>) -> Self {
        Self {
            title,
            state: State::Done,
            summary: summary.into(),
            detail: String::new(),
            actions: Vec::new(),
        }
    }

    fn blocked(title: &'static str, summary: impl Into<String>) -> Self {
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
        .filter(|h| matches!(readiness(facts, choices.backend, *h), Readiness::Ready(_)))
        .collect();
    let mut steps = vec![config_step(facts, choices, &adding)];
    steps.push(path_step(
        "Program paths",
        facts,
        &facts.agents_unit,
        service_programs(facts, choices, &adding),
    ));

    // Hermes runs an agent in the profile named after its instructions; one
    // that runs as itself uses its default profile.
    if choices.backend == Backend::Acp && adding.contains(&Harness::Hermes) && choices.otto {
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
    steps
}

/// The programs the agent service runs itself, by name.
///
/// In Otto: each agent's command, as listed and as about to be added, and
/// Node for the Node scripts. A harness's own CLI (`claude --resume`) runs in
/// the user's terminal, not here.
///
/// In herdr: `hc` and `herdr-app`, and for OpenCode `opencode` (the service
/// makes each session with `opencode import`) and `sqlite3` (it reads them).
/// herdr starts the agents' own programs, with its own PATH.
fn service_programs(facts: &Facts, choices: &Choices, adding: &[Harness]) -> Vec<String> {
    if choices.backend == Backend::Herdr {
        let mut programs: Vec<String> = HERDR_PROGRAMS.map(str::to_owned).to_vec();
        if selected(facts, choices).contains(&Harness::OpenCode) {
            programs.extend(["opencode".to_owned(), "sqlite3".to_owned()]);
        }
        return programs;
    }
    let new_commands = adding.iter().filter_map(|h| {
        command(facts, choices.backend, *h, choices.persona())
            .into_iter()
            .next()
    });
    let commands: Vec<String> = facts.commands.iter().cloned().chain(new_commands).collect();
    let mut programs: Vec<String> = commands
        .iter()
        .filter(|p| !p.contains('/'))
        .cloned()
        .collect();
    // npx and the ACP adapters are Node scripts: `#!/usr/bin/env node`.
    if commands.iter().any(|p| is_node_script(p)) {
        programs.push("node".to_owned());
    }
    programs
}

/// Whether `program` is one of the Node scripts agents run through.
fn is_node_script(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    name == "npx" || name.ends_with("-acp")
}

fn config_step(facts: &Facts, choices: &Choices, adding: &[Harness]) -> Step {
    let title = "Agents";
    let shown = facts.shown(&facts.config_path);
    if let Some(error) = &facts.config_error {
        return Step {
            detail: error.clone(),
            ..Step::blocked(title, format!("{shown} cannot be read: fix it first"))
        };
    }
    let backend = writes_backend(facts, choices);
    if adding.is_empty() && backend.is_none() {
        return match facts.configured.len() {
            0 => Step::blocked(title, "no agent ticked: tick one on the Agents page"),
            1 => Step::done(title, "1 agent added".to_owned()),
            n => Step::done(title, format!("{n} agents added")),
        };
    }
    let mut summary = Vec::new();
    if !adding.is_empty() {
        summary.push(format!("add {}", names(adding)));
    }
    if let Some(backend) = backend {
        summary.push(format!("run the sessions in {}", backend.label()));
    }
    match render_config(facts, choices, adding) {
        Ok(text) => Step {
            title,
            state: State::Todo,
            summary: summary.join(", "),
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

/// The user's `agents.toml` with an agent for each of `adding` appended, and
/// `backend` when setup writes it ([`writes_backend`]).
///
/// Edited with `toml_edit`, so comments and keys setup does not know stay as
/// they were. When the agents came from the system file, its list is copied
/// in first: a user file that lists agents replaces the system's whole.
pub fn render_config(
    facts: &Facts,
    choices: &Choices,
    adding: &[Harness],
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
    if let Some(backend) = writes_backend(facts, choices) {
        doc.insert("backend", value(backend.key()));
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
        let command = command(facts, choices.backend, harness, choices.persona());
        if let Some((program, args)) = command.split_first() {
            table.insert("command", value(program));
            harness::set_list(&mut table, "args", args);
        }
        // The same as an agent added in Settings: one the user is about to
        // try should ask rather than refuse.
        table.insert("permissions", value("ask"));
        match choices.backend {
            Backend::Herdr => harness.write_herdr(&mut table, choices.persona()),
            Backend::Acp => harness.write(&mut table, choices.persona(), &home),
        }
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
    mut programs: Vec<String>,
) -> Step {
    let Some(service_path) = &unit.path else {
        return Step::done(title, "no systemd user session: uses your session's paths");
    };
    programs.sort_unstable();
    programs.dedup();
    let mut missing_dirs: Vec<PathBuf> = Vec::new();
    let mut missing_programs: Vec<String> = Vec::new();
    for program in programs {
        let Some(found) = facts.found.get(&program) else {
            continue;
        };
        if unit.found.contains_key(&program) {
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
        return Step::done(title, "the service finds every program");
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
            "service cannot find {}: add {}",
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
            backend: None,
            found,
            node_version: Some("v24.7.0".into()),
            package_manager: Some("pacman"),
            instructions: vec!["otto".into(), "reviewer".into()],
            agents_unit: unit("otto-agents.service"),
            hermes_profiles: Vec::new(),
            plugins: Some(plugins::Status::default()),
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
            readiness(&facts, Backend::Acp, Harness::Pi),
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
        assert_eq!(missing(&facts, Backend::Acp, Harness::Claude), ["npx"]);
        facts.found.insert(
            "claude-agent-acp".into(),
            PathBuf::from("/home/u/.local/bin/claude-agent-acp"),
        );
        assert!(missing(&facts, Backend::Acp, Harness::Claude).is_empty());
        assert_eq!(
            command(&facts, Backend::Acp, Harness::Claude, "otto"),
            ["claude-agent-acp"]
        );
    }

    #[test]
    fn adds_an_agent_the_way_settings_does() {
        let mut facts = facts();
        facts.user_config = "# mine\nidle_timeout = 0\n".into();
        let choices = Choices::initial(&facts);
        let text = render_config(&facts, &choices, &[Harness::Claude]).unwrap();
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
        let text = render_config(&facts, &choices, &[Harness::Claude]).unwrap();
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
        let text = render_config(&facts, &choices, &[Harness::Claude]).unwrap();
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

    /// The same machine with herdr installed system-wide, where the unit
    /// finds it.
    fn herdr_facts() -> Facts {
        let mut facts = facts();
        for program in ["hc", "herdr-app", "sqlite3"] {
            let path = PathBuf::from("/usr/bin").join(program);
            facts.found.insert(program.into(), path.clone());
            facts.agents_unit.found.insert(program.into(), path);
        }
        facts
    }

    #[test]
    fn herdr_installed_runs_the_harnesses_own_programs() {
        let facts = herdr_facts();
        let choices = Choices::initial(&facts);
        assert_eq!(choices.backend, Backend::Herdr);
        assert_eq!(writes_backend(&facts, &choices), None);
        assert_eq!(choices.picks, [Harness::Claude, Harness::OpenCode]);
        assert_eq!(
            readiness(&facts, Backend::Herdr, Harness::Codex),
            Readiness::OttoOnly
        );
        assert_eq!(
            readiness(&facts, Backend::Herdr, Harness::Pi),
            Readiness::Missing(vec!["pi"])
        );
        assert!(adapters(&facts, &choices).is_empty());
        assert!(herdr_missing(&facts).is_empty());
        assert_eq!(
            agent_programs(&facts, &choices),
            ["claude", "opencode", "sqlite3"]
        );

        let text = render_config(&facts, &choices, &choices.picks).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert!(doc.get("backend").is_none());
        let claude = &doc["agents"][0];
        assert_eq!(claude["command"].as_str(), Some("claude"));
        assert!(claude.get("args").is_none());
        assert!(claude.get("enter").is_none());
        assert_eq!(claude["agent"].as_str(), Some("otto"));
        assert_eq!(claude["permissions"].as_str(), Some("ask"));
        assert_eq!(doc["agents"][1]["command"].as_str(), Some("opencode"));

        // The unit finds herdr; OpenCode lives in ~/.local/bin, which it
        // needs for `opencode import`. Node is not needed.
        let steps = steps(&facts, &choices);
        let path = steps.iter().find(|s| s.title == "Program paths").unwrap();
        let Action::Write { text, .. } = &path.actions[0] else {
            panic!("expected a write: {path:#?}");
        };
        assert!(
            text.contains("Environment=\"PATH=/home/u/.local/bin:/usr/bin\""),
            "{text}"
        );
    }

    #[test]
    fn hc_the_unit_cannot_see_is_added_to_its_path() {
        let mut facts = herdr_facts();
        let local = PathBuf::from("/home/u/.local/bin/hc");
        facts.found.insert("hc".into(), local);
        facts.agents_unit.found.remove("hc");
        let choices = Choices {
            picks: vec![Harness::Claude],
            ..Choices::initial(&facts)
        };
        let steps = steps(&facts, &choices);
        let path = steps.iter().find(|s| s.title == "Program paths").unwrap();
        assert_eq!(path.state, State::Todo);
        assert!(path.summary.contains("cannot find hc"), "{}", path.summary);
    }

    #[test]
    fn herdr_chosen_without_hc_is_written_and_required() {
        let facts = facts();
        let mut choices = Choices::initial(&facts);
        assert_eq!(choices.backend, Backend::Acp);
        choices.set_backend(&facts, Backend::Herdr);
        assert_eq!(writes_backend(&facts, &choices), Some(Backend::Herdr));
        assert_eq!(herdr_missing(&facts), ["hc", "herdr-app"]);
        let text = render_config(&facts, &choices, &choices.picks).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["backend"].as_str(), Some("herdr"));
        assert_eq!(doc["agents"][0]["command"].as_str(), Some("claude"));
        let config = steps(&facts, &choices).remove(0);
        assert!(
            config.summary.ends_with("run the sessions in herdr"),
            "{}",
            config.summary
        );
    }

    #[test]
    fn otto_chosen_over_herdr_is_written() {
        let facts = herdr_facts();
        let mut choices = Choices::initial(&facts);
        choices.set_backend(&facts, Backend::Acp);
        assert_eq!(writes_backend(&facts, &choices), Some(Backend::Acp));
        assert_eq!(
            choices.picks,
            [Harness::Claude, Harness::OpenCode],
            "Codex and Hermes are not installed"
        );
        let text = render_config(&facts, &choices, &choices.picks).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["backend"].as_str(), Some("acp"));
        assert_eq!(doc["agents"][0]["command"].as_str(), Some("npx"));
    }

    #[test]
    fn a_backend_the_user_set_wins_and_is_kept() {
        let mut facts = herdr_facts();
        facts.backend = Some(Backend::Acp);
        facts.user_config = "backend = \"acp\"\n".into();
        let choices = Choices::initial(&facts);
        assert_eq!(choices.backend, Backend::Acp);
        assert_eq!(writes_backend(&facts, &choices), None);
        assert_eq!(
            readiness(&facts, choices.backend, Harness::Claude),
            Readiness::Ready(PathBuf::from("/home/u/.local/bin/claude"))
        );
        let text = render_config(&facts, &choices, &[Harness::Claude]).unwrap();
        assert!(text.starts_with("backend = \"acp\"\n"), "{text}");
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["agents"][0]["command"].as_str(), Some("npx"));
    }

    #[test]
    fn switching_to_herdr_unticks_what_it_cannot_run() {
        let mut facts = herdr_facts();
        facts
            .found
            .insert("codex".into(), PathBuf::from("/usr/bin/codex"));
        facts.configured = vec![Harness::Pi];
        let mut choices = Choices {
            backend: Backend::Acp,
            picks: vec![Harness::Claude, Harness::Codex],
            ..Choices::initial(&facts)
        };
        choices.set_backend(&facts, Backend::Herdr);
        assert_eq!(choices.picks, [Harness::Claude]);
    }
}
