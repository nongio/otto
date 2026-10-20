# otto-dbus

One typed zbus client proxy for every D-Bus interface Otto calls. Otto's own `org.otto.*` interfaces: those the compositor serves (settings, shell, screencast, compositor), otto-islands (dialog, island), otto-files (file picker, files, desk) and otto-stash. And the ones other software serves: UPower (`upower`), logind (`login1`), the Settings portal (`portal_settings`), the Access portal backend (`portal_access`), tray items (`status_notifier_item`) and their menus (`dbusmenu`). A caller takes the proxy from here. It does not declare its own `#[zbus::proxy]` (the `dbus-proxies-in-otto-dbus` rule in `architecture.toml` fails the build if it does) or call `call_method` with a method name in a string.

How it works: [otto-dbus](../../docs/developer/otto-dbus.md).

- **Layer:** foundation. Used by `otto-kit` (re-exported as `otto_kit::dbus`, the path toolkit apps such as `otto-bar` and `otto-auth-ui` use), and directly by `xdg-desktop-portal-otto`, `otto-agents`, `otto-rdp` and `otto-msg`, which link it without pulling in Skia.

## Dependencies
- **Internal:** none.
- **External:** `zbus`, taken from `[workspace.dependencies]` like every other member, so it runs on the same tokio executor as the rest of the workspace. No UI and no async runtime of its own.

## Develop
- `cargo build -p otto-dbus`. The crate has no tests.

## Where things are
- One module per interface, named after it, Otto's own first in `lib.rs` and then the ones other software serves. Each exports `SERVICE` and `PATH`, plus any wire tuple types the interface uses (`WireChoice`, `WireRequest`, `WireLayout`, …). An interface with no fixed bus name (`portal_access`, `status_notifier_item`, `dbusmenu`) has no `SERVICE`: the caller sets the destination, and the path too where it varies.
- zbus generates an async proxy (`SettingsProxy`) and a blocking one (`SettingsProxyBlocking`) for every interface.

## Rules
- A proxy module's doc comment names the interface and the file that serves it. Change the two together.
- A new `#[interface]` server gets its proxy here in the same change: a module named after the interface, exporting `SERVICE` and `PATH`, with the method signatures copied from the server's impl. A server method returning `zbus::fdo::Result<T>` or `Result<T, SomeFault>` becomes `zbus::Result<T>` on the client side. Signals become `#[zbus(signal)]` methods on the proxy.
- Add a new module to the interface table on the [otto-dbus](../../docs/developer/otto-dbus.md) page.
- A freedesktop or other third-party interface gets its proxy here when Otto first calls it, declaring the methods, properties and signals Otto uses. Its doc comment names the interface and who serves it. Two callers of one interface share one proxy, extended to cover both.
- `zbus::fdo` already has proxies for the bus itself (`DBusProxy`) and the standard `org.freedesktop.DBus.*` interfaces; use those rather than declaring them here.
- Inside otto-kit the Settings portal has one shared client, `portal_settings.rs`, built on `portal_settings::PortalSettingsProxy`.

## See also
- [Settings D-Bus API](../../docs/developer/settings-dbus-api.md), [Shell D-Bus API](../../docs/developer/shell-dbus-api.md).
- [otto-foundations](../otto-foundations/AGENTS.md), [otto-kit](../otto-kit/AGENTS.md).
