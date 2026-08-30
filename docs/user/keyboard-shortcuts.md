# Keyboard Shortcuts

Every shortcut in Otto is defined in your config file, under
`[keyboard_shortcuts]`. There are no built-in bindings apart from two escape
hatches (see [Always-on keys](#always-on-keys) below) — if your config has no
`[keyboard_shortcuts]` table, Otto starts with no shortcuts at all.

## Default config

The shipped `/etc/otto/config.toml` (a copy of `otto_config.example.toml`)
defines a full set. Those are the "defaults" referred to throughout this guide.

## Binding syntax

Each entry maps a **trigger** on the left to an **action** on the right:

```toml
[keyboard_shortcuts]
"Ctrl+Esc"          = "Quit"
"Ctrl+Return"       = { run = { cmd = "terminator", args = [] } }
"Logo+B"            = { open_default = "browser" }
"Ctrl+1"            = { builtin = "Workspace", index = 0 }
```

### Triggers

A trigger is zero or more modifiers followed by a key, joined by `+`:

```
Ctrl+Shift+Q
Logo+Space
XF86AudioMute
```

**Modifiers** (case-insensitive, with aliases):

| Modifier | Also accepted as |
|----------|------------------|
| `Ctrl` | `Control`, `Primary` |
| `Alt` | — |
| `Shift` | — |
| `Logo` | `Super`, `Meta`, `Win`, `Command` |

**Keys** are XKB keysym names. Letters are case-insensitive (`W` and `w` bind
the same physical key — use `Shift+W` if you mean shifted). Otto also accepts a
few friendly aliases for names people actually write:

| You can write | XKB name |
|---------------|----------|
| `Esc` | `Escape` |
| `ArrowUp` / `ArrowDown` / `ArrowLeft` / `ArrowRight` | `Up` / `Down` / `Left` / `Right` |

Everything else uses the real keysym name: `Return`, `space`, `Tab`, `Prior`
(Page Up), `Next` (Page Down), `grave` (`` ` ``), `ISO_Left_Tab` (Shift+Tab),
`XF86AudioRaiseVolume`, and so on. `xkbcli list` and
`man xkeyboard-config` are the references; the repo also ships
`scripts/show-keys.sh` to print the keysym for whatever you press.

A trigger Otto cannot parse is **skipped with a warning in the log**, not an
error — so a typo silently costs you that one binding. If a shortcut "does
nothing", check the log first.

Two triggers that resolve to the same key combination collide; the later one
(alphabetically, since the table is sorted) wins, and a warning is logged.

### Action forms

There are four ways to write an action:

**1. A built-in, by name:**
```toml
"Ctrl+Tab" = "ApplicationSwitchNext"
```

**2. A built-in that takes an index:**
```toml
"Ctrl+1" = { builtin = "Workspace", index = 0 }
"Ctrl+F2" = { builtin = "Screen", index = 1 }
```

**3. Run a command:**
```toml
"Ctrl+Return" = { run = { cmd = "alacritty", args = [] } }
"Logo+Shift+B" = { run = { cmd = "firefox", args = ["--private-window"] } }
```
The command is spawned fire-and-forget. It is not run through a shell, so
pipes, globs and `&&` do not work — call `sh -c` explicitly if you need them.

**4. Open the user's default application for a role:**
```toml
"Logo+B"     = { open_default = "browser" }
"Logo+Space" = { open_default = "file_manager" }
"Logo+T"     = { open_default = { role = "terminal", fallback = "alacritty" } }
```

This resolves through your `mimeapps.list` defaults, so it launches whatever
*you* have set as the handler rather than a hard-coded program. Recognised
roles:

| Role | Resolved via |
|------|--------------|
| `browser` | `x-scheme-handler/https`, then `http`, then `text/html` |
| `file_manager` / `files` | `inode/directory` |
| `terminal` / `shell` | `x-scheme-handler/terminal`, then `application/x-terminal` |
| anything containing `/` | used directly as a MIME type |
| anything else | `x-scheme-handler/<role>` |

You can also give a desktop file id directly (`open_default = "firefox.desktop"`).
The optional `fallback` is used when the role resolves to nothing; it may be a
desktop id or a plain command line.

## Built-in actions

### Session

| Action | Effect |
|--------|--------|
| `Quit` | Exit Otto, ending the session |
| `LockSession` | Launch the configured locker — see [Lock Screen](lock-screen.md) |

### Windows

| Action | Effect |
|--------|--------|
| `CloseWindow` | Ask the focused window to close |
| `ToggleMaximizeWindow` | Maximize / restore the focused window (animated) |
| `TileWindowLeft` | Snap the focused window to the left half of its monitor |
| `TileWindowRight` | Snap the focused window to the right half |
| `ToggleDecorations` | Flip every window between client-side and server-side decoration mode |

### Applications

| Action | Effect |
|--------|--------|
| `ApplicationSwitchNext` | Open the app switcher / move forward |
| `ApplicationSwitchPrev` | Move backward in the app switcher |
| `ApplicationSwitchNextWindow` | Cycle windows within the highlighted app |
| `ApplicationSwitchQuit` | Quit the highlighted app |

The switcher stays up as long as you hold the modifier that opened it, and
commits when you release it. See [Exposé & App Switcher](expose-and-switcher.md).

### Workspaces and overview

| Action | Effect |
|--------|--------|
| `Workspace` (needs `index`) | Switch to workspace *N*, zero-based |
| `ExposeShowAll` | Toggle the exposé grid of all windows |
| `ExposeShowDesktop` | Push all windows aside to reveal the desktop |

### Displays

| Action | Effect |
|--------|--------|
| `Screen` (needs `index`) | Warp the pointer to the center of monitor *N*, zero-based |
| `ScaleUp` | Increase the scale factor of the monitor under the pointer |
| `ScaleDown` | Decrease it |
| `RotateOutput` | Rotate the monitor under the pointer by 90° |

`ScaleUp`, `ScaleDown` and `RotateOutput` change the live session only; they do
not write anything back to the config. Use [display profiles](display.md) to
make a scale or rotation stick.

### Hardware keys

| Action | Effect |
|--------|--------|
| `BrightnessUp` / `BrightnessDown` | Screen backlight |
| `VolumeUp` / `VolumeDown` / `VolumeMute` | Audio volume |
| `MediaPlayPause` / `MediaNext` / `MediaPrev` / `MediaStop` | Media player control (MPRIS) |

Volume and brightness changes surface in the [dynamic island](dynamic-island.md)
and can play a feedback sound — see [Audio](audio.md).

### Debugging

| Action | Effect |
|--------|--------|
| `SceneSnapshot` (alias `ExportSceneJson`) | Dump the scene graph to `scene.json` in the working directory |
| `SkpSnapshot` (alias `ExportSceneSkp`) | Dump the focused monitor's render tree as a Skia `.skp` picture |

Useful when reporting a rendering bug.

## Always-on keys

These are handled before the config is consulted and cannot be rebound or
disabled. They exist so a locked, grabbed or misconfigured session is never a
dead end.

| Keys | Effect |
|------|--------|
| `Logo+Q` | Quit Otto immediately |
| `Ctrl+Alt+Backspace` | Quit Otto immediately |
| `Ctrl+Alt+F1`…`F12` | Switch virtual terminal — works even while locked |
| `Ctrl+Alt+Escape` | Lock the session — works whatever holds the keyboard |
| Power button | Runs `power_management.on_power_button` — see [Power Management](power-management.md) |

`Ctrl+Alt+Escape` and the power button are read from raw hardware key codes, so
they work regardless of your keyboard layout and regardless of which client has
grabbed the keyboard — including a fullscreen game or a lock screen.

> If you are testing something over a remote session, note that `Logo+Q` and
> `Ctrl+Alt+Backspace` will kill the compositor instantly. Avoid them.

While the session is locked, **all** configured shortcuts are inactive: the
locker owns the keyboard. Only the always-on keys above still work.

## The shipped default set

For reference, this is what `/etc/otto/config.toml` binds:

```toml
[keyboard_shortcuts]
"Ctrl+Return"               = { open_default = { role = "terminal", fallback = "xdg-terminal-exec" } }
"Ctrl+Space"                = { run = { cmd = "otto-launcher", args = [] } }
"Ctrl+Shift+P"              = { run = { cmd = "otto-launcher", args = ["--windows"] } }

"Ctrl+1"                    = { builtin = "Workspace", index = 0 }
"Ctrl+2"                    = { builtin = "Workspace", index = 1 }
"Ctrl+3"                    = { builtin = "Workspace", index = 2 }
"Ctrl+4"                    = { builtin = "Workspace", index = 3 }

"Ctrl+Tab"                  = "ApplicationSwitchNext"
"Ctrl+Shift+ISO_Left_Tab"   = "ApplicationSwitchPrev"
"Ctrl+grave"                = "ApplicationSwitchNextWindow"
"Ctrl+q"                    = "ApplicationSwitchQuit"

"Ctrl+ArrowUp"              = "ToggleMaximizeWindow"
"Ctrl+ArrowLeft"            = "TileWindowLeft"
"Ctrl+ArrowRight"           = "TileWindowRight"

"Prior"                     = "ExposeShowAll"       # Page Up
"Next"                      = "ExposeShowDesktop"   # Page Down

"XF86MonBrightnessUp"       = "BrightnessUp"
"XF86MonBrightnessDown"     = "BrightnessDown"
"XF86AudioRaiseVolume"      = "VolumeUp"
"XF86AudioLowerVolume"      = "VolumeDown"
"XF86AudioMute"             = "VolumeMute"
"XF86AudioPlay"             = "MediaPlayPause"
"XF86AudioNext"             = "MediaNext"
"XF86AudioPrev"             = "MediaPrev"
"XF86AudioStop"             = "MediaStop"
```

Quitting the session is not in the table: `Logo+Q` and `Ctrl+Alt+Backspace`
are always on (see above). Note that `Ctrl+Q` quits the *highlighted app in the
switcher*, which is easy to hit by accident; rebinding it is a reasonable first
customization.

## Shortcuts and applications

Clients can request a keyboard-shortcuts inhibitor (via
`keyboard-shortcuts-inhibit`), which suspends Otto's bindings while that window
is focused. Remote-desktop clients, virtual machines and terminal multiplexers
use this so that, for example, `Ctrl+Tab` reaches the guest instead of Otto.
The always-on keys above are never inhibited.

## Inside Otto's own applications

Settings, Files and the launcher can be driven entirely from the keyboard —
`Tab` between controls, `Space` or `Enter` to operate one, arrows for sliders
and lists, `Esc` to close a pop-up. None of it is configurable here: these keys
belong to the application, not to the compositor, and they are the same in
every Otto application. The table is in
[Accessibility](accessibility.md#using-ottos-applications-from-the-keyboard),
which is also where to look if you use a screen reader.

## See also

- [Touchpad Gestures](gestures.md) — the pointer-driven equivalents
- [Input](input.md) — keyboard layout, XKB options, repeat rate
- [Configuration](configuration.md) — where config files live
