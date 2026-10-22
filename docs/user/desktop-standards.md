# Desktop standards and protocols

Otto keeps your desktop in the standard places. It reads applications, icon
themes, file associations, the trash and the thumbnail cache from where the
freedesktop.org (XDG) specifications put them, and it serves the shared
Wayland protocols, so the apps, themes and tools you already use keep working
and what Otto writes stays readable by the rest of the system. The tables
below list what is supported today; if something isn't listed, assume it
isn't there yet.

Developers looking for where each one lives in the source want
[XDG specifications](../developer/xdg-specifications.md) and
[Wayland protocols](../developer/wayland.md).

## How we see the Linux desktop

Otto tries to make the everyday easy: things a click away, in one place, with
sensible defaults. That ease sits on top of the system, not in place of it.
Otto keeps what it manages in the places the freedesktop.org (XDG) standards
name, and reads what other software puts there.

- **Nothing to relearn.** Otto reads your default applications from
  `mimeapps.list`, starts what is in `~/.config/autostart`, and picks up icon
  themes from `~/.icons`. What you know about configuring Linux, and the
  dotfiles you keep, still applies.
- **Any part can be swapped.** The trash is the shared trash, so another file
  manager drops in. Notifications, the tray, the desktop portal and layer shell
  are the standard interfaces, so another notification daemon, bar or launcher
  can take over from Otto's. Otto's own parts serve apps that have never heard
  of it.

We will not trade portability for convenience. Where Otto falls short, the gap
is listed under [What is not supported](#what-is-not-supported).

## Desktop specifications

| Standard | What it means in Otto | More |
|----------|-----------------------|------|
| [Base Directory](https://specifications.freedesktop.org/basedir-spec/latest/) | Config, data, cache and state go under the `XDG_*` directories; Otto's log is `~/.local/state/otto/otto.log` | [Configuration](configuration.md) |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry-spec/latest/) | Every installed `.desktop` app shows in the Launcher and can be pinned; an entry's `Actions=` are its right-click menu in the Dock | [Launcher](launcher.md), [Dock](dock.md) |
| [MIME applications](https://specifications.freedesktop.org/mime-apps-spec/latest/) (`mimeapps.list`) | Default apps are read from it, the same file `xdg-mime default` writes. Files' **Open With… → Always use this app** writes the choice back there | [Files](files.md) |
| [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info-spec/latest/) | File types and their icons match the rest of the system. Install `shared-mime-info` if types look generic | [Files](files.md) |
| [Icon Theme](https://specifications.freedesktop.org/icon-theme-spec/latest/) | Any theme in `~/.icons`, `~/.local/share/icons` or `/usr/share/icons` | [Theming](theming.md) |
| [Cursor themes](https://www.freedesktop.org/wiki/Specifications/cursor-spec/) (Xcursor) | Any installed cursor theme and size | [Theming](theming.md) |
| [Sound Theme](https://specifications.freedesktop.org/sound-theme-spec/latest/) | System sounds come from the selected sound theme | [Audio](audio.md) |
| [Trash](https://specifications.freedesktop.org/trash-spec/latest/) | The shared trash, on your home and on other drives: what another file manager deleted can be put back from Otto, and the other way round | [Files](files.md) |
| [Thumbnails](https://specifications.freedesktop.org/thumbnail-spec/latest/) | Thumbnails another file manager already made under `~/.cache/thumbnails` load instantly. Otto reads the cache; it does not add to it | [Files](files.md) |
| [XDG user directories](https://www.freedesktop.org/wiki/Software/xdg-user-dirs/) | Desktop, Downloads, Documents… in the Files sidebar, and the screenshot folder, are where `user-dirs.dirs` says | [Files](files.md) |
| [Autostart](https://specifications.freedesktop.org/autostart-spec/latest/) | Entries in `/etc/xdg/autostart` and `~/.config/autostart` start with the session; can be turned off | [Autostart](autostart.md) |
| [Notifications](https://specifications.freedesktop.org/notification-spec/latest/) | Otto is the notification server: `notify-send` and every app's notifications work, with actions and icons | [Dynamic Island](dynamic-island.md) |
| [Desktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/) | Otto's own backend: screen sharing, screenshots, file dialogs, appearance settings and permission prompts, for browsers, OBS and Flatpak apps | [Screen Sharing](screen-sharing.md) |
| [StatusNotifierItem](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) | Tray icons in the top bar, with their menus (DBusMenu) | [Top Bar](topbar.md) |
| Global menu (`com.canonical.AppMenu`, KDE appmenu) | Apps that export their menu bar show it in the top bar | [Top Bar](topbar.md) |
| [MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/) | Any media player shows in the Dynamic Island and follows the media keys | [Audio](audio.md) |
| [AT-SPI](https://www.freedesktop.org/wiki/Accessibility/AT-SPI2/) | Screen readers read Otto's chrome and Otto's apps | [Accessibility](accessibility.md) |
| Fontconfig | Font names, aliases and your `fonts.conf` rendering rules; see [Fonts and text](#fonts-and-text) | [Theming](theming.md) |

### System services Otto uses

| Service | What for |
|---------|----------|
| systemd-logind | Session, lock on suspend, power actions |
| UPower, power-profiles-daemon | Battery and power mode in the top bar |
| polkit | Otto is the authentication agent: password prompts for admin actions |
| AccountsService | Your name and picture, changing your password in Settings |
| PipeWire | Screen sharing streams, audio volume and the audio visualiser |
| greetd | The login screen |
| LocalSearch (Tracker) | File search |

## Fonts and text

Fonts are resolved through fontconfig, like every other Linux application:
the font you name in Settings is matched against what is installed, aliases
like `sans-serif` work, and `~/.config/fontconfig/fonts.conf` is respected.

Hinting and antialiasing follow fontconfig (`antialias`, `hinting`,
`hintstyle`, `autohint`, `rgba`) in Otto's own text: the dock, the top bar,
titlebars and Otto's apps. Text inside other programs is drawn by those
programs, before Otto sees it; Otto composites the finished image.

**Subpixel text** is per monitor: set it in **Settings → Displays → Subpixel
text**, or with `subpixel` in the display profile. Automatic uses what the
monitor reports, else fontconfig's `rgba`, and draws grayscale when neither
says or when `rgba` is `none`. Details in [Display](display.md).

Otto also tells other toolkits its font and how it draws text, so they look
like Otto beside it:

| Apps | Get Otto's font and settings from | Follows hinting? |
|------|-----------------------------------|------------------|
| Otto and Otto's apps | Otto's configuration and fontconfig | yes |
| GTK 4 | the desktop portal | yes |
| GTK 3, Chromium, Electron | GNOME's settings store, which Otto fills at login | yes |
| Qt | Qt's GTK theme, which Otto selects unless `QT_QPA_PLATFORMTHEME` is set | only on an unscaled display (Qt's own rule) |
| X11 apps (Xwayland) | XSETTINGS | yes |

Changing the font needs a restart; a `fonts.conf` change applies from the
next login.

## Wayland protocols

These are the protocols applications use to talk to Otto. "Trusted apps"
means not offered to sandboxed apps (Flatpak), which cannot list windows,
read the screen or type for you.

### Windows

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `xdg-shell` | Windows and popups: every Wayland app | |
| `xdg-decoration`, KDE server decoration | Lets an app ask Otto to draw its titlebar, or draw its own | |
| `xdg-activation` | An app handing focus to another (clicking a link opens the browser in front) | Only right after a click or key press |
| `xdg-foreign` | A dialog attaching to another app's window (portal file dialogs) | |
| `xdg-dialog` | Marking a dialog modal | Modal dialogs float when tiling |
| `xwayland-shell`, Xwayland keyboard grab | X11 apps | |
| `ext-background-effect` | Blur behind translucent terminals and panels | The whole window is frosted, not a region |

### Display and scaling

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `wl_output`, `xdg-output` | Monitor size, position, scale and subpixel order | |
| `fractional-scale` | Sharp apps at scales like 1.25 or 1.5 | |
| `viewporter` | Apps cropping and scaling their own buffers | |
| `presentation-time` | Accurate frame timing for video and games | Login session only |
| `linux-dmabuf` | GPU buffers, no copies | |
| `linux-drm-syncobj` | Explicit GPU sync, which some drivers need for smooth frames | Login session, when the GPU supports it |
| `drm-lease` | VR headsets taking over a display | Login session only |
| `cursor-shape` | Apps naming a cursor instead of drawing it, so it matches your theme | |
| `wlr-gamma-control` | Colour temperature tools (`gammastep`, `wlsunset`) | Trusted apps, login session only. See [Night Shift](night-shift.md) |

### Input

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `pointer-gestures` | Touchpad swipe, pinch and hold in apps | Login session only. See [Gestures](gestures.md) |
| `relative-pointer`, `pointer-constraints` | Mouse look in games, locking the pointer | Relative motion in the login session only |
| `tablet` | Drawing tablets and pens | |
| `keyboard-shortcuts-inhibit` | Remote desktop and VM apps getting every key | Trusted apps; lock and VT switch still work |
| `text-input-v3`, `input-method-v2` | Input methods (IME) and dictation | Input methods are trusted apps |
| `virtual-keyboard`, `wlr-virtual-pointer` | Tools that type or move the pointer (emoji picker, remote desktop) | Trusted apps |

### Clipboard

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `wl_data_device` | Copy, paste, drag and drop | |
| `primary-selection` | Middle-click paste | |
| `wlr-data-control`, `ext-data-control` | Clipboard managers and `wl-copy`/`wl-paste` | Trusted apps. See [Clipboard](clipboard.md) |

### Panels, taskbars and capture

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `wlr-layer-shell` | Bars, docks, launchers, wallpapers, widgets | Trusted apps. See [Desktop Widgets](desktop-widgets.md) |
| `ext-foreign-toplevel-list` | Listing open windows | Trusted apps |
| `wlr-foreign-toplevel-management` | Taskbars that activate, minimise, maximise or close windows | Trusted apps |
| `wlr-screencopy` | Screenshot and recording tools (`grim`, `wf-recorder`, `wl-mirror`) | Trusted apps. Screen sharing goes through the portal |

### Session and security

| Protocol | What it is for | Notes |
|----------|----------------|-------|
| `ext-session-lock` | The lock screen | Only Otto's own [Lock Screen](lock-screen.md): `swaylock` and similar cannot lock |
| `idle-inhibit` | Video players keeping the screen awake | |
| `security-context` | Flatpak marking its apps as sandboxed | |

### Otto's own

| Protocol | What it is for |
|----------|----------------|
| `otto-surface-style` | Otto's apps shaping and animating their windows (corners, shadows, blur) |
| `otto-dock` | Apps adding items to the Dock |
| `otto-canvas` | Items on the [Side Canvas](side-canvas.md) |
| `otto-text-cursor` | Pickers (emoji) opening beside the text cursor |

## What is not supported

| Missing | What to do instead |
|---------|--------------------|
| Recent files (`recently-used.xbel`) | Files' Recent place lists recently changed files in your folders, not what other apps opened |
| Writing thumbnails to the shared cache | Thumbnails other file managers made are used; Otto's own are not saved for others |
| Startup notification (busy cursor while an app starts) | None |
| `org.freedesktop.FileManager1` ("Show in folder" from other apps) | Open the folder in Files |
| Third-party lock screens | Use Otto's lock screen |
| Text scaling | Text follows the display scale; scale the display to make text bigger |
| Pushing font changes live to GTK 3 and Qt apps | Restart the app after changing fonts |
| `ext-idle-notify` (idle daemons like `swayidle`) | Use Otto's [Power Management](power-management.md) |
| `wlr-output-management` (`wlr-randr`, `kanshi`) | Set displays in Settings or the [display profiles](display.md) |
| `ext-image-copy-capture` | Use `wlr-screencopy` tools or the portal |
| `tearing-control`, `content-type`, `color-management`, `single-pixel-buffer`, `alpha-modifier`, `fifo`, `commit-timing`, `xdg-system-bell`, `xdg-toplevel-icon` | Apps fall back to their default behaviour |
