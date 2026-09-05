# Desktop standards

Otto is a new desktop, but it is not a separate world. It reads the same
application entries, icon themes, MIME database, trash can and thumbnail cache
that GNOME, KDE Plasma and COSMIC use, so what you already have on the machine
keeps working — and what Otto does is visible to those desktops too.

## Why it works this way

Otto's own surface is meant to be easy: things are a click away, in one place,
with sensible defaults, and you should rarely have to go looking for a config
file to get through a day.

That is the front of the desktop, not a replacement for the system underneath
it. Otto stores what it manages in the places the freedesktop.org (XDG)
standards say it belongs, and reads what other software puts there. Two things
follow, and both are the point:

- **You already know where things live.** Your default browser is still
  `mimeapps.list`. Autostart is still a `.desktop` file in
  `~/.config/autostart`. Icon themes are still `~/.icons`. Everything you have
  learned about configuring a Linux desktop — and every script and dotfile
  repository built on it — applies here unchanged. Otto's settings are a
  friendlier way in, never the only way in.
- **You can swap parts out.** The trash is the shared trash, so another file
  manager is a drop-in. Notifications, the tray, the screen-sharing portal and
  the panel protocols are the standard ones, so a third-party bar, launcher,
  wallpaper tool or notification daemon can take over from Otto's, and Otto's
  can serve applications that have never heard of it. Nothing here is a private
  format that locks the rest of the system out.

Convenience that costs you portability is a bad trade. Otto tries not to make
it.

This page is the short version of what that means in practice. Developers
looking for the file-by-file inventory want
[XDG specifications](../developer/xdg-specifications.md).

## Applications

Otto finds applications through their
[`.desktop` entries](https://specifications.freedesktop.org/desktop-entry-spec/latest/)
in the standard locations — anything installed by your package manager, Flatpak
or by hand under `~/.local/share/applications` shows up in the
[Launcher](launcher.md) and can be pinned to the [Dock](dock.md), with no
Otto-specific registration.

An entry's own actions are its menu: if a `.desktop` file declares
`Actions=NewWindow;NewPrivateWindow`, right-clicking that app in the dock
offers them.

**Default applications** come from
[`mimeapps.list`](https://specifications.freedesktop.org/mime-apps-spec/latest/),
the same file `xdg-mime default` writes. Set your browser or editor there — or
with any desktop's settings app — and Otto opens files with it. Otto reads this
file; it never rewrites it.

**File types** are recognised through the
[shared MIME database](https://specifications.freedesktop.org/shared-mime-info-spec/latest/),
so the Kind column and icons in [Files](files.md) agree with the rest of the
system. Install `shared-mime-info` if types look generic.

## Look and feel

**Icon themes** follow the
[Icon Theme specification](https://specifications.freedesktop.org/icon-theme-spec/latest/):
any theme in `~/.icons` or `/usr/share/icons` — Papirus, Adwaita, anything else
— can be selected. **Cursors** are standard Xcursor themes. Both are set in
[Theming](theming.md).

Otto also *publishes* its appearance to applications, over the Settings
portal's `org.freedesktop.appearance` namespace: GTK and Qt apps follow Otto's
light/dark mode, accent color and icon theme without any per-toolkit
configuration.

**Fonts** are resolved through fontconfig, the same system every other Linux
application uses — so the font you name in Otto's settings is matched against
the fonts you have installed, aliases like `sans-serif` work, and rules in
`~/.config/fontconfig/fonts.conf` are respected when choosing the face.

These limits apply to **Otto's own text only** — the dock, the top bar, window
titlebars, and Otto's applications. Text inside the programs you run is drawn
by those programs, with their own font stack, before Otto ever sees it: Otto
composites a finished image and cannot alter how the glyphs in it were
rasterised. Your fontconfig settings are already fully in effect for the text
you spend most of your time reading.

With that scope, three limits worth knowing:

- Otto's chrome draws with grayscale antialiasing, and there is currently no
  way to turn subpixel (LCD) antialiasing on for it. On a HiDPI display this is
  what you would probably pick anyway. On a 1080p display Otto's own dock, bar
  and window titles will look a little lighter and softer than the applications
  running above them.
- Hinting does not follow your fontconfig settings. If you have set a
  `hintstyle` in `fonts.conf`, applications will follow it and Otto's chrome
  will not.
- Otto's font choice applies to Otto. It is not published to applications the
  way its light/dark mode and accent color are, so set the interface font for
  GTK and Qt apps as you normally would — Otto does not override it, and does
  not set it either.

## Files, trash and thumbnails

The **trash** is the
[freedesktop trash can](https://specifications.freedesktop.org/trash-spec/latest/)
at `~/.local/share/Trash` — the same one every other file manager uses. Things
Nautilus or Dolphin deleted appear in Otto's Trash window and can be put back
from it, and vice versa. The dock's bin icon fills when there is something in
it.

**Thumbnails** use the
[shared thumbnail cache](https://specifications.freedesktop.org/thumbnail-spec/latest/)
under `~/.cache/thumbnails`. Pictures another file manager has already
thumbnailed load instantly in Otto, and the ones Otto generates are there for
the next program. See [Files](files.md).

The Files sidebar reads your **XDG user directories** (`user-dirs.dirs`), so
Desktop, Downloads and Documents point where your locale and setup say they do.

## Session

**Autostart** — Otto runs `.desktop` entries from `/etc/xdg/autostart` and
`~/.config/autostart` per the
[Autostart specification](https://specifications.freedesktop.org/autostart-spec/latest/),
so tray applets, sync clients and input methods start as they would elsewhere.
It can be turned off in favour of Otto's own `exec_once`; see
[Autostart](autostart.md).

**Notifications** — Otto is an `org.freedesktop.Notifications` server, so
`notify-send` and any application that sends notifications works. They are
drawn by the [Dynamic Island](dynamic-island.md), with actions and inline icons.

**Screen sharing, screenshots and file dialogs** go through
[xdg-desktop-portal](https://flatpak.github.io/xdg-desktop-portal/docs/):
Otto ships its own backend, which is what lets browsers, OBS, Flatpak apps and
sandboxed applications capture the screen and open files. Setup is in
[Screen Sharing](screen-sharing.md).

**System tray** — the top bar hosts StatusNotifierItem tray icons, the same
ones Plasma shows, including their DBusMenu menus. See [Top Bar](topbar.md).

## Windows

Applications talk to Otto over the standard Wayland shell protocols
(`xdg-shell` and friends), plus `wlr-layer-shell` for panels and widgets — so
third-party bars, launchers and wallpaper tools written for other Wayland
compositors run on Otto. [Desktop Widgets](desktop-widgets.md) covers running
them.

## What is not supported

- **Recent files** (`recently-used.xbel`) — Otto neither reads nor writes the
  recent-documents list.
- **Setting** default applications from inside Otto: `mimeapps.list` is read
  only, so change defaults with `xdg-mime` or another desktop's settings app.
- **Text scaling** — there is no separate text size control; text follows the
  display scale, so making text bigger means scaling the whole display.
- **Font rendering settings** — see the note under
  [Look and feel](#look-and-feel).
