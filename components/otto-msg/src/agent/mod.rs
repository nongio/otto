//! `otto-msg agent`: work on the desktop as an agent, beside the user.
//!
//! Plain `otto-msg` acts as the user: it moves their focus and switches their
//! workspace. `otto-msg agent` asks Otto for a seat of the agent's own and a
//! workspace (`specs/agent-seats.md`), and then clicks, types, captures and
//! launches there, framed in the agent's colour, without touching the
//! user's cursor or keyboard. `agent start` leaves a holder process behind
//! (see `holder`); every other command is one request to it.

mod holder;
mod keys;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use serde_json::{json, Value};

const USAGE: &str = "\
Usage: otto-msg agent <command> [arguments]

Work on the desktop as an agent: a cursor and keyboard of your own, on a
workspace of your own or one the user lends you, framed in your colour.
The user keeps their cursor, keyboard and workspace. Plain `otto-msg` acts
as the user instead: don't use it to work beside them.

Start and stop:
  start <name>              ask for a seat and a workspace of your own
                            (the user is asked the first time)
  start <name> --lend [ws]  ask to work on one of the user's workspaces,
                            by name, or the one they are looking at
  stop                      give the seat back; the workspace and its
                            windows stay for the user
  info                      the seat, the workspace and its size

See:
  capture                   save a PNG of the workspace and print its path
  windows                   the windows on the workspace

Act (x and y are pixels of a capture):
  click <x> <y> [right|middle]
  double-click <x> <y>
  move <x> <y>
  drag <x> <y> <to-x> <to-y>
  scroll <x> <y> up|down [steps]
  type <text>               type text into the focused window
  key <combo>...            press keys: Return, Tab, Escape, ctrl+s, alt+F4
  focus <window>            give your keyboard to a window, by part of its
                            title or app id
  close <window>            close a window
  launch <program> [args]   start a program on your workspace

Options:
  -n, --name <name>         which agent, when more than one is running
                            (or set OTTO_AGENT)

A typical loop: start, launch, capture, click or type, capture again to
check, and stop when done. The user can press Stop on the frame at any
time; then every command fails and you start again only if they agree.
";

pub fn main(args: Vec<String>) -> ExitCode {
    let mut name = std::env::var("OTTO_AGENT").ok().filter(|n| !n.is_empty());
    let mut args = args.into_iter();
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        if !rest.is_empty() {
            rest.push(arg);
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-n" | "--name" => name = args.next(),
            other => rest.push(other.to_string()),
        }
    }
    let Some((command, args)) = rest.split_first() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = match command.as_str() {
        "start" => start(args),
        "serve" => return serve(args),
        command => request(name.as_deref(), command, args),
    };
    match result {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("otto-msg agent: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Where holders listen, readable by the user alone.
fn agents_dir() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let dir = PathBuf::from(runtime).join("otto").join("agents");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("cannot make {}: {err}", dir.display()))?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    Ok(dir)
}

/// The file name an agent's socket and log go by.
fn file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

struct StartArgs {
    name: String,
    lend: Option<String>,
}

fn parse_start(args: &[String]) -> Result<StartArgs, String> {
    let mut name = None;
    let mut lend = None;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lend" => {
                let workspace = match args.peek() {
                    Some(next) if !next.starts_with('-') => args.next().cloned(),
                    _ => None,
                };
                lend = Some(workspace.unwrap_or_default());
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => return Err(format!("unexpected '{other}'")),
        }
    }
    let name = name.ok_or("start needs the agent's name, as the user will see it")?;
    Ok(StartArgs { name, lend })
}

fn start(args: &[String]) -> Result<String, String> {
    let start = parse_start(args)?;
    let dir = agents_dir()?;
    let stem = file_stem(&start.name);
    let socket = dir.join(format!("{stem}.sock"));
    if UnixStream::connect(&socket).is_ok() {
        return Err(format!(
            "'{}' is already running; `otto-msg agent info` describes it",
            start.name
        ));
    }
    let log = std::fs::File::create(dir.join(format!("{stem}.log")))
        .map_err(|err| format!("cannot write the log: {err}"))?;
    let exe = std::env::current_exe().map_err(|err| format!("cannot find otto-msg: {err}"))?;
    let mut command = Command::new(exe);
    command.args(["agent", "serve", &start.name]);
    if let Some(workspace) = &start.lend {
        command.args(["--lend", workspace]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(log);
    // SAFETY: setsid is async-signal-safe; the holder outlives this terminal.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("cannot start the holder: {err}"))?;
    let stdout = child.stdout.take().ok_or("no holder output")?;
    let mut line = String::new();
    // Waits while the user answers Otto's dialog.
    let _ = BufReader::new(stdout).read_line(&mut line);
    let line = line.trim_end();
    if let Some(message) = line.strip_prefix("error ") {
        let _ = child.wait();
        return Err(message.to_string());
    }
    let Some(ready) = line.strip_prefix("ready ") else {
        let _ = child.wait();
        return Err("the holder ended without a word; see its log in $XDG_RUNTIME_DIR/otto/agents".to_string());
    };
    let ready: Value = serde_json::from_str(ready).unwrap_or(Value::Null);
    let pixels = &ready["pixels"];
    Ok(format!(
        "'{}' is working on {} (seat {}, colour {}).\n\
         The screen is {}x{} pixels: `otto-msg agent capture` shows it, and click, move, drag and scroll take its coordinates.\n\
         `otto-msg agent stop` gives the seat back when you are done.",
        start.name,
        if ready["lent"].as_bool() == Some(true) {
            "a workspace the user lent"
        } else {
            "a workspace of its own"
        },
        ready["seat"].as_str().unwrap_or("?"),
        ready["color"].as_str().unwrap_or("?"),
        pixels[0],
        pixels[1],
    ))
}

fn serve(args: &[String]) -> ExitCode {
    let start = match parse_start(args) {
        Ok(start) => start,
        Err(message) => {
            println!("error {message}");
            return ExitCode::FAILURE;
        }
    };
    let socket = match agents_dir() {
        Ok(dir) => dir.join(format!("{}.sock", file_stem(&start.name))),
        Err(message) => {
            println!("error {message}");
            return ExitCode::FAILURE;
        }
    };
    holder::serve(holder::Start {
        name: start.name,
        lend: start.lend,
        socket,
    })
}

/// The socket of the agent to talk to: the one named, or the only one
/// running.
fn pick(name: Option<&str>) -> Result<PathBuf, String> {
    let dir = agents_dir()?;
    if let Some(name) = name {
        let socket = dir.join(format!("{}.sock", file_stem(name)));
        return if socket.exists() {
            Ok(socket)
        } else {
            Err(format!(
                "no agent '{name}' is running; start it with `otto-msg agent start {name}`"
            ))
        };
    }
    let mut running: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "sock"))
        .filter(|path| {
            // A holder that died leaves its socket behind.
            let alive = UnixStream::connect(path).is_ok();
            if !alive {
                let _ = std::fs::remove_file(path);
            }
            alive
        })
        .collect();
    match running.len() {
        0 => Err("no agent is running; start one with `otto-msg agent start <name>`".to_string()),
        1 => Ok(running.remove(0)),
        _ => Err(format!(
            "several agents are running; choose one with --name: {}",
            running
                .iter()
                .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn request(name: Option<&str>, command: &str, args: &[String]) -> Result<String, String> {
    let socket = pick(name)?;
    let mut stream = UnixStream::connect(&socket).map_err(|_| {
        let _ = std::fs::remove_file(&socket);
        "the agent is no longer running; start it again with `otto-msg agent start <name>`"
            .to_string()
    })?;
    writeln!(stream, "{}", json!({"cmd": command, "args": args}))
        .map_err(|err| format!("cannot reach the agent: {err}"))?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|err| format!("no answer from the agent: {err}"))?;
    let reply: Value = serde_json::from_str(&line)
        .map_err(|_| "the agent ended; Otto may have stopped it".to_string())?;
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply["out"].as_str().unwrap_or("").to_string())
    } else {
        Err(reply["error"].as_str().unwrap_or("failed").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn start_reads_a_name_and_a_loan() {
        let own = parse_start(&strings(&["Claude"])).unwrap();
        assert_eq!(own.name, "Claude");
        assert_eq!(own.lend, None);
        let current = parse_start(&strings(&["Claude", "--lend"])).unwrap();
        assert_eq!(current.lend.as_deref(), Some(""));
        let named = parse_start(&strings(&["Claude", "--lend", "Browsing"])).unwrap();
        assert_eq!(named.lend.as_deref(), Some("Browsing"));
        assert!(parse_start(&strings(&[])).is_err());
        assert!(parse_start(&strings(&["a", "b"])).is_err());
    }

    #[test]
    fn a_name_makes_a_safe_file_name() {
        assert_eq!(file_stem("My agent/1"), "My_agent_1");
    }
}
