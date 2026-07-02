# KMS Plane Scanout & Cross-Plane Backdrop Blur

**Status:** draft
**Related specs:** workspaces-multi-output.md

## Summary

Otto splits the scene into per-purpose buffers that are handed to KMS planes
directly (background, windows, expose, overlay UI), so an idle or
partially-changing desktop is composed by the display engine instead of the
GPU. Blur-bearing UI (dock, app switcher, OSD, menus) still shows correct
vibrancy even though the content behind it lives on other planes.

## Goals

- The background, workspace windows, expose view, and overlay UI each render
  into their own scanout-capable buffer; Smithay's DrmCompositor assigns them
  to hardware planes front-first and falls back to GPU compositing per
  element when the kernel rejects a plane.
- A buffer is re-rendered only when damage is recorded under its subtree; an
  idle desktop produces zero re-renders and zero page-flips.
- The topmost non-animating window is offered for direct scanout of its
  client buffer (shadow rendered separately by the windows buffer).
- `BackgroundBlur` layers in the overlay subtree sample a composite of the
  planes below them (background + windows/expose), so vibrancy reflects real
  content even across buffer boundaries.
- The blur composite is rebuilt only when a lower plane recorded damage; a
  rebuild triggers exactly one re-render of each blur-bearing plane. The
  composite is downscaled (currently 1/4 resolution) — blur re-downscales
  its input anyway, so a low-res backdrop is imperceptible but far cheaper.
- Per-buffer damage is reported tightly (FB_DAMAGE_CLIPS) so PSR
  partial-refresh works on eDP; a backdrop change falls back to full-buffer
  damage.

## Non-Goals

- Including direct-scanout client buffers in the blur composite (windows
  under the blur region are demoted to the windows buffer instead).
- Multi-output correctness of the blur composite (assumes the output's
  subtree origin is scene origin; multi-output is untested).
- Tier probing via TEST_ONLY commits — plane acceptance is delegated to
  Smithay's per-frame assignment and fallback.

## Behavior

- When a frame is scheduled, buffers render bottom-up: background first,
  then windows (or expose while expose is active), then the blur composite,
  then the overlay UI.
- When expose is active, the expose buffer replaces the windows buffer in
  the plane stack; its blur samples the downscaled background-only stage of
  the composite, and the overlay's backdrop then includes expose content.
- When a lower plane records damage, the composite is rebuilt and the
  blur-bearing planes re-render once with the new backdrop (triggered by
  the fresh snapshot's unique id).
- A stable fullscreen workspace (single window, no animation, no capture,
  no swipe) direct-scans the client buffer on the PRIMARY plane with all
  chrome planes dropped; frames always render (client commits produce no
  scene damage) and only the fullscreen window receives frame callbacks.
  The compositor swapchain resets on scanout-mode transitions.
- The 3-finger workspace swipe (finger-drag, before any animation) gates
  all direct scanout: the drag moves content with no animation flag, and a
  fixed plane would not follow it.
- The promoted-window set is capped (currently 1): the hardware admits ~5
  simultaneous planes and bg/windows/dock/cursor take four — a second
  client plane evicts the windows plane, which costs more than
  compositing the extra window.
- Scanout candidate selection uses only STABLE geometry (dock bar bounds,
  app-switcher/OSD view bounds, layer-shell Top/Overlay rects, window
  rects) — never per-frame scene state such as bubbled blur regions, which
  are rebuilt every engine update and oscillate promote/demote (content
  flicker). A window overlapping any of those occluders, owning an open
  popup, animating, or covered by a higher window is not promoted.
- A window leaving the scanout set is re-imported and the scene update is
  re-run that same frame, so the first composited frame shows current
  content (no one-frame shadow-only flicker).
- The background element may only direct-scan the PRIMARY plane. On an
  overlay plane it would stack above the primary swapchain and, being
  opaque and full-output, hide every element that fell back to GPU
  compositing ("empty desktop").
- When a screencopy is pending, the whole scene is GPU-composited for that
  frame (planes bypassed) so the capture sees the complete image.
- If a buffer has no new damage, its existing dmabuf is re-submitted with an
  unchanged commit count, which must result in no page-flip for that plane.

## Constraints & Edge Cases

- Overlapping overlay planes must be blended by the hardware; availability
  is device-dependent (see docs/developer/overlay-scanout-hardware.md).
- Blur baked into the overlay buffer double-blends slightly with the live
  planes below (material is semi-opaque); acceptable by design.
- The first frame after startup renders the overlay without a backdrop (the
  lower buffers don't exist yet); the composite arrives on the next frame.
- Removed scene nodes can't be attributed to a subtree; their damage
  conservatively re-renders all plane buffers that frame.

## Rationale

- Per-frame plane assignment with per-element fallback replaced the
  TEST_ONLY tier probe: the kernel's watermark/format decisions vary per
  frame, so pre-grading was both complex and wrong (see
  reference notes on KMS plane strategy).
- The blur composite is downscaled because the blur re-downscales its input
  anyway; a low-res backdrop is imperceptible after blurring but far cheaper
  to build and hold.
- Demoting blurred-over windows from direct scanout was chosen over
  importing client dmabufs into the composite for simplicity; fullscreen
  (dock hidden → empty blur region) still gets direct scanout, which is the
  case that matters most.

## Open Questions

- Should direct-scanout client buffers be imported into the blur composite
  (restoring scanout for windows under the dock)?
- Multi-output: composite and blur-region coordinates per output subtree.
- Whether hidden (shadow-only) window content still counts damage into the
  windows buffer, causing unnecessary re-renders while direct scanout is
  active.
