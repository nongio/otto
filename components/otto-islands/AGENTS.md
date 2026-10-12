# otto-islands

The dynamic island at the top centre of the screen, and the session's notification daemon. It shows notifications, the playing track and other activities as islands, puts up the system dialog other programs ask questions through, and sets badges and progress on dock icons. It is a Wayland client started by the compositor through `exec_once`, and a D-Bus service.

How it works: [otto-islands](../../docs/developer/otto-islands.md).

- **Layer:** app. Used by nothing (an app).

## Dependencies
- **Internal:** `otto-kit` for the app runner, drawing, Otto's client protocols and i18n, and for the `otto-dbus` proxies as `otto_kit::dbus`.
- **External:** owns `org.freedesktop.Notifications` and `org.otto.Island` on the session bus through `zbus` on tokio; `org.otto.Island` serves `org.otto.Island1` at `/org/otto/Island` and `org.otto.Dialog1` at `/org/otto/Dialog`. Follows MPRIS players over D-Bus and their audio through PipeWire (`deny.toml` lets only this crate, the compositor, `otto-kit` and `otto-rdp` link it). Keeps the `notifications` table in xdg-permission-store. Needs `otto_surface_style_v1`, `otto_dock_v1` and `wlr-layer-shell`, so it runs only on Otto. `ureq` fetches album art.

## Develop
- Against a dev compositor: `cargo run -- --winit`, then `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-islands`. Send a test with `notify-send -a Test "Title" "Body"`.
- For real it is an `exec_once` entry in the shipped config (`otto_config.example.toml`). To restart it, kill it and start `otto-islands` again.
- Its bus names are taken without allowing replacement. A copy started while another runs on the same session bus has no notification daemon and no dialog, so stop the running one first.
- Logs: `RUST_LOG=otto_islands=debug` (the default filter is `info`).
- Tests: `cargo test -p otto-islands`.

## Where things are
- `src/main.rs` is the app (islands, layout, hit testing, input, D-Bus startup); `src/renderer.rs` draws islands; `src/state.rs` is the shared model.
- The notification daemon and its permission check are `src/notifications.rs` and `src/notification_permission.rs`.
- `src/dbus_service.rs` serves `Island1` and `Dialog1`; `src/dialog.rs` draws and runs the dialog.
- The music island is `src/music.rs` with its MPRIS, audio-route and spectrum helpers; dock badges are `src/dock_overlays.rs`.

## Rules
- Animation, clipping, shadow and blur are the compositor's, through `otto_surface_style_v1`. The client redraws a buffer only when content changes. Do not add a client-side animation loop.
- The client proxies for `org.otto.Island1` and `org.otto.Dialog1` live in `components/otto-dbus` (`island.rs`, `dialog.rs`), and the bus name and object paths are declared in both crates. A change to either interface goes in both places.
- The dialog is shared: `xdg-desktop-portal-otto`, `otto-agents` and otto-bar's log out ask through `org.otto.Dialog1`. Keep its responses stable.
- The music bars listen to the player's own stream, never to the whole output.

## See also
- [README](README.md).
- Specs: [notification island](../../specs/notification-island.md), [dynamic island](../../specs/dynamic-island.md), [portal access dialog](../../specs/portal-access-dialog.md).
- [Surface style protocol](../../docs/developer/surface-style-protocol.md), [Permissions](../../docs/developer/permissions.md), user guide [Dynamic island](../../docs/user/dynamic-island.md).
- [otto-dbus](../otto-dbus/AGENTS.md).
