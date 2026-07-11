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
- The composite is blurred **once, as a whole image**, before it is handed to
  the consumers; each `BackgroundBlur` layer then seeds the pre-blurred image
  directly and skips its own blur pass. Blurring within a layer's clipped
  (rounded) shape samples transparent pixels at the shape edge, leaving a
  faded rim that re-exposes the raw seed — a whole-image blur has no such edge,
  so the frosted panel keeps a crisp boundary. It is also cheaper (one blur per
  frame instead of one per consumer). The `ExternalBackdrop { image, scale,
  blurred }` handed to a layer carries a `blurred` flag; in-scene blur
  consumers with no external backdrop (context menus, OSD) still run the real
  blur against live scene content.
- The blur composite is rebuilt only when a lower plane recorded damage
  that intersects an active blur consumer's region (the dock strip, the
  switcher strip, or the full output while overlay UI or expose is shown);
  a rebuild triggers exactly one re-render of each blur-bearing plane. The
  composite is downscaled (currently 1/4 resolution) — a low-res backdrop is
  imperceptible after blurring but far cheaper.
  Damage skipped this way marks the composite dirty so a later-activating
  consumer still gets fresh content.
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

- The decomposition is enabled per output at surface creation, only when
  the driver is atomic, at least 3 overlay planes exist after
  driver-specific filtering (NVIDIA's are vetoed), and the output renders
  on the primary GPU. Otherwise — and whenever the background plane has no
  dmabuf at frame time (allocation/Skia-surface failure) — the output
  renders as a single full-scene element, which is the pre-decomposition
  path; the plane machinery would pay all its intermediate renders and
  then GPU-composite every buffer anyway.
- When a frame is scheduled, buffers render bottom-up: background first,
  then windows (or expose while expose is active), then the blur composite,
  then the overlay UI.
- A plane render clips to the damage accumulated since its swapchain slot
  last rendered (slots rotate, so a reacquired slot is several commits
  old); only the clip region is cleared and redrawn. Full render on a
  slot's first use, when the damage history no longer reaches the slot's
  commit, or when the backdrop changed (blur can repaint anywhere).
- A buffer's damage is expanded to the full bounds of any `BackgroundBlur`
  shape it contains that the damage reaches: because a blur samples a
  neighborhood of its input, damage under (or within a blur radius of) a
  blur shape changes the blurred result across the shape, and repainting
  only a sub-rect leaves a visible seam where fresh and stale blur meet. So
  the subtree damage query joins the whole blur shape's bounds (the reach
  test is outset by the blur sigma so damage just outside the shape still
  triggers it). This matches the whole-scene expansion. (The blur layer's
  own damage is already sigma-outset when it is drawn.)
- Plane renders submit to the GPU without a CPU-blocking wait on devices
  with implicit dmabuf fencing (the kernel's atomic commit waits on the
  buffer's reservation fences); on the NVIDIA proprietary driver, which
  has no implicit fencing, the CPU wait is kept.
- A promoted (scanned-out) window's commits skip the scene import
  entirely — importing would only re-render its hidden content layer into
  the windows plane. The skip sets a pending flag that forces the next
  frame to draw, since a skipped import produces no scene damage and the
  client's new buffer must still reach its plane. The window's drop-shadow
  still renders in the windows plane, so the skip path keeps the shadow's
  geometry in sync with the window's current rect (tile/resize) without
  re-importing the surface tree; otherwise the shadow ghosts at the
  pre-change size while the content buffer tiles on its plane.
- When expose is active, the expose buffer replaces the windows buffer in
  the plane stack; its blur samples the downscaled background-only stage of
  the composite, and the overlay's backdrop then includes expose content.
- When a lower plane records damage, the composite is rebuilt and the
  blur-bearing planes re-render once with the new backdrop (triggered by
  the fresh snapshot's unique id).
- A stable fullscreen workspace (single window, no animation, no capture,
  no swipe, no mapped popup — the overlay plane holding popups is dropped
  in this mode) direct-scans the client buffer on the PRIMARY plane with all
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
  flicker). A window overlapping any of those occluders, owning a MAPPED
  popup (mapped, not merely alive — GTK keeps closed popovers' surfaces
  around for reuse), animating, or covered by a higher window is not
  promoted.
- Only windows whose current buffer is a dmabuf are promoted. An SHM
  client (e.g. a CPU-rendered terminal) can never scan out: its element
  would GPU-composite anyway and, being in front, demote every plane
  below it — a net loss over leaving it in the windows plane.
- A window whose surface tree contains subsurfaces (e.g. the SSD
  decoration strips: titlebar, buttons, borders) is promoted in
  "base-only" mode: only the ROOT surface's client buffer is pushed as a
  plane candidate, and only the root surface's draw content is blanked in
  the windows plane — the decoration subsurface layers keep rendering
  there. The decorations never overlap the root surface's rect (titlebar
  above, borders around), so outside the client element the windows plane
  shows through and no cross-plane blending issue exists. Pushing the
  whole tree instead would explode into many mutually-overlapping plane
  candidates that lose the plane auction, GPU-composite, and demote every
  plane below them (z-order). The demotion re-import restores the root
  surface's draw content.
- A window leaving the scanout set is re-imported and the scene update is
  re-run that same frame, so the first composited frame shows current
  content (no one-frame shadow-only flicker).
- The background element may only direct-scan the PRIMARY plane. On an
  overlay plane it would stack above the primary swapchain and, being
  opaque and full-output, hide every element that fell back to GPU
  compositing ("empty desktop").
- When a screencopy is pending, the frame is built from the same plane
  elements but with overlay/primary scanout flags dropped, so every element
  GPU-composites into the primary swapchain and the capture blit sees
  exactly the on-screen stack (the cursor keeps its plane and is excluded).
  Re-rendering the scene tree as one element is NOT equivalent: plane
  subtrees render in isolation and ignore ancestor visibility (e.g.
  workspaces_layer is hidden while expose is shown), so a tree re-render
  diverges from what the planes display.
- The compositor swapchain is reset whenever the frame's element mode
  changes (planes ↔ direct scanout ↔ screencopy composite), before the
  transition frame renders: buffer ages recorded in one mode are
  meaningless in another, and a stale age would leave undamaged regions of
  the first frame showing the previous mode's content.
- If a buffer has no new damage, its existing dmabuf is re-submitted with an
  unchanged commit count, which must result in no page-flip for that plane.
- Plane swapchains whose UI has been closed for a while (expose, app
  switcher, overlay chrome; currently 30 s) are dropped to reclaim their
  GPU memory. Allocation is lazy, so the plane is recreated cold on the
  next active frame — which is a UI-opening animation frame anyway.
- Frame callbacks: every mapped window is throttled per its visibility
  class. Windows behind a fullscreen window and windows fully contained
  in a single higher window's geometry get the 2 Hz Occluded trickle —
  never zero (Chromium's buffer-eviction heuristic blanks canvases when
  callbacks stop entirely). Union coverage is deliberately not computed;
  single-window containment cannot false-positive on partial visibility.

## Constraints & Edge Cases

- Overlapping overlay planes must be blended by the hardware; availability
  is device-dependent (see docs/developer/overlay-scanout-hardware.md).
- Blur baked into the overlay buffer double-blends slightly with the live
  planes below (material is semi-opaque); acceptable by design.
- The first frame after startup renders the overlay without a backdrop (the
  lower buffers don't exist yet); the composite arrives on the next frame.
- Removed scene nodes attribute their damage to the nearest surviving
  ancestor, so a window close re-renders only the windows plane. Only
  removals with no surviving ancestor (whole-tree teardown) fall back to
  damaging every plane buffer that frame.
- Moving content into its own plane subtree breaks views that mirrored the
  old parent: the workspace-selector previews replicate `windows_layer` and
  `workspace_background` as two separate mirrors because the wallpaper no
  longer lives under the workspace view.

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
