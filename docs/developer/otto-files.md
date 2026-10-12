# otto-files

Otto's file browser, file picker and desk, in one binary. The browser is a document window; the picker is a transient window serving another application's open or save request through the desktop portal; the desk is the Desktop folder drawn on a layer surface below every window. Below the chrome they share one directory model, one set of list, grid and column views, and Peek, the Space-bar preview, embedded as a library.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies
- **Internal:** `otto-kit` (windows, drawing, menus, scroll panes), `otto-peek` (the embedded preview, its sandboxed decode worker and the shared thumbnail cache), `otto-media-kit` without default features (video playback in Peek; the GStreamer worker is the separate `otto-media-worker` binary, found beside the executable or on `PATH`), `otto-search` (search and the Recent place through LocalSearch).
- **External:**
  - D-Bus names it owns: `org.otto.Files1` (each browser window), `org.otto.FilePicker1` (the picker, brokered by `xdg-desktop-portal-otto`), `org.otto.Desk1` (the desk). The interfaces are listed in [otto-dbus](otto-dbus.md).
  - Run time: LocalSearch for search and Recent; `otto-stash` on the bus for "Add to Stash"; the external thumbnailers and rasterisers Peek uses.
  - `laye-rs` lays out the panels; `zbus` and `tokio` carry the services and async reads.

## Develop
- `cargo run -p otto-files` browses `$HOME`; pass a folder, a file to select, or `--search 'kind:pdf'`. `--trash` and `--empty-trash` open the Trash window. Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-files`.
- `--desk` runs the desk. The compositor starts it as `otto-files --desk` from `PATH` when `desk.enabled` is on, and restarts it if it dies (`src/desk.rs` in the compositor). Killing it picks up the `otto-files` on `PATH`, not a `target/` build.
- `--picker` serves `org.otto.FilePicker1`. The session bus starts it through [org.otto.FilePicker1.service](../../components/otto-files/org.otto.FilePicker1.service) (`Exec=/usr/local/bin/otto-files --picker`), and the process outlives each request, so stop the running `otto-files --picker` before testing a new picker build.
- `--recognise <path>...` runs text recognition on pictures and exits.
- `RUST_LOG` filters logging (default `info`). `OTTO_FILES_QV_AUTO` and `OTTO_FILES_PALETTE_AUTO` open Peek or the command palette at start, for scripted checks.
- Tests: `cargo test -p otto-files`. Scroll micro-benchmarks in `src/bench.rs` are ignored: `cargo test -p otto-files --lib -- --ignored --nocapture bench`.
- `scripts/` holds the command scripts Files ships (`zip`, `unzip`, `pdf`, `ask`). Users add their own to `~/.config/otto/files-scripts/`; the protocol is in `src/scripts.rs`.

## Code map
- `src/main.rs`: picks the shell from the command line. Everything else is the library.
- `src/app/`: the browser window, split by concern (`listing`, `navigation`, `selection`, `drag_drop`, `file_ops`, `menus`, `palette_session`, `peek_session`, `picking`, `desk_edit`, ...). Tests sit beside them as `*_tests.rs`.
- `src/model.rs`: entries, places, sorting. `src/view.rs` and `src/scene.rs`: window layout and the `lay-rs` scene. `src/pane_surfaces.rs`: Miller columns as subsurfaces.
- `src/picker.rs`, `src/dbus.rs`, `src/picker_dirs.rs`: the picker. `src/desk.rs`, `src/desk_service.rs`: the desk. `src/files_service.rs`: `org.otto.Files1`.
- `src/command.rs`, `src/palette.rs`, `src/scripts.rs`: the command palette and its script provider.
- `src/thumbnails.rs`, `src/photos.rs`, `src/search.rs`, `src/recent.rs`, `src/tasks.rs` (long jobs in the status bar, island and dock), `src/undo_history.rs`.
- `src/places_config.rs` and `src/remembered.rs`: `~/.config/otto/files.toml` and what windows remember between runs.

## Rules
- `otto_peek::run_worker_if_requested()` stays the first call in `main`, before the tokio runtime and any Wayland connection. The decode worker is this binary re-executed; without it a preview starts a second file browser.
- `files.toml` is shared: Peek reads its `[peek]` section and Settings writes the desk's keys. Keep keys in step with `otto-peek/src/config.rs` and `otto-settings/src/panes/desk.rs`.
- The browser, the picker and the desk share the view layer. A change in `src/app/` reaches all three; check each.

## See also
- Specs: [file browser](../../specs/file-browser.md), [file picker](../../specs/file-picker.md), [desk](../../specs/desk.md), [command palette](../../specs/file-command-palette.md), [search language](../../specs/search-language.md).
- [File previews](file-previews.md), [File search](file-search.md), [File icons](file-icons.md), [Scroll pane](scroll-pane.md), [otto-peek](otto-peek.md).
- User guides: [Files](../user/files.md), [custom commands](../user/files-custom-commands.md), [command palette](../user/files-command-palette.md).
