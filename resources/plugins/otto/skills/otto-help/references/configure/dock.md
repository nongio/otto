# Dock

Size, which edge it lives on, magnification, the icon tint — and, in the file,
what is actually in it.

## Exact commands

```sh
# Size, 0.5 to 2.0
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.size d 1.25

# Which edge: "bottom", "left" or "right"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.position s "left"

# Hide until the pointer reaches the edge
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.autohide b true

# Grow the icons under the pointer, and by how much
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.magnification b true
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.genie_scale d 0.7

# Tint the icons one colour
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.colorize_icons b true
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.colorize_color s "#7aa2f7"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv dock.colorize_intensity d 0.6
```

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `dock.size` | double | live | `1.0` | 0.5 – 2.0, step 0.05 | `d 1.25` | Dock size multiplier. |
| `dock.position` | enum | live | `bottom` | `bottom`, `left`, `right` | `s "left"` | Screen edge the dock lives on. |
| `dock.autohide` | bool | live | `false` | `true`, `false` | `b true` | Hide the dock until the pointer reaches its screen edge. |
| `dock.magnification` | bool | live | `true` | `true`, `false` | `b false` | Grow the icons under the pointer. |
| `dock.genie_scale` | double | live | `0.5` | 0.0 – 1.0, step 0.05 | `d 0.7` | How much the icons under the pointer grow. |
| `dock.genie_span` | double | live | `10.0` | 0.0 – 100.0, step 5.0 | `d 25.0` | How sharply the magnification falls off with distance: higher keeps the bump tight around the pointer. |
| `dock.colorize_icons` | bool | live | `false` | `true`, `false` | `b true` | Tint dock icons with a single colour. |
| `dock.colorize_color` | string | live | `#ffffff` | free text | `s "#7aa2f7"` | Colour used to tint dock icons, as a hex string. |
| `dock.colorize_intensity` | double | live | `1.0` | 0.0 – 1.0, step 0.05 | `d 0.6` | How strongly the tint is applied. |

## In the file

```toml
[dock]
bookmarks = [
  { desktop_id = "otto-files.desktop" },
  { desktop_id = "firefox.desktop", label = "Private", exec_args = ["--private-window"] },
]
places = [{ desktop_id = "otto-trash.desktop" }]
trash_desktop_id = "otto-trash.desktop"
trash_path = "$XDG_DATA_HOME/Trash/files"
```

Pinned apps are named by desktop file id; an id with no desktop file installed
is skipped with a warning, so listing apps that may not be there is harmless.
`places` is the strip past the divider, for things that are locations rather
than applications — it holds the Trash unless told otherwise, and `[]` gives a
dock without one.

## Worth knowing

- **Side docks stack vertically** and reserve width instead of height. The
  position is also changeable from the dock handle's own right-click menu.
- **The tint is one tint.** `dock.colorize_color` and `colorize_intensity` are
  shared with the app switcher, which follows along unless
  `appswitcher.colorize_icons` is off. With `colorize_icons = false` there is
  no tint to opt out of.
- **`genie_span` is falloff, not reach**: higher keeps the magnification tight
  around the pointer.
- **The hover balloon under an icon is a label**, not a tooltip — the word
  matters when writing about it.
- Older configs copied the whole `[dock]` table into the user file, which
  silently shadows `/etc/otto/config.toml`. If a system-wide dock change does
  nothing, that is why.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/dock.md
