# Dock

The dock is Otto's task manager, pinned to one edge of the primary monitor —
the bottom by default, or either side. Unlike the top bar and the dynamic
island, it is part of the compositor — there is nothing to start.

## What it shows

Three groups, in order along the dock (left to right, or top to bottom on a
side dock):

1. **Bookmarks** — launchers you pinned in the config.
2. **Running applications** — one icon each, with a dot beside it on the
   screen-edge side.
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
| Right-click an icon | Context menu: Open, Keep in Dock, Quit |
| Drag an icon along the dock | Reorder it; neighbours move out of the way |

While a launch is in flight the icon **bounces**, so you know the click landed
before the window shows up.

The dock takes pointer priority over windows underneath it, so clicks near its
screen edge always reach the dock rather than the window behind.

## Bookmarks

```toml
[dock]
bookmarks = [
  { desktop_id = "otto-files.desktop" },
  { desktop_id = "firefox.desktop", label = "Private", exec_args = ["--private-window"] },
  { desktop_id = "otto-settings.desktop" },
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

### Pinning and reordering without the config file

Right-click any icon — a bookmark or a running app that is not yet in the
dock — and pick **Keep in Dock** to pin it; the same entry, ticked, unpins it
again. The menu also offers **Open** (for an app that is not running) and
**Quit** (for one that is).

Drag an icon along the dock to reorder it. The icons it passes shuffle aside so
the order you are about to commit is visible before you let go, and a press that
does not move still launches or focuses the app.

Both are written back to `bookmarks` in your config, so the dock comes back the
same way next login.

## Appearance and behaviour

```toml
[dock]
size = 1.0             # overall size multiplier, 0.5 – 2.0
position = "bottom"    # "bottom", "left" or "right"
magnification = true   # macOS-style icon magnification on hover
autohide = false       # hide when the pointer leaves
genie_scale = 0.5      # how much icons magnify under the pointer
genie_span = 10.0      # falloff: larger = tighter around the pointer
```

### Position

`position` picks the screen edge the dock is docked to: `"bottom"` (the
default), `"left"` or `"right"`. A side dock stacks its icons vertically,
reserves screen *width* instead of height, magnifies along the pointer's
vertical travel, shows its tooltips beside the icons, and minimizes windows
sideways into itself.

You can also change it while Otto runs: right-click the dock handle (the grip
between the apps and the minimized windows) and pick Bottom, Left or Right. The
choice is written back to the config.

### Size

`size` scales the whole dock. `0.5` is half-height, `2.0` is double. Icon sizes,
padding and the bar height all follow. It takes effect immediately, whether it
is changed from Settings, by dragging the dock handle, or in the config file.

Only `size`, `position`, `autohide`, `magnification` and `bookmarks` are ever
written back by the dock itself; everything else in `[dock]` stays exactly as
you typed it. Older
builds rewrote the whole table instead, leaving a copy of every value in
`~/.config/otto/config.toml` — which then shadowed `/etc/otto/config.toml` and
made editing the system config look like it did nothing. Otto never edits that file for
you: at startup it logs a warning naming the leftover keys, and you delete the
lines you did not write. It cannot tell a copied `colorize_intensity = 1.0`
from one you typed, and guessing wrong would silently turn a setting off.

### Magnification

Icons grow as the pointer approaches, with a Gaussian falloff — the icon under
the pointer is largest and neighbours taper off. `genie_scale` is how much the icon
under the pointer grows; `genie_span` is how sharply the curve falls off, so a
*larger* value keeps the bump tighter around the pointer and a smaller one
spreads it over more neighbours. Both apply live.

Set `magnification = false` for a flat dock with no hover scaling.

### Auto-hide

With `autohide = true` the dock slides away when the pointer leaves and comes
back when the pointer enters the hot zone along the screen edge it lives on.

### Icon colorization

```toml
[dock]
colorize_icons = true
colorize_color = "#ffffff"
colorize_intensity = 1.0
```

Tints every app icon toward a single colour — a monochrome dock. `intensity`
blends between the original icon (`0.0`) and the flat tint (`1.0`). The tint
follows the icons wherever they appear, so the app switcher matches the dock.
All three keys apply live, so dragging the tint strength in Settings repaints
the icons as you drag.

If you want the tint on the dock but not on the cmd-tab panel, opt the switcher
out:

```toml
[appswitcher]
colorize_icons = false
```

That is only a yes-or-no for the switcher: the colour and strength stay in
`[dock]`, since one desktop has one icon tint. With `dock.colorize_icons =
false` there is no tint anywhere, so the key does nothing. The dock's drag
ghost is part of the dock and follows the dock's setting. This key applies live
too.

## Badges and progress

An icon can carry a small **badge** in its corner — a count drawn over the
icon — and a **progress bar** across its foot.

Badges are how unread notifications show up: `otto-islands` is the session's
notification daemon, so it is the only thing that knows how many notifications
an application still has outstanding, and it publishes that count to the dock.
The badge is attached to the sending application's icon even when that app is
not in the dock yet. Notifications that arrive without a desktop-entry hint
(one forwarded from a terminal escape sequence, for instance) are attributed by
resolving the sending process to its desktop entry.

Dismissing the notifications clears the badge. Both the badge and the progress
bar scale with the icon, so they follow magnification.

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

- Bookmarked folders and locations
- Dragging an application into the dock from outside it (the Files window, a
  launcher result) — pinning is the context menu's job
- A per-monitor dock
