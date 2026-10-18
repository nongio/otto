# otto-doctor

`otto-doctor` writes a bug report. It gathers the operating system, graphics cards and renderer, screens, Otto's version and backend, and optionally the configuration files and session logs; lets the user tick, preview and redact each part in a terminal interface; and writes one Markdown file to attach to an issue. It never uploads anything.

- **Layer:** app. Used by nothing.

## Dependencies
- **Internal:** `otto-dbus` (the `org.otto.Compositor` proxy), `otto-foundations` (XDG directories, `tilde`).
- **External:** blocking zbus, serde_json, libc (termios, `poll`, `localtime_r`). No terminal UI crate: raw mode, the alternate screen and key parsing are hand-written in `src/term.rs`. `wl-copy`, when present, for the clipboard.

## Develop
- `cargo run -p otto-doctor -- --print --all` prints a full report without the interface; `--no-clipboard` keeps a write from touching the clipboard.
- Tests: `cargo test -p otto-doctor`. Redaction, the section readers (over a fixture root with `etc/`, `proc/` and `sys/class/drm/`), the report and issue link, key parsing and arguments.
- The interface needs a terminal on both stdin and stdout. Drive it in a pseudo-terminal (Python's `pty`), not in the user's own terminal session.
- The compositor side is tested headless: `cargo test --features headless --test debug_info`.

## Where things are
- `src/main.rs`: arguments, choosing between the interface, `--print` and a non-interactive write, and writing the file.
- `src/sections.rs`: each section's reader; everything it reads comes from `Sources`, so tests point it at a fixture tree.
- `src/compositor.rs`: the `DebugInfo` call and its JSON.
- `src/redact.rs`: home folder, user and host names, IPv4, IPv6 and MAC addresses.
- `src/report.rs`: the Markdown, the file name and the pre-filled issue link.
- `src/term.rs` and `src/tui.rs`: the terminal and the chooser.

## Rules
- Nothing leaves the machine. The tool writes a file, may copy it to the clipboard, and prints a link; the user attaches the file.
- Redaction is on by default, and the preview shows the text after redaction: what the user reads is what is written.
- Sections that can hold personal choices (configuration, logs) are off by default.
- Every section still renders when Otto is not running, saying the compositor is not reachable.
- The `DebugInfo` contract is [compositor-dbus-api.md](../../docs/developer/compositor-dbus-api.md). A new field goes there, into `src/debug_info.rs` at the repository root, and into `src/compositor.rs` here.
- The user guide is [bug-reports.md](../../docs/user/bug-reports.md). After editing it, run `scripts/sync-skill-docs.sh`.

## See also
- [Compositor D-Bus API](../../docs/developer/compositor-dbus-api.md), [otto-dbus](../../docs/developer/otto-dbus.md).
- [Reporting a bug](../../docs/user/bug-reports.md), [Troubleshooting](../../docs/user/troubleshooting.md), [spec](../../specs/otto-doctor.md).
