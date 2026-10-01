//! `otto-sandbox` — run an agent where its only ways out are Otto's.
//!
//! `otto-sandbox run` asks Otto for an agent seat and a workspace of the
//! agent's own, then starts the command under bubblewrap:
//!
//! - Its `WAYLAND_DISPLAY` is a socket Otto accepts the agent's clients on
//!   (`ServeAgentSocket`): stock tools there drive the agent's seat and
//!   nothing else, and are not offered screen capture or the clipboard.
//! - No session bus, no X server, no `/dev/uinput`, a private `/tmp` and
//!   `/run` (so no `ydotoold` socket), and a home holding only the project
//!   directory and what `--bind` adds.
//! - `otto-sandbox ctl` inside asks the launcher, over a socket of its own,
//!   to start an app on the agent's workspace, to capture that workspace, or
//!   to describe it. The launcher makes those calls as the seat's holder.
//!
//! The seat goes when the command exits, and with it every connection of
//! the agent's; the workspace stays, for the user. See
//! `specs/agent-seats.md`, Phase 5.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};
use zbus::blocking::{Connection, Proxy};

const BUS_NAME: &str = "org.otto.Compositor";
const OBJECT_PATH: &str = "/org/otto/Compositor";
const INTERFACE: &str = "org.otto.Compositor";

/// Where the launcher's things appear inside the sandbox.
const INNER_BIN: &str = "/run/otto-sandbox";
const INNER_WAYLAND: &str = "wayland-0";
const INNER_CONTROL: &str = "otto-sandbox.sock";
const INNER_CAPTURES: &str = "otto-sandbox-captures";

const USAGE: &str = "\
Usage: otto-sandbox run [options] -- <command> [args...]
       otto-sandbox ctl <launch <argv...> | capture | workspace>

run: ask Otto for an agent seat and a workspace of the agent's own, and run
<command> in a sandbox whose only ways out are those. Inside, Wayland clients
drive the agent's seat and nothing else; there is no session bus, no X server
and no uinput, and the home directory holds only the project and what --bind
adds. The seat goes when <command> exits; the workspace stays.

Options for run:
  --name <name>       the agent's name, shown by its cursor (default: the
                      command's)
  --dir <dir>         the project directory, writable inside (default: the
                      current one)
  --bind <path>       also make <path> writable inside, at the same place
  --ro-bind <path>    also make <path> readable inside, at the same place
  --env <NAME>        pass the variable NAME through (the environment is
                      otherwise cleared)
  --no-net            no network inside
  --print             print the bubblewrap command instead of running it

ctl, from inside the sandbox:
  launch <argv...>    start an app on the agent's workspace; prints its pid
  capture             capture the agent's workspace; prints the PNG's path
  workspace           prints: output x y width height scale

Example:
  otto-sandbox run --name Claude --bind ~/.claude --bind ~/.claude.json \\
      --ro-bind ~/.local/share/claude --env ANTHROPIC_API_KEY -- claude
";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|a| a.to_str()) {
        Some("run") => run(args.collect()),
        Some("ctl") => ctl(args.collect()),
        Some("-h" | "--help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// Everything `run` was asked for.
#[derive(Debug, Default, PartialEq)]
struct Options {
    name: Option<String>,
    dir: Option<PathBuf>,
    binds: Vec<PathBuf>,
    ro_binds: Vec<PathBuf>,
    envs: Vec<String>,
    no_net: bool,
    print: bool,
    command: Vec<OsString>,
}

fn parse_options(args: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let flag = arg.to_str().unwrap_or("");
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{what} needs a value"));
        match flag {
            "--" => {
                options.command = args.collect();
                break;
            }
            "--name" => options.name = Some(value("--name")?.to_string_lossy().into_owned()),
            "--dir" => options.dir = Some(value("--dir")?.into()),
            "--bind" => options.binds.push(value("--bind")?.into()),
            "--ro-bind" => options.ro_binds.push(value("--ro-bind")?.into()),
            "--env" => options
                .envs
                .push(value("--env")?.to_string_lossy().into_owned()),
            "--no-net" => options.no_net = true,
            "--print" => options.print = true,
            _ => return Err(format!("unknown option {flag:?}")),
        }
    }
    if options.command.is_empty() {
        return Err("no command: put it after --".into());
    }
    Ok(options)
}

/// What the sandbox is made of, once Otto has answered.
#[derive(Debug)]
struct Plan {
    home: PathBuf,
    runtime_dir: PathBuf,
    project: PathBuf,
    /// The launcher's own directory, holding the sockets and captures.
    private: PathBuf,
    exe: PathBuf,
    binds: Vec<PathBuf>,
    ro_binds: Vec<PathBuf>,
    /// Variables set inside, after the environment is cleared.
    env: Vec<(String, String)>,
    net: bool,
    new_session: bool,
    command: Vec<OsString>,
}

/// The bubblewrap command line for `plan`.
fn bwrap_args(plan: &Plan) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |items: &[&dyn AsRef<std::ffi::OsStr>]| {
        a.extend(items.iter().map(|i| i.as_ref().to_os_string()));
    };
    push(&[
        &"--die-with-parent",
        &"--unshare-pid",
        &"--unshare-ipc",
        &"--unshare-uts",
        &"--unshare-cgroup-try",
    ]);
    if !plan.net {
        push(&[&"--unshare-net"]);
    }
    if plan.new_session {
        push(&[&"--new-session"]);
    }
    push(&[&"--ro-bind", &"/usr", &"/usr"]);
    for top in ["/bin", "/sbin", "/lib", "/lib64"] {
        match std::fs::read_link(top) {
            Ok(target) => push(&[&"--symlink", &target, &top]),
            Err(_) if Path::new(top).is_dir() => push(&[&"--ro-bind", &top, &top]),
            Err(_) => {}
        }
    }
    push(&[
        &"--ro-bind",
        &"/etc",
        &"/etc",
        &"--ro-bind-try",
        &"/opt",
        &"/opt",
        &"--proc",
        &"/proc",
        &"--dev",
        &"/dev",
        &"--tmpfs",
        &"/tmp",
        &"--tmpfs",
        &"/run",
        // /etc/resolv.conf often points in here.
        &"--ro-bind-try",
        &"/run/systemd/resolve",
        &"/run/systemd/resolve",
    ]);

    let rt = &plan.runtime_dir;
    push(&[
        &"--dir",
        rt,
        &"--chmod",
        &"0700",
        rt,
        &"--bind",
        &plan.private.join("wayland"),
        &rt.join(INNER_WAYLAND),
        &"--bind",
        &plan.private.join("control"),
        &rt.join(INNER_CONTROL),
        &"--ro-bind",
        &plan.private.join("captures"),
        &rt.join(INNER_CAPTURES),
        &"--dir",
        &INNER_BIN,
        &"--ro-bind",
        &plan.exe,
        &Path::new(INNER_BIN).join("otto-sandbox"),
        &"--tmpfs",
        &plan.home,
        &"--bind",
        &plan.project,
        &plan.project,
    ]);
    for path in &plan.binds {
        push(&[&"--bind", path, path]);
    }
    for path in &plan.ro_binds {
        push(&[&"--ro-bind", path, path]);
    }
    push(&[&"--chdir", &plan.project, &"--clearenv"]);
    for (key, value) in &plan.env {
        push(&[&"--setenv", key, value]);
    }
    push(&[&"--"]);
    a.extend(plan.command.iter().cloned());
    a
}

/// The environment inside: the basics, Otto's, and what `--env` names.
fn inner_env(
    plan_home: &Path,
    runtime_dir: &Path,
    seat: &str,
    workspace: &str,
    envs: &[String],
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut set = |key: &str, value: String| {
        env.retain(|(k, _)| k != key);
        env.push((key.to_string(), value));
    };
    let path = format!("{INNER_BIN}:/usr/local/bin:/usr/bin:/bin");
    set("PATH", path.clone());
    set("HOME", plan_home.display().to_string());
    set("XDG_RUNTIME_DIR", runtime_dir.display().to_string());
    set("WAYLAND_DISPLAY", INNER_WAYLAND.to_string());
    set("XDG_SESSION_TYPE", "wayland".to_string());
    set("OTTO_AGENT_SEAT", seat.to_string());
    set("OTTO_AGENT_WORKSPACE", workspace.to_string());
    for key in [
        "USER",
        "LOGNAME",
        "TERM",
        "COLORTERM",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "TZ",
    ] {
        if let Ok(value) = std::env::var(key) {
            set(key, value);
        }
    }
    for key in envs {
        if let Ok(value) = std::env::var(key) {
            if key == "PATH" {
                set("PATH", format!("{INNER_BIN}:{value}"));
            } else {
                set(key, value);
            }
        }
    }
    env
}

fn run(args: Vec<OsString>) -> ExitCode {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("otto-sandbox: {err}\n");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run_with(options) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("otto-sandbox: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_with(options: Options) -> Result<ExitCode, String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
    let runtime_dir =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?);
    let project = match &options.dir {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().map_err(|e| format!("no current directory: {e}"))?,
    };
    let project = project
        .canonicalize()
        .map_err(|e| format!("{}: {e}", project.display()))?;
    let name = options.name.clone().unwrap_or_else(|| {
        Path::new(&options.command[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Agent".into())
    });
    let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let binds = absolute(&options.binds)?;
    let ro_binds = absolute(&options.ro_binds)?;
    let new_session = unsafe { libc::isatty(0) } == 0;

    if options.print {
        let plan = Plan {
            env: inner_env(
                &home,
                &runtime_dir,
                "agent-N",
                "OUTPUT X Y W H SCALE",
                &options.envs,
            ),
            private: runtime_dir.join("otto-sandbox").join("PID"),
            home,
            runtime_dir,
            project,
            exe,
            binds,
            ro_binds,
            net: !options.no_net,
            new_session,
            command: options.command,
        };
        let line: Vec<String> = std::iter::once("bwrap".to_string())
            .chain(
                bwrap_args(&plan)
                    .iter()
                    .map(|a| shell_quote(&a.to_string_lossy())),
            )
            .collect();
        println!("{}", line.join(" "));
        return Ok(ExitCode::SUCCESS);
    }

    let conn = Connection::session().map_err(|e| format!("no session bus: {e}"))?;
    let otto = Proxy::new(&conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .map_err(|e| format!("cannot reach Otto: {e}"))?;
    eprintln!("otto-sandbox: asking Otto for a seat for {name:?}…");
    let (seat, _color): (String, String) = otto
        .call("RequestAgentSeat", &(name.as_str(),))
        .map_err(|e| format!("no seat: {e}"))?;
    let (output, x, y, width, height, scale): (String, i32, i32, i32, i32, f64) = otto
        .call("RequestOwnWorkspace", &())
        .map_err(|e| format!("no workspace: {e}"))?;
    let workspace = format!("{output} {x} {y} {width} {height} {scale}");

    let private = runtime_dir
        .join("otto-sandbox")
        .join(std::process::id().to_string());
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(private.join("captures"))
        .map_err(|e| format!("{}: {e}", private.display()))?;
    let _cleanup = RemoveOnDrop(private.clone());

    let wayland = UnixListener::bind(private.join("wayland"))
        .map_err(|e| format!("cannot make the Wayland socket: {e}"))?;
    let () = otto
        .call(
            "ServeAgentSocket",
            &(zbus::zvariant::Fd::from(wayland.as_fd()),),
        )
        .map_err(|e| format!("Otto would not serve the agent's socket: {e}"))?;
    // Otto has its own copy; ours would only hold connections nobody accepts.
    drop(wayland);

    let control = UnixListener::bind(private.join("control"))
        .map_err(|e| format!("cannot make the control socket: {e}"))?;
    {
        let conn = conn.clone();
        let private = private.clone();
        let runtime_dir = runtime_dir.clone();
        std::thread::spawn(move || serve_control(control, conn, private, runtime_dir));
    }

    let plan = Plan {
        env: inner_env(&home, &runtime_dir, &seat, &workspace, &options.envs),
        home,
        runtime_dir,
        project,
        private,
        exe,
        binds,
        ro_binds,
        net: !options.no_net,
        new_session,
        command: options.command,
    };
    eprintln!("otto-sandbox: seat {seat}, workspace on {output}; starting the agent");

    // Ctrl-C reaches the agent; the launcher stays to give the seat back.
    unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let mut command = std::process::Command::new("bwrap");
    command.args(bwrap_args(&plan));
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    let status = command
        .status()
        .map_err(|e| format!("cannot start bwrap: {e}"))?;

    let _: Result<bool, _> = otto.call("ReleaseAgentSeat", &());
    eprintln!("otto-sandbox: seat released; the workspace stays");
    Ok(match status.code() {
        Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
        None => ExitCode::FAILURE,
    })
}

/// `arg` as a shell reads it back.
fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:=,@+-".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

fn absolute(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|p| {
            p.canonicalize()
                .map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Answer `otto-sandbox ctl` from inside, one request per connection.
fn serve_control(control: UnixListener, conn: Connection, private: PathBuf, runtime_dir: PathBuf) {
    for stream in control.incoming().flatten() {
        let reply = match read_request(&stream) {
            Ok(request) => handle(&conn, &private, &runtime_dir, &request),
            Err(err) => Err(err),
        };
        let reply = match reply {
            Ok(value) => json!({ "ok": value }),
            Err(err) => json!({ "error": err }),
        };
        let mut stream = stream;
        let _ = writeln!(stream, "{reply}");
    }
}

fn read_request(stream: &UnixStream) -> Result<Value, String> {
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("bad request: {e}"))
}

fn handle(
    conn: &Connection,
    private: &Path,
    runtime_dir: &Path,
    request: &Value,
) -> Result<Value, String> {
    let otto = Proxy::new(conn, BUS_NAME, OBJECT_PATH, INTERFACE).map_err(|e| e.to_string())?;
    match request["cmd"].as_str() {
        Some("launch") => {
            let argv: Vec<String> = request["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if argv.is_empty() {
                return Err("launch what?".into());
            }
            let pid: u32 = otto
                .call("LaunchOnOwnWorkspace", &(argv,))
                .map_err(|e| e.to_string())?;
            Ok(json!(pid))
        }
        Some("capture") => {
            let path: String = otto
                .call("CaptureWorkspace", &("",))
                .map_err(|e| e.to_string())?;
            let path = PathBuf::from(path);
            let file = path.file_name().ok_or("no file name")?.to_owned();
            let inside = private.join("captures").join(&file);
            std::fs::copy(&path, &inside).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::set_permissions(&inside, std::fs::Permissions::from_mode(0o600));
            Ok(json!(runtime_dir
                .join(INNER_CAPTURES)
                .join(file)
                .display()
                .to_string()))
        }
        Some("workspace") => {
            let (output, x, y, width, height, scale): (String, i32, i32, i32, i32, f64) = otto
                .call("RequestOwnWorkspace", &())
                .map_err(|e| e.to_string())?;
            Ok(json!(format!("{output} {x} {y} {width} {height} {scale}")))
        }
        _ => Err("unknown request".into()),
    }
}

// ---------------------------------------------------------------------------
// ctl
// ---------------------------------------------------------------------------

fn ctl(args: Vec<OsString>) -> ExitCode {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let request = match args.first().map(String::as_str) {
        Some("launch") if args.len() > 1 => json!({ "cmd": "launch", "args": &args[1..] }),
        Some("capture") => json!({ "cmd": "capture" }),
        Some("workspace") => json!({ "cmd": "workspace" }),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("otto-sandbox: XDG_RUNTIME_DIR is not set");
        return ExitCode::FAILURE;
    };
    let socket = Path::new(&runtime_dir).join(INNER_CONTROL);
    let reply = UnixStream::connect(&socket)
        .and_then(|mut stream| {
            writeln!(stream, "{request}")?;
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line)?;
            Ok(line)
        })
        .map_err(|e| {
            format!(
                "{}: {e} (is this inside otto-sandbox run?)",
                socket.display()
            )
        });
    let reply: Value = match reply.and_then(|l| serde_json::from_str(&l).map_err(|e| e.to_string()))
    {
        Ok(reply) => reply,
        Err(err) => {
            eprintln!("otto-sandbox: {err}");
            return ExitCode::FAILURE;
        }
    };
    match (&reply["ok"], &reply["error"]) {
        (Value::Null, Value::String(err)) => {
            eprintln!("otto-sandbox: {err}");
            ExitCode::FAILURE
        }
        (Value::String(s), _) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        (ok, _) => {
            println!("{ok}");
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn options_are_parsed_up_to_the_command() {
        let options = parse_options(os(&[
            "--name",
            "Claude",
            "--bind",
            "/a",
            "--ro-bind",
            "/b",
            "--env",
            "KEY",
            "--no-net",
            "--",
            "claude",
            "--name",
            "x",
        ]))
        .unwrap();
        assert_eq!(options.name.as_deref(), Some("Claude"));
        assert_eq!(options.binds, vec![PathBuf::from("/a")]);
        assert_eq!(options.ro_binds, vec![PathBuf::from("/b")]);
        assert_eq!(options.envs, vec!["KEY".to_string()]);
        assert!(options.no_net);
        assert_eq!(options.command, os(&["claude", "--name", "x"]));
        assert!(parse_options(os(&["--name", "x"])).is_err(), "no command");
        assert!(parse_options(os(&["--bogus", "--", "x"])).is_err());
    }

    fn plan() -> Plan {
        Plan {
            home: "/home/u".into(),
            runtime_dir: "/run/user/1000".into(),
            project: "/home/u/proj".into(),
            private: "/run/user/1000/otto-sandbox/7".into(),
            exe: "/usr/bin/otto-sandbox".into(),
            binds: vec!["/home/u/.claude".into()],
            ro_binds: vec![],
            env: vec![("WAYLAND_DISPLAY".into(), "wayland-0".into())],
            net: false,
            new_session: true,
            command: os(&["claude"]),
        }
    }

    fn joined(plan: &Plan) -> String {
        bwrap_args(plan)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_sandbox_hides_the_users_session() {
        let line = joined(&plan());
        // A private /tmp and /run: no ydotoold socket, no session bus, no
        // other Wayland or X socket.
        assert!(line.contains("--tmpfs /tmp"));
        assert!(line.contains("--tmpfs /run "));
        assert!(line.contains("--dev /dev"), "a fresh /dev, without uinput");
        assert!(line.contains("--clearenv"));
        assert!(line.contains("--unshare-net"));
        assert!(line.contains("--new-session"));
        // Home is empty but for the project and what was bound.
        let home = line.find("--tmpfs /home/u").unwrap();
        let project = line.find("--bind /home/u/proj /home/u/proj").unwrap();
        let claude = line.find("--bind /home/u/.claude /home/u/.claude").unwrap();
        assert!(
            home < project && project < claude,
            "binds must follow the empty home"
        );
        // The only Wayland socket is the agent's.
        assert!(
            line.contains("--bind /run/user/1000/otto-sandbox/7/wayland /run/user/1000/wayland-0")
        );
        assert!(line.ends_with("-- claude"));
    }

    #[test]
    fn printed_arguments_read_back_as_one_each() {
        assert_eq!(shell_quote("/usr/bin"), "/usr/bin");
        assert_eq!(shell_quote("eDP-1 0 0"), "'eDP-1 0 0'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn the_network_stays_unless_refused() {
        let mut plan = plan();
        plan.net = true;
        plan.new_session = false;
        let line = joined(&plan);
        assert!(!line.contains("--unshare-net"));
        assert!(!line.contains("--new-session"));
    }

    #[test]
    fn the_environment_is_cleared_but_for_what_is_named() {
        std::env::set_var("OTTO_SANDBOX_TEST_KEY", "secret");
        std::env::set_var("OTTO_SANDBOX_TEST_OTHER", "nope");
        let env = inner_env(
            Path::new("/home/u"),
            Path::new("/run/user/1000"),
            "agent-1",
            "eDP-1 0 0 1440 960 2",
            &["OTTO_SANDBOX_TEST_KEY".into()],
        );
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("OTTO_SANDBOX_TEST_KEY"), Some("secret"));
        assert_eq!(get("OTTO_SANDBOX_TEST_OTHER"), None);
        assert_eq!(get("WAYLAND_DISPLAY"), Some("wayland-0"));
        assert_eq!(get("OTTO_AGENT_SEAT"), Some("agent-1"));
        assert!(get("PATH").unwrap().starts_with(INNER_BIN));
        assert_eq!(get("DBUS_SESSION_BUS_ADDRESS"), None);
        assert_eq!(get("DISPLAY"), None);
    }
}
