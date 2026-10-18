# otto-dbus

One typed zbus client proxy for every `org.otto.*` D-Bus interface: those the compositor serves (settings, shell, screencast, compositor), otto-islands (dialog, island), otto-files (file picker, files, desk) and otto-stash. A caller takes the proxy from here. It does not declare its own `#[zbus::proxy]` or call `call_method` with a method name in a string.

How it works: [otto-dbus](../../docs/developer/otto-dbus.md).

- **Layer:** foundation. Used by `otto-kit` (re-exported as `otto_kit::dbus`, the path toolkit apps use), and directly by `xdg-desktop-portal-otto`, `otto-agents`, `otto-rdp` and `otto-msg`, which link it without pulling in Skia.

## Dependencies
- **Internal:** none.
- **External:** `zbus`, taken from `[workspace.dependencies]` like every other member, so it runs on the same tokio executor as the rest of the workspace. No UI and no async runtime of its own.

## Develop
- `cargo build -p otto-dbus`. The crate has no tests.

## Where things are
- One module per interface, named after it. Each exports `SERVICE` and `PATH`, plus any wire tuple types the interface uses (`WireChoice`, `WireRequest`, …).
- zbus generates an async proxy (`SettingsProxy`) and a blocking one (`SettingsProxyBlocking`) for every interface.

## Rules
- A proxy module's doc comment names the interface and the file that serves it. Change the two together.
- A new `#[interface]` server gets its proxy here in the same change: a module named after the interface, exporting `SERVICE` and `PATH`, with the method signatures copied from the server's impl. A server method returning `zbus::fdo::Result<T>` or `Result<T, SomeFault>` becomes `zbus::Result<T>` on the client side. Signals become `#[zbus(signal)]` methods on the proxy.
- Add a new module to the interface table on the [otto-dbus](../../docs/developer/otto-dbus.md) page.
- The freedesktop interfaces Otto calls are not declared here, because they are not Otto's. Inside otto-kit the Settings portal has one shared client, `portal_settings.rs`, and the FileChooser portal another, `portal_file_chooser.rs`.

## See also
- [Settings D-Bus API](../../docs/developer/settings-dbus-api.md), [Shell D-Bus API](../../docs/developer/shell-dbus-api.md).
- [otto-foundations](../otto-foundations/AGENTS.md), [otto-kit](../otto-kit/AGENTS.md).
