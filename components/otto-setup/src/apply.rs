//! Running the plan: each step's writes and commands, in order, then a
//! check to say whether it worked: `otto-agents doctor` for the agents, a
//! knock on the speech server's door for dictation.

// Rust guideline compliant 2026-02-21

use std::io::Write as _;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::dictation;
use crate::plan::{Action, State, Step};

/// The check that ends a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// `otto-agents doctor`.
    Agents,
    /// Whether the speech server answers.
    SpeechServer,
}

/// How one step went.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The step's title, or "Check" for the doctor run.
    pub title: &'static str,
    /// Whether every action succeeded.
    pub ok: bool,
    /// What was written and run, and what the commands printed.
    pub log: String,
}

/// How often, and how far apart, the check is tried while a service that
/// was just started opens its socket.
const CHECK_TRIES: u32 = 6;
const CHECK_WAIT: Duration = Duration::from_millis(500);

/// Runs the steps still to do, stopping at the first that fails: the later
/// ones build on it. Then runs the check, if anything ran.
pub fn run(steps: &[Step], check: Check) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for step in steps.iter().filter(|step| step.state == State::Todo) {
        let mut log = String::new();
        let ok = step.actions.iter().all(|action| perform(action, &mut log));
        outcomes.push(Outcome {
            title: step.title,
            ok,
            log,
        });
        if !ok {
            return outcomes;
        }
    }
    outcomes.push(match check {
        Check::Agents => doctor(),
        Check::SpeechServer => speech_server(),
    });
    outcomes
}

/// How long the speech server gets to load its model and open its port.
const SERVER_TRIES: u32 = 30;
const SERVER_WAIT: Duration = Duration::from_secs(1);

/// Whether something answers at the speech server's address, tried while
/// a server that was just started loads its model.
fn speech_server() -> Outcome {
    for attempt in 0..SERVER_TRIES {
        if attempt > 0 {
            std::thread::sleep(SERVER_WAIT);
        }
        if TcpStream::connect_timeout(&dictation::SERVER, Duration::from_millis(300)).is_ok() {
            return Outcome {
                title: "Check",
                ok: true,
                log: format!(
                    "The speech server answers at {}. Try Ctrl+D in the launcher.\n",
                    dictation::SERVER
                ),
            };
        }
    }
    Outcome {
        title: "Check",
        ok: false,
        log: format!(
            "Nothing answers at {} after {} seconds. The engine may still be loading its model, or did not start:\n$ systemctl --user status otto-stt-*\n$ journalctl --user -u otto-stt-* -e\n",
            dictation::SERVER,
            SERVER_TRIES
        ),
    }
}

/// Does `action`, saying what happened in `log`.
fn perform(action: &Action, log: &mut String) -> bool {
    match action {
        Action::Write { path, text } => {
            log.push_str(&format!("write {}\n", path.display()));
            match write_atomically(path, text) {
                Ok(()) => true,
                Err(err) => {
                    log.push_str(&format!("  failed: {err}\n"));
                    false
                }
            }
        }
        Action::Run { args } => {
            log.push_str(&format!("$ {}\n", args.join(" ")));
            let Some((program, rest)) = args.split_first() else {
                return true;
            };
            match Command::new(program)
                .args(rest)
                .stdin(Stdio::null())
                .output()
            {
                Ok(output) => {
                    log.push_str(&String::from_utf8_lossy(&output.stdout));
                    log.push_str(&String::from_utf8_lossy(&output.stderr));
                    if !output.status.success() {
                        log.push_str(&format!("  failed: {}\n", output.status));
                    }
                    output.status.success()
                }
                Err(err) => {
                    log.push_str(&format!("  could not run {program}: {err}\n"));
                    false
                }
            }
        }
    }
}

/// `otto-agents doctor`, tried a few times while the service comes up.
fn doctor() -> Outcome {
    let mut last = String::new();
    for attempt in 0..CHECK_TRIES {
        if attempt > 0 {
            std::thread::sleep(CHECK_WAIT);
        }
        let output = Command::new("otto-agents")
            .arg("doctor")
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(output) => {
                last = format!(
                    "$ otto-agents doctor\n{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                if output.status.success() {
                    return Outcome {
                        title: "Check",
                        ok: true,
                        log: last,
                    };
                }
            }
            Err(err) => {
                return Outcome {
                    title: "Check",
                    ok: false,
                    log: format!("could not run otto-agents doctor: {err}\n"),
                }
            }
        }
    }
    Outcome {
        title: "Check",
        ok: false,
        log: last,
    }
}

/// Replaces `path` with `text` in one step, so the service never starts on
/// a half-written file.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".tmp.{}", std::process::id()));
    let temp = std::path::PathBuf::from(temp);
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_into_a_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/agents.toml");
        let mut log = String::new();
        assert!(perform(
            &Action::Write {
                path: path.clone(),
                text: "x = 1\n".into()
            },
            &mut log
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "x = 1\n");
    }

    #[test]
    fn a_failing_command_fails_the_step() {
        let mut log = String::new();
        assert!(!perform(
            &Action::Run {
                args: vec!["false".into()]
            },
            &mut log
        ));
        assert!(log.starts_with("$ false\n"));
    }
}
