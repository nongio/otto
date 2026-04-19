# Matrix L — Direct scanout + skip update_window_view in commit handler

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f + matrix-j + per-window `is_scanned_out` flag + commit-handler gate
**Workload:** chromium fullscreen via `--start-fullscreen` (forces direct scanout)

## Background

User insight: "when chrome commits its texture we have the overhead of drawing the scene". When direct scanout is active (chromium's dmabuf goes straight to a KMS plane), Otto's per-commit `update_window_view` work is wasted — the scene won't composite this window anyway.

## Changes (final state)

1. **Added `is_scanned_out: AtomicBool` on `WindowElement`** with `is_scanned_out()` / `set_scanned_out(bool) -> bool` methods. The setter returns the previous value so callers can detect transitions. (`src/shell/element.rs`)

2. **Set the flag from the udev render path** based on which window is the current scanout candidate (`get_fullscreen_window()` today; future: top-N windows on planes). (`src/udev/render.rs`)

The flag is currently **set but not consumed** — see below for why.

## What was tried but reverted

The commit-handler gate:

```rust
// In src/shell/mod.rs commit():
if !window.is_scanned_out() {
    self.update_window_view(&window);
}
```

Idea: skip the scene-side render-element rebuild when the window's pixels go straight to the plane.

**Measurement (when active and stable)** — 3 iter at chromium fullscreen:

| Metric | Matrix K (scanout, no skip) | **Matrix L (scanout + skip)** | Δ |
|---|---|---|---|
| RCS busy % | 12.8 | **10.1** | -2.7 |
| GPU power W | 0.64 | **0.35** | -45% |
| Pkg power W | 9.47 | **8.55** | -10% |
| GPU freq MHz | 398 | **155** | -60% |
| Otto GPU share % | 6.4 | **~0** (below 0.05%) | -6.4 |
| fps | 60 | 60 | same |

This was the largest single-step CPU/GPU saving in the matrix — Otto's compositor share dropped to *zero* in the measurement.

## Why it was reverted — correctness regression

Pressing F11 to exit fullscreen → chrome appeared at the wrong (small) size, visually broken.

**Cause**: while scanout was active, no commits called `update_window_view`, so the window's render elements (surface tree, sizes, popup overlays) were never refreshed. When scanout exits and the scene takes over, it tries to composite the window using the LAST render elements set before scanout entered, which can be wildly stale (different geometry, sub-surface tree from before chromium resized to fullscreen).

**Forced damage on transition was insufficient**: I added `add_damage(bounds)` to refresh the layer's repaint, but the render elements themselves (size, position, sub-surface layout) are set inside `update_window_view`, which never ran during scanout.

## What's needed for a correct version

For the commit-handler skip to work, the design needs:

1. **Track stale-ness at the window level**: when scanout starts, snapshot the last render-elements state.
2. **On transition exit**, force a synchronous `update_window_view` so render elements match the post-scanout window state, before any frame is composited.
3. **Also handle subsurface commits during scanout**: chromium's GPU-process subsurfaces commit independently and may need their own deferred refresh.

The straightforward implementation is "track that an update is pending; perform it at scanout-exit time before the scene next renders". Doable but needs careful borrow handling between `udev::render` and the `Otto` state methods.

## Verdict — flag KEPT, gate REVERTED

The `is_scanned_out` flag is harmless on its own (no consumer) but provides a clean foundation for future optimisations. Reverting just the commit-handler gate restores correctness; matrix-K's win (12.8% RCS, 60fps) remains intact for fullscreen scenarios.

## Design direction (user request)

> "the design should not require chrome in fullscreen for scanout"

Today's `is_fullscreen_and_stable()` requires `current_workspace.get_fullscreen_mode()` — an xdg-protocol-level fullscreen flag set by the client. This conflates "the window asked to be fullscreen" with "the window is eligible for direct scanout".

The proper design separates them:

- **Per-window scanout eligibility** (geometry + visibility based, not protocol based):
  - Top window in z-order
  - Window's render bounds equal output area
  - Window has a dmabuf buffer in a display-compatible format
  - Nothing visible above it (no layer-shell, no popups, no overlays, no compositor UI)
- **Scanout candidate selection**:
  - 1 plane: pick the top window that satisfies eligibility, if any
  - N planes (future Smithay support): pick top N opaque rects, each on its own plane
  - Cursor always reserved for the cursor plane

The `is_scanned_out` flag I added is the right per-window state for this. The remaining work is replacing `is_fullscreen_and_stable()` and `get_fullscreen_window()` with the geometry-based predicates, and solving the scene-refresh-on-transition issue (the same one that bit matrix-l).

## Net result for this session

**Kept**: per-window `is_scanned_out` flag on `WindowElement` (foundation for future).
**Reverted**: commit-handler gate (correctness regression — needs render-element refresh design).
**Standing wins** (from earlier matrices): matrix-c + matrix-f + matrix-j → ~−8 pts compositor RCS in windowed mode.
