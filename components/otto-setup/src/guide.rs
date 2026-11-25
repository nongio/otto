//! The instructions: for each thing setup looks at, what is there, and the
//! steps to have it. Setup never runs an install itself.
//!
//! The text marks what each line is, so the interface can tell them apart:
//!
//! - `# Title` a heading;
//! - `#you 1. Step`, `#setup 1. Step`, `#done 1. Step` a step, and who does
//!   it: the user, setup on Apply, or nobody since it is done;
//! - `$ command` a command to run, one per line, nothing else on it;
//! - `│ text` the contents of a file;
//! - `  ✓ …`, `  ✗ …` and `  · …` what was found;
//! - anything else, prose.
//!
//! Words: "agent" (never harness), "adapter" (ACP spelled out once), "Otto's
//! agent files" (never plugin), "skills", "added" (never agent list).

// Rust guideline compliant 2026-02-21

use otto_agents_client::harness::{self, Harness};

use crate::plan::{self, Readiness};
use crate::plugins::Skill;
use crate::probe::{Backend, Facts};

/// Where herdr's own instructions are. Setup installs nothing, herdr
/// included.
const HERDR_SITE: &str = "https://herdr.dev";

/// Who does a step.
#[derive(Clone, Copy)]
enum Who {
    /// The user, by hand.
    You,
    /// Setup, when the user applies.
    Setup,
}

/// A step heading: `n. text`, and who does it, or that it is done.
fn step(n: usize, text: &str, who: Who, done: bool) -> String {
    let tag = match (done, who) {
        (true, _) => "#done",
        (false, Who::You) => "#you",
        (false, Who::Setup) => "#setup",
    };
    format!("{tag} {n}. {text}")
}

/// A command line.
fn cmd(command: impl AsRef<str>) -> String {
    format!("$ {}", command.as_ref())
}

/// A line on where `program` was found, or that it was not.
fn found(facts: &Facts, program: &str) -> String {
    match facts.found.get(program) {
        Some(path) => format!("  ✓ {program:<18}{}", facts.shown(path)),
        None => format!("  ✗ {program:<18}not found"),
    }
}

/// A skill's state as shown: what `otto-agents` calls "in the way" is a
/// name already taken by something of the user's.
pub fn skill_state(skill: &Skill) -> &str {
    if skill.in_the_way() {
        "name taken"
    } else {
        &skill.state
    }
}

/// The command that installs Node.js and npm here.
fn node_install(facts: &Facts) -> Option<&'static str> {
    Some(match facts.package_manager? {
        "pacman" => "sudo pacman -S --needed nodejs npm",
        "apt" => "sudo apt install nodejs npm",
        "dnf" => "sudo dnf install nodejs npm",
        "zypper" => "sudo zypper install nodejs npm",
        _ => return None,
    })
}

/// The command that installs `sqlite3` here.
fn sqlite_install(facts: &Facts) -> Option<&'static str> {
    Some(match facts.package_manager? {
        "pacman" => "sudo pacman -S --needed sqlite",
        "apt" => "sudo apt install sqlite3",
        "dnf" => "sudo dnf install sqlite",
        "zypper" => "sudo zypper install sqlite3",
        _ => return None,
    })
}

/// How to sign in to `harness`, after the line saying it keeps its own.
fn sign_in(harness: Harness) -> Vec<String> {
    let name = harness.name().unwrap_or_default();
    let mut out = vec![format!(
        "{name} keeps its own sign-in; Otto never sees your keys."
    )];
    match harness {
        Harness::Claude => out.push(cmd("claude auth login")),
        Harness::Codex => out.push(cmd("codex login")),
        Harness::OpenCode => out.push(cmd("opencode auth login")),
        Harness::Hermes => out.push(cmd("hermes setup")),
        Harness::Pi => {
            out.push(cmd("pi"));
            out.push("then type /login and follow the prompts.".into());
        }
        Harness::Custom => {}
    }
    out
}

/// The way to install `harness`'s own program, without the step heading.
/// Node.js is on the Requirements page only when Otto runs the sessions.
fn install(facts: &Facts, backend: Backend, harness: Harness) -> Vec<String> {
    match (harness, harness.package()) {
        (Harness::Hermes, _) => vec![
            "Hermes is a Python program from Nous Research. Install it with".into(),
            "its own guide (github.com/NousResearch/hermes-agent), then check".into(),
            "that the hermes command runs in a terminal.".into(),
        ],
        (_, Some(package)) => {
            let mut out = vec![cmd(format!("npm install -g {}", package.npm))];
            match (facts.found.contains_key("npm"), backend) {
                (true, _) => {}
                (false, Backend::Acp) => {
                    out.push("npm comes with Node.js: see Node.js on the Requirements page.".into())
                }
                (false, Backend::Herdr) => {
                    out.push("npm comes with Node.js: nodejs.org, or your distribution's".into());
                    out.push("nodejs and npm packages.".into());
                }
            }
            out
        }
        (_, None) => Vec::new(),
    }
}

/// How to install and sign in to `harness`.
pub fn harness(facts: &Facts, choices: &plan::Choices, harness: Harness) -> String {
    let name = harness.name().unwrap_or_default();
    let herdr = choices.backend == Backend::Herdr;
    let mut out = vec![format!("# {name}"), String::new()];
    match plan::readiness(facts, choices.backend, harness) {
        Readiness::Configured if herdr && harness.herdr_program().is_none() => {
            out.push("Already added, but herdr doesn't run it: it runs only while".into());
            out.push("sessions run in Otto. Settings › Agents › Sessions run in".into());
            out.push("switches.".into());
        }
        Readiness::Configured => {
            out.push("Already added. Settings › Agents changes or removes it.".into())
        }
        Readiness::Ready(_) => out.push("Installed. Tick it to add it to Otto.".into()),
        Readiness::Missing(missing) => out.push(format!(
            "Needs {}: do the steps below, then run otto-setup again.",
            missing.join(", ")
        )),
        Readiness::OttoOnly => {
            out.push(format!(
                "herdr runs Claude Code, pi and OpenCode, not {name}. To add {name},"
            ));
            out.push("switch Sessions run in to Otto at the top of this page.".into());
        }
    }
    out.push(String::new());
    if herdr && harness.herdr_program().is_some() {
        out.push(found(facts, harness.program().unwrap_or_default()));
        out.push(String::new());
        out.push(step(
            1,
            &format!("Install {name}"),
            Who::You,
            harness
                .program()
                .is_some_and(|p| facts.found.contains_key(p)),
        ));
        out.extend(install(facts, choices.backend, harness));
        out.push("herdr starts it with a login shell's PATH, so check that it".into());
        out.push("runs in a new terminal:".into());
        out.push(cmd(format!(
            "command -v {}",
            harness.program().unwrap_or_default()
        )));
        out.push(String::new());
        out.push(step(2, "Sign in", Who::You, false));
        out.extend(sign_in(harness));
        return out.join("\n");
    }
    let mut programs: Vec<&str> = harness.program().into_iter().collect();
    if let Some(adapter) = harness.adapter() {
        programs.push(adapter.bin);
        programs.push("npx");
    }
    out.extend(programs.iter().map(|p| found(facts, p)));
    out.push(String::new());

    let has = |p: &str| facts.found.contains_key(p);
    let own = harness.program().is_some_and(has);
    let mut n = 1;
    out.push(step(n, &format!("Install {name}"), Who::You, own));
    out.extend(install(facts, choices.backend, harness));
    out.push(String::new());

    if let Some(adapter) = harness.adapter() {
        n += 1;
        out.push(step(n, "Install its adapter", Who::You, has(adapter.bin)));
        if harness == Harness::Pi {
            out.push("pi cannot start without it:".into());
        } else {
            out.push("Without it, the adapter is downloaded each time the agent".into());
            out.push("starts, which is slower. Install it once:".into());
        }
        out.push(cmd(format!("npm install -g {}", adapter.npm)));
        out.push(String::new());
    }

    n += 1;
    out.push(step(n, "Sign in", Who::You, false));
    out.extend(sign_in(harness));
    if harness == Harness::Hermes {
        out.push(String::new());
        n += 1;
        let profile = facts.hermes_profiles.iter().any(|p| p == harness::OTTO);
        out.push(step(n, "Create Otto's Hermes profile", Who::Setup, profile));
        out.push(format!(
            "Hermes keeps each agent's setup in a profile; Otto's is called {}.",
            harness::OTTO
        ));
        out.push("Apply runs:".into());
        out.push(cmd(format!("hermes profile create {}", harness::OTTO)));
    }
    out.join("\n")
}

/// What ticking Otto does.
pub fn otto(facts: &Facts) -> String {
    let mut out = vec![
        "# Otto".to_owned(),
        String::new(),
        "When ticked, the agents setup adds answer as Otto: in Ask they".to_owned(),
        "know your desktop, its settings and its documentation, and can".to_owned(),
        "change things for you. When unticked, they are plain Claude Code,".to_owned(),
        "Codex and so on, with no knowledge of Otto.".to_owned(),
        String::new(),
        "Agents added before stay as they are; Settings › Agents changes".to_owned(),
        "them.".to_owned(),
    ];
    if !facts.instructions.iter().any(|n| n == harness::OTTO) {
        out.push(String::new());
        out.push("Otto's agent files are missing from /usr/share/otto/plugins/otto.".into());
        out.push("Reinstall the otto package.".into());
    }
    out.join("\n")
}

/// What Studio needs.
pub fn studio(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Studio".to_owned(),
        String::new(),
        "Studio opens a file (a picture, a PDF, a Markdown document) with a".to_owned(),
        "chat beside it, and the agent there works on that one file. It runs".to_owned(),
        "on Claude Code, so Claude Code has to be added too.".to_owned(),
        String::new(),
    ];
    let state = plan::studio(facts, choices);
    match state {
        plan::Studio::Unavailable => {
            out.push("Studio's files are missing. Reinstall the otto package.".into());
            return out.join("\n");
        }
        plan::Studio::Ready if facts.configured.contains(&Harness::Claude) => {
            out.push("  ✓ Claude Code   already added".into());
        }
        plan::Studio::Ready => out.push("  ✓ Claude Code   ticked, added on Apply".into()),
        plan::Studio::NeedsClaude => out.push("  ✗ Claude Code   not ticked".into()),
    }
    out.push(String::new());
    let ready = state == plan::Studio::Ready;
    out.push(step(1, "Add Claude Code", Who::You, ready));
    out.push("Tick Claude Code on the Agents page. If it is not installed,".into());
    out.push("press Enter on its row for how.".into());
    out.push(String::new());
    // Claude Code reads Otto's agent files itself; once they are there and
    // Claude Code is added, Studio works.
    let taught = facts.configured.contains(&Harness::Claude) && facts.plugins.is_some();
    out.push(step(
        2,
        "Teach Claude Code about Studio",
        Who::Setup,
        taught,
    ));
    out.push("Apply runs:".into());
    out.push(cmd("otto-agents plugins install"));
    out.join("\n")
}

/// What runs the sessions, the two choices, and which one setup uses.
pub fn sessions(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Sessions run in".to_owned(),
        String::new(),
        "herdr runs each session as the agent's own program, in a terminal".to_owned(),
        "of its own: Ask shows the conversation, Ctrl+O opens the terminal".to_owned(),
        "in herdr-app, and the session keeps going when Otto's windows".to_owned(),
        "close. It runs Claude Code, pi and OpenCode.".to_owned(),
        String::new(),
        "Otto runs every agent itself, over ACP (Agent Client Protocol), with".to_owned(),
        "an adapter in between for those that need one. Codex and Hermes".to_owned(),
        "run only this way.".to_owned(),
        String::new(),
        "Unless agents.toml says which, sessions run in herdr when it is".to_owned(),
        "installed, and in Otto when it is not.".to_owned(),
        String::new(),
    ];
    for program in plan::HERDR_PROGRAMS {
        out.push(found(facts, program));
    }
    out.push(String::new());
    let label = choices.backend.label();
    match (facts.backend, plan::writes_backend(facts, choices)) {
        (Some(backend), _) => {
            out.push(format!(
                "agents.toml has backend = \"{}\", so sessions run in {label}.",
                backend.key()
            ));
            out.push("Setup keeps that. Settings › Agents › Sessions run in changes it.".into());
        }
        (None, Some(backend)) => {
            out.push(format!(
                "Sessions run in {label}, as picked here. Apply writes"
            ));
            out.push(format!("backend = \"{}\" to agents.toml.", backend.key()));
        }
        (None, None) => {
            let why = match choices.backend {
                Backend::Herdr => "since herdr is installed",
                Backend::Acp => "since herdr isn't installed",
            };
            out.push(format!("Sessions run in {label}, {why}. Space switches;"));
            out.push("Apply then writes the choice to agents.toml.".into());
        }
    }
    if choices.backend == Backend::Herdr {
        out.push(String::new());
        let installed = plan::herdr_missing(facts).is_empty();
        out.push(step(1, "Install herdr", Who::You, installed));
        out.push(format!(
            "Setup installs nothing. herdr's own guide is at {HERDR_SITE};"
        ));
        out.push("then check that both commands are found:".into());
        out.push(cmd("command -v hc herdr-app"));
    }
    out.join("\n")
}

/// herdr, while it runs the sessions: where it is, and how to have it.
pub fn herdr(facts: &Facts) -> String {
    let mut out = vec![
        "# herdr".to_owned(),
        String::new(),
        "Required while sessions run in herdr. hc, herdr-core's command,".to_owned(),
        "runs each session in a terminal of its own; herdr-app shows them,".to_owned(),
        "and is where Ctrl+O opens a session.".to_owned(),
        String::new(),
    ];
    for program in plan::HERDR_PROGRAMS {
        out.push(found(facts, program));
    }
    out.push(String::new());
    out.push(step(
        1,
        "Install herdr",
        Who::You,
        plan::herdr_missing(facts).is_empty(),
    ));
    out.push("Setup installs nothing. Follow herdr's own guide at".into());
    out.push(format!(
        "{HERDR_SITE}, then check that both commands are found:"
    ));
    out.push(cmd("command -v hc herdr-app"));
    out.push(String::new());
    let unit = &facts.agents_unit;
    let seen = unit.path.is_none()
        || plan::HERDR_PROGRAMS
            .iter()
            .all(|p| unit.found.contains_key(*p));
    out.push(step(2, "Let the agent service find them", Who::Setup, seen));
    out.push("Otto's agent service doesn't get your login shell's PATH. When".into());
    out.push("hc or herdr-app is in a folder it doesn't look in, Apply adds".into());
    out.push(format!("the folder in {}.", facts.shown(&unit.dropin)));
    out.join("\n")
}

/// The agents' own programs, which herdr starts.
pub fn programs(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Agent programs".to_owned(),
        String::new(),
        "In herdr each session runs the agent's own program: claude, pi or".to_owned(),
        "opencode. herdr starts it with a login shell's PATH, so a program".to_owned(),
        "that runs in a new terminal runs there too. Node.js and the ACP".to_owned(),
        "adapters aren't needed.".to_owned(),
        String::new(),
    ];
    let programs = plan::agent_programs(facts, choices);
    if programs.is_empty() {
        out.push("  · no agent ticked yet: tick one on the Agents page".into());
        return out.join("\n");
    }
    out.extend(programs.iter().map(|p| found(facts, p)));
    let mut n = 0;
    for harness in plan::selected(facts, choices) {
        let Some(program) = harness.herdr_program() else {
            continue;
        };
        let name = harness.name().unwrap_or_default();
        out.push(String::new());
        n += 1;
        out.push(step(
            n,
            &format!("Install {name}"),
            Who::You,
            facts.found.contains_key(program),
        ));
        out.extend(install(facts, choices.backend, harness));
        out.push(String::new());
        n += 1;
        out.push(step(n, &format!("Sign in to {name}"), Who::You, false));
        out.extend(sign_in(harness));
    }
    if programs.contains(&"sqlite3") {
        out.push(String::new());
        n += 1;
        out.push(step(
            n,
            "Install sqlite3",
            Who::You,
            facts.found.contains_key("sqlite3"),
        ));
        out.push("Otto reads OpenCode's sessions from its database with it.".into());
        match sqlite_install(facts) {
            Some(command) => out.push(cmd(command)),
            None => out.push("Install it from your distribution's packages.".into()),
        }
        out.push(String::new());
        n += 1;
        let unit = &facts.agents_unit;
        let seen = unit.path.is_none()
            || ["opencode", "sqlite3"]
                .iter()
                .all(|p| unit.found.contains_key(*p));
        out.push(step(n, "Let the agent service find them", Who::Setup, seen));
        out.push("The service itself runs opencode to make each session, and".into());
        out.push("sqlite3 to read it. When either is in a folder it doesn't look".into());
        out.push(format!(
            "in, Apply adds the folder in {}.",
            facts.shown(&unit.dropin)
        ));
    }
    out.join("\n")
}

/// Node.js: where it is, and how to install it.
pub fn node(facts: &Facts) -> String {
    let mut out = vec![
        "# Node.js".to_owned(),
        String::new(),
        "Required. Most agents and their adapters install with npm, which".to_owned(),
        "comes with Node.js.".to_owned(),
        String::new(),
    ];
    for program in ["node", "npm", "npx"] {
        out.push(found(facts, program));
    }
    out.push(String::new());
    let state = plan::node(facts);
    let have = state == plan::Node::Ok;
    if let plan::Node::TooOld(major) = state {
        out.push(format!(
            "Installed, but v{major} is too old: the Claude Code adapter needs {}",
            plan::NODE_MIN
        ));
        out.push("or newer.".into());
        out.push(String::new());
    }
    out.push(step(1, "Install Node.js", Who::You, have));
    match node_install(facts) {
        Some(command) => out.push(cmd(command)),
        None => {
            out.push("Install it from your distribution's packages, or from nodejs.org.".into())
        }
    }
    if facts.package_manager == Some("apt") {
        out.push(format!(
            "Debian and Ubuntu's own nodejs package is often older than {}; if",
            plan::NODE_MIN
        ));
        out.push("so, use nvm below, or NodeSource (nodesource.com).".into());
    }
    out.push("Or install it in your home folder with nvm (github.com/nvm-sh/nvm);".into());
    out.push("setup then makes sure Otto's agent service can find it. After".into());
    out.push("installing nvm:".into());
    out.push(cmd(format!("nvm install {}", plan::NODE_MIN)));
    out.push(String::new());
    out.push(step(2, "Check", Who::You, have));
    out.push(cmd("node --version"));
    if let Some(version) = &facts.node_version {
        out.push(format!("Here it prints {version}."));
    }
    out.join("\n")
}

/// The adapters: which are installed, and how to install them.
pub fn adapters(facts: &Facts) -> String {
    let mut out = vec![
        "# Agent adapters".to_owned(),
        String::new(),
        "Required. Otto talks to agents in ACP (Agent Client Protocol).".to_owned(),
        "Claude Code, Codex and pi do not speak it, so each needs a small".to_owned(),
        "adapter in between. pi needs its adapter installed. Claude Code and".to_owned(),
        "Codex can download theirs each time they start, but an installed".to_owned(),
        "adapter starts at once.".to_owned(),
        String::new(),
    ];
    let mut all = Vec::new();
    for harness in plan::HARNESSES {
        if let Some(adapter) = harness.adapter() {
            out.push(found(facts, adapter.bin));
            all.push(adapter.npm);
        }
    }
    out.push(String::new());
    let installed = plan::HARNESSES
        .iter()
        .filter_map(|h| h.adapter())
        .all(|a| facts.found.contains_key(a.bin));
    out.push(step(1, "Install them", Who::You, installed));
    out.push("All three, or only the ones for your agents:".to_owned());
    out.push(cmd(format!("npm install -g {}", all.join(" "))));
    out.push(String::new());
    out.push(step(2, "Check", Who::You, installed));
    out.push("Quit setup, then:".to_owned());
    out.push(cmd("otto-setup --check"));
    out.push(String::new());
    out.push("Agents added before keep downloading theirs. To use the installed".to_owned());
    out.push("adapter, change their command in Settings › Agents.".to_owned());
    out.join("\n")
}

/// Otto's skills: linked where agents look for them, or a name taken.
pub fn skills(facts: &Facts) -> String {
    let mut out = vec![
        "# Otto's skills".to_owned(),
        String::new(),
        "Skills are short guides that teach the agents your desktop: how to".to_owned(),
        "change a setting, find a file, and so on. Setup links".to_owned(),
        "them into ~/.agents/skills, where the agents look for them.".to_owned(),
        String::new(),
    ];
    if facts.plugins.is_none() {
        out.push("  ✗ otto-agents did not answer: is it installed?".to_owned());
    }
    for skill in plan::skills(facts) {
        let mark = if skill.linked() { "✓" } else { "✗" };
        out.push(format!(
            "  {mark} {:<18}{:<12}{}",
            skill.name,
            skill_state(skill),
            skill.path
        ));
    }
    out.push(String::new());
    let skills = plan::skills(facts);
    let taken: Vec<&Skill> = skills.iter().filter(|s| s.in_the_way()).collect();
    let mut n = 0;
    for skill in &taken {
        n += 1;
        out.push(step(
            n,
            &format!("Rename your own {}", skill.name),
            Who::You,
            false,
        ));
        out.push("A file or folder of yours already has this name, and setup will".into());
        out.push("not overwrite it. Rename yours:".into());
        out.push(cmd(format!("mv {0} {0}.mine", skill.path)));
        out.push(String::new());
    }
    let linked = !skills.is_empty() && skills.iter().all(|s| s.linked());
    n += 1;
    out.push(step(n, "Link the skills", Who::Setup, linked));
    out.push("Apply runs:".into());
    out.push(cmd("otto-agents plugins install"));
    out.push(String::new());
    n += 1;
    out.push(step(n, "Check", Who::You, linked));
    out.push(cmd("otto-agents plugins status"));
    out.join("\n")
}

/// Each selected agent's copy of Otto's instructions.
pub fn profiles(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Agent profiles".to_owned(),
        String::new(),
        "Each agent except Claude Code needs its own copy of Otto's".to_owned(),
        "instructions: in OpenCode's agents folder, Codex's config, a wrapper".to_owned(),
        "for pi, a Hermes profile. Claude Code reads them directly.".to_owned(),
        String::new(),
    ];
    let profiles = plan::profiles(facts, choices);
    if !choices.otto {
        out.push("  ✓ none: Otto is unticked".to_owned());
    } else if choices.backend == Backend::Herdr {
        out.push("  ✓ none needed in herdr: Otto hands each agent its".to_owned());
        out.push("    instructions when it starts it.".to_owned());
    } else if profiles.is_empty() {
        out.push("  ✓ none needed for the agents you chose".to_owned());
    }
    for render in &profiles {
        let mark = if render.current() { "✓" } else { "✗" };
        let state = profile_state(&render.state);
        out.push(format!(
            "  {mark} {:<10}{:<22}{}",
            vendor_name(&render.vendor),
            state,
            render.path
        ));
    }
    out.push(String::new());
    let skipped = profiles.iter().any(|r| r.state == "skipped");
    let written = profiles.iter().all(|r| r.current() || r.state == "skipped");
    let mut n = 0;
    if skipped {
        n += 1;
        out.push(step(n, "Create Otto's Hermes profile", Who::Setup, false));
        out.push("When you add Hermes, Apply runs:".into());
        out.push(cmd(format!("hermes profile create {}", harness::OTTO)));
        out.push(String::new());
    }
    n += 1;
    out.push(step(n, "Write the copies", Who::Setup, written && !skipped));
    out.push("Apply runs:".into());
    out.push(cmd("otto-agents plugins install"));
    out.push(String::new());
    n += 1;
    out.push(step(n, "Check", Who::You, written && !skipped));
    out.push(cmd("otto-agents plugins status"));
    out.join("\n")
}

/// A copy's state as shown.
pub fn profile_state(state: &str) -> &str {
    match state {
        "current" => "up to date",
        "skipped" => "waits for its profile",
        other => other,
    }
}

/// An agent's name as people know it, from `otto-agents`' id for it.
pub fn vendor_name(id: &str) -> &str {
    plan::HARNESSES
        .iter()
        .find(|h| h.id() == id)
        .and_then(|h| h.name())
        .unwrap_or(id)
}
