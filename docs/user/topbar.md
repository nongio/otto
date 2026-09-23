# Top Bar

`otto-bar` is Otto's menu bar: a full-width panel along the top edge of the
primary monitor, with frosted-glass blur, rounded bottom corners and a soft
shadow.

## Starting it

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

- **Left.** The focused application's name in bold, then its global menu.
- **Centre.** Deliberately empty, leaving room for the
  [Dynamic Island](dynamic-island.md).
- **Right.** System tray icons, then the battery, then the clock.

The bar reserves its own height as an exclusive zone, so maximized windows and
other panels stop below it rather than sliding underneath.

## Global application menus

The left zone shows the focused window's menu (File, Edit, View and so on),
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

- **GTK 3/4 apps.** These usually export automatically over the GTK
  application-menu D-Bus interfaces.
- **Qt/KDE apps.** These need `appmenu-qt5` / the `AppMenu` platform theme
  plugin.
- **Electron and browsers.** These mostly do not export menus.
- **X11 apps.** These can export via `appmenu-gtk-module` and the
  `UNITY_MENUBAR` path.

The application name shown comes from the window's `app_id` mapped through the
desktop entry database.

## System tray

The right zone implements `StatusNotifierHost`, the modern tray standard
(`org.kde.StatusNotifierItem`). Any app that registers an SNI icon appears here:
Nextcloud, Telegram, Slack, Steam, KeePassXC, network and volume applets.

| Click | Effect |
|-------|--------|
| Left | Open the icon's context menu |
| Right | Activate; usually raises the app's window |
| Middle | Secondary activate (app-defined) |

Icons are ordered by registration
time, newest to the left.

Menus stay current while they are open. When an applet changes its menu (a
Wi-Fi scan finding a network, Bluetooth connecting a device), the bar fetches
the new one and updates the menu in place, keeping your place in it. If the
menu has grown or shrunk it is shown again at its new size, in the same spot.

Legacy XEmbed tray icons (the old X11 system tray) are **not** supported; that
standard has no Wayland equivalent. Apps that only do XEmbed will not appear.


## Battery

On a machine with a battery, the bar draws one: an outline that fills with the
charge, green until 20%, amber to 10%, red below that. A bolt over it means
it is charging; a plug means it is plugged in but not charging, because it is
full or held at a charge limit. The percentage is written inside the glyph.
The menu says the same in words, with how long until full when UPower has
worked it out.

The bar reads UPower itself rather than hosting a tray applet for this. The
tray carries an icon and a tooltip, so a percentage published through it would
be a percentage nobody can see without hovering.

Clicking it opens the power menu: what the battery is doing, what the CPU is
doing, and the power profiles that can be selected.

```
┌──────────────────────────────────┐
│ Battery 87% — 2:14 remaining     │
│ CPU 1.10 GHz average, 3.79 peak  │
│ Governor: powersave              │
├──────────────────────────────────┤
│ ✓ Power Saver                    │
│   Balanced                       │
│   Performance                    │
├──────────────────────────────────┤
│ Power Settings…                  │
└──────────────────────────────────┘
```

### Where the profiles come from

Whichever of these is available, in order:

1. **power-profiles-daemon**, under either of its D-Bus names. This is the
   usual case. Selecting a profile sets it, and polkit handles the permission.
2. **`[[battery.profiles]]` entries** from the config file, each a command. Use
   these when the daemon is absent or masked, which it will be if something
   else manages the CPU: `auto-cpufreq` and power-profiles-daemon conflict,
   and installing one masks the other.
3. **Nothing.** The menu still lists the kernel's governors and ticks the live
   one, greyed out, so it says what the CPU is set to even when it cannot
   change it.

The CPU numbers are read when the menu opens, not on a timer: `scaling_cur_freq`
moves faster than anything could usefully display. Everything else follows
along while the menu is open: select a profile and the tick moves once the
switch lands, without reopening.

### Settings

```toml
# ~/.config/otto/otto-bar.toml

[battery]
show = "auto"            # "auto" | true | false
colored = true           # false draws the fill in the bar's text colour
percentage = "inside"    # "inside" | "beside" | "off"
low_level = 20
critical_level = 10
width = 28
height = 13
color_normal = "#34C759"
color_low = "#FF9F0A"
color_critical = "#FF3B30"
color_charging = "#34C759"
update_interval = 10     # seconds
menu = true
show_battery_info = true
show_cpu_info = true
settings_command = "otto-settings"

# Only needed when power-profiles-daemon is not running. `governor` and `epp`
# are optional: they say when the entry is the live one, and an entry that
# states neither is never check-marked.
[[battery.profiles]]
label = "Power Saver"
command = ["pkexec", "cpupower", "frequency-set", "-g", "powersave"]
governor = "powersave"

[[battery.profiles]]
label = "Performance"
command = ["pkexec", "cpupower", "frequency-set", "-g", "performance"]
governor = "performance"
```

## Clock

The clock is on the far right. Its format is a
[chrono strftime](https://docs.rs/chrono/latest/chrono/format/strftime/index.html)
string:

```toml
# ~/.config/otto/otto-bar.toml
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

`otto-bar` has its own config file. The first one that exists and parses is
used — there is no merging, so a system file shadows a user one:

1. `/etc/otto/otto-bar.toml`
2. `~/.config/otto/otto-bar.toml`
3. `./otto-bar.toml` (development override in the working directory)

`clock_format` and the `[battery]` section are the options. Bar height, colours
and blur follow the compositor's theme.

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

**No menus for any app.** DBusMenu is opt-in per toolkit. See
[Getting an app to export its menu](#getting-an-app-to-export-its-menu) above.

**A tray icon is missing.** It is probably XEmbed-only. Check whether the app
has an SNI or "AppIndicator" option.

**The clock format is ignored.** A malformed strftime string falls back to the
default. Verify the file parses as TOML and the key is at the top level, not in
a table.
