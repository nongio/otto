# XDG specifications

The inventory of the freedesktop.org/XDG file and D-Bus standards Otto
implements, and the system services it talks to. For each one: which side Otto
takes (producer or consumer of the files, server or client of the interface),
how complete the support is, and where the code lives. What this means to a
user is [Open standards](../user/desktop-standards.md).

Why Otto keeps its state in the standard locations, even where a private
format would be less work, is argued once in
[How we see the Linux desktop](../user/desktop-standards.md#how-we-see-the-linux-desktop).
The gaps at the bottom of this page are measured against it.

The `xdg_*` Wayland protocols, like every other protocol Otto serves, are in
[Wayland Protocols](wayland.md). Fonts are [Text rendering](text-rendering.md).

## File specifications

| Specification | Role | Implementation |
|---------------|------|----------------|
| [Base Directory](https://specifications.freedesktop.org/basedir-spec/latest/) | Read + write. `XDG_CONFIG_HOME`, `XDG_CONFIG_DIRS`, `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` are honoured everywhere paths are resolved; the session log is `$XDG_STATE_HOME/otto/otto.log` | `src/config/mod.rs` (config layering), `components/otto-foundations/src/xdg.rs`, `src/session_log.rs` |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry-spec/latest/) | Read. `Name`, `Icon`, `Exec`, `Categories`, `StartupWMClass`, and `[Desktop Action …]` groups; the dock's right-click menu is an entry's own `Actions=` | `components/otto-kit/src/desktop_entry.rs`, `src/workspaces/apps_info.rs`, `components/otto-launcher/` |
| [Desktop Entry — associations (`mimeapps.list`)](https://specifications.freedesktop.org/mime-apps-spec/latest/) | Read + write. Resolves the default application for a MIME type, following the standard search order. Files' Open With, with *Always use this app*, writes `[Default Applications]` and `[Added Associations]` in `$XDG_CONFIG_HOME/mimeapps.list` | `src/config/default_apps.rs`, `components/otto-kit/src/mime_apps.rs` (`set_default`), `components/otto-files/src/app/open_with.rs` |
| [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info-spec/latest/) | Read. `mime/globs2` and `mime/subclasses` from every XDG data dir, with the spec's glob precedence; content sniffing on top | `components/otto-kit/src/filetype/` (`db.rs`, `glob.rs`) |
| [Icon Theme](https://specifications.freedesktop.org/icon-theme-spec/latest/) | Read. Theme lookup with `hicolor` fallback, SVG and raster | `components/otto-kit/src/icons.rs`, `src/utils/mod.rs`, via the `freedesktop-icons` crate |
| [Cursor theme (Xcursor)](https://www.freedesktop.org/wiki/Specifications/cursor-spec/) | Read. `XCURSOR_THEME` / `XCURSOR_SIZE`, loaded per named cursor and scale | `src/cursor.rs` |
| [Sound Theme](https://specifications.freedesktop.org/sound-theme-spec/latest/) | Read. `sound_theme` names the theme; event sounds are looked up by name with fallback | `components/otto-kit/src/sound.rs`, `src/audio/sound_player.rs` |
| [Trash](https://specifications.freedesktop.org/trash-spec/latest/) | Read + write. The home trash (`$XDG_DATA_HOME/Trash`) and each mounted filesystem's own can (`$topdir/.Trash/$uid` or `.Trash-$uid`), with `.trashinfo` sidecars for Put Back and `directorysizes` for folders. A topdir can is used only if it is safe (owned by the user, not a symlink); the rules are in `trash.rs`'s module docs. The dock watches every can to draw a full bin | `components/otto-kit/src/trash.rs` (trashing, cans), `components/otto-files/src/model.rs` (listing, restore, delete), `src/workspaces/trash.rs` (dock) |
| [Thumbnail Managing Standard](https://specifications.freedesktop.org/thumbnail-spec/latest/) | Read. `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`, validated by the `Thumb::MTime` PNG text chunk, honouring failure markers under `thumbnails/fail/otto-files/`. Otto writes nothing into the cache. Implemented directly, no dependency | `components/otto-peek/src/thumbcache.rs` |
| [Autostart](https://specifications.freedesktop.org/autostart-spec/latest/) | Read. `$XDG_CONFIG_DIRS/autostart` then `$XDG_CONFIG_HOME/autostart`, user entries overriding system ones by filename. Gated on `xdg_autostart` in the config | `Otto::launch_xdg_autostart` in `src/input/actions.rs` |
| [XDG user directories](https://www.freedesktop.org/wiki/Software/xdg-user-dirs/) | Read. `user-dirs.dirs` supplies the Files sidebar's Desktop, Downloads, Documents… places, and the portal's screenshot folder | `components/otto-foundations/src/xdg.rs` (`user_dirs`), used by `components/otto-files/src/model.rs` |
| [Fontconfig](https://www.freedesktop.org/wiki/Software/fontconfig/) (not an XDG spec) | Read. Family lookup, aliases and the user's `fonts.conf`; rendering preferences per family. How it is applied and what Otto publishes to other toolkits is [Text rendering](text-rendering.md) | `components/otto-kit/src/text_rendering.rs` |

## D-Bus interfaces

| Specification | Role | Implementation |
|---------------|------|----------------|
| [Desktop Notifications](https://specifications.freedesktop.org/notification-spec/latest/) | Server, spec 1.2. `org.freedesktop.Notifications`: `Notify`, `CloseNotification`, `ActionInvoked`, capabilities `body`, `body-markup`, `actions`, `persistence`, `icon-static`, `action-icons`. Notifications are drawn by the Dynamic Island | `components/otto-islands/src/notifications.rs`, per-app permission in `notification_permission.rs` |
| [XDG Desktop Portal](https://flatpak.github.io/xdg-desktop-portal/docs/) | Backend. `org.freedesktop.impl.portal.{ScreenCast, Settings, Access, Screenshot, FileChooser}`, registered in `otto.portal`. See [xdg-desktop-portal-otto](xdg-desktop-portal-otto.md); the Settings keys are in [Color Scheme](color-scheme-setting.md#the-other-four-namespaces) | `components/xdg-desktop-portal-otto/src/portal/` |
| XDG portal `PermissionStore` | Client. Remembered screencast and notification choices | `components/xdg-desktop-portal-otto/src/portal/remembered.rs` |
| [StatusNotifierItem / `StatusNotifierWatcher`](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) | Host. The top bar's system tray registers as the watcher and tracks items | `components/otto-bar/src/tray.rs` |
| [DBusMenu](https://github.com/AyatanaIndicators/libdbusmenu) | Client: menus for tray items that expose no `ContextMenu` method. Server: an otto-kit app's menu bar, and `otto-rdp`'s indicator | `components/otto-bar/src/dbusmenu.rs`, `components/otto-kit/src/app_menu.rs`, `components/otto-rdp/src/indicator.rs` |
| Global menu: `com.canonical.AppMenu.Registrar` | Server. The top bar shows the focused app's exported menu | `components/otto-bar/src/appmenu.rs`; Qt's equivalent is a Wayland protocol, see [Wayland Protocols](wayland.md) |
| [MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/) | Client. Media keys and the Dynamic Island's player | `src/audio/media_control.rs`, `components/otto-islands/src/mpris.rs` |
| AT-SPI, `org.freedesktop.a11y` | Server. Chrome and otto-kit apps are AT-SPI apps through AccessKit; the compositor serves `KeyboardMonitor` and `PointerLocator` | `src/a11y/`, `components/otto-kit/src/accessibility/` |

## System services

| Service | Role | Implementation |
|---------|------|----------------|
| systemd-logind | Client. Lock, `PrepareForSleep` with a delay inhibitor, power actions; seat access through libseat | `src/lock.rs`, `components/otto-dbus/src/login1.rs` |
| UPower, power-profiles-daemon | Client. Battery and power mode in the top bar | `components/otto-dbus/src/upower.rs`, `components/otto-bar/src/power.rs` |
| polkit | Authentication agent (`libpolkit-agent-1`), supervised by the compositor; the compositor is also an `Authority` client | `components/otto-authorize/src/polkit/`, `src/polkit_agent.rs`, `src/settings_service.rs` |
| AccountsService | Client. Name, picture and password | `components/otto-settings/src/panes/account.rs`, `components/otto-auth-ui/src/user.rs` |
| greetd | Client. The login greeter | `components/otto-greeter/src/greetd.rs` |
| PipeWire | Producer (screencast streams) and client (volume, visualiser, RDP capture) | `src/screenshare/pipewire_stream.rs`, `src/audio/volume.rs` |
| LocalSearch (Tracker 3) | Client over SPARQL; the compositor sets its miner's GSettings | `components/otto-search/`, `src/search_index.rs` |

## Known gaps

- **Recent files** (`recently-used.xbel`) is neither read nor written. Files'
  Recent place is built by walking the XDG user directories by modification
  time instead, so it does not see what other applications opened.
- **Startup notification**: `StartupWMClass` is used for matching windows to
  entries, but Otto does not emit startup-notification IDs of its own.
- **Thumbnails are read only**: Otto uses the shared cache but never adds
  to it.
- **No `org.freedesktop.FileManager1`**: other apps' "Show in folder" does
  not reach Files.
