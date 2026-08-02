# Dock

The dock is Otto's task manager, pinned to the bottom edge of the primary
monitor. Unlike the top bar and the dynamic island, it is part of the
compositor — there is nothing to start.

## What it shows

Three groups, left to right:

1. **Bookmarks** — launchers you pinned in the config.
2. **Running applications** — one icon each, with a dot underneath.
3. **Minimized windows** — a thumbnail per minimized window, in a drawer that
   opens when the first one arrives.

Icons and names come from the application's `.desktop` entry, loaded in the
background so a slow icon theme never stalls the compositor.

## Interacting

| Action | Effect |
|--------|--------|
| Hover an icon | Balloon tooltip with the app's name; icons magnify |
| Click a running app | Raise and focus it |
| Click it again | Cycle to that app's next window |
| Click a bookmark | Focus the running instance, or launch it |
| Click a minimized window | Restore it with the genie animation |

While a launch is in flight the icon **bounces**, so you know the click landed
before the window shows up.

The dock takes pointer priority over windows underneath it, so clicks near the
bottom edge always reach the dock rather than the window behind.

## Bookmarks

```toml
[dock]
bookmarks = [
  { desktop_id = "org.gnome.Nautilus.desktop" },
  { desktop_id = "org.mozilla.firefox.desktop", label = "Web", exec_args = ["--private-window"] },
  { desktop_id = "org.gnome.Terminal.desktop" },
]
```

| Field | Meaning |
|-------|---------|
| `desktop_id` | The `.desktop` file name. Required. |
| `label` | Override the tooltip text. Optional. |
| `exec_args` | Extra arguments appended to the entry's `Exec` line. Optional. |

Find desktop ids with `ls /usr/share/applications ~/.local/share/applications`.

Bookmarks behave exactly like running apps once launched — same icon, same
hover, same window cycling.

There is no way to pin an app by dragging it into the dock yet; bookmarks are
config-only.

## Appearance and behaviour

```toml
[dock]
size = 1.0             # overall size multiplier, 0.5 – 2.0
magnification = true   # macOS-style icon magnification on hover
autohide = false       # hide when the pointer leaves
genie_scale = 0.5      # shape of the minimize animation
genie_span = 10.0      # and how far it reaches
```

### Size

`size` scales the whole dock. `0.5` is half-height, `2.0` is double. Icon sizes,
padding and the bar height all follow. It is read once at startup, so a change
needs a session restart.

Only `autohide`, `magnification` and `bookmarks` are ever written back by the
dock itself; everything else in `[dock]` stays exactly as you typed it. Older
builds rewrote the whole table instead, leaving a copy of every value in
`~/.config/otto/config.toml` — which then shadowed `/etc/otto/config.toml` and
made editing the system config look like it did nothing. Otto drops those
leftovers from the user config on the next start (values you have actually
changed are kept, and the file is rewritten without its comments).

### Magnification

Icons grow as the pointer approaches, with a Gaussian falloff — the icon under
the pointer is largest and neighbours taper off. `genie_scale` and `genie_span`
control the intensity and reach of the curve.

Set `magnification = false` for a flat dock with no hover scaling.

### Auto-hide

With `autohide = true` the dock slides away when the pointer leaves and comes
back when the pointer enters the hot zone along the bottom edge of the screen.

### Icon colorization

```toml
[dock]
colorize_icons = true
colorize_color = "#ffffff"
colorize_intensity = 1.0
```

Tints every dock icon toward a single colour — a monochrome dock. `intensity`
blends between the original icon (`0.0`) and the flat tint (`1.0`).

## Visuals

The dock bar uses background blur and picks its colours from the active theme,
so it follows your light/dark setting. See [Theming](theming.md).

It hides automatically when a window goes fullscreen, and when exposé opens.

## Multi-monitor

The dock lives on the **primary monitor only**. A per-monitor dock is not
implemented.

The primary monitor is the first physical output brought up, or whichever
display profile sets `primary = true` — see [Display](display.md).

## Not yet supported

- Pinning by drag-and-drop
- Right-click context menus on dock icons
- Bookmarked folders and locations
- Moving the dock to another screen edge
- A per-monitor dock
