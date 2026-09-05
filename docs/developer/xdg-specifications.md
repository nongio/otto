# XDG specifications

Otto is not a desktop unto itself: it reads the same `.desktop` files, icon
themes, MIME database and trash can that every other Linux desktop uses, and it
speaks the same protocols to clients. This page is the inventory — which
freedesktop.org/XDG specification Otto implements, on which side of it, and
where the code lives.

Two families are covered:

- **XDG Wayland protocols** — the `xdg_*` shell protocols Otto serves to
  clients. For how protocol handlers are wired at all, see
  [Wayland Protocols](wayland.md).
- **XDG desktop specifications** — the file-format and D-Bus standards that let
  Otto and its applications interoperate with the rest of the desktop.

Protocols outside the XDG namespace (`wlr-*`, `ext-*`, `wp-*`, Otto's own) are
listed in [Wayland Protocols](wayland.md), not here.

## Why the coverage is this wide

Otto owns a lot of the desktop itself — file manager, notification display,
tray host, portal backend, launcher — and the temptation in that position is to
keep the state somewhere convenient for Otto. The rule here is the opposite:
**anything a user could reasonably expect to configure or share with the rest
of the system goes in the standard location, in the standard format, even when
a private one would be less work.**

Two properties are what that buys, and they are worth the extra implementation:

- **Substitutability.** A user must be able to replace any one piece of Otto
  without replacing Otto. That only holds if the seams are the standard ones —
  a shared trash means another file manager is a drop-in; an
  `org.freedesktop.Notifications` server means `dunst` can take over; layer
  shell means a third-party bar works. It cuts the other way too: Otto's dock
  and bar have to be as replaceable as anything else on the desktop.
- **No relearning.** The user already knows Linux. Their default browser is in
  `mimeapps.list`, their autostart is a `.desktop` file, their icons are in
  `~/.icons`. Otto's settings surface should be an easier path to those same
  places, not a second source of truth beside them — which is why, where Otto
  writes a value at all, it writes it where the rest of the system reads it.

The gaps at the bottom of this page are read against that rule: the
`mimeapps.list` one matters not because the spec is unimplemented but because
it is the one place Otto asks the user to leave and use another tool.

## XDG Wayland protocols

| Protocol | Role | Implementation |
|----------|------|----------------|
| [`xdg_shell`](https://wayland.app/protocols/xdg-shell) | Server. Toplevels and popups — the main window protocol | `XdgShellHandler` in `src/shell/xdg.rs`; state in `src/state/mod.rs` |
| [`xdg-decoration-unstable-v1`](https://wayland.app/protocols/xdg-decoration-unstable-v1) | Server. Negotiates server- vs client-side decorations | `src/state/xdg_decoration_handler.rs` (alongside the KDE `org_kde_kwin_server_decoration` equivalent) |
| [`xdg-activation-v1`](https://wayland.app/protocols/xdg-activation-v1) | Server. Focus transfer between clients | `src/state/xdg_activation_handler.rs` |
| [`xdg-foreign-unstable-v2`](https://wayland.app/protocols/xdg-foreign-unstable-v2) | Server. Exporting a surface so another client can parent to it | `delegate_xdg_foreign!` in `src/state/mod.rs` |
| [`xdg-output-unstable-v1`](https://wayland.app/protocols/xdg-output-unstable-v1) | Server. Logical output geometry for clients | `OutputManagerState::new_with_xdg_output` in `src/state/mod.rs` |

Otto's own applications sit on the client side of these through `otto-kit`
(`components/otto-kit/src/components/window/`), so the toolkit exercises the
same protocols the compositor serves.

## XDG desktop specifications

| Specification | Role | Implementation |
|---------------|------|----------------|
| [Base Directory](https://specifications.freedesktop.org/basedir-spec/latest/) | Read + write. `XDG_CONFIG_HOME`, `XDG_CONFIG_DIRS`, `XDG_DATA_HOME`, `XDG_DATA_DIRS`, `XDG_CACHE_HOME` are honoured everywhere paths are resolved | `src/config/mod.rs` (config layering), and every consumer below |
| [Desktop Entry](https://specifications.freedesktop.org/desktop-entry-spec/latest/) | Read. `Name`, `Icon`, `Exec`, `Categories`, `StartupWMClass`, and `[Desktop Action …]` groups — the dock's right-click menu is an entry's own `Actions=` | `components/otto-kit/src/desktop_entry.rs`, `src/workspaces/apps_info.rs`, `components/otto-launcher/` |
| [Desktop Entry — associations (`mimeapps.list`)](https://specifications.freedesktop.org/mime-apps-spec/latest/) | Read. Resolves the default application for a MIME type, following the standard search order | `src/config/default_apps.rs` |
| [Shared MIME-info](https://specifications.freedesktop.org/shared-mime-info-spec/latest/) | Read. `mime/globs2` and `mime/subclasses` from every XDG data dir, with the spec's glob precedence; content sniffing on top | `components/otto-kit/src/filetype/` (`db.rs`, `glob.rs`) |
| [Icon Theme](https://specifications.freedesktop.org/icon-theme-spec/latest/) | Read. Theme lookup with `hicolor` fallback, SVG and raster | `components/otto-kit/src/icons.rs`, `src/utils/mod.rs`, via the `freedesktop-icons` crate |
| [Trash](https://specifications.freedesktop.org/trash-spec/latest/) | Read + write. Files goes through `$XDG_DATA_HOME/Trash/{files,info}` with a `.trashinfo` sidecar (`Path`, `DeletionDate`), which is what makes Put Back work — and what makes Otto's trash the same trash as Nautilus's or Dolphin's. The dock watches the directory with inotify to draw a full bin | `components/otto-files/src/model.rs` (trash + restore), `src/workspaces/trash.rs` (dock state) |
| [Thumbnail Managing Standard](https://specifications.freedesktop.org/thumbnail-spec/latest/) | Read + write. `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of URI>.png`, validated by the `Thumb::MTime` PNG text chunk, with failure markers under `thumbnails/fail/otto-files/`. Implemented directly, no dependency | `components/otto-files/src/thumbcache.rs` |
| [Autostart](https://specifications.freedesktop.org/autostart-spec/latest/) | Read. `$XDG_CONFIG_DIRS/autostart` then `$XDG_CONFIG_HOME/autostart`, user entries overriding system ones by filename. Gated on `xdg_autostart` in the config | `Otto::launch_xdg_autostart` in `src/input/actions.rs` |
| [Desktop Notifications](https://specifications.freedesktop.org/notification-spec/latest/) | Server. `org.freedesktop.Notifications` — `Notify`, `CloseNotification`, `ActionInvoked`, capabilities `body`, `body-markup`, `actions`, `persistence`, `icon-static`, `action-icons`. Notifications are drawn by the Dynamic Island | `components/otto-islands/src/notifications.rs` |
| [XDG Desktop Portal](https://flatpak.github.io/xdg-desktop-portal/docs/) | Backend. `org.freedesktop.impl.portal.{ScreenCast, Settings, Access, Screenshot, FileChooser}`, registered in `otto.portal` | `components/xdg-desktop-portal-otto/src/portal/` |
| [XDG user directories](https://www.freedesktop.org/wiki/Software/xdg-user-dirs/) | Read. `user-dirs.dirs` supplies the Files sidebar's Desktop, Downloads, Documents… places | `components/otto-files/src/model.rs` |
| [Cursor theme (Xcursor)](https://www.freedesktop.org/wiki/Specifications/cursor-spec/) | Read. `XCURSOR_THEME` / `XCURSOR_SIZE`, loaded per named cursor and scale | `src/cursor.rs` |
| [Fontconfig](https://www.freedesktop.org/wiki/Software/fontconfig/) | Read, indirectly. Skia's `FontMgr` on Linux is the fontconfig-backed one, so family lookup, aliases (`sans-serif`) and the user's `fonts.conf` resolve normally — see [Fonts and text](#fonts-and-text) for what is *not* honoured | `components/otto-kit/src/typography.rs`, `src/workspaces/utils/mod.rs` |

### Fonts and text

There is no XDG typography specification, but there is a standards surface
around fonts, and Otto sits on only part of it. Worth knowing before touching
text rendering:

**What comes for free.** The binary links `libfontconfig`, because
`skia::FontMgr::new()` on Linux is `SkFontMgr_New_FontConfig`. Family
resolution, aliases and the user's `~/.config/fontconfig/fonts.conf` therefore
apply to `font_family` without Otto doing anything. This is inherited, not
chosen — a Skia built against a different font host would silently change it.

**What is deliberately different.** Skia draws a string with exactly one
typeface and does no per-glyph fallback, so Otto substitutes a *whole-interface*
covering face rather than the per-run fallback fontconfig would arrange. The
reasoning is in the `covering_typeface` doc comment in
`components/otto-kit/src/typography.rs`: chrome in one language wants one face,
not two disagreeing about metrics halfway along a label. The cost is that a
genuinely mixed-script string can still fall back to boxes.

**Scope, before any of this is weighed.** Client text is rasterised by the
client and reaches Otto as a finished buffer, so none of what follows touches
it — a browser or terminal honours the user's fontconfig in full, and Otto
composites the result without knowing text is in it. What is affected is text
Otto rasterises: compositor chrome (dock, bar, titlebars, switcher, exposé) and
the otto-kit applications, which are separate Wayland clients but draw through
`otto_kit::typography` rather than a toolkit. That share is small today and
grows with each first-party app.

**What is not honoured.** Every font construction site in `typography.rs` and
`src/workspaces/utils/mod.rs` hardcodes `set_subpixel(true)` and
`Edging::SubpixelAntiAlias`, and nothing reads fontconfig's `antialias`,
`hinting`, `hintstyle`, `rgba` or `lcdfilter`. Two separate consequences, and
they need untangling before either is worked on:

*Hinting is live and ignores your config.* Hinting is a `Font` property applied
at glyph rasterization, independent of the surface, so it takes effect — but
Otto never calls `Font::set_hinting`, so glyphs are hinted to Skia's default
rather than to the user's `hintstyle`. This is the part a fontconfig read would
actually fix, and it is self-contained.

*Subpixel AA is requested and then silently dropped.* LCD text requires the
target surface to declare a pixel geometry, and the offscreen surfaces lay-rs
caches layer content into declare none — `gpu::surfaces::render_target(…, None,
…)` in `layers/src/drawing/scene.rs`, `canvas.new_surface(&info, None)` in
`drawing/layer.rs`, and `PixelGeometry::Unknown` in `renderer/skia_fbo.rs`.
They are `AlphaType::Premul` besides, which is the other condition under which
Skia declines LCD text. So in the retained path — which is nearly all chrome —
the `SubpixelAntiAlias` request falls back to grayscale regardless of what the
font asked for.

That second one is the user-visible one: **subpixel antialiasing is effectively
unavailable in Otto's chrome, on any display, with no setting that enables it.**

The sharpest case is the server-side titlebar, and it is worth understanding
before dismissing the whole area as cosmetic. Otto defaults to server-side
decorations, so most windows get a title Otto rasterises — `Titlebar` →
`Label` → `styles::BODY_EMPHASIZED` in
`components/otto-kit/src/components/titlebar/decoration.rs`, into a cached
layer surface, hence grayscale. A GTK client with client-side decorations draws
its headerbar title through its own stack, hinted and subpixel per the user's
fontconfig. The two then sit side by side on one desktop, same position, same
role, same glyph sizes — a controlled comparison the user cannot avoid looking
at. Glyph count is not what makes this visible; adjacency is.
On a HiDPI panel that is close to what you would choose anyway, and is what
GNOME defaults to; on a 1080p panel it reads as lighter, softer text than the
applications running above it. Enabling it means threading `SurfaceProps`
through lay-rs's cached-surface creation — upstream work in the engine, not a
change Otto can make alone — and then answering which output's geometry a
cached picture composited to two panels should use. See the pixel-geometry note
below.

**The `BGRH` hardcode.** `src/renderer/skia_surface.rs` builds both main
surfaces with `PixelGeometry::BGRH` and the comment "for font rendering
optimisations", while `src/udev/device.rs` reads the connector's real subpixel
layout into the `Output` and nothing ever reads it back. Given the paragraph
above, this constant governs only text drawn straight into the main render
target, so it is closer to dead code than to a rendering bug — but it is
misleading as written, and the correct per-output value is already sitting
there unused.

**What Otto does not publish.** The Settings portal carries no font key, and
XSETTINGS publishes only the scaling ones (`Xft/DPI`,
`Gdk/WindowScalingFactor`, `Gdk/UnscaledDPI` — see `apply_xwayland_xsettings`
in `src/state/mod.rs`). Other desktops publish `font-name` and
`text-scaling-factor` under `org.gnome.desktop.interface`, and `Gtk/FontName`
plus the `Xft/*` rendering keys over XSETTINGS. Otto's font choice therefore
stops at Otto's own chrome.

### Portal namespaces

The Settings portal publishes `org.freedesktop.appearance` — `color-scheme`,
`accent-color`, `icon-theme` — which is how GTK and Qt applications learn
Otto's light/dark state. Keys with no portal-standard home go out under
`org.gnome.desktop.sound` (sound theme) and Otto's own `org.otto.desktop`
namespace rather than being invented inside the freedesktop one. See
[Color Scheme](color-scheme-setting.md).

## Adjacent standards (not freedesktop-hosted)

| Standard | Role | Implementation |
|----------|------|----------------|
| [StatusNotifierItem / `StatusNotifierWatcher`](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/) | Host. The top bar's system tray registers as the watcher and tracks items | `components/otto-bar/src/tray.rs` |
| [DBusMenu](https://github.com/AyatanaIndicators/libdbusmenu) | Client. Menus for tray items that expose no `ContextMenu` method | `components/otto-bar/src/dbusmenu.rs` |

## Known gaps

- **Recent files** (`recently-used.xbel`) is neither read nor written.
- **Startup notification** — `StartupWMClass` is used for matching windows to
  entries, but Otto does not emit startup-notification IDs of its own.
- **Trash** is implemented by `otto-files`; the compositor only reads whether
  the can is empty. There is no D-Bus trash service.
- **`mimeapps.list`** is read only — Otto never writes a default association
  back.
- **Font rendering preferences** — fontconfig resolves *which* face is used but
  not *how* it is rasterised: hinting ignores the user's `hintstyle`, and
  subpixel antialiasing is unavailable in the retained path regardless of what
  is configured. See [Fonts and text](#fonts-and-text).
- **Font settings are not published** — no portal font key, no `Gtk/FontName`
  or `Xft/Antialias` in XSETTINGS, so applications do not follow Otto's font.
- **No text scaling factor** — text size follows the output scale only, so
  enlarging text means enlarging everything.
