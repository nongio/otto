# Recommended Settings

Otto ships with defaults that try to be uncontroversial. This page is something
else: the settings Otto's author actually runs, day to day, on a laptop. They
are a starting point rather than a second set of defaults — copy the whole
thing, or take the parts you like.

Most of the page is about the **keyboard**, because a Mac-like layout is the
one part of Otto that is genuinely fiddly to reproduce from scratch. If that is
what you came for, skip to [A Mac-like keyboard](#a-mac-like-keyboard).

Everything here goes in `~/.config/otto/config.toml`. See
[Configuration](configuration.md) for how that file merges with the system one.

## A Mac-like keyboard

The goal is the muscle memory: `Cmd+C` copies, `Cmd+W` closes the window,
`Cmd+Space` opens the launcher, `Cmd+Shift+4` takes a screenshot — while the
real `Ctrl` key stays free for the terminal, so `^C` and `^W` still do what a
shell expects.

Three separate things have to line up for that, and each one fails quietly on
its own.

### 1. Map Cmd onto Ctrl at the layout level

```toml
[input]
xkb_options = ["altwin:ctrl_win"]
```

This is what makes `Cmd+C` arrive at applications as the `Ctrl+C` they
understand. It is an XKB option, so it happens below Otto: every client sees
Ctrl, and no application needs to know anything.

### 2. Tell Otto to match shortcuts on Cmd only

```toml
[input]
mac_style_modifiers = true
```

Without this, step 1 has an ugly side effect: Cmd and the real Ctrl key now
produce the *same* event, so a `Ctrl+W` binding fires from both — and pressing
`^W` to delete a word in a terminal closes the window instead.

With it, Otto looks at the physical keycode behind the modifier and matches its
own shortcuts on Cmd alone, leaving the real Ctrl key to the focused
application. Bindings are still *written* as `Ctrl+…`; they simply follow the
Cmd key. This is covered in more detail under
[Mac-style modifiers](input.md#mac-style-modifiers).

The setting defaults to following `xkb_options`, so step 1 alone usually
implies it — set it explicitly anyway, so the config says what it means.

### 3. Bind the shifted keysym, not the digit

This is the one that wastes an afternoon.

Otto matches shortcuts on the **shifted** keysym — the character your layout
actually produces once Shift is down. On a US layout `Shift+3` is `numbersign`,
not `3`, so a binding written `"Ctrl+Shift+3"` never fires. Nothing errors; the
binding is simply never reached.

The fix is to bind both forms. It costs a duplicate line and it works on every
layout, including ones where the digits sit behind different symbols:

```toml
# Cmd+Shift+3 — whole screen
"Ctrl+Shift+3".run          = { cmd = "~/.local/bin/shot", args = ["full"] }
"Ctrl+Shift+numbersign".run = { cmd = "~/.local/bin/shot", args = ["full"] }

# Cmd+Shift+4 — select a region
"Ctrl+Shift+4".run      = { cmd = "~/.local/bin/shot", args = [] }
"Ctrl+Shift+dollar".run = { cmd = "~/.local/bin/shot", args = [] }

# Cmd+Shift+5 — region, to the clipboard
"Ctrl+Shift+5".run       = { cmd = "~/.local/bin/shot", args = ["copy"] }
"Ctrl+Shift+percent".run = { cmd = "~/.local/bin/shot", args = ["copy"] }
```

If you are on a non-US layout, run `scripts/show-keys.sh` and press the
combination — it prints the keysym Otto will see, which is the name to bind.

### The full keyboard block

```toml
[input]
xkb_options = ["altwin:ctrl_win"]
mac_style_modifiers = true

[keyboard_shortcuts]
# Windows
"Ctrl+w"           = "CloseWindow"
"Ctrl+ArrowUp"     = "ToggleMaximizeWindow"
"Ctrl+ArrowLeft"   = "TileWindowLeft"
"Ctrl+ArrowRight"  = "TileWindowRight"

# Switching
"Ctrl+Tab"                 = "ApplicationSwitchNext"
"Ctrl+Shift+ISO_Left_Tab"  = "ApplicationSwitchPrev"
"Ctrl+grave"               = "ApplicationSwitchNextWindow"
"Ctrl+Q"                   = "ApplicationSwitchQuit"

# Exposé, on the page keys
Prior = "ExposeShowAll"
Next  = "ExposeShowDesktop"

# Workspaces
"Ctrl+1" = { builtin = "Workspace", index = 0 }
"Ctrl+2" = { builtin = "Workspace", index = 1 }
"Ctrl+3" = { builtin = "Workspace", index = 2 }
"Ctrl+4" = { builtin = "Workspace", index = 3 }

# Launcher
"Ctrl+Space".run   = { cmd = "otto-launcher", args = [] }
"Ctrl+Shift+P".run = { cmd = "otto-launcher", args = ["--windows"] }

# Media and brightness keys, straight through
XF86AudioRaiseVolume = "VolumeUp"
XF86AudioLowerVolume = "VolumeDown"
XF86AudioMute        = "VolumeMute"
XF86AudioPlay        = "MediaPlayPause"
XF86AudioNext        = "MediaNext"
XF86AudioPrev        = "MediaPrev"
XF86MonBrightnessUp   = "BrightnessUp"
XF86MonBrightnessDown = "BrightnessDown"
```

Read with the two `[input]` lines above, every `Ctrl+…` in that table is a
`Cmd+…` under your fingers.

Two things deliberately *not* bound: `Cmd+C`, `Cmd+V` and `Cmd+X` have no
entries, because step 1 already delivers them to applications as copy, paste
and cut — binding them in Otto would take them away. And `Ctrl+Esc` is left as
the shipped `Quit`; on a Mac-style layout that is `Cmd+Esc`, which is hard to
press by accident.

### Escape hatches

Two keys work regardless of your config, so a broken keyboard section can never
lock you out: `Ctrl+Alt+Backspace` (which becomes `Cmd+Alt+Backspace` here)
quits the compositor, and `Ctrl+Alt+F1`–`F12` switch VTs. VT switching is read
from raw keycodes, so it works from either Ctrl key.

## Key repeat

```toml
keyboard_repeat_delay = 395    # ms before repeat starts, default 300
keyboard_repeat_rate  = 30     # repeats per second, default 30
```

These are top-level keys, not inside `[input]`. The default 300 ms delay starts
repeating while you are still holding a key deliberately — around 400 ms is
enough to stop `jj` in an editor turning into a run of `j`s, without feeling
sluggish.

## Trackpad and pointer

```toml
[input]
scroll_speed = 0.25                        # default 1.0
tap_enabled = true
tap_drag_enabled = true
touchpad_click_method = "clickfinger"
touchpad_dwt_enabled = true
touchpad_natural_scroll_enabled = true
pointer_accel_speed = 0.0                  # -1.0 slowest, 1.0 fastest
```

`scroll_speed` is the one worth changing. The default `1.0` passes scroll events
through untouched, which on a high-resolution trackpad sends a page flying past
on a short two-finger swipe. `0.25` is what makes a Wayland trackpad feel like a
Mac one; if you mostly use a mouse wheel, leave it nearer `1.0` — the multiplier
applies to both.

The rest are already the shipped defaults, listed so you can see the set that
goes together: tap to click, click-by-finger-count, no cursor jump while typing,
natural scrolling.

## Display and appearance

```toml
screen_scale = 2.0
theme_scheme = "Light"
accent_color = "orange"
font_family = "Inter"
cursor_size = 32
background_image = "/path/to/your/wallpaper.jpg"

[audio]
sound_enabled = true
sound_theme = "Pop"

[dock]
position = "left"
size = 0.95
magnification = true
autohide = false
```

`screen_scale = 2.0` suits a HiDPI laptop panel; on a 1080p external screen
`1.0` or `1.5` is the sane choice — and if you have both,
[Display](display.md) covers per-output scaling, which is what you actually
want.

A dock on the left keeps the full width of a laptop screen for windows.
`magnification` is the macOS-style zoom under the cursor.

## Where these differ from the shipped defaults

Everything else on this page matches what Otto already ships. These are the
real departures:

| Setting | Default | Recommended | Why |
|---------|---------|-------------|-----|
| `scroll_speed` | `1.0` | `0.25` | Untouched scroll events overshoot badly on a trackpad |
| `keyboard_repeat_delay` | `300` | `395` | 300 ms starts repeating during a deliberate hold |
| `input.xkb_options` | unset | `["altwin:ctrl_win"]` | The Cmd key, and the reason for most of this page |
| `input.mac_style_modifiers` | follows the layout | `true` | Say it explicitly rather than inferring it |
| `dock.position` | bottom | `"left"` | Screen height is scarcer than width on a laptop |

## See also

- [Configuration](configuration.md) — where config files live and how they merge
- [Keyboard Shortcuts](keyboard-shortcuts.md) — binding syntax and the complete action list
- [Input](input.md) — every keyboard, touchpad and pointer option in full
- [Display](display.md) — per-output scaling and monitor arrangement
- [Theming](theming.md) — schemes, accent colors, fonts, wallpaper, icons
