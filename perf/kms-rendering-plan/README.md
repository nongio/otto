# KMS Plane Rendering Plan

Hardware: Intel GPU, 8 planes per CRTC — 1 Primary + 6 Overlays + 1 Cursor.

## Target plane layout (per output, bottom → top)

```
6  Dock                   overlay plane   SceneDmabufElement (swapchain)
5  Popups                 overlay plane   Wayland surface dmabuf
4  Overlay UI             overlay plane   SceneDmabufElement (swapchain)
   (app switcher, workspace selector,
    layer_shell_overlay, OSD, DnD)
3  Top N windows          overlay planes  Wayland surface dmabufs (N configurable, default 1)
2  Windows / Expose       overlay plane   SceneDmabufElement (swapchain)
1  Background             primary plane   SceneDmabufElement (swapchain)
   (background_view + layer_shell_bg)
   ────────────────────────────────────
   Cursor                 cursor plane    always on
```

Fixed overlays: dock(1) + popups(1) + overlay UI(1) + windows/expose(1) = 4.
Remaining for top windows: 6 − 4 = 2. `top_window_planes` config knob (default 1).

Expose mode is a render-mode switch on plane 2 — same SceneDmabufElement, same
swapchain slot, different Skia draw pass. No extra plane or transition needed.

## What we have proven (2026-04-19)

- Cursor → cursor plane: always on.
- Top focused window → overlay plane: `Kind::ScanoutCandidate` proven, no tearing.
- Semi-transparent overlay → overlay plane: `plane_alpha` proven at 0.5.
- Smithay patches (`feat/dmabuf-scanout`): `UnderlyingStorage::Dmabuf`, opaque
  overlap rule relaxed, `GbmFramebufferExporter` dmabuf path.

## Phases

| Phase | File | Status |
|-------|------|--------|
| 1 — Swapchain infra | [phase-1-swapchain.md](phase-1-swapchain.md) | todo |
| 2 — Layer restructuring | [phase-2-layer-restructuring.md](phase-2-layer-restructuring.md) | todo |
| 3 — Background on primary | [phase-3-background-plane.md](phase-3-background-plane.md) | todo |
| 4 — Windows + expose plane | [phase-4-windows-plane.md](phase-4-windows-plane.md) | todo |
| 5 — Dock + Overlay UI planes | [phase-5-dock-overlay-planes.md](phase-5-dock-overlay-planes.md) | todo |
| 6 — Cross-plane blur | [phase-6-cross-plane-blur.md](phase-6-cross-plane-blur.md) | todo |
| 7 — Telemetry + frame callbacks | [phase-7-polish.md](phase-7-polish.md) | todo |
