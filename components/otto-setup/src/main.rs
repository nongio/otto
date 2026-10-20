//! `otto-setup`: set Otto up the first time, from a terminal.
//!
//! Starts with the agents: picks the harnesses that are installed, writes
//! them into `agents.toml`, gives the service the PATH they live on, links
//! Otto's skills and starts the service, previewing each step before it
//! runs. The user guide is `docs/user/agents.md`.

// Rust guideline compliant 2026-02-21

mod apply;
mod plan;
mod probe;
mod tui;

use std::process::ExitCode;

use crate::plan::State;
use crate::probe::Facts;

const USAGE: &str = "\
Usage: otto-setup [agents] [options]

Set Otto up the first time. `agents` (the default, and for now the only
topic) picks the agents Otto runs, writes them into agents.toml, makes sure
the agent service finds them, and starts it. Nothing changes until you apply.

Options:
      --check    print what is set up and what is left, and exit 1 when
                 something is left to do
      --apply    do it without asking, with the agents that are installed
  -V, --version  print the version and exit
  -h, --help     print this and exit

With a terminal and no option, it opens an interactive setup; without one it
behaves as --check.
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
            other => return Err(format!("no setup called {other:?}; there is `agents`")),
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

/// Prints the plan for the agents that are installed.
fn check(facts: &Facts) -> ExitCode {
    let picks = plan::initial_picks(facts);
    println!("Agents");
    for harness in plan::HARNESSES {
        let name = harness.name().unwrap_or_default();
        let line = match plan::readiness(facts, harness) {
            plan::Readiness::Configured => "in your agent list".to_owned(),
            plan::Readiness::Installed(path) if picks.contains(&harness) => {
                format!("would add ({})", facts.shown(&path))
            }
            plan::Readiness::Installed(path) => facts.shown(&path),
            plan::Readiness::Missing(program) => format!("{program} not found"),
        };
        println!("  {name:<14}{line}");
    }
    println!("\nSteps");
    let steps = plan::steps(facts, &picks);
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

/// Applies the plan for the agents that are installed, saying how it went.
fn apply(facts: &Facts) -> ExitCode {
    let steps = plan::steps(facts, &plan::initial_picks(facts));
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
