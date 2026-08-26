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
- Works for toplevels, popups, subsurfaces and layer-shell surfaces alike.
- Real clients work unmodified: foot (`blur=yes`), wezterm
  (`wayland_window_background_blur`), ghostty (`background-blur`) on builds
  that speak this protocol.

## Non-Goals

- Honouring the region's *shape*. The region is a switch: any region with area
  blurs the whole surface, an empty one blurs nothing. See *Rationale*.
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
  positive area, the surface's backdrop is blurred from that frame on. Where
  the client's buffer is translucent, the blurred backdrop shows through; where
  it is opaque, nothing visible changes.
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

- **Region as a switch.** Otto's blur is a per-layer effect over the layer's
  (rounded) bounds; carving it to arbitrary rectangles would need a new
  masking path in the scene engine. Every known client asks for its whole
  surface, so the switch gives the right result today and leaves partial
  regions as a future refinement rather than a blocker.
- **Same machinery as otto-surface-style.** Reusing the blend mode that
  otto-kit windows use means the plane backdrop seeding, the material-aware
  scanout rules and the decoration blur all apply for free, and a foot window
  looks exactly like an otto-kit popup.

## Open Questions

- Should partial regions be honoured once the scene engine can mask a blur to
  a set of rectangles?
