# Text rendering

How Otto rasterises the text it draws itself (compositor chrome and the
otto-kit apps), and what it tells the toolkits that draw their own text, so
GTK, Qt and X11 applications look like Otto beside them. What a user sets is
in [Theming](../user/theming.md#fonts) and
[Display](../user/display.md#display-profiles); the fontconfig row of the
standards inventory is [XDG specifications](xdg-specifications.md).

## Fontconfig

The binary links `libfontconfig`, because
`skia::FontMgr::new()` on Linux is `SkFontMgr_New_FontConfig`. Family
resolution, aliases and the user's `~/.config/fontconfig/fonts.conf` therefore
apply to `font_family` without Otto doing anything. This is inherited, not
chosen. A Skia built against a different font host would silently change it.

Skia draws a string with exactly one
typeface and does no per-glyph fallback, so Otto substitutes a *whole-interface*
covering face rather than the per-run fallback fontconfig would arrange. The
reasoning is in the `covering_typeface` doc comment in
`components/otto-kit/src/typography.rs`: chrome in one language wants one face,
not two disagreeing about metrics halfway along a label. The cost is that a
genuinely mixed-script string can still fall back to boxes.

Client text is rasterised by the
client and reaches Otto as a finished buffer, so none of what follows touches
it. A browser or terminal honours the user's fontconfig in full, and Otto
composites the result without knowing text is in it. What is affected is text
Otto rasterises: compositor chrome (dock, bar, titlebars, switcher, exposé) and
the otto-kit applications, which are separate Wayland clients but draw through
`otto_kit::typography` rather than a toolkit. That share is small today and
grows with each first-party app.

## How it is rasterised

`otto_kit::text_rendering` asks fontconfig the way
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
Where geometry is declared is covered under [Per-output pixel geometry](#per-output-pixel-geometry).

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

## Per-output pixel geometry

An output's subpixel layout is the display
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

## What Otto publishes

Otto's UI font and its text rendering go out to the
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
installed schema lacks is skipped: `font-rendering` exists from GNOME 47. The same
module writes the colour scheme for libadwaita; see [Color Scheme](color-scheme-setting.md).

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
[Settings D-Bus API](settings-dbus-api.md)), which works the values out on
its own thread because the subpixel order comes from the outputs. The
values are worked out afresh on every portal read and every XSETTINGS send,
so a client that asks later gets the current outputs' order, but no change
is ever pushed: the portal emits no `SettingChanged` for the font keys, and
GSettings is written once, at startup. `font_family` needs a restart and the
fontconfig answer is cached for the process either way. XSETTINGS goes out
when Xwayland is ready, when the scale changes and when the cursor theme or
size does.

Other desktops also publish `text-scaling-factor`; Otto does not (see
[Known gaps](#known-gaps)).

## Known gaps

- **Subpixel text in plane slots and cached layers**: compositor chrome gets
  the output's subpixel layout on the output's own buffer, but GL plane slots
  and lay-rs image-cached layers are transparent and draw greyscale.
- **Font settings are not pushed**: the font and rendering keys are current whenever a client reads
  them, but no change signal goes out when an output's subpixel layout
  changes, and the GSettings copy GTK 3 and Qt read is written at startup
  only.
- **No subpixel text on winit or x11**: those backends report no layout for
  their output, so only a login session (udev) draws subpixel text.
- **Chrome ignores output rotation for subpixel**: otto-kit apps turn the
  order with the output transform, the compositor's own chrome does not.
- **Qt text on a scaled output is unhinted**: Qt's own rule at a device pixel
  ratio other than 1, which no setting Otto publishes reaches.
- **No text scaling factor**: text size follows the output scale only, so
  enlarging text means enlarging everything.
