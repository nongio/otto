# otto-settings

Otto's Settings app: a window with a sidebar of panes, each row bound to a setting. It reads and writes Otto's configuration through the compositor's `org.otto.Settings` D-Bus interface, and talks directly to the system services some panes cover (accounts, sound, the file index, the agent and dictation units). With no compositor answering, the panes draw placeholder values and changes are not saved, which is enough to judge a layout.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` (window, drawing, widgets, the `org.otto.Settings` proxy in `otto_kit::dbus::settings`, the item list for sidebar search), `otto-search` (the Search pane's view of LocalSearch), `otto-agents-client` (the harness presets the Agents pane writes, shared with `otto-setup`).
- **External:**
  - D-Bus: `org.otto.Settings` (the compositor), `org.freedesktop.Accounts` (Account pane; passwords hashed with `sha-crypt`), the portal's `FileChooser` and `PermissionStore`, LocalSearch.
  - Commands: `pactl` for Sound (PipeWire through `pipewire-pulse`, or PulseAudio); `systemctl --user` for `localsearch-3.service`, the `otto-stt-<engine>` dictation units, and the `otto-agents` and `otto-agents-bridge` units.
  - Files: XKB's `evdev.xml` for keyboard layouts, installed cursor themes (`xcursor`). `toml_edit` writes `agents.toml`, `dictation.toml` and `files.toml` keeping their comments.

## Develop
- `cargo run -p otto-settings` opens the window; `--pane dock` opens on a pane, `--setting dock.size` on a setting. Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-settings`.
- `cargo run -p otto-settings -- --png out.png [Pane] [--search text]` renders panes offscreen to a PNG, with no display.
- It is an ordinary app started from its desktop entry or the dock. Close the window and run it again to try a change.
- `OTTO_SETTINGS_NO_BLUR` turns off the frosted sidebar.
- Tests: `cargo test -p otto-settings`.

## Where things are
- `src/main.rs` is the app: window, input, editing, pop-ups.
- `src/settings_client.rs` is the `org.otto.Settings` client. Schema, values and the overridden set are fetched once into a store the draw path reads without the bus.
- `src/model.rs` lays out every pane; `src/panes/` has one module per pane, some owning files and units of their own.
- `src/view.rs` and `src/widgets.rs` draw the window and the row controls; `src/preview.rs` is `--png`.

## Rules
- A row that carries a setting's identifier must use one the compositor's schema serves (`src/settings/schema.rs` at the repository root). Ranges, help text and whether a change applies live or on restart come from the schema, not from the pane.
- Identifiers in the schema are a contract: never rename or repurpose one.
- The Agents pane writes `agents.toml`, which `otto-agents` reads (`components/otto-agents/src/config.rs`). The Desk group writes `files.toml`, which `otto-files` reads. Keep the keys in step with their readers.
- The draw path runs every frame and must not block: bus and process calls run on threads and wake the loop.

## See also
- [Spec](../../specs/settings-app.md), [settings D-Bus contract](../../docs/developer/settings-dbus-api.md), [Permissions](../../docs/developer/permissions.md), [Scroll pane](../../docs/developer/scroll-pane.md).
- User guide [Settings](../../docs/user/settings.md).
