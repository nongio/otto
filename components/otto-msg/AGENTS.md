# otto-msg

`otto-msg` drives the Otto compositor from a script. It runs i3-syntax commands, prints the window tree, workspaces, outputs, inputs and the compositor's debug info as JSON, and follows events. It is a command-line client of the compositor's `org.otto.Shell1` and `org.otto.Compositor` D-Bus services, with no Wayland connection of its own.

`otto-msg doctor` writes a bug report: it gathers the operating system, graphics cards and renderer, screens, Otto's version and backend, and optionally the configuration files and logs; lets the user tick, preview and redact each part in a terminal interface; and writes one Markdown file to attach to an issue. It never uploads anything.

- **Layer:** app. Used by nothing (an app); the Otto plugin's skills call it.

## Dependencies
- **Internal:** `otto-dbus` (the `org.otto.Shell1` and `org.otto.Compositor` proxies), `otto-foundations` (XDG directories, `tilde`).
- **External:** a running Otto on the session bus for everything but `doctor`, which works without one. Blocking zbus, serde_json, libc (termios, `poll`, `localtime_r`, `getuid`). No terminal UI crate: raw mode, the alternate screen and key parsing are hand-written in `src/doctor/term.rs`. `doctor` runs `journalctl` when Otto logs to the journal, and `wl-copy`, when present, for the clipboard.

## Develop
- `cargo run -p otto-msg -- focus right`, or `cargo run -p otto-msg -- -t get_tree`. `--help` lists the options.
- `cargo run -p otto-msg -- doctor --print --all` prints a full report without the interface; `--no-clipboard` keeps a write from touching the clipboard.
- It talks to whichever Otto owns `org.otto.Shell1` on the session bus.
- Tests: `cargo test -p otto-msg`: argument parsing, and for `doctor` redaction, the section readers (over a fixture root with `etc/`, `proc/` and `sys/class/drm/`), log discovery, the report and issue link, and key parsing. The commands themselves are tested in the compositor, without the bus: `cargo test --features headless --test tiling_scripting`, and `--test debug_info` for `DebugInfo`.
- The doctor interface needs a terminal on both stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`), not in the user's own terminal session.

## Where things are
- `src/main.rs`: argument parsing, one call per `-t` type, and JSON printing.
- `src/doctor/`: `mod.rs` (options, choosing between the interface, `--print` and a non-interactive write), `sections.rs` (each section's reader, everything read through `Sources`), `logs.rs` (where Otto's logs are), `summary.rs` (the last `otto::summary` line in a log, used when the compositor cannot answer), `compositor.rs` (the `DebugInfo` call), `redact.rs`, `report.rs` (Markdown, file name, issue link), `term.rs` and `tui.rs` (the terminal and the chooser).
- The services live in the compositor: `src/shell_service.rs` and `src/screenshare/dbus_service.rs` at the repository root; `DebugInfo`'s JSON is built in `src/debug_info.rs`.

## Rules
- The wire contracts are [shell-dbus-api.md](../../docs/developer/shell-dbus-api.md) and [compositor-dbus-api.md](../../docs/developer/compositor-dbus-api.md). A change to the commands or the JSON goes there and into the user guides, [scripting.md](../../docs/user/scripting.md) and [bug-reports.md](../../docs/user/bug-reports.md).
- `doctor` sends nothing anywhere. It writes a file, may copy it to the clipboard, and prints a link; the user attaches the file.
- Redaction is on by default, and the preview shows the text after redaction: what the user reads is what is written. Sections that can hold personal choices (configuration, logs) are off by default.
- Every doctor section still renders when Otto is not running: from the last `otto::summary` log line, marked as not live, or else saying the compositor is not reachable. The line's format is in [compositor-dbus-api.md](../../docs/developer/compositor-dbus-api.md); the compositor writes it in `src/debug_info.rs`.
- The user guides are copied into the Otto plugin's help skill. After editing one, run `scripts/sync-skill-docs.sh`; CI checks the copies with `--check`.

## See also
- [Shell D-Bus API](../../docs/developer/shell-dbus-api.md), [Compositor D-Bus API](../../docs/developer/compositor-dbus-api.md), [otto-dbus](../../docs/developer/otto-dbus.md).
- [Scripting guide](../../docs/user/scripting.md), [Reporting a bug](../../docs/user/bug-reports.md), [spec](../../specs/bug-reports.md), [Tiling plan](../../docs/developer/tiling-plan.md).
