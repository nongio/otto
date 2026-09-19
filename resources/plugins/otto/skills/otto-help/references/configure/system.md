# Sound, the app switcher, and the rest

The settings that do not need a page of their own: interface sounds, the app
switcher, and the file-only keys for workspaces and accessibility.

Elsewhere: the lid and the power button are in
[lid-and-power.md](lid-and-power.md); starting programs with the session is in
[autostart-commands.md](autostart-commands.md) and
[autostart-xdg.md](autostart-xdg.md).

## Exact commands

```sh
# Interface sounds on or off
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv audio.sound_enabled b false

# Which XDG sound theme to use; empty auto-detects
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv audio.sound_theme s "freedesktop"

# Show the app switcher on the output the pointer is on
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv appswitcher.follow_cursor b false

# Let the dock's icon tint reach the app switcher
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv appswitcher.colorize_icons b false
```

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `audio.sound_enabled` | bool | live | `true` | `true`, `false` | `b false` | Play sound feedback for interface events. |
| `audio.sound_theme` | string | live | `""` (empty) | free text | `s "freedesktop"` | XDG sound theme name. Empty auto-detects. |
| `appswitcher.follow_cursor` | bool | live | `true` | `true`, `false` | `b false` | Show the app switcher on the output the pointer is on. |
| `appswitcher.colorize_icons` | bool | live | `true` | `true`, `false` | `b false` | Let the dock's icon tint reach the app switcher. Does nothing while the dock's tint is off. |

## Sound

`audio.sound_theme` is an XDG sound theme name — `freedesktop`, `Pop`, `ocean`
— loaded from `/usr/share/sounds/<theme>/stereo/`. Empty auto-detects. Custom
sounds in Otto's `resources/` take precedence over the theme's.

## File-only keys

```toml
[workspaces]
switch_duration = 0.6     # seconds to scroll from one workspace to the next
switch_bounce = 0.1       # how much that overshoots before it settles

# Written by Otto itself: a workspace renamed in the selector lands here.
# The key is "<output>:<position>", counting from 0.
[workspaces.entries."eDP-1:0"]
name = "Mail"
tiling = true

[accessibility]
enabled = true            # own org.freedesktop.a11y.Manager, for a screen reader
```

A screen reader's keybindings do not work at all without `[accessibility]
enabled`, which also needs at-spi2-core installed. A nested Otto leaves the bus
name to the compositor hosting it whatever this says.

## Other files

- `~/.config/otto/otto-bar.toml` — the top bar. `clock_format` is its only
  option, a `chrono` strftime string. The first file that exists wins, with no
  merging: `/etc/otto/otto-bar.toml`, then the user's, then `./otto-bar.toml`.
- `~/.config/otto/agents.toml` — which coding agents the launcher's Ask mode
  can reach.
- Colour temperature at night is an external tool over
  `zwlr-gamma-control-v1` — `wlsunset` or `gammastep` from `[[exec_once]]`.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/audio.md
- https://github.com/nongio/otto/blob/main/docs/user/night-shift.md
- https://github.com/nongio/otto/blob/main/docs/user/topbar.md
