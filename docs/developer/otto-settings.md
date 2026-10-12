# otto-settings

Otto's Settings app: a window with a sidebar of panes, each row bound to a setting. It is a Wayland client that reads and writes Otto's configuration through the compositor's `org.otto.Settings` D-Bus interface, and talks directly to the system services some panes cover (accounts, sound, the file index, the agent and dictation units). With no compositor answering, the panes draw placeholder values and changes are not saved, which is enough to judge a layout.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies
- **Internal:** `otto-kit` (window, drawing, widgets, the `org.otto.Settings` proxy in `otto_kit::dbus::settings`, the item list for sidebar search), `otto-search` (the Search pane's view of LocalSearch).
- **External:**
  - D-Bus: `org.otto.Settings` (the compositor), `org.freedesktop.Accounts` (Account pane; passwords hashed with `sha-crypt`), the portal's `FileChooser` and `PermissionStore`, LocalSearch.
  - Commands: `pactl` for Sound (PipeWire through `pipewire-pulse`, or PulseAudio), `systemctl --user` for `localsearch-3.service`, the `otto-stt-<engine>` dictation units, and the `otto-agents` and `otto-agents-bridge` units.
  - Files: XKB's `evdev.xml` for keyboard layouts, installed cursor themes (`xcursor`). `toml_edit` writes `agents.toml`, `dictation.toml` and `files.toml` while keeping their comments.

## Develop
- `cargo run -p otto-settings` opens the window; `--pane dock` opens on a pane, `--setting dock.size` on a setting. Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-settings`.
- `cargo run -p otto-settings -- --png out.png [Pane] [--search text]` renders panes offscreen to a PNG, with no display.
- It is an ordinary app started from its desktop entry or the dock. Close the window and run it again to try a change.
- `OTTO_SETTINGS_NO_BLUR` turns off the frosted sidebar.
- Tests: `cargo test -p otto-settings`.

## Code map
- `src/main.rs`: the app: window, input, editing, pop-ups.
- `src/settings_client.rs`: the `org.otto.Settings` client. Schema, values and the overridden set are fetched once and kept in a store the draw path reads without the bus.
- `src/model.rs`: the layout of every pane and the rows no setting serves, with placeholder values.
- `src/panes/`: one module per pane; `agents/`, `dictation.rs`, `desk.rs` and `search.rs` own files and units of their own.
- `src/view.rs` (titlebar, sidebar, pane), `src/widgets.rs` (row controls), `src/sidebar_search.rs`, `src/sheet.rs` (the Account pane's sheets).
- `src/pulse.rs` (Sound via `pactl`), `src/xkb_registry.rs`, `src/discovery.rs` (choices that depend on what is installed), `src/preview.rs` (`--png`).

## Rules
- A row that carries a setting's identifier must use one the compositor's schema serves ([src/settings/schema.rs](../../src/settings/schema.rs)). Ranges, help text and whether a change applies live or on restart come from the schema, not from the pane.
- Identifiers in the schema are a contract: never rename or repurpose one.
- The Agents pane writes `agents.toml`, which `otto-agents` reads (`components/otto-agents/src/config.rs`). The Desk group writes `files.toml`, which `otto-files` reads. Keep the keys in step with their readers.
- The draw path runs every frame and must not block: bus and process calls run on threads and wake the loop.

## See also
- [Spec](../../specs/settings-app.md), [settings D-Bus contract](settings-dbus-api.md), [Permissions](permissions.md), [Scroll pane](scroll-pane.md).
- User guide [Settings](../user/settings.md).
