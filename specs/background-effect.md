# Background Effect (ext-background-effect-v1)

**Status:** draft
**Related specs:** [plane-scanout.md](./plane-scanout.md), [window-decorations.md](./window-decorations.md)

## Summary

Otto implements the standard `ext-background-effect-v1` Wayland protocol so any
client — a terminal with a translucent background, a panel, a launcher — can
ask for the pixels behind its surface to be blurred, without using Otto's own
`otto_surface_style_v1`. The frost it gets is the same one otto-kit apps get.

## Goals

- A client that binds `ext_background_effect_manager_v1` is told the compositor
  can blur.
- A surface that commits a non-empty blur region is rendered over a blurred
  backdrop, on every backend and whether or not the window is on its own KMS
  plane.
- Removing the region (a `NULL` region, or destroying the effect object) takes
  the blur away again on the next commit.
- The blur covers the region the client asked for, not the whole surface: a
  panel that paints a transparent margin around its body keeps the frost inside
  the body.
- Works for toplevels, popups, subsurfaces and layer-shell surfaces alike.
- Real clients work unmodified: foot (`blur=yes`), wezterm
  (`wayland_window_background_blur`), ghostty (`background-blur`) on builds
  that speak this protocol, and fcitx5's `classicui` candidate panel
  (`EnableBlur=True`, with `BlurMask`/`BlurMargin` in the theme).

## Non-Goals

- Honouring a region of *arbitrary* shape. The region is reduced to one rounded
  rectangle — its bounding box plus the corner radius it describes. A region
  that is not a rounded rectangle (an L, a pair of disjoint islands) blurs its
  bounding box. See *Rationale*.
- A client-chosen blur radius, tint or vibrancy. The look is compositor policy
  and matches Otto's own chrome.
- The legacy `org_kde_kwin_blur` protocol. Clients that still use it fall back
  to no blur; the ones that matter have moved or are moving to the standard.

## Behavior

- When a client binds the manager global, the compositor immediately sends
  `capabilities` with the `blur` bit set.
- `get_background_effect` on a surface that already has a live effect object
  raises the `background_effect_exists` protocol error.
- `set_blur_region` is double-buffered: it changes nothing until the surface's
  next `wl_surface.commit` (for a synchronized subsurface, the parent's). The
  region's rectangles are copied at request time; the `wl_region` may be
  destroyed immediately afterwards.
- When a commit carries a region containing at least one added rectangle with
  positive area, the surface's backdrop is blurred from that frame on, over the
  bounding box of those rectangles and rounded to the radius they describe.
  Where the client's buffer is translucent, the blurred backdrop shows through;
  where it is opaque, nothing visible changes. Outside the region the backdrop
  is left alone, so content behind a transparent margin stays legible through
  whatever the client paints there.
- A commit that moves or resizes the region re-applies the blur at the new
  geometry, even though the surface was already blurring.
- The region is trimmed to the surface. "The whole surface" is idiomatically
  spelled as an unbounded region — foot sends `add(0, 0, i32::MAX, i32::MAX)` —
  and the bounds reach the renderer as a rounded rect of their own rather than
  being clipped to the surface's layer, so an untrimmed one frosts a rectangle
  of desktop beside the window.
- Subtractive rectangles are ignored; they can only shrink the region.
- When a commit carries a `NULL` region, or an empty one, the blur is removed
  from that frame on.
- Destroying the effect object removes the blur on the surface's next commit,
  and the surface may obtain a new effect object afterwards.
- `set_blur_region` on an effect whose surface has been destroyed raises the
  `surface_destroyed` protocol error.
- A blurred window is treated as carrying compositor-drawn pixels: it is never
  handed to a KMS plane as a raw client buffer (that would drop the blur), only
  as a re-rendered subtree. A window that is already promoted the raw way when
  its blur arrives is demoted that same frame.
- A surface that also carries an `otto_surface_style_v1` `BackgroundBlur` keeps
  blurring when its background-effect region is removed; the style owns the
  blur too.

## Constraints & Edge Cases

- The blur samples what is actually behind the surface — windows below it,
  not just the wallpaper — on both the composited and the multi-plane path.
- A region set before the surface is first mapped is applied by the mapping
  commit.
- A region whose rectangles all lie outside the surface still counts as
  non-empty; the protocol clips to the surface, and a whole-surface blur is
  the result either way.
- Nothing is re-applied on commits where the effective on/off state did not
  change, so a client committing every frame pays nothing for the protocol.

## Rationale

- **Region as one rounded rectangle.** Otto's blur is a per-layer effect over a
  rounded rect, so the region is reduced to one: `Layer::set_blur_bounds`
  overrides the layer's own bounds with what the client asked for. Reducing it
  to a *switch* instead — blur the whole surface whenever the region has area —
  was the original design, on the assumption that clients ask for their whole
  surface. fcitx5's candidate panel does not: it draws its own drop shadow into
  a transparent margin and asks for the body only. Blurring the whole surface
  then averaged the document behind the margin away to white, and the shadow
  landed on that smear instead of on the window below.
- **Recovering the corner radius.** Clients hand over a rounded rectangle as a
  stack of scanlines. Each inset row is a point on the corner arc, which pins
  the radius: `r = row + inset + sqrt(2 * row * inset)`. Single-row estimators
  are off by a couple of pixels once the client's rasterisation rounds, so
  every inset row votes and the median wins. Without this the frost squares off
  the corners of a rounded panel.
- **Same machinery as otto-surface-style.** Reusing the blend mode that
  otto-kit windows use means the plane backdrop seeding, the material-aware
  scanout rules and the decoration blur all apply for free, and a foot window
  looks exactly like an otto-kit popup.

## Open Questions

- Should genuinely non-rectangular regions be honoured, once the scene engine
  can mask a blur to an arbitrary path rather than one rounded rect? No client
  has been seen asking for one.
