# otto-input-overlay

`otto-input-overlay` draws on-screen overlays of touchpad fingers and pressed keys, for screen recordings. It is a Wayland client that reads the kernel's evdev nodes directly, because per-finger touchpad positions and keys meant for other windows never reach a Wayland client. A tray applet switches each overlay on and off and quits.

It sits in the **app** layer (see [Architecture Rules](architecture.md)) and is used by nothing: it is an app.

## Dependencies

- **Internal:** `otto-kit` (layer and subsurface surfaces, theme tokens for colours).
- **External:** read access to `/dev/input/event*` (the `input` group, or sudo). Layer shell from the compositor. It publishes `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu` with zbus.

## Develop

- `WAYLAND_DISPLAY=wayland-1 cargo run -p otto-input-overlay -- [--position POS] [/dev/input/eventN]`. The full option list is the doc comment at the top of [src/main.rs](../../components/otto-input-overlay/src/main.rs).
- With no device argument it picks the first touchpad listed in `/proc/bus/input/devices`.
- Tests: `cargo test -p otto-input-overlay` (key labels only).

## Code map

- [src/main.rs](../../components/otto-input-overlay/src/main.rs): options, evdev polling, one layer surface with the two overlays as subsurfaces.
- [src/keys.rs](../../components/otto-input-overlay/src/keys.rs): keycodes to chord labels.
- [src/indicator.rs](../../components/otto-input-overlay/src/indicator.rs): the tray applet.

## Rules

- Key labels come from the physical keycode on a US layout, not the active keymap: the overlay shows which keys were pressed.
- The layer surface covers the output and never moves. The overlays' own input regions are empty and the parent's input region tracks where they are, so clicks beside them reach the windows below. Update that region whenever an overlay moves or resizes.
