# otto-files

Otto's file browser, file picker and desk, in one binary. The browser is a document window; the picker is a transient window serving another app's open or save request through the desktop portal; the desk is the Desktop folder drawn on a layer surface below every window. They share one directory model, one set of list, grid and column views, and Peek (the Space-bar preview) embedded as a library.

How it works: [otto-files](../../docs/developer/otto-files.md).

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` (windows, drawing, menus, scroll panes), `otto-peek` (the embedded preview, its sandboxed decode worker, the shared thumbnail cache), `otto-media-kit` without default features (video in Peek; the GStreamer worker is the separate `otto-media-worker` binary, found beside the executable or on `PATH`), `otto-search` (search and the Recent place through LocalSearch).
- **External:** owns `org.otto.Files1` (each browser window), `org.otto.FilePicker1` (the picker, brokered by `xdg-desktop-portal-otto`) and `org.otto.Desk1` (the desk); the interfaces are in `otto-dbus`. At run time: LocalSearch, `otto-stash` on the bus for "Add to Stash", the thumbnailers and rasterisers Peek uses. `laye-rs` lays out the panels; `zbus` and `tokio` carry the services and async reads.

## Develop
- `cargo run -p otto-files` browses `$HOME`; pass a folder, a file to select, or `--search 'kind:pdf'`. `--trash` and `--empty-trash` open the Trash window. Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-files`.
- `--desk` runs the desk. The compositor starts `otto-files --desk` from `PATH` when `desk.enabled` is on and restarts it if it dies (`src/desk.rs` in the compositor). Killing it picks up the `otto-files` on `PATH`, not a `target/` build.
- `--picker` serves `org.otto.FilePicker1`. The session bus starts it through `org.otto.FilePicker1.service` (`Exec=/usr/local/bin/otto-files --picker`) and the process outlives each request, so stop the running `otto-files --picker` before testing a new picker build.
- `--recognise <path>...` runs text recognition on pictures and exits.
- `RUST_LOG` filters logging (default `info`). `OTTO_FILES_QV_AUTO` and `OTTO_FILES_PALETTE_AUTO` open Peek or the command palette at start, for scripted checks.
- Tests: `cargo test -p otto-files`. Scroll benchmarks (ignored): `cargo test -p otto-files --lib -- --ignored --nocapture bench`.
- `scripts/` holds the command scripts Files ships (`zip`, `unzip`, `pdf`, `ask`). Users add their own to `~/.config/otto/files-scripts/`; the protocol is in `src/scripts.rs`.

## Where things are
- `src/main.rs` picks the shell from the command line; everything else is the library.
- `src/app/` is the browser window, split by concern, with tests beside each part as `*_tests.rs`.
- `src/model.rs` holds entries, places and sorting; `src/view.rs` and `src/scene.rs` the layout and the `lay-rs` scene.
- The picker, the desk and `org.otto.Files1` have modules of their own (`picker*`, `desk*`, `files_service.rs`). The command palette is `command.rs`, `palette.rs` and `scripts.rs`.

## Rules
- `otto_peek::run_worker_if_requested()` stays the first call in `main`, before the tokio runtime and any Wayland connection. The decode worker is this binary re-executed; without it a preview starts a second file browser.
- `files.toml` is shared: Peek reads its `[peek]` section and Settings writes the desk's keys. Keep keys in step with `otto-peek/src/config.rs` and `otto-settings/src/panes/desk.rs`.
- The browser, the picker and the desk share the view layer. A change in `src/app/` reaches all three; check each.

## See also
- Specs: [file browser](../../specs/file-browser.md), [file picker](../../specs/file-picker.md), [desk](../../specs/desk.md), [command palette](../../specs/file-command-palette.md), [search language](../../specs/search-language.md).
- [File previews](../../docs/developer/file-previews.md), [File search](../../docs/developer/file-search.md), [File icons](../../docs/developer/file-icons.md), [Scroll pane](../../docs/developer/scroll-pane.md), [otto-peek](../otto-peek/AGENTS.md).
- User guides: [Files](../../docs/user/files.md), [custom commands](../../docs/user/files-custom-commands.md), [command palette](../../docs/user/files-command-palette.md).
