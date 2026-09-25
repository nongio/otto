# Settings

`otto-settings` edits Otto's configuration while it is running, so you can
change how the desktop behaves without hand-writing TOML and without restarting
anything you don't have to.

> **First version.** The app covers a good share of the configuration and is
> meant for daily use, but not every option in `otto_config.toml` has a control
> yet. Anything missing is still editable by hand. See
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
| Agents | The default agent for Ask, and each agent's name, harness, command, permissions, model, folder and colour |

## How it works

The compositor owns the configuration file; the app is a D-Bus client. It reads
the schema Otto publishes and sets values, and a value you change in the app is
written back to the config file. There is no file watcher, though: edit the file
by hand while the app is open and the app will not notice. Reopen it to see the
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
`[displays.named.<connector>]` in the config file. See
[Display configuration](display.md).

Scale is the exception: it is bound, but it is the global `screen_scale`, not a
per-display value.

## Shortcuts

The Keyboard pane lists the configured shortcuts and lets you add and remove
them. It edits the same `[keyboard_shortcuts]` table the config file has, so
anything you bind here is a normal entry you can also read and edit by hand.

## Agents

The Agents pane edits `~/.config/otto/agents.toml`, the file behind Ask. The
agent service only reads it when it starts, so changes here wait: press
**Apply** to save them and restart the service, or **Revert** to drop them.
Both stay greyed out until there is something to save. The **Agent service**
row at the top says whether the service is running, with **Start** or
**Restart**.
Apply also runs `otto-agents plugins install`, which gives every harness
other than Claude Code its own copy of the instructions. Hermes is the one it
can't finish: it needs a profile for each set of instructions, and the pane
tells you the `hermes profile create` line to run.
Only the keys you changed are written, so your comments and anything the pane
doesn't show stay put. An empty model means the agent's own default, and an empty folder means Ask
starts its sessions in a scratch folder, `~/.local/state/otto/ask`. **New agent** adds
one on Claude Code, and each agent's **Rename** and **Remove** buttons do what
they say; nothing is written until you apply.

Picking another **Harness** (Claude Code, Codex, OpenCode, Hermes or pi) sets
the agent up for it: the command, how it finds Otto's instructions, and how
`Ctrl+O` continues a session in a terminal. It also clears the model, since
each harness names models differently.
The **Command** field changes the command alone.

**Instructions** picks who the agent is. **Default** runs the harness as
itself (Claude Code as plain Claude Code), **otto** is Otto's own, and below
that are the agent files you've added under
`~/.local/share/otto/plugins/<plugin>/agents/` (see
[Add an agent of your own](agents.md#add-an-agent-of-your-own)). **Open** next
to **Instructions file** opens the one in use, and **Open** next to
**Configuration file** opens `agents.toml` itself.
See [Ask and Agents](agents.md) for what each setting does.

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
