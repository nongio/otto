//! The instructions: for each thing setup looks at, what is there, and what
//! the user runs for what is not. Setup never runs an install itself.

// Rust guideline compliant 2026-02-21

use otto_agents_client::harness::{self, Harness};

use crate::plan::{self, Bridge, Readiness};
use crate::plugins::Skill;
use crate::probe::Facts;

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
    match plan::readiness(facts, harness) {
        Readiness::Configured => out.push("Already in your agent list.".into()),
        Readiness::Ready(_) => out.push("Ready: tick it to add an agent that runs on it.".into()),
        Readiness::Missing(missing) => out.push(format!(
            "Missing: {}. Install it as below, then run otto-setup again.",
            missing.join(", ")
        )),
    }
    out.push(String::new());
    let mut programs: Vec<&str> = harness.program().into_iter().collect();
    if let Some(adapter) = harness.adapter() {
        programs.push(adapter.bin);
        programs.push("npx");
    }
    out.extend(programs.iter().map(|p| found(facts, p)));
    out.push(String::new());

    out.push("# Install".into());
    match (harness, harness.package()) {
        (Harness::Hermes, _) => {
            out.push("Hermes Agent is a Python program from Nous Research. Install it as".into());
            out.push("its own guide says (github.com/NousResearch/hermes-agent), so that".into());
            out.push("`hermes` is on your PATH.".into());
        }
        (_, Some(package)) => {
            out.push(format!("  npm install -g {}", package.npm));
            if !facts.found.contains_key("npm") {
                out.push("npm comes with Node.js: see Node.js on the Requirements page.".into());
            }
        }
        (_, None) => {}
    }
    if let Some(adapter) = harness.adapter() {
        out.push(String::new());
        if harness == Harness::Pi {
            out.push("pi speaks to Otto through its ACP adapter, which must be installed:".into());
        } else {
            out.push("Otto starts it through its ACP adapter. Without it installed, npx".into());
            out.push("fetches the adapter from npm at every start; to install it once:".into());
        }
        out.push(format!("  npm install -g {}", adapter.npm));
    }
    out.push(String::new());

    out.push("# Sign in".into());
    out.push("Otto uses the harness's own login; it never sees your keys.".into());
    out.extend(
        match harness {
            Harness::Claude => vec!["  claude          then /login, and follow it"],
            Harness::Codex => vec!["  codex login"],
            Harness::OpenCode => vec!["  opencode auth login"],
            Harness::Hermes => vec!["  hermes setup    pick a model provider, as its guide says"],
            Harness::Pi => vec!["  pi              then /login, and follow it"],
            Harness::Custom => vec![],
        }
        .into_iter()
        .map(str::to_owned),
    );
    if harness == Harness::Hermes {
        out.push(String::new());
        out.push("Hermes runs an agent in a profile named after its instructions".into());
        out.push(format!(
            "({}, for Otto's); setup creates it with `hermes profile create`",
            harness::OTTO
        ));
        out.push("when it is missing.".into());
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
    match plan::studio(facts, choices) {
        plan::Studio::Unavailable => {
            out.push("Otto's plugin has no Studio here: reinstall Otto's package.".into());
        }
        plan::Studio::Ready if facts.configured.contains(&Harness::Claude) => {
            out.push("  ✓ Claude Code agent   in your agent list".into());
        }
        plan::Studio::Ready => out.push("  ✓ Claude Code agent   added by setup".into()),
        plan::Studio::NeedsClaude => {
            out.push("  ✗ Claude Code agent   none".into());
            out.push(String::new());
            out.push("Tick Claude Code above; if it is not installed, its row shows how.".into());
        }
    }
    out.push(String::new());
    out.push("Nothing else: the Review page's `otto-agents plugins install` gives".into());
    out.push("Claude Code the Studio instructions.".into());
    out.join("\n")
}

/// A heading marked done when `done`, so the instructions stay whole even
/// for what is already there.
fn step(n: usize, text: &str, done: bool) -> String {
    if done {
        format!("# {n}. {text}  (done)")
    } else {
        format!("# {n}. {text}")
    }
}

/// Node.js: where it is, and how to install it.
pub fn node(facts: &Facts) -> String {
    let mut out = vec![
        "# Node.js".to_owned(),
        String::new(),
        "Required.".to_owned(),
    ];
    out.push("Claude Code and Codex reach Otto through ACP adapters written for".to_owned());
    out.push("Node.js, and pi and cc-connect install with npm.".to_owned());
    out.push(String::new());
    for program in ["node", "npm", "npx"] {
        out.push(found(facts, program));
    }
    out.push(String::new());
    out.push(match &facts.node_version {
        Some(version) => format!("Installed: {version}."),
        None => "Not installed.".to_owned(),
    });
    out.push(String::new());
    out.push(step(1, "Install", facts.node_version.is_some()));
    match node_install(facts) {
        Some(command) => out.push(format!("  {command}")),
        None => out.push("  from your distribution's packages, or nodejs.org".to_owned()),
    }
    out.push(String::new());
    out.push("Or, to keep it in your home folder, nvm (github.com/nvm-sh/nvm).".to_owned());
    out.push("Setup then adds nvm's folder to the agent service's PATH.".to_owned());
    out.push(String::new());
    out.push(step(2, "Check", facts.node_version.is_some()));
    out.push("  node --version".to_owned());
    out.join("\n")
}

/// The ACP adapters: which are installed, and how to install them.
pub fn adapters(facts: &Facts) -> String {
    let mut out = vec![
        "# ACP adapters".to_owned(),
        String::new(),
        "Required. Claude Code, Codex and pi do not speak ACP, the protocol".to_owned(),
        "Otto talks to agents in; an adapter stands between, and without it the".to_owned(),
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
    out.push(step(1, "Install the ones your agents need", installed));
    out.push(format!("  npm install -g {}", all.join(" ")));
    out.push(String::new());
    out.push(step(2, "Check", installed));
    out.push("  otto-setup --check".to_owned());
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
        out.push("  otto-agents could not say: is it installed?".to_owned());
    }
    for skill in skills {
        let mark = if skill.linked() { "✓" } else { "✗" };
        out.push(format!(
            "  {mark} {:<18}{:<12}{}",
            skill.name, skill.state, skill.path
        ));
    }
    out.push(String::new());
    let linked = !skills.is_empty() && skills.iter().all(Skill::linked);
    out.push(step(1, "Link them", linked));
    out.push("  otto-agents plugins install".to_owned());
    out.push("Apply runs it for you.".to_owned());
    out.push(String::new());
    let in_the_way = skills.iter().any(Skill::in_the_way);
    out.push(step(2, "Move aside what is in the way", !in_the_way));
    out.push("A skill \"in the way\" has a file or folder of yours where its link".to_owned());
    out.push("goes; the installer leaves it alone. Move it aside, for example".to_owned());
    out.push("  mv ~/.agents/skills/otto-help ~/.agents/skills/otto-help.mine".to_owned());
    out.push("then link again.".to_owned());
    out.push(String::new());
    out.push(step(3, "Check", linked));
    out.push("  otto-agents plugins status".to_owned());
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
        out.push("  Otto is unticked: the new agents are the plain harness.".to_owned());
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
    let current = profiles.iter().all(|r| r.current());
    out.push(step(1, "Write them", current));
    out.push("  otto-agents plugins install".to_owned());
    out.push("Apply runs it for you.".to_owned());
    out.push(String::new());
    let skipped = profiles.iter().any(|r| r.state == "skipped");
    out.push(step(2, "Make Hermes's profile", !skipped));
    out.push(format!("  hermes profile create {}", harness::OTTO));
    out.push("Apply runs it for you when you add a Hermes agent.".to_owned());
    out.push(String::new());
    out.push(step(3, "Check", current));
    out.push("  otto-agents plugins status".to_owned());
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
    out.push(String::new());
    out.push(match plan::bridge(facts) {
        Bridge::Unavailable(why) => format!("Not available: {why}."),
        Bridge::Running => "Running. Settings › Agents › Chat bridge turns it off.".into(),
        Bridge::Off => "Set up, but off. Tick it to start it now and at every login.".into(),
        Bridge::Ready => "Ready. Tick it, and setup points [bridge] in agents.toml at \
                          cc-connect and starts the bridge."
            .into(),
        Bridge::Missing(_) => "Not set up yet. The steps:".into(),
    });
    out.push(String::new());
    let has_cc = facts.found.contains_key("cc-connect");
    out.push(step(1, "Install cc-connect", has_cc));
    out.push("  npm install -g cc-connect".into());
    out.push(String::new());
    out.push(step(2, "Make a Telegram bot", facts.cc_config_exists));
    out.push("In Telegram, send /newbot to @BotFather and keep the token it".into());
    out.push("gives you. Ask @userinfobot for your numeric user ID.".into());
    out.push(String::new());
    out.push(step(3, &format!("Write {config}"), facts.cc_config_exists));
    out.push("Readable by you alone (chmod 600): it holds the token.".into());
    out.push(String::new());
    let otto_agents = facts.found.get("otto-agents").map_or_else(
        || "/usr/bin/otto-agents".to_owned(),
        |p| p.display().to_string(),
    );
    let text = CC_CONFIG
        .replace("<home>", &facts.home.display().to_string())
        .replace("<the path of otto-agents>", &otto_agents);
    for line in text.lines() {
        out.push(format!("  {line}"));
    }
    out.push(String::new());
    out.push("  mkdir -p ~/.local/state/otto/remote".into());
    out.push(String::new());
    let on = matches!(plan::bridge(facts), Bridge::Running);
    out.push(step(4, "Turn it on", on));
    out.push("Tick the bridge here and apply, or Settings › Agents › Chat bridge.".into());
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
