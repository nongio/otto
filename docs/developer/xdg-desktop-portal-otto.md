# xdg-desktop-portal-otto

`xdg-desktop-portal-otto` is Otto's backend for xdg-desktop-portal. The `xdg-desktop-portal` frontend hands it the ScreenCast, Settings, Access, Screenshot and FileChooser requests apps make, and it answers them through Otto's own services. It is a headless D-Bus service that owns `org.freedesktop.impl.portal.desktop.otto`; it draws nothing and has no Wayland connection.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** `otto-dbus` (proxies for `org.otto.ScreenCast` and `org.otto.Settings` in the compositor, `org.otto.Dialog1` in otto-islands, `org.otto.FilePicker1` in otto-files), `otto-foundations` (XDG directories, `file://` URIs).
- **External:** the `xdg-desktop-portal` frontend, xdg-permission-store (remembered screen shares), `grim` for screenshots. Built on zbus and tokio.

## Develop

- Installed as `/usr/libexec/xdg-desktop-portal-otto`, bus-activated through `org.freedesktop.impl.portal.desktop.otto.service` and run by the `xdg-desktop-portal-otto.service` user unit. `otto.portal` tells the frontend which interfaces it serves, for sessions whose `XDG_CURRENT_DESKTOP` contains `otto`.
- `cargo run -p xdg-desktop-portal-otto` replaces the running backend: it takes the bus name even when another instance holds it, and the old one exits.
- After the backend restarts, run `scripts/portal-refresh.sh`. The frontend caches the backend's properties when it loads it, so a stale frontend rejects every screencast with "Unavailable cursor mode".
- Logs go to stderr and follow `RUST_LOG`.
- Tests: `cargo test -p xdg-desktop-portal-otto`. The end-to-end screencast test is in the compositor: `cargo test --features headless --test screenshare`.

## Code map

- [src/main.rs](../../components/xdg-desktop-portal-otto/src/main.rs): exports every interface, then claims the bus name.
- [src/portal/](../../components/xdg-desktop-portal-otto/src/portal/): one module per portal (`interface.rs` is ScreenCast, `settings.rs`, `access.rs`, `screenshot.rs`, `file_chooser.rs`), plus the Request and Session objects, `restore.rs` (restore tokens) and `remembered.rs` (the permission store).
- [src/otto_client/](../../components/xdg-desktop-portal-otto/src/otto_client/): wrappers over the Otto services each portal calls.

## Rules

- Claim the bus name only after every interface is exported, and always with replacement allowed. A backend left over from an earlier login otherwise keeps the name.
- A new portal interface goes in `otto.portal`'s `Interfaces=` and in `portals.conf.example`, which the packages install.
- The portal brokers and others render: the Access dialog is drawn by otto-islands, the file chooser by otto-files. Keep windows out of this crate.
- Cursor modes keep the portal's bitmask numbering end to end; the details are in the screenshare spec.

## See also

- [README](../../components/xdg-desktop-portal-otto/README.md), [ScreenCast backend contract](../../components/xdg-desktop-portal-otto/ScreenCast-backend-spec.md), [frontend behaviour](../../components/xdg-desktop-portal-otto/ScreenCast-frontend-spec.md)
- [Screenshare](screenshare.md), [Permissions](permissions.md), [XDG specifications](xdg-specifications.md)
- Specs: [screenshare](../../specs/screenshare.md), [portal Access dialog](../../specs/portal-access-dialog.md), [file picker](../../specs/file-picker.md)
