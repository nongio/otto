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
| Displays | Resolution, refresh rate and arrangement of connected monitors, and the global interface scale |
| Dock | Size, position, auto-hide, magnification, icon colorization |
| Keyboard | Layout and options, repeat rate, and the shortcut list |
| Trackpad & Mouse | Tap to click, drag lock, natural scrolling, click method, scroll and pointer speed |
| Sound | Interface sounds on or off, and which sound theme to use |
| Power | What the lid switch and the power button do |
| Lock & Login | Auto-lock timeout, and which lock screen and greeter to run |

## How it works

The compositor owns the configuration file; the app is a D-Bus client. It reads
the schema Otto publishes and sets values, and a value you change in the app is
written back to the config file. There is no file watcher, though: edit the file
by hand while the app is open and the app will not notice — reopen it to see the
change.

Nearly everything applies the moment you change it: the Dock's size and
behaviour, the keyboard layout and repeat, the touchpad and pointer options,
appearance and accent colour, the sound, power and lock settings. Only the
interface font, the display scale, the GTK theme, the display language and the
greeter need a restart, and the app badges those and only those — a badge you
can catch lying is a badge you stop reading.

## Displays

The Displays pane probes the outputs that are actually connected, so the modes
listed are the ones your monitor reports, not a guess.

**Nothing here is persisted yet.** Resolution, refresh rate, primary display, on
or off, and arrangement are all deliberately unbound: they are per-output
settings, and Otto has no display-identity scheme that survives a monitor moving
to a different port or a dock reshuffle, so inventing a wire contract keyed on
connector name now would have to be supported forever. Your changes apply to the
session and are gone at restart. To make them stick, write them under
`[displays.named.<connector>]` in the config file — see
[Display configuration](display.md).

Scale is the exception: it is bound, but it is the global `screen_scale`, not a
per-display value.

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
