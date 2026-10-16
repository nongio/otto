# Wallpaper

**Status:** stable  
**Related specs:** [settings-app.md](./settings-app.md), [multi-output.md](./multi-output.md), [workspaces-multi-output.md](./workspaces-multi-output.md), [workspace-reorder.md](./workspace-reorder.md)

## Summary

The image the desktop shows behind the windows: which one, how it fills each
screen, and the guarantee that it reaches the screen sharp, without aliasing,
and identical however a frame is repainted.

## Goals

- A wallpaper fills its screen in one of six layouts: fill, fit, stretch,
  centre, tile and span.
- The part of a cropped image that shows, and where a smaller one sits, can be
  chosen; a filled or spanned image can be zoomed into.
- One output, or one workspace, can show a wallpaper of its own.
- What reaches the screen is the image resampled once, well, for that screen:
  never aliased, never blocky, never softened per frame.
- Repainting part of the screen (the damage around a moving window) gives
  exactly the pixels a whole-screen paint gives.
- Changing the wallpaper never blocks the desktop.

## Non-Goals

- Slideshows, time-of-day or light/dark variants, animated wallpapers. A
  `wlr-layer-shell` background client (`swaybg`, `swww`, `wpaperd`) covers
  those and draws above this wallpaper.
- Effects on the wallpaper (blur, dimming).
- Colour management beyond converting the image's embedded profile to sRGB.

## Behavior

### Settings

- `background_image` is the image; empty means none. `background_color` is the
  colour of the gradient shown without an image, and of what shows around a
  fitted or centred image or through a transparent one.
- `background_mode` is the layout:
  - `fill` (the default): scaled to cover the screen, the excess cropped.
  - `fit`: scaled to fit inside the screen, whole, the colour around it.
  - `stretch`: the screen's exact shape, ignoring the image's aspect ratio.
  - `center`: at its own size, the colour around it or cropped.
  - `tile`: repeated at its own size from the screen's top-left corner.
  - `span`: one image across every screen, filled over the box around all of
    them; each screen shows its own part.

  An unknown value is `fill`.
- `background_position_x` and `background_position_y` are fractions from 0 to
  1 and work as CSS `object-position`: the image point at that fraction lands
  at the same fraction of the screen. `0.5, 0.5` centres; `0.5, 0.0` keeps the
  top edge of a cropped image, or puts a smaller one against the top. They
  apply to fill, fit, centre and span; values outside 0 to 1 are clamped.
- `background_zoom`, from 1 to 8, magnifies a filled or spanned image beyond
  just covering, panned by the position. Values outside the range are clamped.
- In centre and tile an image pixel is a logical point, so the image is the
  same size on every screen whatever its scale.
- All six settings apply live over `org.otto.Settings`.

### Overrides

- `[background_outputs."<output>"]` overrides any of the six for one output,
  with the same names minus the `background_` prefix (`image`, `color`,
  `mode`, `position_x`, `position_y`, `zoom`).
- `[workspaces.entries."<output>:<position>".background]` overrides them for
  one workspace, over its output's.
- What a workspace shows is the global settings, then its output's override,
  then its own, field by field. An empty `image` in an override means no
  image.
- A workspace's override travels with it when the workspaces are reordered.

### Commands

- `output <name>|* bg|background <file> [<mode>] [<#colour>]` sets an
  output's override, or with `*` the global settings, as sway's command does;
  `output <name>|* bg <#colour> solid_color` sets no image and that colour;
  `output <name>|* bg reset` removes the override (or resets the global
  settings).
- `background <file> [<mode>] [<#colour>]`, `background <#colour>
  solid_color` and `background reset` do the same for the focused workspace.
- A mode or colour left out keeps what is set. The path is taken as written,
  spaces included; a `;` in it needs the path in double quotes. `~/` is the
  home folder; any other relative path is refused, since it would mean a
  different file at the next login.
- An image that cannot be opened, or is not an image, is refused with the
  reason, and nothing is written.
- Every change is written to the configuration file, leaving the rest of the
  file as it was.

### What reaches the screen

- Each screen gets a frame exactly its size: the image laid out, resampled
  once with a kernel that widens with the ratio (so shrinking averages every
  source pixel, and fine detail does not alias), filtering linear light, with
  the colour behind. Edges of a fitted or centred image are on whole pixels;
  where an image lands 1:1 along an axis it is moved onto a whole pixel and
  its pixels are copied, not resampled.
- The frame is drawn 1:1, so every screen pixel is one frame pixel and no
  filtering happens per frame. A repaint cropped to any rect gives exactly the
  pixels of a whole-screen paint.
- Copies of the wallpaper drawn small (the overview's backdrop, the workspace
  selector's previews) are minified with mipmaps and do not alias.
- An SVG is drawn as vectors at its own aspect ratio.
- The camera's orientation tag (EXIF) and an embedded colour profile are
  honoured.
- Frames are rendered off the compositor's thread. Until a new one is ready a
  workspace keeps showing what it showed; with nothing shown yet it shows its
  black backing rather than flash the gradient.
- One frame serves every workspace that shows the same image, layout and
  colour on a screen of the same size.

### The lock screen and the greeter

- They show the global wallpaper in the global layout, as the desktop does.
  An output's or a workspace's own wallpaper does not reach them.

### Settings app

- The background image row's preview is the primary screen, in its shape,
  showing the wallpaper as the desktop lays it out with the current layout,
  position, zoom and colour.
- Clicking the preview, in a layout that has a position, sets the position to
  the point clicked, which then stays in view.
- Layout is a pop-up; zoom and the two positions are sliders, greyed out in a
  layout that does not use them.

## Constraints & Edge Cases

- An unreadable image, or a file that is not an image, shows the gradient and
  is reported to the settings app or the command that set it. A file that
  looks like an image but cannot be decoded shows the gradient and is logged.
- Only a failure of the image a change sets refuses that change: an output's
  or a workspace's own image that has gone missing does not block a change to
  the global settings.
- A file rewritten in place is decoded again on the next reload.
- A mode or scale change re-renders the frames for that screen.
- A span is laid out on the box around every screen in logical points, so
  screens of different scale meet without a seam; a screen's gap or offset in
  the layout is part of the box.

## Rationale

- **1:1 frames, not a scaled shader.** A wallpaper scaled at draw time puts
  pixel centres between texels; where one lands exactly on the boundary,
  nearest sampling picks a side by rounding, which differs between a whole
  paint and a cropped one, and between the two triangles of a rect. That is
  the shimmer along a diagonal from a dragged window (nongio/otto#361). A frame
  drawn 1:1 has no boundary to land on, on any GPU.
- **Resampling on the CPU, once.** Skia's per-draw cubic reads a fixed 4x4
  block, which aliases past 2x reduction; a widened kernel does not, and is
  paid once per change, off-thread.
- **Linear light.** Averaging encoded sRGB darkens fine contrast (1 px black
  and white stripes average to 128 instead of the 188 that is half the light).
- **`object-position` for the position.** One rule for cropped and smaller
  images, always valid, never uncovering the screen, and familiar.
- **Pixels as points in centre and tile.** An image looks the same size on a
  HiDPI screen as on a normal one, as on macOS.
- **sway's `output … bg`.** Scripts port unchanged; the modes are sway's.

## Open Questions

- Per-output and per-workspace wallpapers have no control in the settings app
  yet; they are set through the configuration file or the commands.
- The lock screen could follow an output's own wallpaper on that output.
