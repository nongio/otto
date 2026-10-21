//! The instructions: for each thing setup looks at, what is there, and the
//! steps to have it. Setup never runs an install itself.
//!
//! The text marks what each line is, so the interface can tell them apart
//! and `--check` readers can too:
//!
//! - `# Title` a heading;
//! - `#you 1. Step`, `#setup 1. Step`, `#done 1. Step` a step, and who does
//!   it: the user, setup on Apply, or nobody since it is done;
//! - `$ command` a command to run, one per line, nothing else on it;
//! - `│ text` the contents of a file to write;
//! - `  ✓ …` and `  ✗ …` what was found;
//! - anything else, prose.

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
        Readiness::Configured => "Already in your agent list.".to_owned(),
        Readiness::Ready(_) => "Ready: tick it to add an agent that runs on it.".to_owned(),
        Readiness::Missing(missing) => format!(
            "Missing {}: do the steps below, then run otto-setup again.",
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
            out.push("Hermes Agent is a Python program from Nous Research: install it as".into());
            out.push("its own guide says (github.com/NousResearch/hermes-agent), so that".into());
            out.push("`hermes` is on your PATH.".into());
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
        out.push(step(
            n,
            "Install its ACP adapter",
            Who::You,
            has(adapter.bin),
        ));
        if harness == Harness::Pi {
            out.push("pi reaches Otto only through its adapter:".into());
        } else {
            out.push("Without it, npx fetches the adapter at every start. Once:".into());
        }
        out.push(cmd(format!("npm install -g {}", adapter.npm)));
        out.push(String::new());
    }

    n += 1;
    out.push(step(n, "Sign in", Who::You, false));
    out.push("Otto uses the harness's own login and never sees your keys.".into());
    match harness {
        Harness::Claude => out.push(cmd("claude auth login")),
        Harness::Codex => out.push(cmd("codex login")),
        Harness::OpenCode => out.push(cmd("opencode auth login")),
        Harness::Hermes => out.push(cmd("hermes setup")),
        Harness::Pi => {
            out.push(cmd("pi"));
            out.push("then type /login inside it, and follow it.".into());
        }
        Harness::Custom => {}
    }
    if harness == Harness::Hermes {
        out.push(String::new());
        n += 1;
        let profile = facts.hermes_profiles.iter().any(|p| p == harness::OTTO);
        out.push(step(n, "Make Otto's profile", Who::Setup, profile));
        out.push("Hermes runs an agent in a profile named after it. Apply runs:".into());
        out.push(cmd(format!("hermes profile create {}", harness::OTTO)));
    }
    out.join("\n")
}

/// What ticking Otto does.
pub fn otto(facts: &Facts) -> String {
    let mut out = vec![
        "# Otto".to_owned(),
        String::new(),
        "Ticked, the agents setup adds answer as Otto: in Ask they know your".to_owned(),
        "desktop, its settings and its documentation, and can change things for".to_owned(),
        "you. Unticked, they are the plain harness (Claude Code, Codex, …).".to_owned(),
        String::new(),
        "Agents already in your list keep what they have; Settings › Agents".to_owned(),
        "changes it.".to_owned(),
    ];
    if !facts.instructions.iter().any(|n| n == harness::OTTO) {
        out.push(String::new());
        out.push("Otto's plugin was not found (/usr/share/otto/plugins/otto): reinstall".into());
        out.push("Otto's package.".into());
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
        "on Claude Code, so a Claude Code agent has to be in your list.".to_owned(),
        String::new(),
    ];
    let state = plan::studio(facts, choices);
    match state {
        plan::Studio::Unavailable => {
            out.push("Otto's plugin has no Studio here: reinstall Otto's package.".into());
            return out.join("\n");
        }
        plan::Studio::Ready if facts.configured.contains(&Harness::Claude) => {
            out.push("  ✓ Claude Code agent   in your agent list".into());
        }
        plan::Studio::Ready => out.push("  ✓ Claude Code agent   added by setup".into()),
        plan::Studio::NeedsClaude => out.push("  ✗ Claude Code agent   none".into()),
    }
    out.push(String::new());
    let ready = state == plan::Studio::Ready;
    out.push(step(1, "Have a Claude Code agent", Who::You, ready));
    out.push("Tick Claude Code on this page; if it is not installed, its row says".into());
    out.push("how.".into());
    out.push(String::new());
    out.push(step(
        2,
        "Give Claude Code the Studio instructions",
        Who::Setup,
        false,
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
        "Required. Claude Code and Codex reach Otto through ACP adapters".to_owned(),
        "written for Node.js, and pi and cc-connect install with npm.".to_owned(),
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
        None => out.push("From your distribution's packages, or nodejs.org.".to_owned()),
    }
    out.push("Or keep it in your home folder with nvm (github.com/nvm-sh/nvm);".to_owned());
    out.push("setup then adds nvm's folder to the agent service's PATH.".to_owned());
    out.push(String::new());
    out.push(step(2, "Check", Who::You, have));
    out.push(cmd("node --version"));
    if let Some(version) = &facts.node_version {
        out.push(format!("It prints {version} here."));
    }
    out.join("\n")
}

/// The ACP adapters: which are installed, and how to install them.
pub fn adapters(facts: &Facts) -> String {
    let mut out = vec![
        "# ACP adapters".to_owned(),
        String::new(),
        "Required. Claude Code, Codex and pi do not speak ACP, the protocol".to_owned(),
        "Otto talks to agents in: an adapter stands between, and without it the".to_owned(),
        "agent does not start. Installed, it starts at once and new agents use".to_owned(),
        "it. Not installed, Claude Code and Codex fetch it with npx at every".to_owned(),
        "start; pi needs it installed.".to_owned(),
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
    out.push("All of them, or only those of your agents:".to_owned());
    out.push(cmd(format!("npm install -g {}", all.join(" "))));
    out.push(String::new());
    out.push(step(2, "Check", Who::You, installed));
    out.push(cmd("otto-setup --check"));
    out.push(String::new());
    out.push("Agents already in your list keep their command: point them at the".to_owned());
    out.push("adapter in Settings › Agents if you want them to use it.".to_owned());
    out.join("\n")
}

/// Otto's skills: linked where agents look for them, or what is in the way.
pub fn skills(facts: &Facts) -> String {
    let mut out = vec![
        "# Otto's skills".to_owned(),
        String::new(),
        "What the agents know about the desktop: how to change a setting, send".to_owned(),
        "a file to your phone, and so on. They are linked into ~/.agents/skills,".to_owned(),
        "where every harness looks for skills.".to_owned(),
        String::new(),
    ];
    let skills = plan::skills(facts);
    if facts.plugins.is_none() {
        out.push("  ✗ otto-agents could not say: is it installed?".to_owned());
    }
    for skill in skills {
        let mark = if skill.linked() { "✓" } else { "✗" };
        out.push(format!(
            "  {mark} {:<18}{:<12}{}",
            skill.name, skill.state, skill.path
        ));
    }
    out.push(String::new());
    let blocked: Vec<&Skill> = skills.iter().filter(|s| s.in_the_way()).collect();
    let mut n = 0;
    if !blocked.is_empty() {
        n += 1;
        out.push(step(n, "Move aside what is in the way", Who::You, false));
        out.push("Something of yours is where the link goes, and the installer leaves".into());
        out.push("it alone. Rename it:".into());
        for skill in &blocked {
            out.push(cmd(format!("mv {0} {0}.mine", skill.path)));
        }
        out.push(String::new());
    }
    let linked = !skills.is_empty() && skills.iter().all(Skill::linked);
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

/// Each selected harness's copy of Otto's agent.
pub fn profiles(facts: &Facts, choices: &plan::Choices) -> String {
    let mut out = vec![
        "# Agent profiles".to_owned(),
        String::new(),
        "Every harness but Claude Code reads Otto's agent from a copy in its".to_owned(),
        "own format and place: OpenCode's agents folder, Codex's config, pi's".to_owned(),
        "wrapper, Hermes's profile. Claude Code reads the plugin itself.".to_owned(),
        String::new(),
    ];
    let profiles = plan::profiles(facts, choices);
    if !choices.otto {
        out.push("  ✓ none: Otto is unticked, the new agents are the plain harness".to_owned());
    } else if profiles.is_empty() {
        out.push("  ✓ none needed for the agents you selected".to_owned());
    }
    for render in &profiles {
        let mark = if render.current() { "✓" } else { "✗" };
        out.push(format!(
            "  {mark} {:<10}{:<11}{}",
            render.vendor, render.state, render.path
        ));
    }
    out.push(String::new());
    let current = profiles.iter().all(|r| r.current() || r.state == "skipped");
    out.push(step(1, "Write them", Who::Setup, current));
    out.push("Apply runs:".into());
    out.push(cmd("otto-agents plugins install"));
    out.push(String::new());
    let skipped = profiles.iter().any(|r| r.state == "skipped");
    if skipped {
        out.push(step(2, "Make Hermes's profile", Who::Setup, false));
        out.push("When you add a Hermes agent, Apply runs:".into());
        out.push(cmd(format!("hermes profile create {}", harness::OTTO)));
        out.push(String::new());
    }
    out.push(step(
        if skipped { 3 } else { 2 },
        "Check",
        Who::You,
        current && !skipped,
    ));
    out.push(cmd("otto-agents plugins status"));
    out.join("\n")
}

/// The chat bridge: what is there, and every step, those done marked so.
pub fn bridge(facts: &Facts) -> String {
    let mut out = vec![
        "# Chat bridge".to_owned(),
        String::new(),
        "Optional. Message your agents from Telegram, or another chat app,".to_owned(),
        "through cc-connect. Permission requests are still answered at the desk.".to_owned(),
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
        out.push(format!("  ✓ {:<18}{command}", "[bridge]"));
    }
    if let Bridge::Unavailable(why) = plan::bridge(facts) {
        out.push(format!("  ✗ {:<18}{why}", "bridge unit"));
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
        &format!("Write {config}"),
        Who::You,
        facts.cc_config_exists,
    ));
    out.push("With your token and user ID in place of the <…>:".into());
    let otto_agents = facts.found.get("otto-agents").map_or_else(
        || "/usr/bin/otto-agents".to_owned(),
        |p| p.display().to_string(),
    );
    let text = CC_CONFIG
        .replace("<home>", &facts.home.display().to_string())
        .replace("<the path of otto-agents>", &otto_agents);
    out.extend(text.lines().map(|line| format!("│ {line}")));
    out.push("It holds the token, so keep it yours alone, and make the folder the".into());
    out.push("bridge works in:".into());
    out.push(cmd("chmod 700 ~/.cc-connect"));
    out.push(cmd(format!("chmod 600 {config}")));
    out.push(cmd("mkdir -p ~/.local/state/otto/remote"));
    out.push(String::new());

    let on = matches!(plan::bridge(facts), Bridge::Running);
    out.push(step(4, "Turn it on", Who::Setup, on));
    out.push("Tick the bridge and apply: setup points [bridge] in agents.toml at".into());
    out.push("cc-connect and starts the bridge. Settings › Agents › Chat bridge".into());
    out.push("turns it on and off later.".into());
    out.push(String::new());
    out.push("Who can reach the agents, and what the phone may do:".into());
    out.push("components/otto-agents/docs/cc-connect.md, in Otto's source.".into());
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
