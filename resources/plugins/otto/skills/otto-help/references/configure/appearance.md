# Appearance

How the desktop looks: light or dark, the accent, the material behind Otto's
own panels, the wallpaper, fonts, cursors, icons and the interface language.
Everything here applies live except the four marked `restart`.

## Exact commands

```sh
# Dark or light
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv theme_scheme s "Dark"

# Accent: blue purple pink red orange yellow green mint teal cyan indigo brown gray,
# or a "#RRGGBB" colour
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv accent_color s "purple"

# Wallpaper: an absolute path. Empty string for none
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv background_image s "/home/me/Pictures/wall.jpg"

# The colour behind it, as hex
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv background_color s "#101014"

# Turn the blurred translucent material off
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv frosting b false

# Square corners
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv rounded_corners b false

# Window controls on the right, and show the zoom button
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv window_controls_side s "right"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv show_maximize_button b true

# Cursor and icons (names of directories under /usr/share/icons)
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv cursor_theme s "Adwaita"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv cursor_size i 32
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv icon_theme s "Papirus"

# Interface font, and language — both need a restart
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv font_family s "Cantarell"
busctl --user call org.otto.Settings /org/otto/Settings org.otto.Settings Set sv locales as 1 "en-GB"
```

**Apply** is `live` (now) or `restart` (at the next login).

## The settings

| Identifier | Type | Apply | Default | Allowed | Example | What it does |
|---|---|---|---|---|---|---|
| `theme_scheme` | enum | live | `Light` | `Light`, `Dark` | `s "Dark"` | Light or dark colour scheme. |
| `accent_color` | color | live | `blue` | `blue`, `purple`, `pink`, `red`, `orange`, `yellow`, `green`, `mint`, `teal`, `cyan`, `indigo`, `brown`, `gray` | `s "purple"` | A palette name, which follows the light and dark schemes, or a #RRGGBB colour. |
| `rounded_corners` | bool | live | `true` | `true`, `false` | `b false` | The dock, the top bar, window decorations and the desktop's own panels. |
| `frosting` | bool | live | `true` | `true`, `false` | `b false` | The translucent, blurred material behind the dock, the top bar, the launcher and the desktop's own panels. |
| `window_controls_side` | enum | live | `left` | `left`, `right` | `s "right"` | Which end of a window's titlebar the close, minimize and zoom controls sit at. |
| `show_maximize_button` | bool | live | `false` | `true`, `false` | `b true` | Show the zoom control in a window's titlebar. Off by default: a double click on the titlebar zooms a window either way. |
| `background_image` | string | live | `""` (empty) | free text | `s "/home/you/Pictures/wallpaper.jpg"` | Path to the desktop background image. Empty for none. |
| `background_color` | string | live | `#1a1a2e` | free text | `s "#101014"` | Desktop background colour, as a hex string. |
| `font_family` | string | restart | `Inter` | free text | `s "Cantarell"` | Font family used by Otto's own interface. |
| `cursor_theme` | string | live | `Notwaita-Black` | free text | `s "Adwaita"` | Name of the XCursor theme. |
| `cursor_size` | int | live | `24` | 16 – 96, step 8 | `i 32` | Cursor size in logical pixels. |
| `icon_theme` | string | live | `""` (empty) | free text | `s "Papirus"` | Name of the icon theme. Empty auto-detects. |
| `gtk_theme` | string | restart | `""` (empty) | free text | `s "Adwaita-dark"` | GTK theme name handed to clients. Empty auto-detects. |
| `locales` | string-list | restart | `[]` | `""`, `en-GB`, `en-US`, `de`, `es`, `fr`, `it`, `ja`, `pl`, `pt-BR`, `ru`, `uk`, `zh-CN` | `as 1 "en-GB"` | The language Otto and the applications it starts are drawn in. Empty follows the environment. |

## Worth knowing

- **The accent is a name, not a colour.** `blue`, `purple`, `pink`, `red`,
  `orange`, `yellow`, `green`, `mint`, `teal`, `cyan`, `indigo`, `brown`,
  `gray` follow the light and dark schemes and stay legible in both. A
  `#RRGGBB` or `#RGB` literal is accepted too and is taken at face value.
- **`theme_scheme` reaches other applications.** Otto publishes it through the
  Settings portal as `org.freedesktop.appearance color-scheme`, so GTK4 and
  the top bar re-colour within about a second. Unsandboxed GTK3 and Chrome read
  dconf instead and will not follow.
- **`frosting` is the translucent blurred material** behind the dock, the top
  bar, the launcher and Otto's own panels. Off gives flat opaque surfaces,
  which is also the thing to try when the desktop feels slow.
- **The wallpaper is global** — one image for every monitor and every
  workspace. `background_image` is an absolute path, scaled to cover, decoded
  at the largest monitor's resolution. `background_color` is the bottom of the
  gradient drawn when the image is missing, so it is worth setting anyway. For
  per-monitor or animated wallpaper, run a layer-shell daemon (`swaybg`,
  `swww`, `wpaperd`) from `[[exec_once]]` instead.
- **Theme names are directory names and case-sensitive.** `ls /usr/share/icons
  ~/.local/share/icons` lists what is installed. An empty `icon_theme`
  auto-detects; a name that is not there falls back to hicolor and looks
  broken rather than erroring.
- **`font_family` is Otto's own interface only** — dock labels, exposé titles,
  the app switcher. Applications keep their own fonts. It needs a restart.
- **`gtk_theme` is handed to clients**, not used by Otto itself.
- **`locales` is the language Otto and the apps it starts are drawn in.**
  Empty follows the environment. The picker offers the catalogues that are
  actually compiled in; a restart is needed either way.

## Read next

- https://github.com/nongio/otto/blob/main/docs/user/theming.md
- https://github.com/nongio/otto/blob/main/docs/user/customization.md
