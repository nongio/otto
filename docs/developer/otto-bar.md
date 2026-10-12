# otto-bar

The top menu bar. It shows the focused app's global menu on the left, and tray icons, the keyboard layout, the battery and the clock on the right. It is a Wayland client on a `wlr-layer-shell` surface, started by the compositor through `exec_once`. It follows window focus and settings over D-Bus, and serves the tray and app-menu registries itself.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** `otto-kit` for the app runner, menus, drawing and i18n, and for the `org.otto.*` client proxies it re-exports from [otto-dbus](otto-dbus.md) as `otto_kit::dbus`.
- **External:**
  - `zbus` on tokio for everything on the session bus.
  - Calls `org.otto.Shell1` (focused window, input devices, commands), `org.otto.Settings` (the `topbar.*` settings, followed live through `Changed`), `org.otto.Dialog1` on otto-islands (the log out question) and UPower, with sysfs as the fallback.
  - Serves `org.kde.StatusNotifierWatcher` (tray) and `com.canonical.AppMenu.Registrar` (menus of GTK and X11 Qt apps). Wayland Qt apps report their menu to the compositor over `org_kde_kwin_appmenu` instead, and the bar reads it from `org.otto.Shell1`.
  - Needs `wlr-layer-shell` from the compositor.

## Develop

- Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-bar`.
- For real it is an `exec_once` entry in the shipped config (`otto_config.example.toml`). To restart it in a session, kill it and start `otto-bar` again.
- Logs: `RUST_LOG=otto_bar=debug` (the default filter is `info`).
- Tests: `cargo test -p otto-bar`.
- Its own config is `otto-bar.toml`, read from `/etc/otto/`, `~/.config/otto/`, then the working directory. See the README.

## Code map

- `src/main.rs` starts tokio, i18n and the app runner.
- `src/app.rs` is the bar app: layout, menus, pointer and keyboard.
- `src/bar.rs` draws the bar.
- `src/appmenu.rs` finds the focused window's menu; `src/dbusmenu.rs` is the `com.canonical.dbusmenu` client.
- `src/tray.rs` is the SNI watcher and item reader.
- `src/keyboard_layout.rs` is the layout indicator and its menu.
- `src/battery.rs` draws the battery; `src/power.rs` reads UPower, sysfs and the CPU state.
- `src/clock.rs`, `src/logout.rs`, `src/settings.rs` (shared value handling for `org.otto.Settings`), `src/config.rs` (`otto-bar.toml`).

## Rules

- The tray watcher and app-menu registrar names are taken without allowing replacement. A second bar on the same session bus gets neither, so a dev copy started next to the live bar shows no tray and no app menu.
- Clock visibility and format come from the compositor's `topbar.*` settings first. `clock_format` in `otto-bar.toml` is only the fallback for an empty setting.
- With `topbar.show_app_menu` off the bar also gives up the registrar name, so apps keep their menus in their own windows. Keep that when changing `appmenu.rs`.
- Battery and CPU state are drawn by the bar and never published as an SNI item (see the non-goals in the spec).

## See also

- [README](../../components/otto-bar/README.md) for the config file.
- [Top bar spec](../../specs/topbar.md) and the user guide [Top bar](../user/topbar.md).
- [Shell D-Bus API](shell-dbus-api.md) and [Settings D-Bus API](settings-dbus-api.md).
