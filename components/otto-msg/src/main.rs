//! `otto-msg` — drive the Otto compositor from a script.
//!
//! The same shape as `i3-msg` and `swaymsg`, over `org.otto.Shell1` instead of
//! a Unix socket. The command grammar and the JSON are i3's, so a script
//! written for either ports with a rename.
//!
//! The wire contract is `docs/developer/shell-dbus-api.md`; the user-facing
//! guide is `docs/user/scripting.md`.

use std::process::ExitCode;

use otto_dbus::shell::ShellProxyBlocking;
use serde_json::Value;
use zbus::blocking::Connection;

const USAGE: &str = "\
Usage: otto-msg [options] [command]

Drive the Otto compositor: run i3-syntax commands, read its tree, or follow
its events.

Options:
  -t, --type <type>   run_command (default), get_tree, get_workspaces,
                      get_outputs, get_inputs, subscribe
  -m, --monitor       with -t subscribe, keep printing events
  -r, --raw           print raw JSON rather than pretty-printing it
  -p, --pretty        pretty-print even when stdout is not a terminal
  -q, --quiet         print nothing; the exit status is the answer
  -v, --version       print the version and exit
  -h, --help          print this and exit

Examples:
  otto-msg focus right
  otto-msg 'move container to workspace 3; workspace 3'
  otto-msg -t get_tree | jq -r '.. | select(.focused? == true) | .name'
  otto-msg -m -t subscribe '[\"workspace\",\"window\"]'
";

#[derive(PartialEq, Eq, Clone, Copy)]
enum Kind {
    RunCommand,
    GetTree,
    GetWorkspaces,
    GetOutputs,
    GetInputs,
    Subscribe,
}

impl Kind {
    /// The `-t` names, in i3's spelling. `swaymsg` accepts both the
    /// underscored and the hyphenated forms, so this does too.
    fn parse(word: &str) -> Option<Kind> {
        match word.replace('-', "_").as_str() {
            "run_command" | "command" => Some(Kind::RunCommand),
            "get_tree" => Some(Kind::GetTree),
            "get_workspaces" => Some(Kind::GetWorkspaces),
            "get_outputs" => Some(Kind::GetOutputs),
            "get_inputs" => Some(Kind::GetInputs),
            "subscribe" => Some(Kind::Subscribe),
            _ => None,
        }
    }
}

struct Options {
    kind: Kind,
    monitor: bool,
    raw: bool,
    quiet: bool,
    /// Everything that was not an option: the command text, or the event list
    /// for `subscribe`.
    rest: Vec<String>,
}

/// Hand-written, because the whole grammar is five flags and a tail — and
/// because everything after the first non-option word is the *command*, which
/// an argument parser would try to interpret.
fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut options = Options {
        kind: Kind::RunCommand,
        monitor: false,
        raw: false,
        quiet: false,
        rest: Vec::new(),
    };
    let mut pretty = false;
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        // The first word that is not an option ends the options: `otto-msg
        // resize shrink width 10 ppt` must not have `-10` read as a flag.
        if !options.rest.is_empty() {
            options.rest.push(arg);
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-v" | "--version" => {
                println!("otto-msg {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-m" | "--monitor" => options.monitor = true,
            "-r" | "--raw" => options.raw = true,
            "-p" | "--pretty" => pretty = true,
            "-q" | "--quiet" => options.quiet = true,
            "-t" | "--type" => {
                let Some(word) = args.next() else {
                    return Err("-t needs a message type".to_string());
                };
                options.kind =
                    Kind::parse(&word).ok_or_else(|| format!("unknown message type '{word}'"))?;
            }
            other => {
                if let Some(word) = other.strip_prefix("--type=") {
                    options.kind = Kind::parse(word)
                        .ok_or_else(|| format!("unknown message type '{word}'"))?;
                } else {
                    options.rest.push(other.to_string());
                }
            }
        }
    }
    if pretty {
        options.raw = false;
    }
    Ok(options)
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1).collect()) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("otto-msg: {message}");
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("otto-msg: could not reach the session bus: {err}");
            return ExitCode::FAILURE;
        }
    };
    let proxy = match ShellProxyBlocking::new(&connection) {
        Ok(proxy) => proxy,
        Err(err) => {
            eprintln!("otto-msg: could not reach Otto: {err}");
            return ExitCode::FAILURE;
        }
    };

    match options.kind {
        Kind::Subscribe => subscribe(&proxy, &options),
        Kind::RunCommand => run_command(&proxy, &options),
        kind => get(&proxy, kind, &options),
    }
}

fn run_command(proxy: &ShellProxyBlocking<'_>, options: &Options) -> ExitCode {
    let command = options.rest.join(" ");
    if command.trim().is_empty() {
        eprintln!("otto-msg: nothing to run");
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    }
    let results = match proxy.run_command(&command) {
        Ok(results) => results,
        Err(err) => {
            eprintln!("otto-msg: {err}");
            return ExitCode::FAILURE;
        }
    };

    // `swaymsg`'s reply shape: an object per command, with the message only on
    // the ones that failed.
    let reply: Vec<Value> = results
        .iter()
        .map(|(success, message)| {
            if *success {
                serde_json::json!({"success": true})
            } else {
                serde_json::json!({"success": false, "error": message})
            }
        })
        .collect();
    let failed = results.iter().any(|(success, _)| !success);

    if !options.quiet {
        print_json(&Value::Array(reply), options.raw);
        // The reply JSON is machine-readable; the human reading a terminal
        // gets the reason on stderr where it will not be piped into `jq`.
        for (success, message) in &results {
            if !success {
                eprintln!("otto-msg: {message}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn get(proxy: &ShellProxyBlocking<'_>, kind: Kind, options: &Options) -> ExitCode {
    let answer = match kind {
        Kind::GetTree => proxy.get_tree(),
        Kind::GetWorkspaces => proxy.get_workspaces(),
        Kind::GetOutputs => proxy.get_outputs(),
        Kind::GetInputs => proxy.get_inputs(),
        Kind::RunCommand | Kind::Subscribe => unreachable!("not a query"),
    };
    let json = match answer {
        Ok(json) => json,
        Err(err) => {
            eprintln!("otto-msg: {err}");
            return ExitCode::FAILURE;
        }
    };
    if options.quiet {
        return ExitCode::SUCCESS;
    }
    match serde_json::from_str::<Value>(&json) {
        Ok(value) => print_json(&value, options.raw),
        // Otto sent something that is not JSON; pass it through rather than
        // swallow it.
        Err(_) => println!("{json}"),
    }
    ExitCode::SUCCESS
}

/// `-t subscribe '["workspace","window"]'`: print events as they arrive.
///
/// Without `-m` this prints the first matching event and exits, which is what
/// `swaymsg -t subscribe` does.
fn subscribe(proxy: &ShellProxyBlocking<'_>, options: &Options) -> ExitCode {
    let wanted = match parse_event_list(&options.rest.join(" ")) {
        Ok(wanted) => wanted,
        Err(message) => {
            eprintln!("otto-msg: {message}");
            return ExitCode::FAILURE;
        }
    };
    let signals = match proxy.inner().receive_all_signals() {
        Ok(signals) => signals,
        Err(err) => {
            eprintln!("otto-msg: could not subscribe: {err}");
            return ExitCode::FAILURE;
        }
    };
    for message in signals {
        let Some(member) = message.header().member().map(|m| m.to_string()) else {
            continue;
        };
        let Some(kind) = event_name(&member) else {
            continue;
        };
        if !wanted.iter().any(|w| w == kind) {
            continue;
        }
        let Ok(payload) = message.body().deserialize::<String>() else {
            continue;
        };
        if !options.quiet {
            match serde_json::from_str::<Value>(&payload) {
                Ok(value) => print_json(&value, options.raw),
                Err(_) => println!("{payload}"),
            }
        }
        if !options.monitor {
            return ExitCode::SUCCESS;
        }
    }
    ExitCode::SUCCESS
}

/// i3's event name for one of the interface's signals.
fn event_name(member: &str) -> Option<&'static str> {
    match member {
        "WorkspaceChanged" => Some("workspace"),
        "WindowChanged" => Some("window"),
        "InputChanged" => Some("input"),
        _ => None,
    }
}

/// The event list, as i3 takes it: a JSON array of names. A bare name is
/// accepted too, since typing the brackets past a shell is a nuisance.
fn parse_event_list(text: &str) -> Result<Vec<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("subscribe needs a list of events, e.g. '[\"workspace\"]'".to_string());
    }
    let names: Vec<String> = if text.starts_with('[') {
        serde_json::from_str::<Vec<String>>(text)
            .map_err(|err| format!("could not read the event list: {err}"))?
    } else {
        text.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|word| !word.is_empty())
            .map(|word| word.trim_matches(['"', '\''].as_slice()).to_string())
            .collect()
    };
    for name in &names {
        if !matches!(name.as_str(), "workspace" | "window") {
            return Err(format!(
                "Otto does not publish a '{name}' event yet (workspace, window)"
            ));
        }
    }
    if names.is_empty() {
        return Err("subscribe needs at least one event".to_string());
    }
    Ok(names)
}

fn print_json(value: &Value, raw: bool) {
    if raw {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Options {
        parse_args(words.iter().map(|w| w.to_string()).collect()).expect("parses")
    }

    #[test]
    fn the_default_is_a_command() {
        let options = args(&["focus", "right"]);
        assert!(options.kind == Kind::RunCommand);
        assert_eq!(options.rest.join(" "), "focus right");
    }

    #[test]
    fn options_stop_at_the_first_command_word() {
        // `-r` here is part of nothing: it comes after the command started.
        let options = args(&["resize", "grow", "width", "-r"]);
        assert_eq!(options.rest.join(" "), "resize grow width -r");
        assert!(!options.raw);
    }

    #[test]
    fn the_type_is_read_in_both_spellings() {
        assert!(args(&["-t", "get_tree"]).kind == Kind::GetTree);
        assert!(args(&["--type=get-outputs"]).kind == Kind::GetOutputs);
        assert!(args(&["-t", "get_inputs"]).kind == Kind::GetInputs);
    }

    #[test]
    fn an_event_list_reads_as_json_or_as_bare_names() {
        assert_eq!(
            parse_event_list("[\"workspace\",\"window\"]").unwrap(),
            vec!["workspace".to_string(), "window".to_string()]
        );
        assert_eq!(
            parse_event_list("workspace window").unwrap(),
            vec!["workspace".to_string(), "window".to_string()]
        );
        assert!(parse_event_list("mode").is_err());
        assert!(parse_event_list("").is_err());
    }
}
