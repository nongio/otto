# Matrix A — Otto post-PR-#98 + lay-rs damage refactor

**Date:** 2026-04-18
**Otto commit:** main @ `75fdc64` (PR #98 merged) + lay-rs damage refactor in working tree
**Hardware:** Intel TigerLake-LP GT2 Iris Xe Graphics, eDP-1 @ 2880x1920 @ 120Hz, scale 2

Three scenarios, each measured for 10s with `intel_gpu_top -J -s 1000` + `perf record -F 99 -g -p <otto>`.

## Comparison

| Scenario | RCS % | GPU W | Pkg W | Freq MHz | RC6 % | Compositor vs client GPU |
|---|---|---|---|---|---|---|
| [idle-no-clients](idle-no-clients/) | 0 | 0 | — | 0 | 100 | n/a |
| [ghostty](ghostty/) (idle terminal) | 4.2 | 0.25 | 5.0 | 57 | 95.7 | otto 3.8% / ghostty 0.3% |
| [chrome-animated](chrome-animated/) (Otto, 200-arc canvas) | **62.6** | 5.91 | 14.2 | 972 | 36.2 | **otto 60.5% / chromium 2.4%** |
| [anvil-chrome-animated](anvil-chrome-animated/) (Anvil, same canvas) | **28.0** | 2.10 | 11.6 | 717 | 69.0 | anvil 14.1% / chromium 10.8% |

## Key observations

1. **Idle baseline is correctly zero** — Otto does no work without clients.
2. **Idle terminal costs 4% GPU and 5W** — driven by per-frame lay-rs traversal even though nothing changes.
3. **Otto vs Anvil with identical chromium workload**: Otto uses 2.24× the GPU and consumes 2.81× the GPU power. The compositor share alone is **4.3× higher on Otto**.
4. **Per-frame cost is even worse**: user observed the canvas animation visually ran ~2× faster on Anvil — Otto throttles chromium to ~60fps while Anvil delivers ~120fps. Adjusting for fps, **Otto costs ~4.5× per frame** vs Anvil under identical content.

## Top hotspots common to Otto scenarios

- `subtree_has_visible_drawables` (lay-rs scene walk)
- `Arc::into_any_arc` / `Arc::drop_slow` (lay-rs Arc churn)
- `SkSurface::recordingContext` / `SkRTree::search` (Skia picture rebuild)

## Next experiments

- Confirm fps via `wp-presentation-time` feedback (user-observed 60 vs 120 hypothesis)
- Matrix B: same scenarios with `image_cached(true)` on workspace_layer
- Matrix C: same scenarios on Otto without lay-rs damage refactor (pre-stash) for clean A/B
