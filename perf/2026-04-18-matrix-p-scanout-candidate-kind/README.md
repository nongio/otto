# Matrix P — Kind::ScanoutCandidate for window elements

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f + matrix-j + matrix-n + scene_element-in-scanout-list (matrix-o) + commit-handler skip + Kind::ScanoutCandidate

## Background

Cursor plane works in Otto today. Why doesn't an overlay plane for the top window work the same way?

Reading Smithay (`src/backend/drm/compositor/mod.rs:3727`):
```rust
// only try to assign elements on an overlay plane that indicate so
if element.kind() != Kind::ScanoutCandidate && element.kind() != Kind::Cursor {
    return Err(None);
}
```

→ Smithay's overlay-plane assignment **requires `Kind::ScanoutCandidate`** on the element. Cursor elements have `Kind::Cursor`. Window elements via `Window::render_elements` use `Kind::Unspecified` (hardcoded in smithay's `space::wayland::window::render_elements`).

So our window has never qualified for an overlay plane.

## Change

Replaced the call to `fullscreen_win.render_elements(...)` in the udev render path with a direct call to `render_elements_from_surface_tree(..., Kind::ScanoutCandidate)`.

```rust
let window_elements_rendered: Vec<WaylandSurfaceRenderElement<_>> =
    render_elements_from_surface_tree(
        renderer, &wl_surface, scanout_window_location, scale, 1.0,
        Kind::ScanoutCandidate,  // ← was Kind::Unspecified inside Window::render_elements
    );
```

## Result

| Metric | Matrix N (no scene) | Matrix O (scene, Unspecified) | **Matrix P (scene, ScanoutCandidate)** |
|---|---|---|---|
| RCS busy | 15.9% | 35.2% | **36.6%** |
| GPU power | 1.35W | 2.45W | **2.90W** |
| Otto share | 8.4% | 28.8% | **30.2%** |
| fps | 60 | 58 | 55 |
| Scene damage | n/a | updated=0 | updated=0 |

**No improvement.** The kind change didn't change the measurement. So either:

1. The element kind was preserved but Smithay rejected for *another* reason (overlap with primary plane element)
2. The kind was lost when wrapping in `Wrap<WindowRenderElement>`
3. Smithay's free-plane check failed (no available overlay planes on this hardware)

## The likely root cause

Smithay's `try_assign_overlay_plane` (line 3837):
```rust
if overlaps_with_primary_plane_element && !is_underlay {
    trace!("element overlaps with element on primary plane");
    return Err(None);
}
```

Our `scene_element` covers the entire output. The window overlaps it everywhere. So even with `Kind::ScanoutCandidate`, the overlay assignment fails because the window overlaps the scene.

## Why we can't easily verify

`tracing` is configured with `release_max_level_debug` in Otto's `Cargo.toml`. **Trace! macros are stripped at compile time in release builds.** Smithay's per-element decision logging is invisible without rebuilding with `release_max_level_trace`.

## What's needed to actually achieve scene+overlay

Smithay's overlap rule means: **for overlay-plane scanout to apply, the primary plane elements must not overlap the overlay candidate's geometry.** Two paths:

1. **Render scene to leave a transparent hole where the window will be** — explicitly mark the window's area as not-rendered in the scene. Then scene's bbox excludes the window area, no overlap, overlay assignment succeeds. Needs lay-rs hook to skip a layer's rendering.

2. **Patch Smithay to relax the rule** — Smithay's check is conservative (assumes primary plane content under an overlay would be visible if overlay has alpha). For an opaque overlay (like chromium), the underlying primary content is fully covered and the rule should be skippable. Could be a Smithay PR.

## Verdict

`Kind::ScanoutCandidate` change is **kept** (it's the correct hint for Smithay's API regardless), but **alone is insufficient** to enable scene+overlay-plane multi-scanout. Need scene-with-hole or Smithay relaxation.

## Diagnostic enabling for future investigation

To debug Smithay's plane assignment in future sessions, change Otto's `Cargo.toml`:

```diff
 tracing = { version = "0.1.37", features = [
     "max_level_trace",
-    "release_max_level_debug",
+    "release_max_level_trace",
 ] }
```

Then `RUST_LOG=warn,smithay::backend::drm::compositor=trace ./target/release/otto --tty-udev` will produce per-element overlay-assignment decisions.
