//! The instructions: for each thing setup looks at, what is there, and what
//! the user runs for what is not. Setup never runs an install itself.

// Rust guideline compliant 2026-02-21

use otto_agents_client::harness::{self, Harness};

use crate::plan::{self, Bridge, Readiness};
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
                out.push("npm comes with Node.js: see Node.js on the Extras page.".into());
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

/// What the instruction choice means, and the sets there are.
pub fn instructions(facts: &Facts, persona: &str) -> String {
    let mut out = vec![
        "# Instructions".to_owned(),
        String::new(),
        "Who the new agents are. With Otto's, an agent answers as Otto and".to_owned(),
        "knows the desktop: its skills, its settings, its documentation. With".to_owned(),
        "the harness's own, it is the plain harness.".to_owned(),
        String::new(),
        "Only agents setup adds now get this; Settings › Agents changes it for".to_owned(),
        "any agent later.".to_owned(),
        String::new(),
    ];
    for name in plan::Choices::personas(facts) {
        let mark = if name == persona { "▸" } else { " " };
        out.push(format!("  {mark} {}", persona_label(&name)));
    }
    if facts.instructions.is_empty() {
        out.push(String::new());
        out.push("No plugin with instructions was found: is Otto's plugin installed".into());
        out.push("(/usr/share/otto/plugins/otto)?".into());
    }
    out.join("\n")
}

/// How an instruction set is named in the interface.
pub fn persona_label(name: &str) -> String {
    match name {
        "" => "the harness's own".to_owned(),
        harness::OTTO => "Otto".to_owned(),
        other => other.to_owned(),
    }
}

/// Node.js: where it is, or how to install it.
pub fn node(facts: &Facts) -> String {
    let mut out = vec!["# Node.js".to_owned(), String::new()];
    out.push("Claude Code and Codex reach Otto through ACP adapters written for".to_owned());
    out.push("Node.js, and pi and cc-connect install with npm.".to_owned());
    out.push(String::new());
    for program in ["node", "npm", "npx"] {
        out.push(found(facts, program));
    }
    out.push(String::new());
    if let Some(version) = &facts.node_version {
        out.push(format!("Installed: {version}. Nothing to do."));
        return out.join("\n");
    }
    out.push("# Install".to_owned());
    match node_install(facts) {
        Some(command) => out.push(format!("  {command}")),
        None => out.push("  from your distribution's packages, or nodejs.org".to_owned()),
    }
    out.push(String::new());
    out.push("Or, to keep it in your home folder, nvm (github.com/nvm-sh/nvm).".to_owned());
    out.push("Setup then adds nvm's folder to the agent service's PATH.".to_owned());
    out.join("\n")
}

/// The ACP adapters: which are installed, and how to install the rest.
pub fn adapters(facts: &Facts) -> String {
    let mut out = vec![
        "# ACP adapters".to_owned(),
        String::new(),
        "Claude Code, Codex and pi do not speak ACP, the protocol Otto talks to".to_owned(),
        "agents in; an adapter stands between. Installed, it starts at once and".to_owned(),
        "new agents use it. Not installed, Claude Code and Codex fetch it with".to_owned(),
        "npx at every start; pi needs it installed.".to_owned(),
        String::new(),
    ];
    let mut missing = Vec::new();
    for harness in plan::HARNESSES {
        if let Some(adapter) = harness.adapter() {
            out.push(found(facts, adapter.bin));
            if !facts.found.contains_key(adapter.bin) {
                missing.push(adapter.npm);
            }
        }
    }
    out.push(String::new());
    if missing.is_empty() {
        out.push("All installed. Nothing to do.".to_owned());
    } else {
        out.push("# Install".to_owned());
        out.push(format!("  npm install -g {}", missing.join(" ")));
        out.push(String::new());
        out.push("Agents already in your list keep their command: point them at the".to_owned());
        out.push("adapter in Settings › Agents if you want them to use it.".to_owned());
    }
    out.join("\n")
}

/// The chat bridge: what is there, and the steps that are the user's.
pub fn bridge(facts: &Facts) -> String {
    let mut out = vec![
        "# Chat bridge".to_owned(),
        String::new(),
        "Talk to Otto's agents from a chat app, Telegram for one, through".to_owned(),
        "cc-connect. Permission requests are still answered at the desk.".to_owned(),
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
    match plan::bridge(facts) {
        Bridge::Unavailable(why) => out.push(format!("Not available: {why}.")),
        Bridge::Running => {
            out.push("Running. Settings › Agents › Chat bridge turns it off.".into())
        }
        Bridge::Off => {
            out.push("Set up, but off. Tick it to start it now and at every login.".into())
        }
        Bridge::Ready => {
            out.push("Ready. Tick it and setup points [bridge] in agents.toml at".into());
            out.push("cc-connect with your configuration, and starts the bridge.".into());
        }
        Bridge::Missing(_) => {
            // Only the steps still to do, numbered as they come.
            let mut n = 0;
            let mut heading = |out: &mut Vec<String>, text: &str| {
                n += 1;
                out.push(format!("# {n}. {text}"));
            };
            if !facts.found.contains_key("cc-connect") {
                heading(&mut out, "Install cc-connect");
                out.push("  npm install -g cc-connect".into());
                out.push(String::new());
            }
            if !facts.cc_config_exists {
                heading(&mut out, "Make a Telegram bot");
                out.push("In Telegram, send /newbot to @BotFather and keep the token it".into());
                out.push("gives you. Ask @userinfobot for your numeric user ID.".into());
                out.push(String::new());
                heading(&mut out, &format!("Write {config}"));
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
            }
            out.push("Then run otto-setup again and tick the bridge. Who can reach the".into());
            out.push("agents, and what the phone may do: components/otto-agents/docs/".into());
            out.push("cc-connect.md, in Otto's source.".into());
        }
    }
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
