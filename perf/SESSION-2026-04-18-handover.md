# Handover — 2026-04-18 dmabuf scanout architecture

This document captures the state at the end of a long session that explored
direct scanout for the top window via Smithay overlay planes, with the goal
of getting Otto's compositor cost down to anvil parity (or below) for typical
windowed workloads.

## TL;DR

- The full architecture is wired up in the working tree. **Code compiles, all
  22 headless tests pass, otto runs without crashing.**
- The remaining unknown: when chromium is windowed, the scanned-out scene
  dmabuf doesn't visually appear on screen (red debug paint never visible).
  Most likely Smithay's plane-assignment is rejecting our element, but
  `tracing` `release_max_level_debug` strips the trace! macros that would
  tell us why.
- Several validated wins from earlier in the session (matrix C / F / J)
  are also in the working tree and can be committed independently.

## What's in the working tree

Everything below is uncommitted.

### Otto

| File | Change |
|---|---|
| `src/render_elements/scene_dmabuf_element.rs` | New module: GBM-allocated, dmabuf-exported, GL-texture-imported, Skia-surface-wrapped scene element. Currently in DEBUG mode painting solid red ONCE on the first frame. |
| `src/render_elements/workspace_render_elements.rs` | New `SceneDmabuf` variant in the macro. |
| `src/render_elements/output_render_elements.rs` | `SceneDmabufElement: (RenderElement<R>)` added to where bound. |
| `src/render_elements/mod.rs` | New `pub mod scene_dmabuf_element;`. |
| `src/render.rs` | `SceneDmabufElement: RenderElement<R>` added to where bound. |
| `src/render_elements/scene_element.rs` | matrix-c always-on damage clipping + scene-update logging behind `otto::scene` target. |
| `src/skia_renderer.rs` | New `create_surface_from_dmabuf` helper. |
| `src/udev/render.rs` | fps counter, per-window scanned_out flag, new scanout candidate selection (any top window), correct geometry-aware position via `render_location`, `Kind::ScanoutCandidate` for window elements, lazy SceneDmabufElement setup, scanout branch uses `WorkspaceRenderElements::SceneDmabuf` (currently DEBUG-DISABLED — see "open knobs" below). |
| `src/udev/types.rs` | `SurfaceData.scene_dmabuf_element: Option<SceneDmabufElement>` field. |
| `src/udev/device.rs` | Initialises new field to `None`. |
| `src/headless.rs` | Fixed `tick()` and `settle()` for new lay-rs `UpdateStats` return type. |
| `src/shell/element.rs` | Per-window `is_scanned_out: AtomicBool` flag. |
| `src/shell/mod.rs` | Commit-handler skip when `window.is_scanned_out()`, with verification log under `otto::scanout`. |
| `src/workspaces/workspace.rs` | matrix-f dock icon picture+image cache. |
| `src/workspaces/window_view/view.rs` | matrix-j shadow-only cache (instead of outer window cache). |
| `src/workspaces/dock/render.rs` | matrix-f dock icon `picture_cached(true).image_cache(true)`. |
| `src/workspaces/mod.rs` | New `is_top_window_scanout_eligible()` + `get_top_window()`. |
| `tests/headless_basic.rs` | 5 new tests for the scanout candidate selection logic. |
| `otto_config.toml` | xdg-desktop-portal-otto autostart commented out (its watchdog kept killing otto during this session). |

### Smithay (`/home/riccardo/dev/smithay/` on branch `feat/dmabuf-scanout`)

Two patches in working tree (uncommitted):

1. **Dmabuf scanout support** — adds `UnderlyingStorage::Dmabuf(&Dmabuf)`,
   `ExportBuffer::Dmabuf(&Dmabuf)`, etc., so a render element can hand the
   DrmCompositor a dmabuf directly (not just a wl_buffer). Five files,
   ~28 lines.
2. **Overlap-rule relaxation for opaque overlays** — in
   `try_assign_overlay_plane`, the overlap-with-primary-plane-element check
   is skipped when the candidate is fully opaque. One-line change at
   `src/backend/drm/compositor/mod.rs:3837`.

Otto's `Cargo.toml` is pinned to `path = "../smithay"` to use this fork.

## What the matrix established (validated, committable)

See `perf/2026-04-18-matrix-{a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p}/README.md` for
the full per-experiment writeups. The clean keepers:

| Matrix | Change | Effect |
|---|---|---|
| C | Always-on damage-region clipping in scene_element.rs | -3pts compositor RCS, -200MHz GPU freq |
| E | fps counter in `frame_finish` | Instrumentation; revealed Otto delivers ~35fps not 60 |
| F | Dock icon picture+image cache | -3.3pts RCS |
| J | Shadow-only cache instead of outer window cache | **-6 to -8pts RCS, biggest single win** |

Cumulative C+F+J: ~-8pts compositor share for windowed chromium.

## What the dmabuf-scanout work is meant to deliver

When a top window is windowed (not fullscreen-protocol):

1. Window's wl_buffer dmabuf → overlay KMS plane (no GPU compositing for it)
2. Scene rendered into our own dmabuf-backed Skia surface → primary KMS plane
   (cached across frames; only re-rendered when scene is genuinely dirty)
3. Result: compositor GPU cost drops to ~zero when only chromium is animating
   (matrix-K showed 12.8% RCS for fullscreen scanout; this would deliver
   similar for windowed)

Per-frame expected: anvil-parity (~0.23%/frame).

## What's not yet verified

We never visually saw the dmabuf hit the screen in this session. Specifically:

- `pushed SceneDmabuf element` log fires (so the element IS allocated and
  pushed to Smithay's elements list)
- But the screen remains black around chromium, with no red rectangle
- `SceneDmabufElement::draw()` log was added to detect composite fallback
  but we couldn't get a stable run long enough to read it

Possible reasons (in priority order):

1. **Smithay is putting chromium on primary plane (direct scanout) and
   ignoring our dmabuf scene element entirely.** When chromium is the only
   plane-eligible element, primary plane direct-scanout uses chromium's
   buffer; the scene becomes fallback composite, but our `RenderElement::draw`
   is empty so primary stays empty.
2. **Format mismatch** — we allocate ARGB8888 + Linear modifier; the
   primary plane might require XRGB8888 or an Intel-tiled modifier.
3. **opaque_regions reporting issue** — we declare full opaque, but Smithay's
   per-element computed `element_is_opaque` flag might not match what we
   declare (depends on Smithay's internal logic).

## How to debug visually next session

Prerequisites: a fresh boot, no concurrent compositor.

1. **Confirm `otto::scanout` "pushed SceneDmabuf element" fires** at otto
   startup with chromium running (already verified earlier).
2. **Check whether `SceneDmabufElement::draw()` log fires**:
   - Source location: `src/render_elements/scene_dmabuf_element.rs` — the
     `RenderElement<UdevRenderer>::draw` impl has a one-shot log under
     `otto::scanout`.
   - If it fires → Smithay rejected the plane assignment; we're in composite
     fallback. The empty `draw()` body explains the black screen. Fix:
     implement `draw()` properly to sample the dmabuf-backed Skia texture
     onto the renderer's frame.
   - If it does NOT fire → Smithay assigned our element to a plane but the
     dmabuf content isn't visible. Likely the chromium overlay is on top of
     our scene and chromium covers the visible area; the scene IS being
     scanned out but to a plane behind chromium. Verify by closing chromium
     while otto runs — should see red.
3. **If still mysterious, enable Smithay traces**:
   - `Cargo.toml`: change `release_max_level_debug` → `release_max_level_trace`
   - **Important**: my earlier attempt this session did not work because the
     features-union with another dep keeps `_debug` set. Need to also patch
     smithay's own Cargo.toml or override at a different level.
   - Alternative: build otto in **debug mode** (`cargo run --features ...`)
     where trace! macros are not stripped. Then `RUST_LOG=warn,smithay::backend::drm::compositor=trace`
     will show per-element plane-assignment decisions.

## How to re-enable the dmabuf scene path

It is currently DISABLED in `src/udev/render.rs` (the scanout branch falls
back to legacy `SceneElement`). To re-enable, restore the original block:

```rust
if let Some(dmabuf_scene) = surface.scene_dmabuf_element.clone() {
    if let Err(e) = dmabuf_scene.ensure_render_target(renderer.as_mut()) {
        tracing::warn!(...);
        elements.push(OutputRenderElements::from(WorkspaceRenderElements::Scene(scene_element)));
    } else {
        dmabuf_scene.update();
        elements.push(OutputRenderElements::from(
            WorkspaceRenderElements::SceneDmabuf(dmabuf_scene),
        ));
    }
} else {
    elements.push(OutputRenderElements::from(WorkspaceRenderElements::Scene(scene_element)));
}
```

(Currently replaced with `let _ = &surface.scene_dmabuf_element;` + push of
the legacy element only.)

## To restore the actual scene render (instead of red debug)

`src/render_elements/scene_dmabuf_element.rs::update()` currently paints
solid red ONCE. To restore proper scene rendering, replace the body with
the original lay-rs `render_node_tree` path (commented in the file's git
history; or reconstruct from `SceneElement::update()` body, drawing into
`inner.skia_surface.canvas()` instead of the renderer's primary surface).

## Open knobs to investigate

1. **`is_animating` sticks true** — `is_top_window_scanout_eligible()` was
   returning false because of this. We bypassed the check earlier in the
   session (now re-enabled). Some startup transition's `on_finish` callback
   doesn't fire. Worth tracing back to what transitions are pending.
2. **xdg-desktop-portal-otto watchdog timeout** — kills otto after 3 failed
   pings. Otto config has the autostart commented out for this debug, but
   the portal can also start via D-Bus activation. May need to set the
   watchdog timeout higher in the portal.
3. **Composite-fallback `draw()` is empty** — must be implemented for
   correctness when Smithay can't plane us. The path: sample our dmabuf-
   backed Skia surface (or its underlying GL texture) onto the frame at
   `dst`, restricted to `damage`. Smithay has helpers like
   `TextureRenderElement` / `WaylandSurfaceRenderElement` that wrap a
   texture and provide `draw()`; consider building our element to delegate
   draw() to a wrapped TextureRenderElement.

## Disk hygiene reminder

End-of-session disk: `/dev/nvme0n1p2 441G 387G 32G 93%`. This was largely
otto target dirs across `~/dev/otto*`, plus 1GB of leftover `.skp`
snapshots in `otto_4/` (cleaned at end). `~/.cache` is 59G — periodic
`du -sh ~/.cache/*` and prune may help.

## Files to read first next session

1. This file.
2. `perf/PLAN.md` — the prioritised follow-up plan derived from the matrix.
3. `perf/2026-04-18-matrix-p-scanout-candidate-kind/README.md` — explains
   why Smithay was rejecting overlay-plane assignment, and the fix
   (`Kind::ScanoutCandidate` + opacity-aware overlap rule).
4. `src/render_elements/scene_dmabuf_element.rs` — the new module; read
   its module-level docstring + the `update()` debug version to see where
   we left off.
