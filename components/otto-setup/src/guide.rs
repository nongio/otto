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

use crate::plan::{self, Bridge, Readiness};
use crate::plugins::Skill;
use crate::probe::Facts;

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

/// How to install and sign in to `harness`.
pub fn harness(facts: &Facts, harness: Harness) -> String {
    let name = harness.name().unwrap_or_default();
    let mut out = vec![format!("# {name}"), String::new()];
    out.push(match plan::readiness(facts, harness) {
        Readiness::Configured => {
            "Already added. Settings › Agents changes or removes it.".to_owned()
        }
        Readiness::Ready(_) => "Installed. Tick it to add it to Otto.".to_owned(),
        Readiness::Missing(missing) => format!(
            "Needs {}: do the steps below, then run otto-setup again.",
            missing.join(", ")
        ),
    });
    out.push(String::new());
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
    match (harness, harness.package()) {
        (Harness::Hermes, _) => {
            out.push("Hermes is a Python program from Nous Research. Install it with".into());
            out.push("its own guide (github.com/NousResearch/hermes-agent), then check".into());
            out.push("that the hermes command runs in a terminal.".into());
        }
        (_, Some(package)) => {
            out.push(cmd(format!("npm install -g {}", package.npm)));
            if !has("npm") {
                out.push("npm comes with Node.js: see Node.js on the Requirements page.".into());
            }
        }
        (_, None) => {}
    }
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
    out.push(format!(
        "{name} keeps its own sign-in; Otto never sees your keys."
    ));
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
    let have = facts.node_version.is_some();
    out.push(step(1, "Install Node.js", Who::You, have));
    match node_install(facts) {
        Some(command) => out.push(cmd(command)),
        None => {
            out.push("Install it from your distribution's packages, or from nodejs.org.".into())
        }
    }
    out.push("Or install it in your home folder with nvm (github.com/nvm-sh/nvm);".into());
    out.push("setup then makes sure Otto's agent service can find it.".into());
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
/// The chat bridge's own are optional while the bridge is off.
pub fn skills(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Otto's skills".to_owned(),
        String::new(),
        "Skills are short guides that teach the agents your desktop: how to".to_owned(),
        "change a setting, send a file to your phone, and so on. Setup links".to_owned(),
        "them into ~/.agents/skills, where the agents look for them.".to_owned(),
        String::new(),
    ];
    if facts.plugins.is_none() {
        out.push("  ✗ otto-agents did not answer: is it installed?".to_owned());
    }
    for skill in plan::skills(facts) {
        let needed = plan::skill_needed(facts, choices, skill);
        let mark = match (skill.linked(), needed) {
            (true, _) => "✓",
            (false, true) => "✗",
            (false, false) => "·",
        };
        let note = if needed {
            ""
        } else {
            "  optional, for the chat bridge"
        };
        out.push(format!(
            "  {mark} {:<18}{:<12}{}{note}",
            skill.name,
            skill_state(skill),
            skill.path
        ));
    }
    out.push(String::new());
    let skills = plan::needed_skills(facts, choices);
    let taken: Vec<&&Skill> = skills.iter().filter(|s| s.in_the_way()).collect();
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

/// The chat bridge: what is there, and every step, those done marked so.
pub fn bridge(facts: &Facts) -> String {
    let mut out = vec![
        "# Chat bridge".to_owned(),
        String::new(),
        "Optional. Message your agents from Telegram, through cc-connect.".to_owned(),
        "When an agent asks for permission, you still answer at the computer.".to_owned(),
        String::new(),
        found(facts, "cc-connect"),
    ];
    let config = facts.shown(&facts.cc_config);
    out.push(if facts.cc_config_exists {
        format!("  ✓ {:<18}{config}", "configuration")
    } else {
        format!("  ✗ {:<18}{config} not found", "configuration")
    });
    if let Some(command) = &facts.bridge_command {
        out.push(format!("  ✓ {:<18}{command}", "in agents.toml"));
    }
    if let Bridge::Unavailable(why) = plan::bridge(facts) {
        out.push(format!("  ✗ {:<18}{why}", "bridge service"));
    }
    out.push(String::new());

    let has_cc = facts.found.contains_key("cc-connect");
    out.push(step(1, "Install cc-connect", Who::You, has_cc));
    out.push(cmd("npm install -g cc-connect"));
    out.push(String::new());

    out.push(step(
        2,
        "Make a Telegram bot",
        Who::You,
        facts.cc_config_exists,
    ));
    out.push("In Telegram, send /newbot to @BotFather and keep the token it gives".into());
    out.push("you. Ask @userinfobot for your numeric user ID.".into());
    out.push(String::new());

    out.push(step(
        3,
        &format!("Create {config}"),
        Who::You,
        facts.cc_config_exists,
    ));
    out.push("Paste this, replacing the two <…> with your token and user ID:".into());
    let otto_agents = facts.found.get("otto-agents").map_or_else(
        || "/usr/bin/otto-agents".to_owned(),
        |p| p.display().to_string(),
    );
    let text = CC_CONFIG
        .replace("<home>", &facts.home.display().to_string())
        .replace("<the path of otto-agents>", &otto_agents);
    out.extend(text.lines().map(|line| format!("│ {line}")));
    out.push("The file holds your bot token, so make it readable only by you.".into());
    out.push("Then create the folder the bridge works in:".into());
    out.push(cmd("chmod 700 ~/.cc-connect"));
    out.push(cmd(format!("chmod 600 {config}")));
    out.push(cmd("mkdir -p ~/.local/state/otto/remote"));
    out.push(String::new());

    let on = matches!(plan::bridge(facts), Bridge::Running);
    out.push(step(4, "Turn it on", Who::Setup, on));
    out.push("Tick Chat bridge on the Requirements page and press Apply. Setup".into());
    out.push("adds cc-connect to agents.toml and starts the bridge. Turn it on or".into());
    out.push("off later in Settings › Agents › Chat bridge.".into());
    out.push(String::new());
    out.push("Who can message the agents, and what they can do from a phone:".into());
    out.push("components/otto-agents/docs/cc-connect.md in Otto's source code.".into());
    out.join("\n")
}

/// cc-connect's configuration with Otto's agents behind a Telegram bot, as
/// `docs/cc-connect.md` gives it.
const CC_CONFIG: &str = r#"language = "en"

[[projects]]
name = "otto"
disabled_commands = ["shell", "dir", "cd", "restart", "upgrade", "cron", "commands"]

[projects.agent]
type = "acp"

[projects.agent.options]
work_dir = "<home>/.local/state/otto/remote"
cmd = "<the path of otto-agents>"
args = ["acp"]
display_name = "Otto"

[[projects.platforms]]
type = "telegram"

[projects.platforms.options]
token = "<the bot token>"
allow_from = "<your user ID>""#;
