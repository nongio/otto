# otto-bar

The top menu bar: the focused app's global menu on the left, and tray icons, the keyboard layout, the battery and the clock on the right. It is a Wayland client on a `wlr-layer-shell` surface, started by the compositor through `exec_once`. It follows window focus and settings over D-Bus, and serves the tray and app-menu registries itself.

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` for the app runner, menus, drawing and i18n, and for the `org.otto.*` proxies it re-exports from `otto-dbus` as `otto_kit::dbus`.
- **External:** `zbus` on tokio for the session bus. Calls `org.otto.Shell1` (focused window, input devices, commands), `org.otto.Settings` (the `topbar.*` settings, followed through `Changed`), `org.otto.Dialog1` on otto-islands (the log out question) and UPower, with sysfs as the fallback. Serves `org.kde.StatusNotifierWatcher` (tray) and `com.canonical.AppMenu.Registrar` (menus of GTK and X11 Qt apps); Wayland Qt apps report their menu to the compositor over `org_kde_kwin_appmenu`, and the bar reads it from `org.otto.Shell1`. Needs `wlr-layer-shell`.

## Develop
- Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-bar`.
- For real it is an `exec_once` entry in the shipped config (`otto_config.example.toml`). To restart it in a session, kill it and start `otto-bar` again.
- Logs: `RUST_LOG=otto_bar=debug` (the default filter is `info`).
- Tests: `cargo test -p otto-bar`.
- Its own config is `otto-bar.toml`, read from `/etc/otto/`, `~/.config/otto/`, then the working directory. See the README.

## Where things are
- `src/app.rs` is the bar app (layout, menus, input) and `src/bar.rs` draws it.
- The global menu is `src/appmenu.rs` with the `com.canonical.dbusmenu` client in `src/dbusmenu.rs`; the tray is `src/tray.rs`.
- The right-hand indicators each have a module (keyboard layout, battery and power, clock, log out).
- `src/settings.rs` handles `org.otto.Settings` values; `src/config.rs` reads `otto-bar.toml`.

## Rules
- The tray watcher and app-menu registrar names are taken without allowing replacement. A second bar on the same session bus gets neither, so a dev copy started next to the live bar shows no tray and no app menu.
- Clock visibility and format come from the compositor's `topbar.*` settings first. `clock_format` in `otto-bar.toml` is only the fallback for an empty setting.
- With `topbar.show_app_menu` off the bar also gives up the registrar name, so apps keep their menus in their own windows. Keep that when changing `appmenu.rs`.
- Battery and CPU state are drawn by the bar and never published as an SNI item (see the non-goals in the spec).

## See also
- [README](README.md) for the config file.
- [Top bar spec](../../specs/topbar.md) and the user guide [Top bar](../../docs/user/topbar.md).
- [Shell D-Bus API](../../docs/developer/shell-dbus-api.md), [Settings D-Bus API](../../docs/developer/settings-dbus-api.md).
