//! `otto-setup`: set Otto up the first time, from a terminal.
//!
//! Starts with the agents: finds what is installed, adds agents for the
//! harnesses the user ticks with the instructions they pick, gives the
//! services the PATH their programs live on, links Otto's skills, and turns
//! the agent service and the chat bridge on, previewing each step before it
//! runs. What it does not do itself (install Node.js, a harness, an ACP
//! adapter or cc-connect, sign in) it explains. The user guide is
//! `docs/user/agents.md`.

// Rust guideline compliant 2026-02-21

mod apply;
mod guide;
mod plan;
mod plugins;
mod probe;
mod tui;

use std::process::ExitCode;

use crate::plan::{Bridge, Choices, State};
use crate::probe::Facts;

const USAGE: &str = "\
Usage: otto-setup [agents] [options]

Sets Otto up the first time. For now there is one topic, `agents`
(the default): the AI agents that answer in Ask and run your Sessions.
It adds the agents you pick to agents.toml, makes sure Otto's agent
service can find them, and starts it; it can also turn on the Telegram
chat bridge. It installs nothing itself: for anything missing (Node.js,
an agent, an adapter, cc-connect) it shows the commands to run.
Nothing changes until you apply. Settings › Agents has the rest.

Options:
      --check    print what is set up and what is left, and exit 1 when
                 something is left to do
      --apply    apply without asking, adding every agent that is ready
  -V, --version  print the version and exit
  -h, --help     print this and exit

Run in a terminal with no option, it opens the interactive setup;
otherwise it behaves as --check.
";

/// What the command line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Interactive,
    Check,
    Apply,
    Help,
    Version,
}

/// Parses the arguments after the program name.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut command = Command::Interactive;
    for arg in args {
        match arg.as_str() {
            "agents" => {}
            "--check" => command = Command::Check,
            "--apply" => command = Command::Apply,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            other => {
                return Err(format!(
                    "no topic called {other:?}; the only one is `agents`"
                ))
            }
        }
    }
    Ok(command)
}

fn main() -> ExitCode {
    let command = match parse_args(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("otto-setup: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let interactive =
        otto_term_kit::is_tty(libc::STDIN_FILENO) && otto_term_kit::is_tty(libc::STDOUT_FILENO);
    match command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("otto-setup {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Interactive if interactive => match tui::run(Facts::read()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("otto-setup: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Interactive | Command::Check => check(&Facts::read()),
        Command::Apply => apply(&Facts::read()),
    }
}

/// Prints what is there and the plan for the agents that are ready.
fn check(facts: &Facts) -> ExitCode {
    let choices = Choices::initial(facts);
    println!("Agents");
    for harness in plan::HARNESSES {
        let name = harness.name().unwrap_or_default();
        let line = match plan::readiness(facts, harness) {
            plan::Readiness::Configured => "already added".to_owned(),
            plan::Readiness::Ready(path) if choices.picks.contains(&harness) => {
                format!("ready to add ({})", facts.shown(&path))
            }
            plan::Readiness::Ready(path) => facts.shown(&path),
            plan::Readiness::Missing(missing) => format!("needs {}", missing.join(", ")),
        };
        println!("  {name:<16}{line}");
    }
    let yes = |on: bool| if on { "on" } else { "off" };
    println!("  {:<16}{}", "Otto", yes(choices.otto));
    let studio = match plan::studio(facts, &choices) {
        plan::Studio::Ready => "ready",
        plan::Studio::NeedsClaude => "needs Claude Code",
        plan::Studio::Unavailable => "Studio's files are missing",
    };
    println!("  {:<16}{studio}", "Studio");
    println!("\nRequirements");
    let node = match plan::node(facts) {
        plan::Node::Missing => "not installed".to_owned(),
        plan::Node::TooOld(major) => {
            format!("v{major} is too old: needs {} or newer", plan::NODE_MIN)
        }
        plan::Node::Ok => facts.node_version.clone().unwrap_or_default(),
    };
    println!("  {:<16}{node}", "Node.js");
    let adapters: Vec<String> = plan::adapters(facts, &choices)
        .into_iter()
        .filter_map(|(harness, state)| {
            let bin = harness.adapter()?.bin;
            Some(match state {
                plan::Adapter::Installed => format!("{bin} installed"),
                plan::Adapter::Npx => format!("{} downloads its own", harness.name()?),
                plan::Adapter::Missing => format!("{bin} missing"),
            })
        })
        .collect();
    let adapters = if adapters.is_empty() {
        "none needed".to_owned()
    } else {
        adapters.join(", ")
    };
    println!("  {:<16}{adapters}", "Adapters");
    let skills = plan::skills(facts);
    let skills = if facts.plugins.is_none() {
        "otto-agents cannot say".to_owned()
    } else {
        skills
            .iter()
            .map(|s| format!("{} {}", s.name, guide::skill_state(s)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("\nFound");
    println!("  {:<16}{skills}", "Otto's skills");
    // One entry per harness: a profile can be several files.
    let mut profiles: Vec<String> = plan::profiles(facts, &choices)
        .iter()
        .map(|r| {
            format!(
                "{} {}",
                guide::vendor_name(&r.vendor),
                guide::profile_state(&r.state)
            )
        })
        .collect();
    profiles.dedup();
    let profiles = if profiles.is_empty() {
        "none needed".to_owned()
    } else {
        profiles.join(", ")
    };
    println!("  {:<16}{profiles}", "Agent profiles");
    println!("\nOptional");
    let cc = facts
        .found
        .get("cc-connect")
        .map_or_else(|| "not installed".to_owned(), |p| facts.shown(p));
    println!("  {:<16}{cc}", "cc-connect");
    let bridge = match plan::bridge(facts) {
        Bridge::Running => "running".to_owned(),
        Bridge::Off => "set up, off".to_owned(),
        Bridge::Ready => "ready to turn on".to_owned(),
        Bridge::Missing(missing) => format!("needs {}", missing.join(", ")),
        Bridge::Unavailable(why) => why.to_owned(),
    };
    println!("  {:<16}{bridge}", "Chat bridge");
    println!("\nSteps");
    let steps = plan::steps(facts, &choices);
    for step in &steps {
        let mark = match step.state {
            State::Done => "ok  ",
            State::Todo => "todo",
            State::Blocked => "!!  ",
        };
        println!("  [{mark}] {:<18}{}", step.title, step.summary);
    }
    if steps.iter().any(|s| s.state == State::Todo) {
        println!("\nRun otto-setup in a terminal to choose and apply, or otto-setup --apply.");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Applies the plan for the agents that are ready, saying how it went.
fn apply(facts: &Facts) -> ExitCode {
    let steps = plan::steps(facts, &Choices::initial(facts));
    if steps.iter().all(|s| s.state != State::Todo) {
        println!("Nothing to do.");
        return ExitCode::SUCCESS;
    }
    let outcomes = apply::run(&steps);
    for outcome in &outcomes {
        let mark = if outcome.ok { "ok  " } else { "fail" };
        println!("[{mark}] {}", outcome.title);
        for line in outcome.log.lines() {
            println!("       {line}");
        }
    }
    if outcomes.iter().all(|o| o.ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn parses_the_topic_and_the_modes() {
        assert_eq!(parse(&[]), Ok(Command::Interactive));
        assert_eq!(parse(&["agents", "--check"]), Ok(Command::Check));
        assert_eq!(parse(&["--apply"]), Ok(Command::Apply));
        assert!(parse(&["display"]).is_err());
        assert!(parse(&["--frobnicate"]).is_err());
    }
}
