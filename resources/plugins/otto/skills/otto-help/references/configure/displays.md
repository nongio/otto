# Displays

Scale, resolution, refresh rate and where each monitor sits. Only the global
scale is a setting; outputs come and go with the hardware, so they have methods
of their own instead.

## Exact commands

```sh
# What is connected: name, connector, size, refresh (in mHz), position, scale
busctl --user --json=short call org.otto.Settings /org/otto/Settings org.otto.Settings ListOutputs

# Global interface scale, 0.5 to 4.0 in steps of 0.25. Needs a restart
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv screen_scale d 1.5

# Pin one monitor: connector, width, height, refresh_hz, x, y, primary
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    SetOutputProfile suudiib eDP-1 2256 1504 60.0 0 0 true
```

**Do not change the scale on a session someone is using.** Set it and tell them
to log out and back in.

## The setting

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `screen_scale` | double | restart | `1.0` | 0.5 – 4.0, step 0.25 | `d 1.5` | Global scale factor applied to the desktop. |

## The output methods

```sh
# what is connected: name, connector, width, height, refresh (mHz), x, y, scale, virtual
busctl --user --json=short call org.otto.Settings /org/otto/Settings \
    org.otto.Settings ListOutputs

# pin a mode, a position and the primary flag for one connector
# signature: connector, width, height, refresh_hz, x, y, primary
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings \
    SetOutputProfile suudiib eDP-1 2256 1504 60.0 0 0 true
```

`SetOutputProfile` writes `[displays.named.<connector>]` and **always applies at
the next start**, never now: a modeset made under a running session cannot be
undone if the display does not come back. A zero width, height or refresh leaves
that part unset, so a monitor can be moved without naming a mode. Primary is a
choice *among* displays — turning it on for one means turning it off for the
others.

The connector is the handle, which is why display settings are not final: a
monitor moved to another port is, as far as the config knows, another monitor.

## In the file

```toml
# Matched by connector name. `otto --probe` prints the names and their modes —
# but it is a standalone tool: running it takes over the session, so do not run
# it on a desktop someone is using.
[displays.named."eDP-1"]
primary = true
resolution = { width = 2256, height = 1504 }
refresh_hz = 60.0
position = { x = 0, y = 0 }

# Matched by prefix, for "any HDMI monitor I plug in". Named profiles win.
[[displays.generic]]
match = { connector_prefix = "HDMI" }
refresh_hz = 60.0
position = { x = 1920, y = 0 }

# The most a panel may reserve per screen edge, in logical points. 0 is unlimited.
[layer_shell]
max_top = 100
max_bottom = 100
max_left = 50
max_right = 50
```

Every profile field is optional; omit `resolution` and the preferred mode is
used. `"winit"` is a valid key for the windowed development backend.

## Worth knowing

- **Monitors are one horizontal row.** No vertical stacking, no grids. Without
  a `position` each monitor is placed to the right of everything already
  placed, and a `position` that would overlap another monitor is rejected in
  favour of automatic placement. Positions are recomputed on every hotplug,
  mode change and resume.
- **The primary monitor carries the chrome** — dock, top bar, dynamic island —
  and is the first output brought up unless a profile says otherwise.
- **Scale is global.** `screen_scale` applies to the whole desktop and needs a
  restart. `ScaleUp` / `ScaleDown` bound to a key change the monitor under the
  pointer live, but are not saved. Do not change scale on a running session
  someone is using — set it and restart.
- **`RotateOutput`** rotates the monitor under the pointer by 90°, live only.
  There is no rotation field in a profile.
- **Mirroring does not exist yet**, and neither does a per-monitor wallpaper.
- The Settings app's Displays pane probes real modes but **persists nothing**
  except the scale; the file is how a display setting survives a restart.

## Read next

- https://nongio.github.io/otto/display/
- [virtual-outputs.md](virtual-outputs.md) — screens with no display behind them
