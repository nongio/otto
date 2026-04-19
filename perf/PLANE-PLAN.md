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
3  Top N windows          overlay planes  Wayland surface dmabufs (N configurable, default 1)
2  Windows / Expose       overlay plane   SceneDmabufElement (swapchain)
   (all non-top windows rendered together;
    expose mode renders into the same plane —
    no transition cost, just a different render pass)
1  Background             primary plane   SceneDmabufElement (swapchain)
   (background_view + layer_shell_bg)
   ────────────────────────────────────
   Cursor                 cursor plane    always on
```

Planes 1, 2, 4, 6 are Skia-rendered scenes exported as dmabufs (SceneDmabufElement).
Planes 3, 5 are raw Wayland client dmabufs handed straight to KMS.

Fixed overlays: dock(1) + popups(1) + overlay UI(1) + windows/expose(1) = 4.
Remaining overlays available for top windows: 6 − 4 = **2 by default**.
N is a config knob (`top_window_planes`, default 1). Max useful value is 2
given the hardware budget; raising it reduces the windows/expose plane or
drops one of the fixed UI planes.

Expose mode is a render-mode switch on plane 2: instead of compositing the
non-top windows at their normal positions, the same SceneDmabufElement renders
the expose grid. Same plane, same dmabuf slot — no layer rebuild or transition
plane needed.

When a plane assignment fails Smithay falls back to GPU compositing for that
element only.

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

## P-Plane-1 — Non-top windows + expose on a shared overlay plane

**Current**: non-top windows and expose are part of the lay-rs scene, composited
into the primary plane via Skia every frame.

**Target**: a dedicated `SceneDmabufElement` (swapchain) for all non-top windows.
This element renders the `workspace_windows_container` layers (minus the top
window) and is assigned to its own overlay plane.

Expose mode is a render-mode flag on the same element: when expose is active,
render the expose grid into the same swapchain slot instead of the normal window
layout.  No extra plane, no transition — just a different Skia draw pass into the
same dmabuf.

**Benefits**:
- Fixed overlay budget: exactly 1 plane for all non-top windows, regardless of count.
- Expose is free: same plane, same infrastructure, mode switch only.
- No per-window format/scaling constraints (Skia composites them internally).

**Damage**: only re-render this plane when a non-top window has new damage or
the expose animation is running.  Static backgrounds of non-top windows cost
nothing.

**Effort**: 2 days.  Requires P-Plane-0 swapchain infra first (shared pattern).

---

## P-Plane-2 — Dock on its own overlay plane

**Current**: dock is part of the scene — it lives in the lay-rs tree and is
composited into the primary plane via Skia every frame.

**Target**: render the dock into its own `SceneDmabufElement` dmabuf, assign
to an overlay plane.  Dock updates (badge counts, hover effects) don't
invalidate the scene or the window planes.

**Constraints**:
- Dock has rounded corners → transparent pixels outside the visible rect.
  Fine on a plane — display controller alpha-blends with the plane below.
- Background blur (frosted glass) samples pixels from planes below the dock.
  Those planes are separate dmabufs; the dock's Skia surface can't sample them
  directly.  Solved by P-Plane-5 (cross-plane blur via dmabuf reimport).

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

## P-Plane-5 — Cross-plane backdrop blur via dmabuf reimport

**Problem**: planes below a Skia-rendered element (e.g., the blur behind the
dock, or a frosted-glass app-switcher overlay) are separate dmabufs — the
upper plane's Skia canvas can't sample them directly.

**Solution**: before rendering a plane that needs backdrop blur, reimport the
dmabufs from the planes below it as `skia::Image` objects, composite them into
a temporary Skia surface (respecting each plane's `plane_alpha`), apply the
blur filter to the relevant region, then use the blurred result as the backdrop
when drawing the plane's own content.

**Implementation**:
1. `SkiaRenderer::import_image_from_dmabuf(dmabuf)` — already designed:
   `dmabuf → EGLImage → GL texture → skia::Image` via the existing
   `import_egl_image` + `import_skia_image_from_texture` path.
2. Each `SceneDmabufElement` that needs blur holds references to the
   `SceneDmabufElement`s below it (passed in at construction or as a slice).
3. On `update()`, before rendering own content:
   a. For each lower plane: call `import_image_from_dmabuf` on its current slot.
   b. Draw lower images onto a scratch Skia surface (size = blur region only,
      not full output — keep the scratch small).
   c. Apply `skia::image_filters::blur` to the scratch.
   d. Draw the blurred scratch as the backdrop, then render own content on top.
4. **Cache invalidation**: only redo the blur blit if any lower plane's
   `current_commit()` has advanced since last frame.  Static background behind
   a static dock = zero extra GPU work.

**Scope**: dock blur, app-switcher frosted glass, any future overlay with
backdrop filter.  The general pattern is reusable across all Skia planes.

**Effort**: 1–2 days after P-Plane-0 and `import_image_from_dmabuf` land.

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
2. **P-Plane-0** (background swapchain) — highest GPU impact, infra all others share.
3. **P-Plane-1** (windows + expose plane) — built on P-Plane-0 swapchain pattern.
4. **P-Plane-2** (dock plane) — after P-Plane-0.
5. **P-Plane-5** (cross-plane blur) — after P-Plane-0/2, restores blur across planes.
6. **P-Plane-4** (frame callbacks) — polish once plane assignment is stable.
