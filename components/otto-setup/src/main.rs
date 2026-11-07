//! `otto-setup`: set Otto up the first time, from a terminal.
//!
//! Two topics so far. The agents: finds what is installed, adds agents for
//! the harnesses the user ticks with the instructions they pick, gives the
//! services the PATH their programs live on, links Otto's skills, and turns
//! the agent service and the chat bridge on. Dictation: finds the speech
//! engines `install.sh` left, picks one, starts its server and turns on
//! otto-dictate at login. Each step is previewed before it runs. What setup
//! does not do itself (install Node.js, a harness, an adapter, cc-connect,
//! an engine; sign in) it explains. The user guides are `docs/user/agents.md`
//! and the Dictation section of `docs/user/settings.md`.

// Rust guideline compliant 2026-02-21

mod apply;
mod dictation;
mod dictation_guide;
mod guide;
mod plan;
mod plugins;
mod probe;
mod tui;

use std::process::ExitCode;

use crate::apply::Check;
use crate::plan::{Bridge, Choices, State, Step};
use crate::probe::Facts;

const USAGE: &str = "\
Usage: otto-setup [agents|dictation] [options]

Sets Otto up the first time. Two topics:

  agents      the AI agents that answer in Ask and run your Sessions
              (the default). Adds the agents you pick to agents.toml,
              makes sure Otto's agent service can find them, and starts
              it; it can also turn on the Telegram chat bridge.
  dictation   speaking into the launcher, the Ask chat and any text
              field. Picks the speech engine, starts its server and
              starts otto-dictate at login.

It installs nothing itself: for anything missing (Node.js, an agent,
an adapter, cc-connect, a speech engine) it shows the commands to run.
Nothing changes until you apply. Settings has the rest.

Options:
      --check    print what is set up and what is left, and exit 1 when
                 something is left to do, by setup or by you
      --apply    apply without asking, with the choices setup would make
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

/// The topic named on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Topic {
    Agents,
    Dictation,
}

/// Parses the arguments after the program name.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(Topic, Command), String> {
    let mut command = Command::Interactive;
    let mut topic = Topic::Agents;
    for arg in args {
        match arg.as_str() {
            "agents" => topic = Topic::Agents,
            "dictation" => topic = Topic::Dictation,
            "--check" => command = Command::Check,
            "--apply" => command = Command::Apply,
            "-h" | "--help" => return Ok((topic, Command::Help)),
            "-V" | "--version" => return Ok((topic, Command::Version)),
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            other => {
                return Err(format!(
                    "no topic called {other:?}; the topics are `agents` and `dictation`"
                ))
            }
        }
    }
    Ok((topic, command))
}

fn main() -> ExitCode {
    let (topic, command) = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("otto-setup: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let interactive =
        otto_term_kit::is_tty(libc::STDIN_FILENO) && otto_term_kit::is_tty(libc::STDOUT_FILENO);
    match (command, topic) {
        (Command::Help, _) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        (Command::Version, _) => {
            println!("otto-setup {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        (Command::Interactive, _) if interactive => {
            match tui::run(Facts::read(), dictation::Facts::read()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("otto-setup: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        (Command::Interactive | Command::Check, Topic::Agents) => check(&Facts::read()),
        (Command::Interactive | Command::Check, Topic::Dictation) => {
            check_dictation(&dictation::Facts::read())
        }
        (Command::Apply, Topic::Agents) => {
            let facts = Facts::read();
            apply(
                &plan::steps(&facts, &Choices::initial(&facts)),
                Check::Agents,
            )
        }
        (Command::Apply, Topic::Dictation) => {
            let facts = dictation::Facts::read();
            apply(
                &dictation::steps(&facts, &dictation::Choices::initial(&facts)),
                Check::SpeechServer,
            )
        }
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
    print_steps(&plan::steps(facts, &choices), "otto-setup")
}

/// Prints what dictation has and lacks, and the plan.
fn check_dictation(facts: &dictation::Facts) -> ExitCode {
    use dictation::{Dictate, Status, ENGINES};
    let choices = dictation::Choices::initial(facts);
    println!("Engines");
    for engine in ENGINES {
        let line = match dictation::status(facts, engine) {
            Status::Running { enabled: true } => "running, starts at login".to_owned(),
            Status::Running { enabled: false } => "running".to_owned(),
            Status::Stopped { enabled: true } => "installed, stopped; starts at login".to_owned(),
            Status::Stopped { enabled: false } => "installed, off".to_owned(),
            Status::Missing(missing) => format!("not installed: missing {}", missing.join(" and ")),
            Status::Unmanaged => "no systemd user session".to_owned(),
        };
        let picked = if choices.engine == Some(engine) {
            " (picked)"
        } else {
            ""
        };
        println!("  {:<16}{line}{picked}", engine.name());
    }
    println!(
        "  {:<16}{}",
        "Speech server",
        if facts.answering {
            format!("answers at {}", dictation::SERVER)
        } else {
            format!("nothing at {}", dictation::SERVER)
        }
    );
    println!("\nRequirements");
    let pipewire = match (&facts.pipewire, facts.pipewire_running) {
        (Some(_), true) => "PipeWire running".to_owned(),
        (Some(path), false) => format!("PipeWire at {}, not running", facts.shown(path)),
        (None, _) => "PipeWire not installed".to_owned(),
    };
    println!("  {:<16}{pipewire}", "Microphone");
    println!(
        "  {:<16}{}",
        "GPU driver",
        if facts.vulkan {
            "Vulkan driver installed"
        } else {
            "no Vulkan driver"
        }
    );
    let missing = facts.missing_tools();
    let tools = if missing.is_empty() {
        "all there".to_owned()
    } else {
        format!("missing {}", missing.join(", "))
    };
    println!("  {:<16}{tools}", "Build tools");
    let source = facts
        .source
        .as_deref()
        .map_or_else(|| "no checkout found".to_owned(), |dir| facts.shown(dir));
    println!("  {:<16}{source}", "Otto's source");
    println!("\nOptional");
    let dictate = match dictation::dictate(facts) {
        Dictate::NotInstalled => "otto-dictate not installed".to_owned(),
        Dictate::Off => "otto-dictate installed, not started at login".to_owned(),
        Dictate::On => "otto-dictate starts at login".to_owned(),
    };
    println!("  {:<16}{dictate}", "Other fields");
    let shortcut = facts
        .shortcut
        .as_deref()
        .map_or("none bound to otto-dictate", |key| key);
    println!("  {:<16}{shortcut}", "Shortcut");
    print_steps(&dictation::steps(facts, &choices), "otto-setup dictation")
}

/// Prints the steps, and exits 1 when one is left to do.
fn print_steps(steps: &[Step], command: &str) -> ExitCode {
    println!("\nSteps");
    for step in steps {
        let mark = match step.state {
            State::Done => "ok  ",
            State::Todo => "todo",
            State::Blocked => "!!  ",
        };
        println!("  [{mark}] {:<18} {}", step.title, step.summary);
    }
    let todo = steps.iter().any(|s| s.state == State::Todo);
    let blocked = steps.iter().any(|s| s.state == State::Blocked);
    if todo {
        println!("\nRun {command} in a terminal to choose and apply, or {command} --apply.");
    } else if blocked {
        println!("\nSomething is left to do by hand: {command} in a terminal shows the steps.");
    }
    if todo || blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Applies the steps, saying how it went.
fn apply(steps: &[Step], check: Check) -> ExitCode {
    if steps.iter().all(|s| s.state != State::Todo) {
        println!("Nothing to do.");
        return ExitCode::SUCCESS;
    }
    let outcomes = apply::run(steps, check);
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

    fn parse(args: &[&str]) -> Result<(Topic, Command), String> {
        parse_args(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn parses_the_topic_and_the_modes() {
        assert_eq!(parse(&[]), Ok((Topic::Agents, Command::Interactive)));
        assert_eq!(
            parse(&["agents", "--check"]),
            Ok((Topic::Agents, Command::Check))
        );
        assert_eq!(parse(&["--apply"]), Ok((Topic::Agents, Command::Apply)));
        assert_eq!(
            parse(&["dictation", "--check"]),
            Ok((Topic::Dictation, Command::Check))
        );
        assert!(parse(&["display"]).is_err());
        assert!(parse(&["--frobnicate"]).is_err());
    }
}
