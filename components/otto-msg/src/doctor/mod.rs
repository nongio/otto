//! `otto-msg doctor`: gather what a bug report about Otto needs.
//!
//! Collects the operating system, graphics, screens, Otto's version, the
//! configuration and the logs; lets the user choose and preview each part;
//! then writes a Markdown report to attach to an issue.
//! Nothing is uploaded. The user guide is `docs/user/bug-reports.md`.

// Rust guideline compliant 2026-02-21

mod compositor;
mod logs;
mod redact;
mod report;
mod sections;
mod summary;
mod tui;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use otto_foundations::xdg;

use crate::doctor::redact::Redactor;
use crate::doctor::report::{Redaction, Timestamp};
use crate::doctor::sections::Sources;

const USAGE: &str = "\
Usage: otto-msg doctor [options]

Gather what a bug report about Otto needs, choose what to share, and write it
as a Markdown file to attach to an issue. Nothing is uploaded.

Options:
      --no-redact      keep the home folder, user and host names, and IP and
                       MAC addresses as they are
  -p, --print          write the report to stdout instead of choosing
  -o, --output <path>  write the report here instead of
                       $XDG_STATE_HOME/otto/otto-report-<time>.md
  -n, --lines <n>      keep the last <n> lines of each log (default 2000)
      --no-clipboard   do not copy the report to the clipboard
  -V, --version        print the version and exit
  -h, --help           print this and exit

Without --print, and with a terminal, it opens an interactive chooser.
";

/// How many log lines are kept from the end of each log by default.
///
/// Enough to cover the startup and the minutes before a crash at the default
/// `info` level, while keeping the report small enough to attach.
const DEFAULT_LOG_LINES: usize = 2000;

/// The command-line options.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    redact: bool,
    print: bool,
    output: Option<PathBuf>,
    log_lines: usize,
    clipboard: bool,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Run(Options),
    Help,
    Version,
}

/// Parses the arguments after the program name.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut options = Options {
        redact: true,
        print: false,
        output: None,
        log_lines: DEFAULT_LOG_LINES,
        clipboard: true,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "--no-redact" => options.redact = false,
            "-p" | "--print" => options.print = true,
            "-o" | "--output" => options.output = Some(PathBuf::from(value("--output")?)),
            "-n" | "--lines" => {
                let text = value("--lines")?;
                options.log_lines = text
                    .parse()
                    .map_err(|_| format!("--lines takes a number, not {text:?}"))?;
            }
            "--no-clipboard" => options.clipboard = false,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    if options.print && options.output.is_some() {
        return Err("--print and --output cannot be used together".into());
    }
    Ok(Command::Run(options))
}

/// Runs `otto-msg doctor` with the arguments after `doctor`.
pub fn main(args: Vec<String>) -> ExitCode {
    let options = match parse_args(args) {
        Ok(Command::Run(options)) => options,
        Ok(Command::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("otto-msg {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("otto-msg doctor: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("otto-msg doctor: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options) -> Result<(), String> {
    let compositor = compositor::query();
    let home = xdg::home();
    let (config_files, config_from_compositor) = match &compositor {
        Ok(info) if !info.config_files.is_empty() => (info.config_files.clone(), true),
        _ => (usual_config_files(home.as_deref()), false),
    };
    let sources = Sources {
        root: PathBuf::from("/"),
        log_dir: xdg::state_home().map(|dir| dir.join("otto")),
        config_files,
        config_from_compositor,
        compositor,
        log_lines: options.log_lines,
        // SAFETY: getuid has no preconditions.
        uid: unsafe { libc::getuid() },
        journal: logs::read_journal,
    };
    let mut sections = sections::gather(&sources);
    let redactor = Redactor::new(
        home.map(|h| h.to_string_lossy().into_owned()),
        otto_foundations::user::login_name(),
        host_name(),
    );
    let mut redact = options.redact;
    let redaction = |on: bool| {
        if on {
            Redaction::On(&redactor)
        } else {
            Redaction::Off
        }
    };
    let when = Timestamp::now();

    if options.print || !otto_term_kit::is_tty(libc::STDOUT_FILENO) {
        print!("{}", report::render(&sections, redaction(redact), when));
        return Ok(());
    }
    if otto_term_kit::is_tty(libc::STDIN_FILENO) {
        let outcome = tui::run(&mut sections, &redactor, &mut redact)
            .map_err(|e| format!("terminal: {e}"))?;
        if outcome == tui::Outcome::Quit {
            println!("Nothing written.");
            return Ok(());
        }
    }

    let text = report::render(&sections, redaction(redact), when);
    let path = match &options.output {
        Some(path) => path.clone(),
        None => xdg::state_home()
            .ok_or("no home folder to write the report to; use --output")?
            .join("otto")
            .join(when.file_name()),
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, &text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    println!("Report written to {}", xdg::tilde(&path));
    if options.clipboard && copy_to_clipboard(&text) {
        println!("It is on the clipboard too.");
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    println!(
        "\nOpen a new issue and attach the file:\n{}",
        report::issue_url(&sections, redaction(redact), &file_name)
    );
    Ok(())
}

/// The configuration files Otto usually reads, for when it cannot say.
///
/// The system and user files, plus the local overrides in the home folder,
/// which is where a login session starts.
fn usual_config_files(home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = xdg::otto_config_paths("config.toml");
    if let Some(home) = home {
        for name in [
            "otto_config.toml",
            "otto_config.tty-udev.toml",
            "otto_config.udev.toml",
        ] {
            files.push(home.join(name));
        }
    }
    files
}

/// The host name, as the kernel has it.
fn host_name() -> Option<String> {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .map(|h| h.trim().to_owned())
                .filter(|h| !h.is_empty() && h != "localhost")
        })
}

/// Copies `text` with `wl-copy`, when there is a Wayland session to copy in.
fn copy_to_clipboard(text: &str) -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return false;
    }
    let child = std::process::Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    // wl-copy forks to serve the clipboard and its first process exits once
    // it has read the text.
    child.wait().is_ok_and(|status| status.success()) && written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn defaults_redact_and_keep_two_thousand_lines() {
        let Ok(Command::Run(options)) = parse(&[]) else {
            panic!("no arguments runs");
        };
        assert!(options.redact && options.clipboard && !options.print);
        assert_eq!(options.log_lines, 2000);
    }

    #[test]
    fn reads_every_option() {
        let Ok(Command::Run(options)) = parse(&[
            "--no-redact",
            "-o",
            "/tmp/r.md",
            "--lines=50",
            "--no-clipboard",
        ]) else {
            panic!("valid options");
        };
        assert!(!options.redact && !options.clipboard);
        assert_eq!(options.output.as_deref(), Some(Path::new("/tmp/r.md")));
        assert_eq!(options.log_lines, 50);
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
        assert_eq!(parse(&["--help", "--bogus"]), Ok(Command::Help));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--lines", "many"]).is_err());
        assert!(parse(&["--output"]).is_err());
        assert!(parse(&["--print", "--output", "x"]).is_err());
        assert!(parse(&["--frobnicate"]).is_err());
        assert!(parse(&["--all"]).is_err());
    }
}
