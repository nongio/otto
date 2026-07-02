# Phase 7 — Telemetry + frame callbacks

**Status**: todo  
**Effort**: 1 day  
**Depends on**: Phases 3–5 stable

## P-Plane-3 — Plane assignment telemetry

Without logging we can't tell if Smithay's atomic test is silently rejecting
our elements.

`DrmCompositor::render_frame()` returns a `RenderFrameResult` that lists which
elements landed on planes vs were composited. Hook into this:

```rust
// in render_surface, after render_frame():
for (element_id, plane) in &frame_result.plane_assignments {
    tracing::debug!(target: "otto::planes", element=?element_id, plane=?plane);
}
```

Log a summary at 1 Hz behind `RUST_LOG=otto::planes=debug`:
```
planes: primary=background overlay=[top_win, windows, overlay_ui, dock] composited=[]
```

Track `planes_hit` / `planes_miss` counters in `SurfaceData` behind
`#[cfg(feature = "dev")]`.

## P-Plane-4 — Frame callbacks tied to plane assignment

When a window is on an overlay plane the compositor doesn't render it per-frame,
but the client still needs `wl_surface.frame` callbacks to advance its animation.

After plane assignment is stable:
- Window assigned to overlay plane → send frame callback at display refresh rate
  unconditionally (even if compositor rendered nothing that frame).
- Window GPU-composited → frame callback on page flip (current behaviour).
- Window occluded / minimised → throttled (see frame-callback throttle plan in
  memory).

**Integration point**: after `render_frame()`, check which window elements are
in `frame_result.plane_assignments`; for those, fire frame callbacks immediately
rather than waiting for the compositor render path.

## Config knob

Add `top_window_planes: u32` (default 1, max 2) to `otto_config.toml`.
Read in `SurfaceData` initialisation to determine how many top-window overlay
slots to allocate before the shared windows plane.
