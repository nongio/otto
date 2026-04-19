# KMS Plane Utilisation Plan

Hardware: Intel GPU, 8 planes per CRTC — 1 Primary + 6 Overlays + 1 Cursor.

When an element is assigned to a plane the display controller composites it in
hardware.  The GPU does zero work for that surface on idle frames.  This is the
biggest single lever left after the Skia/lay-rs optimisations in PLAN.md.

---

## Target plane layout (per output, bottom → top)

```
6  Dock                   overlay plane   SceneDmabufElement (swapchain)
5  Popups                 overlay plane   Wayland surface dmabuf
4  Overlay UI             overlay plane   SceneDmabufElement (swapchain)
   (app switcher, workspace selector,
    layer_shell_overlay, OSD, DnD)
3  Top window             overlay plane   Wayland surface dmabuf  ← already working
2  Other windows          overlay planes  Wayland surface dmabufs (one plane each)
1  Background             primary plane   SceneDmabufElement (swapchain)
   (background_view + layer_shell_bg)
   ────────────────────────────────────
   Cursor                 cursor plane    always on
```

Planes 1, 4, 6 are Skia-rendered scenes exported as dmabufs.
Planes 2, 3, 5 are raw Wayland client dmabufs handed straight to KMS.
Total overlays needed: up to 5 + N windows; with 6 overlays available this
covers the common case (1 focused window + overlay UI + dock + popups).
When a plane assignment fails (format mismatch, overlap, scaling) Smithay falls
back to GPU compositing for that element only.

---

## What we have proven (2026-04-19)

- **Cursor → cursor plane**: works, always on.
- **Top focused window → overlay plane**: proven. `Kind::ScanoutCandidate` +
  `render_elements_from_surface_tree` → Smithay atomic-tests and assigns.
- **Scene → primary plane via Smithay swapchain**: works, no tearing.
- **Semi-transparent overlay → overlay plane**: `plane_alpha` property, proven
  with a 0.5-opacity `SceneDmabufElement`.
- **Smithay patches** (`feat/dmabuf-scanout`): `UnderlyingStorage::Dmabuf`,
  opaque-overlay overlap rule relaxed, `GbmFramebufferExporter` dmabuf path.

---

## P-Plane-0 — Scene on primary via SceneDmabufElement swapchain

**Current cost**: scene is Skia-rendered into Smithay's swapchain every frame
(GPU composite: read scene layers → write framebuffer → primary plane).

**Target**: scene rendered into a GBM swapchain (2-3 slots).  On each frame,
acquire slot → render → release.  Expose the slot's dmabuf via
`UnderlyingStorage::Dmabuf` → primary plane.  When scene is static, the
previous slot is reused — zero GPU work.

**Why swapchain (not single buffer)**: single buffer = KMS scans while GPU
writes = tearing.  We hit this exact bug in the session — the circle had
artifacts until we dropped `SceneDmabufElement` from the primary path.

**Implementation**:
1. Replace `OnceLock<Dmabuf>` + `_gbm_buffer` in `SceneDmabufElement` with
   `Swapchain<GbmAllocator<DrmDeviceFd>>` (same type as `GbmDrmCompositor`'s
   internal swapchain).
2. On `update()`: `swapchain.acquire()` → render into slot → `slot.export()`
   → store as current dmabuf → `underlying_storage()` returns it.
3. On VBlank (`frame_submitted` callback): `swapchain.submitted()` to release
   the slot.
4. Damage tracking: if `engine.update()` reports no damage AND previous slot
   is still valid, skip render and return the same dmabuf.

**Expected impact**: compositor GPU share → ~0% on static-scene frames
(background, dock unchanged, only window animating).  Largest single win.

**Effort**: 1–2 days.

---

## P-Plane-1 — All visible non-overlapping windows on overlay planes

**Current**: only the top focused window gets `Kind::ScanoutCandidate`.
Other windows are composited into the scene.

**Target**: mark ALL window surface elements as `ScanoutCandidate`.  Smithay's
`DrmCompositor` iterates planes top→bottom and atomic-tests each element;
non-overlapping windows each get their own overlay plane automatically.

**Constraints**:
- Overlapping windows: only one can be on a plane (the topmost one passes the
  atomic test; the rest fall back to GPU composite).
- Format: client must have submitted a dmabuf with a plane-compatible format
  (ARGB8888 / XRGB8888 / ABGR2101010).  SHM buffers never qualify.
- Scaling: overlay planes may not support arbitrary scaling — if window size ≠
  buffer size the atomic test fails and Smithay falls through to GPU composite.

**Implementation**: change the element-building path to tag all window elements
(not just the top one) with `Kind::ScanoutCandidate`.  No other change needed
— Smithay handles the rest via atomic test.

**Expected impact**: on a two-window tiled layout, both windows avoid GPU
composite.  Scales linearly with number of non-overlapping visible windows (up
to 6 overlays).

**Effort**: hours.  The change is one-line; the work is testing edge cases.

---

## P-Plane-2 — Dock on its own overlay plane

**Current**: dock is part of the scene — it lives in the lay-rs tree and is
composited into the primary plane via Skia every frame.

**Target**: render the dock into its own `SceneDmabufElement` dmabuf, assign
to an overlay plane.  Dock updates (badge counts, hover effects) don't
invalidate the scene or the window planes.

**Constraints**:
- Dock has rounded corners → pixels outside the visible rect are transparent.
  Transparent pixels on a plane are fine as long as the plane below shows
  through — display controller alpha-blends them.  No opaque-region issue.
- Blur behind dock: if dock has a frosted-glass blur, we can't do it purely on
  a plane (blur samples from below, which is a different plane).  Options:
  a) bake the blur into the dmabuf each frame the scene changes (acceptable if
     scene is static most of the time);
  b) disable blur for dock when on overlay plane.

**Expected impact**: dock updates no longer re-render the full scene.

**Effort**: 2–3 days (SceneDmabufElement swapchain must land first).

---

## P-Plane-3 — Telemetry: plane assignment success rate

Without logging we can't tell if Smithay's atomic test is rejecting our
candidates silently.

**Implementation**:
- Hook into `DrmCompositor::render_frame()` result: it already returns which
  elements were assigned to planes vs composited.  Log a summary at 1 Hz:
  `planes: primary=scene overlay=[win0, win1] composited=[win2]`.
- Track running totals (`planes_hit`, `planes_miss`) behind `feature = "dev"`.

**Effort**: half day.  Prerequisite for validating P-Plane-0 through P-Plane-2.

---

## P-Plane-4 — Frame callbacks tied to plane assignment

When a window is on an overlay plane the compositor doesn't render it — it must
still send `wl_surface.frame` callbacks so the client advances.  When a window
is GPU-composited the frame callback already fires on page flip.

**Current**: frame callbacks fire uniformly.  There is a separate frame-callback
throttle plan in memory (Focused/Secondary/Occluded states).

**Integration**: after plane assignment is stable, wire frame callback rate:
- Window on overlay plane: callback at display refresh rate (always, even if
  compositor didn't render).
- Window GPU-composited but visible: callback on page flip.
- Window occluded / minimised: throttled or suppressed.

**Effort**: 1 day.  Depends on P-Plane-3 telemetry to confirm which path a
client is on.

---

## Suggested order

1. **P-Plane-3** (telemetry) — know what's actually being assigned before
   changing anything.
2. **P-Plane-1** (all windows as candidates) — trivial code change, high ROI.
3. **P-Plane-0** (scene swapchain) — highest GPU impact, most implementation
   work.
4. **P-Plane-2** (dock plane) — after P-Plane-0 lands (shares the swapchain
   infra).
5. **P-Plane-4** (frame callbacks) — polish, after the above stabilise.
