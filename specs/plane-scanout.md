# KMS Plane Scanout & Cross-Plane Backdrop Blur

**Status:** draft
**Related specs:** workspaces-multi-output.md, multi-output.md

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
- The topmost non-animating window is promoted onto a hardware plane of its
  own, by whichever of two tiers fits it (see "Promotion tiers"): its raw
  client buffer when that buffer already describes the finished window, or a
  compositor-rendered buffer of its whole subtree when it does not.
- `BackgroundBlur` layers in the overlay subtree sample a composite of the
  planes below them (background + windows/expose), so vibrancy reflects real
  content even across buffer boundaries.
- The **middle** plane (windows, or exposé while it is up) is a blur consumer
  too — a server-side titlebar blurs what is behind the window. It is handed
  the *background-only* stage of the composite, and its titlebars opt into
  `blur_include_content` so the real blur also picks up the windows the same
  pass painted beneath them. That snapshot is cached and re-taken only when the
  background changes: a consumer re-renders its whole buffer whenever its
  backdrop's `unique_id` changes, so a per-rebuild snapshot would turn every
  window animation into a full-plane redraw. For the same reason the middle
  plane is deliberately *not* part of the rebuild `interest` set — window
  damage is constant and must not drive composite rebuilds.
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
- The external backdrop reaches only layers whose *own* blend mode is
  `BackgroundBlur`. A blur nested inside a **mirrored** subtree does not get it:
  `Layer::as_content()` re-renders the leader's tree with no backdrop, so its
  blur samples the destination canvas — i.e. whatever the mirror's own plane has
  already painted. Anything relying on such a mirror (exposé's window previews,
  whose decorations blur) must have its backdrop content painted into the same
  plane; see [Exposé](../docs/developer/expose.md).
- The whole-image blur carries a vibrancy tone map (saturation boost plus a
  gentle downward gain and a small bias), because skipping the layer's own blur
  pass also skips lay-rs' tone map. Without it a frosted panel over a flat
  white window composites back to white and its boundary disappears; the tone
  map shifts the backdrop a few percent darker and a little more saturated so
  the material stays distinguishable on any background.
- The blur composite is rebuilt only when a lower plane recorded damage
  that intersects an active blur consumer's region, or when a promoted
  window committed a new buffer whose rect intersects such a region
  (promoted commits produce no scene damage, so the commit flag is the only
  change signal); a rebuild triggers exactly one re-render of each
  blur-bearing plane. The consumer regions are: the dock strip, the switcher
  strip, and — for the overlay plane — the layer-shell chrome surfaces'
  rects (top bar, islands) plus the bounds of any mapped popup, each outset
  by the blur sampling radius, in the steady state; the interest widens to
  the full output only while something transient or unbounded is up (expose,
  OSD, tiling overlay, DnD, a selector animation). A steady-state window
  redrawing below the chrome band, or beside an open menu, therefore rebuilds
  nothing.
  Rebuilds caused by desktop damage (bg/middle planes, promoted commits) are
  additionally rate-limited (currently one per 100 ms): a client committing
  full-rect damage at frame rate under a blur consumer must not force the
  composite plus a full-res re-render of every blur plane per commit — blur
  is a low-frequency visual and the dirty flag carries the staleness to the
  next allowed frame. The same limit covers popup repaints: a popup redrawing
  its own content is exactly the frame-rate source the limit exists for. Only
  the first build and a STRUCTURAL popup change — a popup mapping, unmapping,
  becoming visible or moving — bypass it, so a popup's blur is correct on the
  first frame it is visible. The limit is suspended entirely while the user is
  actively driving
  content (expose, a workspace swipe or its settle animation, and — via the
  `pointer_interaction` stamp's 200 ms recency — a window being dragged,
  resized or scrolled) — a 10 Hz blur under a 120 Hz motion reads as
  judder, and those states are transient so they cannot re-open the idle
  rebuild storm. The composite is downscaled (currently 1/4 resolution) — a low-res
  backdrop is imperceptible after blurring but far cheaper.
  Damage skipped this way marks the composite dirty so a later-activating
  consumer still gets fresh content. Frames that bypass the plane path
  entirely while still consuming engine damage (fullscreen direct scanout,
  forced full-GPU composite) also mark the composite dirty, so the first
  planes frame after them rebuilds instead of seeding consumers with stale
  content.
- Per-buffer damage is reported tightly (FB_DAMAGE_CLIPS) so PSR
  partial-refresh works on eDP; a backdrop change falls back to full-buffer
  damage.

- Direct-scanout (promoted) windows are folded into the blur composite by
  blitting their client dmabuf — the same buffer KMS scans out, wrapped
  zero-copy through the renderer's dmabuf import cache — on top of the
  windows-plane snapshot. Their content is not drawn in the scene, so
  without this the topmost window would be absent from every blur backdrop
  (dock bubbles/popups showing pre-window content), and promote/demote
  transitions would visibly flip the blur between with-window and
  without-window composites. Occluder-based demotion (below) still keeps
  windows under the primary chrome strips unpromoted, but overlay UI that
  appears above a promoted window (tooltips, dock popups, islands) relies
  on this fold-in.

## Non-Goals

- Tier probing via TEST_ONLY commits — plane acceptance is delegated to
  Smithay's per-frame assignment and fallback.

## Behavior

- Every plane buffer's rendered content — background, windows, expose,
  overlay UI, dock, switcher, and the cross-plane backdrop composite used
  for blur — is anchored to its own output's top-left corner. This is
  inherent rather than corrected for: every output's scene subtree lives at
  scene coordinate (0,0) and a CRTC's plane elements only ever walk that
  output's own subtree, so an output's placement in the shared (global)
  multi-output layout never affects what its plane buffers render (see
  multi-output.md).
- Direct-scanout promotion is evaluated independently per output: candidates
  are drawn only from that output's own current workspace, and the
  promoted-window cap (see below) applies per output, not globally. Dock and
  OSD occluder rects are only applied on the primary output — that chrome is
  primary-only and never occludes a secondary output's candidates. The
  app-switcher occluder rect is applied on whichever output currently hosts
  the switcher panel, which need not be the primary (see multi-output.md).
  The set of windows actually applied to plane state
  is the union of every output's per-output candidate set, so one output's
  promotion decision cannot demote a window promoted on another output.
- Likewise, an app switcher shown on one output does not block fullscreen
  direct scanout on another: the fullscreen-stability check consults the
  switcher's host output, not its global visibility.
- The dock strip plane follows the dock's configured screen edge: a bottom
  band for `dock.position = "bottom"`, a left or right column otherwise. The
  strip is allocated against that edge, so moving the dock at runtime drops
  and re-allocates the plane.
- The strip's thickness is at least a fixed fraction of the output (a quarter
  of its height for a bottom dock, half its width for a side dock, each
  capped), and grows to the dock's own reach: the configured icon size fully
  magnified, lifted by a launch bounce, with its label balloon open past it,
  plus bar padding. A big dock's bouncing icon must never leave the strip and
  be cropped mid-air. The reach follows the dock configuration (size,
  magnification) live: the plane is re-allocated in place, before the frame
  renders, when it changes.
- The dock and app-switcher strip planes themselves are pushed only to the
  CRTC of the output that actually hosts that chrome — always the primary
  for the dock, the switcher's current host output for the switcher; every
  other output never submits a plane for that role. This is both a
  correctness fix (that chrome has no content on the other outputs) and a
  fetch-bandwidth saving — an otherwise-empty full-width strip plane on a
  secondary output still costs the display engine fetch budget to scan out.
- The overlay plane is pushed on demand — an empty full-screen ARGB buffer
  must not occupy a plane slot — so it is gated on the overlay chrome that is
  actually live: layer-shell Top/Overlay surfaces, popups, the workspace
  selector, OSD, tiling overlay, DnD. Because the layer-shell scene containers
  are primary-only chrome, the primary output's gate counts layer-shell
  surfaces mapped on *any* output: a surface mapped to another output still
  renders into the primary's overlay plane, and gating per-output would leave
  it in a buffer no CRTC ever scans out (invisible until unrelated chrome
  happened to activate the plane).
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
  commit, or when the backdrop changed (blur can repaint anywhere). A
  render that bails after the damage check (no free swapchain slot, export
  or surface failure) forces its next successful render to redraw fully —
  the frame's damage evidence is gone by then (engine damage clears at end
  of frame), and clipping past it would leave the region permanently stale.
- Subtree damage is reported in scene coordinates and mapped into buffer
  coordinates by subtracting only the output's scene origin and the
  element's viewport — never the root node's own `render_position()`. The
  root's position already carries the dynamic workspace-scroll offset, and
  the subtree is drawn with that offset re-applied, so subtracting it would
  double-count: on any workspace but the first, the windows and background
  plane roots sit a full output width to the left and every dirty rect
  would clamp to a sliver at the buffer edge, freezing those planes.
- Damage that falls entirely outside a buffer after that mapping does not
  render it: a window on a workspace scrolled off screen changes nothing
  visible, and rendering would repaint — and report FB_DAMAGE_CLIPS for —
  an edge sliver on every frame it animates. Scrolling that workspace back
  into view damages the moved subtree itself, so it repaints then.
- A buffer's damage is expanded to the full bounds of any `BackgroundBlur`
  shape it contains that the damage reaches: because a blur samples a
  neighborhood of its input, damage under (or within a blur radius of) a
  blur shape changes the blurred result across the shape, and repainting
  only a sub-rect leaves a visible seam where fresh and stale blur meet. So
  the subtree damage query joins the whole blur shape's bounds (the reach
  test is outset by the blur sigma so damage just outside the shape still
  triggers it). This matches the whole-scene expansion. (The blur layer's
  own damage is already sigma-outset when it is drawn.)
- Plane buffers are offscreen EGLImage render targets, which Mesa iris does
  not attach implicit dma-fences to, so the atomic commit does not wait for
  their GL writes on its own — something must, or planes flip half-drawn.
  That wait happens **once per frame**, after the last plane render and
  before the buffers are handed to the DRM compositor: every plane's slot
  surface is built from the renderer's single shared Skia `DirectContext`,
  so one sync covers all of them. The per-plane flushes submit without
  blocking (they still submit rather than merely record, because the
  backdrop composite samples an earlier plane's snapshot from the same
  context and the queued order is the correct order).
  Waiting per plane instead serialised CPU against GPU once per plane, and
  that blocked time — nearly all of a plane render — was the dominant term
  in the frame budget.
  Delivering the fence to KMS as `IN_FENCE_FD` instead would be better
  still, but is not currently possible: smithay populates `PlaneConfig.sync`
  only from a `ScanoutBuffer::Wayland` acquire point, so a dmabuf-backed
  plane element cannot carry a sync point without extending smithay.
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
  The windows buffer is rendered once on the expose entry edge to keep its
  swapchain warm for the expose→windows transition, and not again while
  expose is up: it is not pushed as a plane element there, so per-frame
  re-rendering spent a substantial share of each expose frame on pixels
  that never reach the screen. Content that changed during expose is still correct on
  exit — the damage history no longer reaches the reacquired slot's commit,
  which forces a full re-render on the first composited frame.
- Expose counts as active for the whole of a finger gesture, from
  `expose_gesture_start` until the gesture commits, regardless of what the
  gesture accumulator reads. The accumulator is reset to exactly 0 (or 1000
  when closing) at gesture start, and a fast swipe saturates at the clamp
  and stays there, so a rule based on the accumulator alone reads "not
  transitioning" for those frames while `show_all` is not yet committed —
  the expose plane leaves the stack, the windows plane is pushed, and the
  screen flicks back to the normal layout mid-gesture.
- When a lower plane records damage, the composite is rebuilt and the
  blur-bearing planes re-render once with the new backdrop (triggered by
  the fresh snapshot's unique id).
- A stable fullscreen workspace (single window, no animation, no capture,
  no swipe, no mapped popup — the overlay plane holding popups is dropped
  in this mode) direct-scans the client buffer on the PRIMARY plane with all
  chrome planes dropped; frames always render (client commits produce no
  scene damage) and only the fullscreen window receives frame callbacks.
  The compositor swapchain resets on scanout-mode transitions.
  XWayland fullscreen windows use the same path — this needs the
  clear-color CCS modifiers stripped from advertised dmabuf formats and
  the explicit-sync acquire blocker (both in place); routing them through
  the composite/promotion path instead freezes the output on the first
  promoted frame.
- The 3-finger workspace swipe (finger-drag, before any animation) gates
  all direct scanout: the drag moves content with no animation flag, and a
  fixed plane would not follow it.
- The promoted-window set is capped (currently 1): the hardware admits ~5
  simultaneous planes and bg/windows/dock/cursor take four — a second
  client plane evicts the windows plane, which costs more than
  compositing the extra window. The cap is shared across both tiers: at
  most one window is promoted per output, by one tier or the other. Tier 2
  is the more exposed of the two here, because unlike tier 1 it does not
  also save the per-frame GPU pass — if its plane evicts the windows plane
  it is a straight loss, so it must be measured, not assumed.
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
- A window Otto decorates itself carries its titlebar on a compositor
  layer above the client, and the client's content starts one bar height
  below the window's origin. The promoted buffer is placed at that
  content origin, not at the window's — placing it at the window origin
  scans the client out over its own titlebar, which is invisible to a
  screenshot (screencopy forces a composite) and shows only on hardware.
### Promotion tiers

A hardware plane scans out a rectangle of pixels: it cannot clip to a shape,
and it carries one buffer. Whether a window can use one therefore depends on
whether its client buffer already contains the finished window. Two tiers
answer that differently; they share every stability gate (topmost, not
animating, no mapped popup, no overlapping chrome) and are mutually
exclusive, because they compete for the same scarce plane slot.

**Tier 1 — raw scanout.** The client's own dmabuf goes to the plane. Zero GPU
work per client frame: the compositor never touches the pixels. Requires the
buffer to be self-describing — a dmabuf (not SHM), no subsurface overlapping
the root, and nothing the compositor draws for the window.

**Tier 2 — subtree plane.** The window's whole lay-rs subtree is rendered into
a buffer of its own, which is what goes to the plane. Costs one GPU pass per
client frame, so it never displaces tier 1; what it buys over compositing is
damage isolation — the shared windows plane no longer repaints when this
window updates — and a page flip of its own. It takes the windows tier 1
cannot: everything the compositor draws is simply drawn into the plane.

`WindowElement::has_material` is the tier-1 disqualifier, kept current by the
surface-style requests as they land. It covers everything the compositor
paints or clips for a window that its buffer does not contain:

- a background colour or a `BackgroundBlur` (`otto-surface-style` material),
- a non-zero corner radius, and
- a border.

Corner radius is the sharpest case. The rounding exists only as a lay-rs clip
in the composite path; the client's buffer has square corners. Scanning that
buffer out raw put square corners on screen, intermittently — whenever an
otto-kit window happened to satisfy the other tier-1 rules. Such a window now
falls to tier 2, where the compositor draws the clip into the plane buffer.

Tier 2's mechanics:

- Promotion reparents the window's `window_layer` out of its workspace's
  `windows_layer` and into the output's `promoted_plane` container. The
  windows plane stops drawing the window purely because it is no longer in
  that subtree — there is no hidden or blanked state to keep in sync, and
  nothing to re-import on demotion. The container mirrors the current
  workspace's `windows_layer` position, size and clipping, so the move is
  geometrically a no-op. It is re-applied every frame, because other paths
  (`raise_window_to_front` above all) reparent window layers without
  knowing about promotion.
- The plane buffer is the subtree's own bounds — shadow safe area included —
  cropped to the output, and it is re-allocated whenever the window resizes.
  A resize drops every swapchain slot, so the resize must happen before the
  element renders in the same frame or the window blinks out of the stack.
- The plane sits directly above the windows plane and below all chrome
  planes. Its buffer is folded into the cross-plane backdrop composite right
  after the middle plane, so the dock and menus blur it like any other
  desktop content, and its damage joins the middle plane's for deciding when
  that composite is rebuilt.
- A tier-2 window carrying a `BackgroundBlur` is handed the composite-so-far
  (bg + windows plane) as a RAW backdrop and blurs it itself, clipped to its
  own shape — unlike the chrome planes, which seed a pre-blurred image.
- Promotion uses the same 500 ms stability window as tier 1, and for one more
  reason: it moves a live window subtree between scene containers, so a
  candidate flickering in and out of eligibility would churn the plane every
  frame. Demotion, like tier 1's, is immediate.
- `touch /tmp/otto-no-window-plane` disables tier 2 alone, leaving tier 1
  untouched, so the two can be measured against each other and against plain
  compositing (`/tmp/otto-no-scanout` disables both).

Tier 1 keeps its "base-only" handling of material windows — blanking the
texture rather than hiding the layer — as the fallback for a window that
acquires a material while already promoted the plain way. It is demoted on
the next pass and re-promoted through tier 2.

A material opts into `blur_include_content`, like the server-side titlebar:
what a window's frost has to blur is usually the window BELOW it in the same
plane, and a plane's seeded backdrop holds the content below that plane only,
so seeding it alone would leave the window underneath sharp.
- The vibrancy tone map applied to the seeded backdrop is lay-rs'
  (`layers::drawing::vibrancy_color_filter`), not a second set of
  constants here. A consumer seeding a pre-blurred backdrop skips lay-rs'
  blur pass and the grading that goes with it; grading differently on this
  side makes one material take on two tints depending on which path drew
  it — a window's frost reading one way on its plane and another in
  expose, where the previews blur in the scene.
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
  A window with a committed `ext-background-effect-v1` blur region is
  translucent and never counts as a cover. `background`/`bottom`
  layer-shell surfaces are classified the same way: fully inside one
  opaque window (or behind a fullscreen one) on the output's current
  workspace → 2 Hz, otherwise output refresh; never during exposé or
  show-desktop.
- Otto maintains a session-wide adaptive plane budget on top of the
  per-output decomposition decision above: it follows the kernel log for
  display-engine underrun reports and sheds plane usage globally when one
  is seen. There is no standard KMS event for this, so detection matches
  per-driver log phrasing — i915 reports "FIFO underrun", amdgpu/DC
  reports "underflow" (HUBP/DCN) — and only when the line also carries a
  display-context word (drm, i915, amdgpu, pipe, crtc, hubp, display), so
  an unrelated "underrun"/"underflow" line from another subsystem (audio,
  serial) can never trigger a plane-budget reduction. A display underrun
  means the display engine failed to fetch the currently-configured planes
  in time; the affected pipe scans out solid garbage (bright green on
  Intel) from the point in the frame where the fetch fell behind, even
  though every plane's buffer content is perfectly valid — reducing plane
  count is the only fix, there is nothing wrong with any individual buffer
  to repair. The first underrun disables
  direct-scanout window promotion on every output (candidates fall back to
  compositing into the windows buffer instead); a second underrun disables
  the plane decomposition entirely on every output (full GPU composite,
  the same path used when decomposition isn't supported at all). Both
  steps are applied globally rather than per output, because display fetch
  bandwidth is shared across pipes rather than partitioned per output.
- The adaptive plane budget is sticky for the running session: once shed,
  plane usage is not restored until Otto restarts. Shedding immediately
  forces a full re-render of every plane element on every output, so the
  lighter configuration is visible on the very next frame rather than only
  once each region happens to redraw on its own.
- A debug trigger (`touch /tmp/otto-full-redraw`) forces that same full
  re-render of every plane element on every output on demand, without a
  real underrun — useful for confirming a fallback configuration renders
  correctly. It fires once per file creation; remove and re-touch the file
  to trigger it again.
- A separate debug trigger (`echo ActionName > /tmp/otto-action`) executes
  a builtin shortcut action (e.g. an expose or workspace-switch action) as
  if its key had been pressed, then requests a redraw so the resulting
  scheduled scene changes apply on the next frame. This exists because
  virtual-keyboard/virtual-pointer input used by test harnesses bypasses
  the libinput shortcut layer entirely, so there is otherwise no way to
  drive compositor shortcuts remotely; an unresolvable or backend-specific
  action name is logged and ignored rather than crashing the session.

## Constraints & Edge Cases

- Overlapping overlay planes must be blended by the hardware; availability
  is device-dependent (see docs/developer/overlay-scanout-hardware.md).
- Plane buffers hold premultiplied alpha, so every plane that exposes the
  KMS "pixel blend mode" property must be set to *Pre-multiplied*. The raw
  enum value is driver-defined (i915: `Pre-multiplied=0, Coverage=1,
  None=2`) and is resolved by name from the property. Selecting "Coverage"
  multiplies alpha in a second time, so anything scanned out at partial
  alpha — a fading blur panel above all — darkens mid-animation while both
  endpoints still look correct, and only in the scanout path (the GPU
  composite fallback always blends premultiplied).
- Blur baked into the overlay buffer double-blends slightly with the live
  planes below (material is semi-opaque); acceptable by design.
- The first frame after startup renders the overlay without a backdrop (the
  lower buffers don't exist yet); the composite arrives on the next frame.
- Skia's cached GL state is reset (`DirectContext::reset`) at the start of
  every plane render. Smithay executes raw GL on the same EGL context
  between plane renders (dmabuf imports, composite frames, cursor uploads),
  and Ganesh trusts its state cache across flushes — when the two disagree
  (scissor, FBO binding, viewport) a plane's draws are silently dropped and
  its buffer keeps the cleared color. A plane whose subtree then reports no
  further damage scans that bad buffer out indefinitely: one lost render
  during an exposé transition left the wallpaper of an empty workspace
  permanently black (a workspace with a window recovered by accident — the
  window's damage kept forcing healthy re-renders). Verified causally by
  toggling the reset at runtime on one binary: disabled reproduced the
  black 3/3, enabled ran 8/8 clean.
- The background plane only (`honor_ancestor_visibility`) skips rendering
  while an ancestor of its subtree root is hidden in the **scene arena**.
  Exposé hides `workspaces_layer` — the background plane's parent — for its
  whole lifetime, so a render in that window can only produce an empty
  buffer, and `Layer::set_hidden` reaches the arena one engine update after
  the model, leaving edge frames where the flags disagree. The skip leaves
  `force_full` armed so the frame scheduled by the un-hide's engine damage
  repaints the plane in full. This must stay opt-in: every other plane
  subtree deliberately ignores ancestor visibility (exposé itself lives
  under the hidden `workspaces_layer` and must keep rendering there).
- Removed scene nodes attribute their damage to the nearest surviving
  ancestor, so a window close re-renders only the windows plane. Only
  removals with no surviving ancestor (whole-tree teardown) fall back to
  damaging every plane buffer that frame.
- Moving content into its own plane subtree breaks views that mirrored the
  old parent: the workspace-selector previews replicate `windows_layer` and
  `workspace_background` as two separate mirrors because the wallpaper no
  longer lives under the workspace view.
- On the i915 driver, an underrun is reported once per affected pipe until
  that pipe's next modeset — a second underrun on the same pipe with no
  intervening modeset produces no further kernel report. Escalating the
  adaptive plane budget from level 1 to level 2 therefore typically needs a
  modeset (e.g. DPMS cycle, mode change, hotplug) to happen in between the
  two underrun episodes; a session that underruns repeatedly without any
  modeset can stay stuck at level 1 even though the underlying overcommit
  is still occurring.

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
- Plane buffers originally assumed every output's subtree sat at the shared
  scene origin, so an output placed elsewhere by the (then side-by-side)
  scene layout rendered its content shifted — mostly black except for a
  strip. The fix went through two stages: first, each plane buffer's render
  translate explicitly subtracted the output's own static scene placement;
  later this was superseded by making every output's subtree live at scene
  (0,0) and overlap, so a CRTC's plane elements are output-local simply by
  only ever walking their own output's subtree — no placement subtraction
  is needed at all (see multi-output.md). Since the backdrop composite is
  built per output surface from those same buffers, this also makes
  cross-plane blur correct per output with no separate fix needed.
- Direct-scanout promotion originally computed one global candidate set
  from the primary output's topmost window and applied it to every CRTC,
  which painted the primary's window on every screen and scanned out
  garbage buffers on secondary outputs. Candidates are now sourced from
  each output's own space, and the applied set is the union of every
  output's candidates (rather than each output's promotion overwriting the
  shared set) so outputs stop demoting each other's promoted windows every
  frame.
- The adaptive plane budget follows the live kernel journal rather than
  pre-computing a plane budget from mode/format math, because the actual
  fetch-bandwidth ceiling is driver- and GPU-specific and impractical to
  model up front (diagnosed in practice on an eDP 2.8K@120 + DP 4K@60 dual
  setup with the full plane stack on both outputs) — the same reasoning
  that already ruled out TEST_ONLY tier probing above. The kernel's own
  underrun report is also the only reliable signal available: the failure
  mode leaves every buffer valid, so there is no compositor-side state to
  detect it from directly. The reduction is sticky for the session, rather
  than probing back up, because the overcommit that caused the underrun is
  a property of the current output configuration and content and would
  simply recur.

## Open Questions

- Should direct-scanout client buffers be imported into the blur composite
  (restoring scanout for windows under the dock)?
- Whether hidden (shadow-only) window content still counts damage into the
  windows buffer, causing unnecessary re-renders while direct scanout is
  active.
