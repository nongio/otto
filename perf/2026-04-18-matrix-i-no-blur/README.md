# Matrix I — Disable BackgroundBlur on dock_background_bar

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f base (dock icon cache) + dock_background_bar `BlendMode::BackgroundBlur` → `BlendMode::Normal`
**Workload:** chromium animated (3 iterations)

## Hypothesis

BackgroundBlur is one of Skia's most expensive ops (read framebuffer + blur shader + recombine). Disabling it on the always-visible dock should shave significant GPU work.

## Result vs matrix-f

Three iterations of each:

| Build | RCS avg | GPU W | Pkg W | Otto share |
|---|---|---|---|---|
| Matrix F (BackgroundBlur on) | **58.7%** | 5.1 | 20.3 | 55.5 |
| Matrix I (BackgroundBlur off) | **61.4%** | 4.9 | 20.0 | 58.3 |
| Δ | **+2.7 pts (worse!)** | similar | similar | +2.8 pts |

## Why the regression

Counter-intuitive result. The likely explanation:

`BlendMode::BackgroundBlur` produces an **opaque output**: reads pixels behind, blurs them, paints the result. Once the dock layer is rendered, it fully occludes whatever was underneath.

`BlendMode::Normal` with the dock's `background_color = materials_medium` (which has alpha < 1.0) produces a **semi-transparent overlay**. The dock's bar shows the underlying chromium animation through it. Damage from chromium has to propagate through the dock area in EVERY frame, increasing the size of redrawn region per frame.

So the blur was acting as **damage occlusion**: anywhere the dock covers, the dock provides the final pixels regardless of what's underneath, so chromium's animation under the dock can be skipped. Removing the blur means chromium's underlying area gets redrawn each frame too.

## Verdict — REVERTED

The dock's `BackgroundBlur` is paying for itself by acting as an opacity barrier. Disabling it costs more than it saves.

## Implication

Two related thoughts for future P-items:
1. **Make the dock fully opaque** (background_color with alpha=1.0) without blur — would test whether the occlusion alone (sans blur) is the win, isolating the blur cost.
2. **The lay-rs occlusion culling is doing real work**. Otto's `occlusion_culling` config flag likely matters more than I'd been treating it.

## Note on what BackgroundBlur actually costs

Skia's backdrop blur on a 100% opaque output should be cheap once cached (one-time blur per source change). What likely makes it expensive in practice is that the blur's source (chromium's frame) changes every frame, so the blur has to be recomputed every frame. The cost of `the blur recomputation` ≈ the cost saved by skipping `the chromium redraw under the dock`. Net: roughly break-even on this workload, with a slight tilt toward the blur winning by 2-3 pts.
