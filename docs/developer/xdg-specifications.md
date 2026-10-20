# XDG specifications

Otto is not a desktop unto itself: it reads the same `.desktop` files, icon
themes, MIME database and trash can that every other Linux desktop uses, and it
speaks the same protocols to clients. This page is the inventory: which
freedesktop.org/XDG specification Otto implements, on which side of it, and
where the code lives.

Two families are covered:

- **XDG Wayland protocols**: the `xdg_*` shell protocols Otto serves to
  clients. For how protocol handlers are wired at all, see
  [Wayland Protocols](wayland.md).
- **XDG desktop specifications**: the file-format and D-Bus standards that let
  Otto and its applications interoperate with the rest of the desktop.

Protocols outside the XDG namespace (`wlr-*`, `ext-*`, `wp-*`, Otto's own) are
listed in [Wayland Protocols](wayland.md), not here.

## Why the coverage is this wide

Otto owns a lot of the desktop itself: file manager, notification display,
tray host, portal backend, launcher. The temptation in that position is to
keep the state somewhere convenient for Otto. The rule here is the opposite:
**anything a user could reasonably expect to configure or share with the rest
of the system goes in the standard location, in the standard format, even when
a private one would be less work.**

Two properties are what that buys, and they are worth the extra implementation:

- **Substitutability.** A user must be able to replace any one piece of Otto
  without replacing Otto. That only holds if the seams are the standard ones:
  a shared trash means another file manager is a drop-in; an
  `org.freedesktop.Notifications` server means `dunst` can take over; layer
  shell means a third-party bar works. It cuts the other way too: Otto's dock
  and bar have to be as replaceable as anything else on the desktop.
- **No relearning.** The user already knows Linux. Their default browser is in
  `mimeapps.list`, their autostart is a `.desktop` file, their icons are in
  `~/.icons`. Otto's settings surface should be an easier path to those same
  places, not a second source of truth beside them. That is why, where Otto
  writes a value at all, it writes it where the rest of the system reads it.

The gaps at the bottom of this page are read against that rule: the
`mimeapps.list` one matters not because the spec is unimplemented but because
it is the one place Otto asks the user to leave and use another tool.

## XDG Wayland protocols

| Protocol | Role | Implementation |
|----------|------|----------------|
| [`xdg_shell`](https://wayland.app/protocols/xdg-shell) | Server. Toplevels and popups, the main window protocol | `XdgShellHandler` in `src/shell/xdg.rs`; state in `src/state/mod.rs` |
| [`xdg-decoration-unstable-v1`](https://wayland.app/protocols/xdg-decoration-unstable-v1) | Server. Negotiates server- vs client-side decorations | `src/state/xdg_decoration_handler.rs` (alongside the KDE `org_kde_kwin_server_decoration` equivalent) |
| [`xdg-activation-v1`](https://wayland.app/protocols/xdg-activation-v1) | Server. Focus transfer between clients | `src/state/xdg_activation_handler.rs` |
| [`xdg-foreign-unstable-v2`](https://wayland.app/protocols/xdg-foreign-unstable-v2) | Server. Exporting a surface so another client can parent to it | `delegate_xdg_foreign!` in `src/state/mod.rs` |
| [`xdg-output-unstable-v1`](https://wayland.app/protocols/xdg-output-unstable-v1) | Server. Logical output geometry for clients | `OutputManagerState::new_with_xdg_output` in `src/state/mod.rs` |
| [`xdg-dialog-v1`](https://wayland.app/protocols/xdg-dialog-v1) | Server. The global is served so clients can mark a toplevel modal; nothing reads the hint | `delegate_xdg_dialog!` in `src/state/mod.rs` |

Otto's own applications sit on the client side of these through `otto-kit`
(`components/otto-kit/src/components/window/`), so the toolkit exercises the
same protocols the compositor serves.

## XDG desktop specifications

| Specification | Role | Implementation |
|---------------|------|----------------|
| [Base Directory](https://specifications.freedesktop.org/basedir-spec/latest/) | Read + write. `XDG_CONFIG_HOME`, `XDG_CONFIG_DIRS`, `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `XDG_CACHE_HOME` are honoured everywhere paths are resolved | `src/config/mod.rs` (config layering), and every consumer below |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry-spec/latest/) | Read. `Name`, `Icon`, `Exec`, `Categories`, `StartupWMClass`, and `[Desktop Action …]` groups; the dock's right-click menu is an entry's own `Actions=` | `components/otto-kit/src/desktop_entry.rs`, `src/workspaces/apps_info.rs`, `components/otto-launcher/` |
| [Desktop Entry — associations (`mimeapps.list`)](https://specifications.freedesktop.org/mime-apps-spec/latest/) | Read. Resolves the default application for a MIME type, following the standard search order | `src/config/default_apps.rs` |
| [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info-spec/latest/) | Read. `mime/globs2` and `mime/subclasses` from every XDG data dir, with the spec's glob precedence; content sniffing on top | `components/otto-kit/src/filetype/` (`db.rs`, `glob.rs`) |
| [Icon Theme](https://specifications.freedesktop.org/icon-theme-spec/latest/) | Read. Theme lookup with `hicolor` fallback, SVG and raster | `components/otto-kit/src/icons.rs`, `src/utils/mod.rs`, via the `freedesktop-icons` crate |
| [Trash](https://specifications.freedesktop.org/trash-spec/latest/) | Read + write. The home trash `$XDG_DATA_HOME/Trash/{files,info}` with a `.trashinfo` sidecar (`Path`, `DeletionDate`), which is what makes Put Back work, and what makes Otto's trash the same trash as Nautilus's or Dolphin's. The sidecar is created first with `O_EXCL` to reserve the name. Files on another filesystem go to that filesystem's `$topdir/.Trash/$uid` (sticky, not a symlink) or `$topdir/.Trash-$uid`, with `Path` relative to `$topdir`; the home trash is the fallback. Trashed folders get a `directorysizes` line (skipped past 10 000 entries). The topdir is the longest mount point above the file in `/proc/self/mountinfo`. A topdir can is used only when it is a real directory owned by the user, not group- or other-writable, with `files/`, `info/` and `directorysizes` real and the user's too, and a shared `.Trash` sticky and root's or the user's (otherwise the next can, finally the home trash); trashing, deleting, Put Back from a topdir can (renamed, never copied, through `O_NOFOLLOW` folders), sidecar and `directorysizes` reads (regular files, size-capped, `O_NONBLOCK`) and the `directorysizes` temporary (`O_EXCL`, `0600`) work from `O_NOFOLLOW` directory descriptors; listing and watching a can go by path, read-only. A topdir sidecar's `Path=` must be relative with no `..` for Put Back. The Trash window, Empty Trash and the dock list the home can plus the checked topdir cans of every mounted filesystem (`/proc/self/mountinfo`, pseudo-filesystems, autofs, network filesystems and FUSE daemons left out; one can seen through two mounts listed once); the dock watches them with inotify off the compositor thread, and the mount table with `poll`, to draw a full bin; the Trash window watches every can's `files/` | `components/otto-kit/src/trash.rs` (trashing, cans), `components/otto-files/src/model.rs` (listing, restore, delete), `src/workspaces/trash.rs` (dock state) |
| [Thumbnail Managing Standard](https://specifications.freedesktop.org/thumbnail-spec/latest/) | Read. `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`, validated by the `Thumb::MTime` PNG text chunk, honouring failure markers under `thumbnails/fail/otto-files/`. Otto writes nothing into the cache. Implemented directly, no dependency | `components/otto-files/src/thumbcache.rs` |
| [Autostart](https://specifications.freedesktop.org/autostart-spec/latest/) | Read. `$XDG_CONFIG_DIRS/autostart` then `$XDG_CONFIG_HOME/autostart`, user entries overriding system ones by filename. Gated on `xdg_autostart` in the config | `Otto::launch_xdg_autostart` in `src/input/actions.rs` |
| [Desktop Notifications](https://specifications.freedesktop.org/notification-spec/latest/) | Server. `org.freedesktop.Notifications`: `Notify`, `CloseNotification`, `ActionInvoked`, capabilities `body`, `body-markup`, `actions`, `persistence`, `icon-static`, `action-icons`. Notifications are drawn by the Dynamic Island | `components/otto-islands/src/notifications.rs` |
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
`rgba` onto every `Font` the font caches build and every paragraph
`TextStyle` the theme hands out. Subpixel glyph positioning is kept to no and
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

GTK 4 on Wayland reads these from the portal, and since 4.16 it ignores the
hinting, antialiasing and order keys unless `font-rendering` is `manual`: in
its default, `automatic`, it picks its own. The portal adds `manual` to the
compositor's keys for that reason. GTK 3 on Wayland reads
`org.gnome.desktop.interface` from GSettings (dconf) instead, which Otto does
not write, and Qt's own platform themes read neither.

The portal does not link otto-kit or fontconfig: it asks the compositor
(`org.otto.Settings.GetFontSettings`, see
[settings-dbus-api.md](settings-dbus-api.md)), which works the values out on
its own thread because the subpixel order comes from the outputs. None of
these is relayed live: `font_family` needs a restart, the fontconfig answer is
cached for the process, and an output plugged in later does not re-publish
the order. XSETTINGS is re-sent whenever the scale is.

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

## Adjacent standards (not freedesktop-hosted)

| Standard | Role | Implementation |
|----------|------|----------------|
| [StatusNotifierItem / `StatusNotifierWatcher`](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) | Host. The top bar's system tray registers as the watcher and tracks items | `components/otto-bar/src/tray.rs` |
| [DBusMenu](https://github.com/AyatanaIndicators/libdbusmenu) | Client. Menus for tray items that expose no `ContextMenu` method | `components/otto-bar/src/dbusmenu.rs` |

## Known gaps

- **Recent files** (`recently-used.xbel`) is neither read nor written. Files'
  Recent place is built by walking the XDG user directories by modification
  time instead, so it does not see what other applications opened.
- **Startup notification**: `StartupWMClass` is used for matching windows to
  entries, but Otto does not emit startup-notification IDs of its own.
- **Trash** is implemented by `otto-files`; the compositor only reads whether
  the can is empty. There is no D-Bus trash service.
- **`mimeapps.list`** is read only: Otto never writes a default association
  back.
- **Subpixel text in plane slots and cached layers**: compositor chrome gets
  the output's subpixel layout on the output's own buffer, but GL plane slots
  and lay-rs image-cached layers are transparent and draw greyscale. See
  [Fonts and text](#fonts-and-text).
- **Font settings are published once**: the font and rendering keys (see
  [Fonts and text](#fonts-and-text)) are not re-announced when an output's
  subpixel layout changes, and GTK3 outside a sandbox reads dconf rather than
  the portal, so it sees only the XSETTINGS copy under Xwayland.
- **No text scaling factor**: text size follows the output scale only, so
  enlarging text means enlarging everything.
