# Theming

Otto's look is set entirely from the config file. There is no theme picker GUI.

## Light and dark

```toml
theme_scheme = "Light"   # or "Dark"
```

This drives the compositor's own chrome — the dock, app switcher, exposé,
workspace selector — and is also **exported to applications** through the XDG
Desktop Portal Settings interface as `org.freedesktop.appearance color-scheme`.

That means GTK apps, Firefox, Chromium, Electron apps and the
[top bar](topbar.md) all follow this one setting, and switch within about a
second when you change it. No restart needed for clients that listen for the
change signal.

For this to work, the Otto portal backend must be selected for the Settings
interface — see [Screen Sharing](screen-sharing.md#portal-setup), which covers
the same `portals.conf`.

```toml
# gtk_theme = "Adwaita"
```

`gtk_theme` is recorded for reference only; Otto does not apply it. Set your GTK
theme the usual way (`gsettings`, `~/.config/gtk-3.0/settings.ini`).

## Accent colour

```toml
accent_color = "blue"
```

The accent tints selection borders in the workspace selector, the exposé window
highlight, and the controls in Otto's own apps — toggles, sliders, focus rings
and selected rows. Applications outside Otto can follow it too: it is published
as `accent-color` in the `org.freedesktop.appearance` portal namespace.

Changing it takes effect immediately; no restart is needed. Available values:

`red`, `orange`, `yellow`, `green`, `mint`, `teal`, `cyan`, `blue`, `indigo`,
`purple`, `pink`, `gray`, `brown`

Arbitrary hex colours are not accepted here — pick from the list.

## Wallpaper

```toml
background_image = "/usr/share/otto/background.jpg"
background_color = "#2c2ca0"
```

`background_image` is an absolute path to the image shown on the desktop.

`background_color` is the fallback: when the image is missing or fails to load,
Otto draws a gradient using this as the bottom colour. It is worth setting to
something you like even if you use an image.

The wallpaper is global — the same on every monitor and every workspace.

Otto also supports the `wlr-layer-shell` background layer, so wallpaper daemons
like `swaybg` and `swww` work if you want animated or per-monitor backgrounds:

```toml
[[exec_once]]
cmd = "swaybg"
args = ["-i", "/path/to/wallpaper.jpg", "-m", "fill"]
```

## Fonts

```toml
font_family = "Inter"
```

The font used by Otto's own UI — dock tooltips, exposé window titles, the app
switcher. Applications use their own font settings; this does not change them.

Otto ships with [Inter](https://rsms.me/inter/) as its default. Any font
installed on the system works.

## Cursor

```toml
cursor_theme = "Notwaita-Black"
cursor_size = 24
```

Cursor theme names are the directory names under `/usr/share/icons/` and
`~/.local/share/icons/`, and they are **case-sensitive**. Check what you have:

```sh
ls /usr/share/icons ~/.local/share/icons
```

Otto also implements `wp-cursor-shape-v1`, so applications that use the modern
cursor-shape protocol get themed cursors without shipping their own bitmaps.

## Icon theme

```toml
icon_theme = "Adwaita"
```

Used for application icons in the dock, the app switcher and notification
islands. Commented out or absent, Otto auto-detects a reasonable theme from
what is installed.

Popular choices: `Adwaita`, `Papirus`, `WhiteSur`, `Fluent`. Otto's own
screenshots use the
[Fluent icon theme](https://github.com/vinceliuice/Fluent-icon-theme).

Icons are looked up per the freedesktop icon theme specification, including
inheritance — a theme that lacks an icon falls back to its parent.

## Language

```toml
locales = ["en_US", "en"]
```

Preferred languages for application names and descriptions read from `.desktop`
files. Use standard locale identifiers, most specific first, and prefer the
region-qualified form (`lang_COUNTRY`): `["fr_FR", "fr"]` means "French (France)
if the entry has that variant, plain French otherwise, English as the ultimate
fallback from the entry's untranslated name". Matching follows the freedesktop
Desktop Entry spec and falls back automatically from region to bare language, so
listing both `zh_CN` and `zh` catches entries that localize either form.

This affects names shown in the dock, app switcher and menus. It does not change
Otto's own UI language.

## A worked example

A dark HiDPI setup:

```toml
screen_scale = 2.0

theme_scheme = "Dark"
accent_color = "purple"
font_family = "Inter"

background_image = "/home/me/Pictures/wallpaper.jpg"
background_color = "#101018"

cursor_theme = "Adwaita"
cursor_size = 32
icon_theme = "Papirus-Dark"

locales = ["en_US", "en"]
```

## Troubleshooting

**Icon or cursor theme not found.** Names are case-sensitive and must match the
directory name exactly. Check with `ls /usr/share/icons/`. Some themes need a
separate package for their cursor variant.

**Applications ignore light/dark.** They read it from the XDG Settings portal,
which needs `xdg-desktop-portal` running with Otto selected for
`org.freedesktop.impl.portal.Settings` in your `portals.conf`. Verify with:

```sh
gdbus call --session --dest org.freedesktop.portal.Desktop \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.Settings.Read \
  org.freedesktop.appearance color-scheme
```

`1` means dark, `2` means light, `0` means no preference.

**The wallpaper is a gradient instead of my image.** The path is wrong or
unreadable. Check the log — Otto reports the failure and falls back to
`background_color`.

## Not yet supported

- Per-monitor or per-workspace wallpaper
- Automatic light/dark switching by time of day
- Custom hex accent colours
- Animated wallpaper without an external daemon
