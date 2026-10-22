# XDG specifications

This page is the inventory of the freedesktop.org/XDG specifications and
`xdg_*` protocols Otto implements. For each one it lists which side Otto takes
(producer or consumer of the files, server or client of the protocol), how
complete the support is, and where the code lives. What this means to a user
is [Desktop standards](../user/desktop-standards.md).

Why Otto keeps its state in the standard locations, even where a private
format would be less work, is argued once in
[How we see the Linux desktop](../user/desktop-standards.md#how-we-see-the-linux-desktop).
The gaps at the bottom of this page are measured against it.

Two families are covered:

- **XDG Wayland protocols**: the `xdg_*` shell protocols Otto serves to
  clients. For how protocol handlers are wired at all, see
  [Wayland Protocols](wayland.md).
- **XDG desktop specifications**: the file-format and D-Bus standards that let
  Otto and its applications interoperate with the rest of the desktop.

Protocols outside the XDG namespace (`wlr-*`, `ext-*`, `wp-*`, Otto's own) are
listed in [Wayland Protocols](wayland.md), not here.

## XDG Wayland protocols

| Protocol | Role | Implementation |
|----------|------|----------------|
| [`xdg_shell`](https://wayland.app/protocols/xdg-shell) | Server. Toplevels and popups, the main window protocol | `XdgShellHandler` in `src/shell/xdg.rs`; state in `src/state/mod.rs` |
| [`xdg-decoration-unstable-v1`](https://wayland.app/protocols/xdg-decoration-unstable-v1) | Server. Negotiates server- vs client-side decorations | `src/state/xdg_decoration_handler.rs` (alongside the KDE `org_kde_kwin_server_decoration` equivalent) |
| [`xdg-activation-v1`](https://wayland.app/protocols/xdg-activation-v1) | Server. Focus transfer between clients | `src/state/xdg_activation_handler.rs` |
| [`xdg-foreign-unstable-v2`](https://wayland.app/protocols/xdg-foreign-unstable-v2) | Server. Exporting a surface so another client can parent to it | `XdgForeignHandler` in `src/state/mod.rs` |
| [`xdg-output-unstable-v1`](https://wayland.app/protocols/xdg-output-unstable-v1) | Server. Logical output geometry for clients | `OutputManagerState::new_with_xdg_output` in `src/state/mod.rs` |
| [`xdg-dialog-v1`](https://wayland.app/protocols/xdg-dialog-v1) | Server. Clients mark a toplevel modal; tiling floats a modal parentless window | `XdgDialogHandler` in `src/state/mod.rs`, read in `src/shell/tiling.rs` |

Otto's own applications sit on the client side of these through `otto-kit`
(`components/otto-kit/src/components/window/`), so the toolkit exercises the
same protocols the compositor serves.

## XDG desktop specifications

| Specification | Role | Implementation |
|---------------|------|----------------|
| [Base Directory](https://specifications.freedesktop.org/basedir-spec/latest/) | Read + write. `XDG_CONFIG_HOME`, `XDG_CONFIG_DIRS`, `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `XDG_CACHE_HOME` are honoured everywhere paths are resolved | `src/config/mod.rs` (config layering), and every consumer below |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry-spec/latest/) | Read. `Name`, `Icon`, `Exec`, `Categories`, `StartupWMClass`, and `[Desktop Action …]` groups; the dock's right-click menu is an entry's own `Actions=` | `components/otto-kit/src/desktop_entry.rs`, `src/workspaces/apps_info.rs`, `components/otto-launcher/` |
| [Desktop Entry — associations (`mimeapps.list`)](https://specifications.freedesktop.org/mime-apps-spec/latest/) | Read + write. Resolves the default application for a MIME type, following the standard search order. Files' Open With, with *Always use this app*, writes `[Default Applications]` and `[Added Associations]` in `$XDG_CONFIG_HOME/mimeapps.list` | `src/config/default_apps.rs`, `components/otto-kit/src/mime_apps.rs` (`set_default`), `components/otto-files/src/app/open_with.rs` |
| [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info-spec/latest/) | Read. `mime/globs2` and `mime/subclasses` from every XDG data dir, with the spec's glob precedence; content sniffing on top | `components/otto-kit/src/filetype/` (`db.rs`, `glob.rs`) |
| [Icon Theme](https://specifications.freedesktop.org/icon-theme-spec/latest/) | Read. Theme lookup with `hicolor` fallback, SVG and raster | `components/otto-kit/src/icons.rs`, `src/utils/mod.rs`, via the `freedesktop-icons` crate |
| [Trash](https://specifications.freedesktop.org/trash-spec/latest/) | Read + write. The home trash `$XDG_DATA_HOME/Trash/{files,info}` with a `.trashinfo` sidecar (`Path`, `DeletionDate`), which is what makes Put Back work, and what makes Otto's trash the same trash as Nautilus's or Dolphin's. The sidecar is created first with `O_EXCL` to reserve the name. Files on another filesystem go to that filesystem's `$topdir/.Trash/$uid` (sticky, not a symlink) or `$topdir/.Trash-$uid`, with `Path` relative to `$topdir`; the home trash is the fallback. Trashed folders get a `directorysizes` line (skipped past 10 000 entries). The topdir is the longest mount point above the file in `/proc/self/mountinfo`. A topdir can is used only when it is a real directory owned by the user, not group- or other-writable, with `files/`, `info/` and `directorysizes` real and the user's too, and a shared `.Trash` sticky and root's or the user's (otherwise the next can, finally the home trash); trashing, deleting, Put Back from a topdir can (renamed, never copied, through `O_NOFOLLOW` folders), sidecar and `directorysizes` reads (regular files, size-capped, `O_NONBLOCK`) and the `directorysizes` temporary (`O_EXCL`, `0600`) work from `O_NOFOLLOW` directory descriptors; listing and watching a can go by path, read-only. A topdir sidecar's `Path=` must be relative with no `..` for Put Back. The Trash window, Empty Trash and the dock list the home can plus the checked topdir cans of every mounted filesystem (`/proc/self/mountinfo`, pseudo-filesystems, autofs, network filesystems and FUSE daemons left out; one can seen through two mounts listed once); the dock watches them with inotify off the compositor thread, and the mount table with `poll`, to draw a full bin; the Trash window watches every can's `files/` | `components/otto-kit/src/trash.rs` (trashing, cans), `components/otto-files/src/model.rs` (listing, restore, delete), `src/workspaces/trash.rs` (dock state) |
| [Thumbnail Managing Standard](https://specifications.freedesktop.org/thumbnail-spec/latest/) | Read. `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`, validated by the `Thumb::MTime` PNG text chunk, honouring failure markers under `thumbnails/fail/otto-files/`. Otto writes nothing into the cache. Implemented directly, no dependency | `components/otto-peek/src/thumbcache.rs` |
| [Autostart](https://specifications.freedesktop.org/autostart-spec/latest/) | Read. `$XDG_CONFIG_DIRS/autostart` then `$XDG_CONFIG_HOME/autostart`, user entries overriding system ones by filename. Gated on `xdg_autostart` in the config | `Otto::launch_xdg_autostart` in `src/input/actions.rs` |
| [Desktop Notifications](https://specifications.freedesktop.org/notification-spec/latest/) | Server, spec 1.2. `org.freedesktop.Notifications`: `Notify`, `CloseNotification`, `ActionInvoked`, capabilities `body`, `body-markup`, `actions`, `persistence`, `icon-static`, `action-icons`. Notifications are drawn by the Dynamic Island | `components/otto-islands/src/notifications.rs`, per-app permission in `notification_permission.rs` |
| [XDG Desktop Portal](https://flatpak.github.io/xdg-desktop-portal/docs/) | Backend. `org.freedesktop.impl.portal.{ScreenCast, Settings, Access, Screenshot, FileChooser}`, registered in `otto.portal` | `components/xdg-desktop-portal-otto/src/portal/` |
| [XDG user directories](https://www.freedesktop.org/wiki/Software/xdg-user-dirs/) | Read. `user-dirs.dirs` supplies the Files sidebar's Desktop, Downloads, Documents… places, and the portal's screenshot folder | `components/otto-foundations/src/xdg.rs` (`user_dirs`), used by `components/otto-files/src/model.rs` |
| [Cursor theme (Xcursor)](https://www.freedesktop.org/wiki/Specifications/cursor-spec/) | Read. `XCURSOR_THEME` / `XCURSOR_SIZE`, loaded per named cursor and scale | `src/cursor.rs` |
| [Fontconfig](https://www.freedesktop.org/wiki/Software/fontconfig/) | Read, indirectly. Skia's `FontMgr` on Linux is the fontconfig-backed one, so family lookup, aliases (`sans-serif`) and the user's `fonts.conf` resolve normally; rendering preferences (`antialias`, `hinting`, `hintstyle`, `autohint`, `rgba`) are read per family; see [Fonts and text](#fonts-and-text) | `components/otto-kit/src/text_rendering.rs`, `typography.rs`, `src/utils/font_cache.rs` |

### Fonts and text

There is no XDG typography specification, but there is a standards surface
around fonts, and Otto sits on only part of it. Worth knowing before touching
text rendering:

**What comes for free.** The binary links `libfontconfig`, because
`skia::FontMgr::new()` on Linux is `SkFontMgr_New_FontConfig`. Family
resolution, aliases and the user's `~/.config/fontconfig/fonts.conf` therefore
apply to `font_family` without Otto doing anything. This is inherited, not
chosen. A Skia built against a different font host would silently change it.

**What is deliberately different.** Skia draws a string with exactly one
typeface and does no per-glyph fallback, so Otto substitutes a *whole-interface*
covering face rather than the per-run fallback fontconfig would arrange. The
reasoning is in the `covering_typeface` doc comment in
`components/otto-kit/src/typography.rs`: chrome in one language wants one face,
not two disagreeing about metrics halfway along a label. The cost is that a
genuinely mixed-script string can still fall back to boxes.

**Scope, before any of this is weighed.** Client text is rasterised by the
client and reaches Otto as a finished buffer, so none of what follows touches
it. A browser or terminal honours the user's fontconfig in full, and Otto
composites the result without knowing text is in it. What is affected is text
Otto rasterises: compositor chrome (dock, bar, titlebars, switcher, exposé) and
the otto-kit applications, which are separate Wayland clients but draw through
`otto_kit::typography` rather than a toolkit. That share is small today and
grows with each first-party app.

**How it is rasterised.** `otto_kit::text_rendering` asks fontconfig the way
Pango and Chromium do (substitute the pattern, then match, so rules targeting
either apply) and maps `antialias`, `hinting`, `hintstyle`, `autohint` and
`rgba` onto every `Font` the font caches build, and all of them but
`autohint` onto every paragraph `TextStyle` the theme hands out (Skia's
`TextStyle` has no force-autohint switch, so paragraph text ignores it). Subpixel glyph positioning is kept to no and
slight hinting, since medium and full fit stems to the pixel grid. The answer
is cached per family for the process, so a `fonts.conf` change applies from
the next login. `lcdfilter` is not applied: Skia filters LCD glyphs with its
own default and has no per-font control.

Every antialiased font *requests* subpixel text, and the surface decides:
fontconfig's `rgba` takes part in which outputs declare a layout (below), not
in the font, so `rgba = none` still turns it off except on a display set up
for it. Skia draws per-channel coverage only
into a surface that declares a pixel geometry, and only an opaque one may,
since per-channel coverage blended through one alpha shows as colour fringes.
Where geometry is declared is covered under *Per-output pixel geometry* below.

Compositor chrome is drawn through lay-rs, and its text gets the output's
layout wherever it lands on that output's buffer:

- **Recorded content** (a layer's draw closure, kept as a picture) is
  rasterised when it is played back, with the target's layout. The
  server-side titlebar — `Titlebar` → `Label` → `styles::BODY_EMPHASIZED` in
  `components/otto-kit/src/components/titlebar/decoration.rs`, the text that
  sits right beside a GTK headerbar drawn subpixel by the client — the dock's
  tooltips and menu, the app switcher, exposé's title plates and the
  workspace selector all take this path.
- **Fades** are groups, which Skia draws greyscale. lay-rs's `save_fade`
  starts the group from the target when the target declares a layout (and is
  therefore opaque) and keeps the layout, so text does not change weight
  when a menu or a window finishes fading in. A frosted layer's fade group
  already starts from the target and keeps it too.
- **Image-cached layers** are rasterised into transparent offscreens and stay
  greyscale: dock badges and the workspace selector's thumbnails.

**Per-output pixel geometry.** An output's subpixel layout is the display
profile's `subpixel` key (the Settings app's Displays pane writes it); else
none when fontconfig's `rgba` is `none`, the user's own switch; else what the
connector reports; else fontconfig's `rgba` (`src/udev/device.rs`). It lands
in the `Output`'s physical properties, so clients read the same value
from `wl_output`. Around each output's frame the udev backend hands it to the
renderer (`SkiaDeviceRenderer::set_target_pixel_geometry`), which declares it
on that output's swapchain buffers and nothing else: plane slots, screencast
and offscreen targets keep `PixelGeometry::Unknown`, because they are blended
after Otto draws them and per-channel coverage does not survive a single-alpha
blend. `surface_props` in `crates/otto-render/src/renderer/skia_surface.rs`
carries the reasoning. Most connectors report `Unknown`, so without the
profile key text stays grayscale. The Vulkan renderer, the default, draws
everything into the swapchain. The GL renderer with plane decomposition on
draws chrome into plane slots, which stay greyscale: a slot is transparent
wherever nothing is drawn and the display engine blends it with one alpha,
so only what falls back to the swapchain gets the layout there.

otto-kit apps read the same layout from `wl_output` and draw subpixel text
only on a surface that is opaque all over — its opaque region, set with
`Window::set_opaque_region`, covers the whole surface — shown pixel for pixel
on one output (`BaseWaylandSurface::pixel_geometry`). A pixel reaches the
compositor with one alpha, so on a frosted sidebar or a translucent bar the
per-channel coverage would fringe; every such surface draws grayscale.

**What Otto publishes.** Otto's UI font and its text rendering go out to the
toolkits that rasterise their own text, so GTK, Qt and X11 applications draw
like Otto beside them. `Otto::font_settings` in `src/state/mod.rs` builds them
once for both channels:

- the font is `font_family` at `styles::BODY` size, as a Pango description in
  points at 96 DPI (`otto_kit::text_rendering::font_name`): Otto's 13 px body
  text is `Inter 9.75`, the same height in a GTK label as in Otto's chrome;
- the rendering is `TextRendering::system()`, fontconfig's answer with no
  family named, mapped by `TextRendering::published`;
- the subpixel order is the outputs' when every output that knows its layout
  names the same one (X11 has one screen, the portal one value), else
  fontconfig's `rgba`. It goes out only where Otto itself would draw subpixel
  text: outputs with no layout (`rgba = none` in `fonts.conf` gives them none
  unless a display's own setting says otherwise), or no known order at all,
  publish grayscale.

| Channel | Keys |
|---------|------|
| XSETTINGS (`apply_xwayland_xsettings`) | `Xft/Antialias` (0/1), `Xft/Hinting` (0/1), `Xft/HintStyle` (`hintnone`…`hintfull`), `Xft/RGBA` (`none`, `rgb`, `bgr`, `vrgb`, `vbgr`), `Gtk/FontName` |
| Settings portal, `org.gnome.desktop.interface` | `font-name`, `font-antialiasing` (`none`, `grayscale`, `rgba`), `font-hinting` (`none`…`full`), `font-rgba-order` (`rgb` when unknown: the key has no "none"), `font-rendering` (always `manual`) |

Who reads which channel, and whether the hint style then shows in their
pixels (measured with GTK 3.24.52, GTK 4.22, Qt 6.11 and Chromium, each
started with a fontconfig forcing one style):

| Application | Font and colour scheme | Rasterisation | Hint styles differ? |
|-------------|------------------------|---------------|---------------------|
| Otto, otto-kit apps | config | fontconfig | yes; medium draws as full (FreeType's one target for both) |
| GTK 4 on Wayland | Settings portal | portal keys, with `font-rendering` `manual` | yes |
| GTK 3 on Wayland, unsandboxed | GSettings, which Otto writes | GSettings | in grayscale yes; with `rgba` antialiasing cairo hints every style alike |
| GTK 3 sandboxed, or `GTK_USE_PORTAL=1` | Settings portal | the same keys from the portal | as above |
| libadwaita (colour scheme) | GSettings `color-scheme` | — | — |
| Qt 6 on Wayland | its `gtk3` platform theme, which Otto names (so GSettings) | fontconfig | only at device pixel ratio 1 |
| Chromium, Electron | GTK 3 (GSettings) | fontconfig | yes |
| X11 clients under Xwayland | XSETTINGS | XSETTINGS `Xft/*` | Qt: only at 96 DPI |

GTK 4 reads the portal, and since 4.16 it ignores the hinting, antialiasing
and order keys unless `font-rendering` is `manual`: in its default,
`automatic`, it picks its own. The portal adds `manual` to the compositor's
keys for that reason. GTK 3 on Wayland reads `org.gnome.desktop.interface`
from GSettings (dconf) instead, so a session of Otto's own (the udev backend,
never a nested one) writes the same five keys there at startup with
`gsettings`, only where they differ (`src/gnome_interface.rs`). A key the
installed schema lacks is skipped: `font-rendering` exists from GNOME 47. libadwaita
outside a sandbox likewise takes the colour scheme from GSettings'
`color-scheme`, not the portal, so the same module writes `prefer-dark` or
`default` there at startup and whenever `theme_scheme` changes.

Qt chooses a platform theme by `XDG_CURRENT_DESKTOP`, and for a name it does
not know, `otto` among them, it falls back to its generic theme: `Sans Serif
9`, no colour scheme. So Otto names Qt's own `gtk3` theme in
`QT_QPA_PLATFORMTHEME` for everything it spawns and for the session's
services, unless the variable is already set (`src/qt_platform_theme.rs`);
that theme takes the font (as `Inter 9`: it keeps whole points) and the
colour scheme from what Otto wrote to GSettings, the scheme live. Hinting
and antialiasing Qt reads from fontconfig itself, like Otto, with one rule of
its own (`qfontconfigdatabase.cpp`, `isDprScaling`): at a device pixel ratio
other than 1 every font is drawn unhinted and antialiased whatever fontconfig
says, so on a scaled output Qt text never follows the hint style, under
Wayland or at a scaled `Xft/DPI` under Xwayland alike. Chromium takes its
rasterisation from fontconfig too, and its font from GTK 3.

The portal does not link otto-kit or fontconfig: it asks the compositor
(`org.otto.Settings.GetFontSettings`, see
[settings-dbus-api.md](settings-dbus-api.md)), which works the values out on
its own thread because the subpixel order comes from the outputs. The
values are worked out afresh on every portal read and every XSETTINGS send,
so a client that asks later gets the current outputs' order, but no change
is ever pushed: the portal emits no `SettingChanged` for the font keys, and
GSettings is written once, at startup. `font_family` needs a restart and the
fontconfig answer is cached for the process either way. XSETTINGS goes out
when Xwayland is ready, when the scale changes and when the cursor theme or
size does.

Other desktops also publish `text-scaling-factor`; Otto does not (see Known
gaps).

### Portal namespaces

The Settings portal publishes `org.freedesktop.appearance` (`color-scheme`,
`accent-color`, `icon-theme`), which is how GTK and Qt applications learn
Otto's light/dark state. Keys with no portal-standard home go out under
`org.gnome.desktop.sound` (sound theme), `org.gnome.desktop.interface` (font
and text rendering, see [Fonts and text](#fonts-and-text)) and Otto's own `org.otto.desktop`
namespace rather than being invented inside the freedesktop one. See
[Color Scheme](color-scheme-setting.md).

The window controls' side and the zoom button also go out as
`org.gnome.desktop.wm.preferences button-layout` (`close,minimize:` on the
left, `:minimize,close` on the right, with `maximize` when the zoom button is
shown), so toolkits that draw their own titlebar put their buttons where
Otto's are. GTK4 reads the Settings portal by default and follows it live.
GTK3 outside a sandbox — Chrome's default toolkit among them — reads dconf
directly and only consults the portal under `GTK_USE_PORTAL=1`.

## D-Bus services and other standards

| Standard | Role | Implementation |
|----------|------|----------------|
| [StatusNotifierItem / `StatusNotifierWatcher`](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) | Host. The top bar's system tray registers as the watcher and tracks items | `components/otto-bar/src/tray.rs` |
| [DBusMenu](https://github.com/AyatanaIndicators/libdbusmenu) | Client: menus for tray items that expose no `ContextMenu` method. Server: an otto-kit app's menu bar, and `otto-rdp`'s indicator | `components/otto-bar/src/dbusmenu.rs`, `components/otto-kit/src/app_menu.rs`, `components/otto-rdp/src/indicator.rs` |
| Global menu: `com.canonical.AppMenu.Registrar`, KDE `org_kde_kwin_appmenu` | Server. The top bar shows the focused app's exported menu | `components/otto-bar/src/appmenu.rs`, `src/state/kde_appmenu.rs` |
| [Sound Theme](https://specifications.freedesktop.org/sound-theme-spec/latest/) | Read. `sound_theme` names the theme; event sounds are looked up by name with fallback | `components/otto-kit/src/sound.rs`, `src/audio/sound_player.rs` |
| [MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/) | Client. Media keys and the Dynamic Island's player | `src/audio/media_control.rs`, `components/otto-islands/src/mpris.rs` |
| AT-SPI, `org.freedesktop.a11y` | Server. Chrome and otto-kit apps are AT-SPI apps through AccessKit; the compositor serves `KeyboardMonitor` and `PointerLocator` | `src/a11y/`, `components/otto-kit/src/accessibility/` |
| XDG portal `PermissionStore` | Client. Remembered screencast and notification choices | `components/xdg-desktop-portal-otto/src/portal/remembered.rs` |
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
- **Trash** is implemented by `otto-files`; the compositor only reads whether
  the can is empty. There is no D-Bus trash service.
- **Thumbnails are read only**: Otto uses the shared cache but never adds
  to it.
- **No `org.freedesktop.FileManager1`**: other apps' "Show in folder" does
  not reach Files.
- **Subpixel text in plane slots and cached layers**: compositor chrome gets
  the output's subpixel layout on the output's own buffer, but GL plane slots
  and lay-rs image-cached layers are transparent and draw greyscale. See
  [Fonts and text](#fonts-and-text).
- **Font settings are not pushed**: the font and rendering keys (see
  [Fonts and text](#fonts-and-text)) are current whenever a client reads
  them, but no change signal goes out when an output's subpixel layout
  changes, and the GSettings copy GTK 3 and Qt read is written at startup
  only.
- **No subpixel text on winit or x11**: those backends report no layout for
  their output, so only a login session (udev) draws subpixel text.
- **Chrome ignores output rotation for subpixel**: otto-kit apps turn the
  order with the output transform, the compositor's own chrome does not.
- **Qt text on a scaled output is unhinted**: Qt's own rule at a device pixel
  ratio other than 1, which no setting Otto publishes reaches (see
  [Fonts and text](#fonts-and-text)).
- **No text scaling factor**: text size follows the output scale only, so
  enlarging text means enlarging everything.
