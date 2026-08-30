# Settings

`otto-settings` edits Otto's configuration while it is running, so you can
change how the desktop behaves without hand-writing TOML and without restarting
anything you don't have to.

> **First version.** The app covers a good share of the configuration and is
> meant for daily use, but not every option in `otto_config.toml` has a control
> yet. Anything missing is still editable by hand — see
> [Configuration](configuration.md).

## Opening it

Launch **Settings** from the Dock or the launcher, or run `otto-settings`.

## What it edits

| Pane | Covers |
|------|--------|
| General | Light or dark appearance, accent colour, interface font, GTK theme, desktop background colour and image, pointer and icon themes |
| Displays | Resolution, refresh rate, scale and arrangement of connected monitors |
| Dock | Size, position, auto-hide, magnification, icon colorization |
| Keyboard | Layout and options, repeat rate, and the shortcut list |
| Trackpad & Mouse | Tap to click, drag lock, natural scrolling, click method, scroll and pointer speed |
| Sound | Output and input devices and levels |
| Power | Idle, sleep and lid behaviour |
| Lock & Login | Locking, the greeter, and fingerprint unlock |

## How it works

The compositor owns the configuration file; the app is a D-Bus client. It reads
the schema Otto publishes, sets values, and watches for changes — so a value you
edit in the file by hand shows up in the app, and a value you change in the app
is written back to the same file.

Most settings apply the moment you change them: the Dock's size and behaviour,
the touchpad and pointer options, appearance and accent colour. A few — the
keyboard layout among them — are marked as needing a restart, and the app says
so next to the control rather than pretending the change took.

## Displays

The Displays pane probes the outputs that are actually connected, so the modes
listed are the ones your monitor reports, not a guess. Arrangement, mode and
scale are saved per display set, so unplugging and reconnecting the same
monitors brings your layout back.

## Shortcuts

The Keyboard pane lists the configured shortcuts and lets you add and remove
them. It edits the same `[keyboard_shortcuts]` table the config file has, so
anything you bind here is a normal entry you can also read and edit by hand.

## From the keyboard

Every row is a `Tab` stop with a ring around it, and every control can be
operated without a pointer — `Space` or `Enter` flips a switch, presses a
button, opens a pop-up or starts editing a field; the arrows move a slider.
A pop-up opens on its current value rather than cycling, because each value
commits as soon as it is chosen. See
[Accessibility](accessibility.md#using-ottos-applications-from-the-keyboard).

Two things still need the pointer: dragging screens around the arrangement
diagram in the Displays pane, and the shortcut lines in the Keyboard pane.

## Not there yet

- Not every configuration key has a control; the file remains the complete
  surface.
- Display mirroring, which the compositor does not do yet either.
