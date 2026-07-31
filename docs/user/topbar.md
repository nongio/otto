# Top Bar

`otto-bar` is Otto's menu bar: a full-width panel along the top edge of the
primary monitor, with frosted-glass blur, rounded bottom corners and a soft
shadow.

It is a separate program, not part of the compositor. Start it from your
config:

```toml
[[exec_once]]
cmd = "otto-bar"
```

See [Autostart](autostart.md) for other ways to launch it.

## Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ Firefox  File  Edit  View  Help      (island)      🔊 🔋  Mar 23, 21:16│
└──────────────────────────────────────────────────────────────────────┘
  ← left zone ────────────────►      centre       ← right zone ───────►
```

- **Left** — the focused application's name in bold, then its global menu.
- **Centre** — deliberately empty, leaving room for the
  [Dynamic Island](dynamic-island.md).
- **Right** — system tray icons, then the clock.

The bar reserves its own height as an exclusive zone, so maximized windows and
other panels stop below it rather than sliding underneath.

## Global application menus

The left zone shows the focused window's menu — File, Edit, View and so on —
sourced over D-Bus using the `com.canonical.dbusmenu` protocol. This is the same
mechanism Unity and KDE's global menu use.

| Action | Effect |
|--------|--------|
| Click a top-level title | Drop the menu down |
| `↑` / `↓` | Move through items |
| `→` / `Enter` on a submenu | Open it |
| `←` | Back to the parent menu |
| `Enter` | Activate the highlighted item |
| `Escape` | Close |

Hovering a submenu opens it after a short delay (about 300 ms) rather than
instantly, so passing over an item on the way somewhere else does not fire it.
Exactly one item is highlighted anywhere in the menu tree at a time.

Menu items support labels, icons, keyboard-shortcut hints, separators,
checkboxes, radio groups and arbitrarily nested submenus. Disabled items are
dimmed and inert.

### Getting an app to export its menu

Not every application exports a DBusMenu. When one does not, the left zone shows
just the application's name and the app keeps drawing its own menu bar in its
window.

- **GTK 3/4 apps** — usually export automatically over the GTK application-menu
  D-Bus interfaces.
- **Qt/KDE apps** — need `appmenu-qt5` / the `AppMenu` platform theme plugin.
- **Electron and browsers** — mostly do not export menus.
- **X11 apps** — can export via `appmenu-gtk-module` and the `UNITY_MENUBAR`
  path.

The application name shown comes from the window's `app_id` mapped through the
desktop entry database.

## System tray

The right zone implements `StatusNotifierHost`, the modern tray standard
(`org.kde.StatusNotifierItem`). Any app that registers an SNI icon appears here:
Nextcloud, Telegram, Slack, Steam, KeePassXC, network and volume applets.

| Click | Effect |
|-------|--------|
| Left | Open the icon's context menu |
| Right | Activate — usually raises the app's window |
| Middle | Secondary activate (app-defined) |

Hovering shows the tooltip the app supplies. Icons are ordered by registration
time, newest to the left.

Legacy XEmbed tray icons (the old X11 system tray) are **not** supported; that
standard has no Wayland equivalent. Apps that only do XEmbed will not appear.

## Clock

The clock is on the far right. Its format is a
[chrono strftime](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)
string:

```toml
# ~/.config/otto/topbar.toml
clock_format = "%B %-d, %A %H:%M"
```

That default renders as `March 23, Thursday 21:16`. Some other useful formats:

| Format | Renders as |
|--------|------------|
| `"%H:%M"` | `21:16` |
| `"%a %d %b  %H:%M"` | `Thu 23 Mar  21:16` |
| `"%I:%M %p"` | `09:16 PM` |
| `"%Y-%m-%d %H:%M:%S"` | `2026-03-23 21:16:04` |

## Configuration

`otto-bar` has its own config file, loaded from — in order, later overriding
earlier:

1. `/etc/otto/topbar.toml`
2. `~/.config/otto/topbar.toml`
3. `./otto_topbar.toml` (development override in the working directory)

`clock_format` is currently the only option. Bar height, colours and blur follow
the compositor's theme.

## Theming

The bar follows the system light/dark setting, read from the XDG Settings portal
(`org.freedesktop.appearance color-scheme`). Change `theme_scheme` in your Otto
config and the bar re-colours within about a second, no restart needed. See
[Theming](theming.md).

## Multi-monitor

The bar appears on the **primary monitor only**. A reduced clock-and-tray bar
for secondary monitors is designed but not implemented.

## Troubleshooting

**The bar does not appear.** Check it is running (`pgrep otto-bar`) and that it
found the Wayland socket. Run it by hand from a terminal inside the session to
see errors.

**No menus for any app.** DBusMenu is opt-in per toolkit — see
[Getting an app to export its menu](#getting-an-app-to-export-its-menu) above.

**A tray icon is missing.** It is probably XEmbed-only. Check whether the app
has an SNI or "AppIndicator" option.

**The clock format is ignored.** A malformed strftime string falls back to the
default. Verify the file parses as TOML and the key is at the top level, not in
a table.
