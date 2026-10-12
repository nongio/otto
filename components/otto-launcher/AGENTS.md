# otto-launcher

Otto's keyboard launcher: a fullscreen layer-shell overlay that takes the keyboard, filters apps and open windows as you type, and runs the one you pick. It also hosts Ask, the chat with an agent, and the agents mode that lists `otto-agents` sessions. It starts fresh on a keystroke and exits when something is chosen; there is no daemon.

How it works: [otto-launcher](../../docs/developer/otto-launcher.md).

- **Layer:** app. Used by nothing (an app). `otto-canvas` and other apps start it as a program (`--session`, `--ask --send`).

## Dependencies
- **Internal:** `otto-kit` (app loop, layer-shell and subsurfaces, text input, scroll pane, frosted material), `otto-agents-kit` with `dictation` (Ask's chat and log, the session feed, Ctrl+D dictation), `otto-peek` (thumbnails of attached files, decoded by a sandboxed worker).
- **External:** `zwlr_layer_shell_v1` for the overlay and `zwlr-foreign-toplevel-management-v1` for the window list, both from the compositor. `laye-rs` and `taffy` lay out the card. `freedesktop-desktop-entry` reads the installed apps. Ask needs a running `otto-agents`.

## Develop
- Otto binds it to a key (`"Ctrl+Space" = { run = { cmd = "otto-launcher", args = [] } }` in `otto_config.example.toml`). A dev build runs the same way: `cargo run -p otto-launcher`, or under a dev compositor `cargo run -- --winit` then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-launcher`.
- Modes: `--apps`, `--windows`, `--all`, `--ask`, `--agents`; `--file PATH`, `--session URI`, `--selection` and `--send` start in Ask. Started as `otto-ask` (a symlink the package installs) it opens in Ask.
- It grabs the keyboard, so for work on how it looks render the card offscreen: `cargo run -p otto-launcher --example preview -- /tmp/launcher "term"`.
- `RUST_LOG` filters logging (default `info`); `OTTO_AGENTS_URL` picks another agent service.
- Tests: `cargo test -p otto-launcher`.

## Where things are
- `src/main.rs` is the app: argument parsing, modes, input, Ask and agents mode.
- `src/view.rs` is the card as a `lay-rs` scene; `src/lib.rs` exposes it so the example and tests build it without a compositor.
- `src/source.rs` is the `Source` trait, with one source per kind of result (apps, windows, arithmetic).
- `src/drafts.rs` keeps unsent Ask text per session in `$XDG_STATE_HOME/otto/ask-drafts.json`.

## Rules
- Do not start it on the live session to check a visual change; render with `cargo run -p otto-launcher --example preview` instead.
- `otto_peek::run_worker_if_requested()` stays the first line of `main`: the thumbnail worker is this binary re-executed.
- A new kind of result is a new `Source`; filtering, layout and input do not change for it.
- Rows, ranking and list keys are shared with the canvas through `otto-agents-kit` and `otto_kit::components::item_list`. Change them there.

## See also
- [Launcher spec](../../specs/launcher.md), user guide [Launcher](../../docs/user/launcher.md).
- [Agents](../../docs/developer/agents.md), [Side canvas](../../docs/developer/side-canvas.md).
- [otto-agents-kit](../otto-agents-kit/AGENTS.md), [otto-peek](../otto-peek/AGENTS.md).
