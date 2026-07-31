# Display

Scaling, monitor arrangement, resolutions, virtual outputs and panel zones.

## Finding your monitors

```sh
otto --probe
```

This prints every connector Otto can see with its available resolutions and
refresh rates, then exits. Use the connector names it reports (`eDP-1`,
`HDMI-A-1`, `DP-2`, …) in the config below.

## Scaling

```toml
screen_scale = 1.0
```

The global scale factor: `1.0` for a standard-DPI display, `2.0` for HiDPI,
and fractional values in between (`1.25`, `1.5`) for the awkward middle ground.
Otto implements `wp-fractional-scale-v1`, so well-behaved clients render crisply
at fractional scales instead of being upscaled.

To change the scale of a single monitor while the session runs, put the pointer
on it and use the `ScaleUp` / `ScaleDown`
[shortcut actions](keyboard-shortcuts.md). That change is not saved — use a
display profile for a permanent setting.

X11 applications get the scale via XSETTINGS, so they size correctly under
XWayland.

## Display profiles

A profile pins a monitor's resolution, refresh rate and position. There are two
kinds: **named** (matched by connector name) and **generic** (matched by a
pattern).

### Named profiles

```toml
[displays.named."eDP-1"]
primary = true
resolution = { width = 2256, height = 1504 }
refresh_hz = 60.0
position = { x = 0, y = 0 }

[displays.named."DP-1"]
resolution = { width = 3840, height = 2160 }
refresh_hz = 144.0
position = { x = 2256, y = 0 }
```

| Field | Meaning |
|-------|---------|
| `name` | Optional friendly label |
| `primary` | Mark this monitor as primary — where the dock and top bar go |
| `resolution` | Mode to set, in pixels |
| `refresh_hz` | Refresh rate; combined with `resolution` to pick a mode |
| `position` | Where the monitor sits in the desktop layout, in logical points |

Every field is optional. Omit `resolution` and Otto picks the preferred mode.

The key is the connector name. `"winit"` is a valid key for the windowed
development backend:

```toml
[displays.named."winit"]
resolution = { width = 1280, height = 1000 }
refresh_hz = 60.0
```

### Generic profiles

Match by connector prefix instead of exact name — handy for "any HDMI monitor I
plug in":

```toml
[[displays.generic]]
match = { connector_prefix = "HDMI" }
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 1920, y = 0 }
```

Named profiles take precedence over generic ones.

## Monitor arrangement

Monitors are arranged in a **single horizontal row**. Vertical stacking and grid
arrangements are not supported.

Without a configured `position`, the first monitor sits at the origin and each
one after it is placed immediately to the right of everything already placed.

A configured `position` is honoured **as long as it does not overlap** another
monitor's area. An overlapping position is rejected and that monitor falls back
to automatic left-to-right placement — monitors are never allowed to overlap.

Positions are recomputed from scratch whenever anything changes: a hotplug, a
mode change, or waking from suspend. This keeps the layout consistent instead of
drifting.

### Hotplug

Plugging in a monitor adds it to the right of the existing row and gives it its
own set of [workspaces](workspaces.md). Unplugging removes it; windows on it
move to a remaining monitor.

The dock, top bar and dynamic island all live on the primary monitor.

## Primary monitor

The primary monitor is the first physical output brought up, unless a display
profile sets `primary = true`.

It matters because the dock, the top bar and the dynamic island are shown there
and only there.

## Virtual outputs

A virtual output is a monitor with no physical display behind it. Otto renders
it like any other screen and pushes the frames to a PipeWire stream, where any
PipeWire client can pick them up — OBS, a recorder, or the
[RDP bridge](remote-desktop.md).

```toml
[[virtual_outputs]]
name = "virtual-1"
resolution = { width = 1920, height = 1080 }
refresh_hz = 60.0
position = { x = 3840, y = 0 }   # optional
interactive = false
```

| Field | Meaning |
|-------|---------|
| `name` | Output name, used to address it from clients |
| `resolution` | Size in pixels |
| `refresh_hz` | Frame rate; defaults to 60 |
| `position` | Where it sits in the layout; follows the same overlap rule as physical monitors |
| `interactive` | Accept remote pointer and keyboard input aimed at this output |

Set `interactive = true` for a screen you intend to control remotely (RDP). Leave
it `false` for a view-only feed (recording, AirPlay).

Otto logs the PipeWire node id at startup:

```
Virtual output 'virtual-1' started (PipeWire node 42)
```

Connect to it with any PipeWire client:

```sh
gst-launch-1.0 pipewiresrc path=42 ! videoconvert ! autovideosink
```

A virtual output behaves exactly like a physical one: its own workspaces, its
own exposé, its own workspace selector. Windows can be dragged onto it.

> If a virtual output has no configured `position` it defaults to the same
> origin as the first monitor, which overlaps it. Give it an explicit position
> to the right of your real screens.

## Panels and exclusive zones

Layer-shell clients — the top bar, docks, notification daemons — can reserve
space along a screen edge so maximized windows do not slide underneath. Otto
caps how much any one client may claim:

```toml
[layer_shell]
max_top = 100       # logical points
max_bottom = 100
max_left = 50
max_right = 50
```

`0` means unlimited. These values are in logical points and are multiplied by
the scale factor internally. Raise them if you run a tall custom panel that
looks clipped.

## Screen rotation

`RotateOutput` (bind it in `[keyboard_shortcuts]`) rotates the monitor under the
pointer by 90°. Like the scale actions, this affects the live session only and
is not written to the config. There is no rotation field in display profiles yet.

## Wallpaper

Wallpaper and background colour are global rather than per-monitor. See
[Theming](theming.md).

## Night shift

Colour temperature is handled by external tools over
`zwlr-gamma-control-v1`. See [Night Shift](night-shift.md).

## Not yet supported

- Display mirroring
- Vertical or grid monitor arrangement
- Rotation and scale in display profiles
- A graphical display settings panel
- Per-monitor wallpaper
