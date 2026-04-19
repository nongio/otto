# Scenario: Otto + 1 idle ghostty

**Date:** 2026-04-18
**Otto:** main @75fdc64 + lay-rs damage refactor (working tree)
**Workload:** ghostty terminal, idle prompt (no typing, no output)

## Result

| Metric | Value |
|---|---|
| Otto CPU | ~0% (top sample) |
| GPU RCS busy | **4.2%** avg (1.7-6.6) |
| GPU power | 0.25W |
| Pkg power | **5.0W** |
| GPU freq | 57 MHz (deep idle) |
| RC6 sleep | 95.7% |

**Per-client GPU**: otto 3.8% / ghostty 0.3% — Otto's compositing is most of the GPU load.

## Top CPU hotspots (perf, 10s)

1. `SkSurface::recordingContext()` — 7.34%
2. **`layers::engine::stages::update_node::subtree_has_visible_drawables` — 6.79%**
3. `<T as DowncastSync>::into_any_arc` — 6.32%
4. `AtomicDrmSurface::page_flip` — 6.32%
5. `SkRTree::search` — 5.39%

## Conclusion

Otto pays ~5W package power and ~4% GPU just to keep an idle terminal on screen. The biggest CPU costs are **lay-rs scene-tree traversal** (`subtree_has_visible_drawables` 6.8% + `Arc::into_any_arc` 6.3% = ~13%) — Otto walks the scene tree every frame even though nothing changed. This is exactly what the damage refactor's `nodes_repainted == 0` short-circuit is meant to skip; the fact that the traversal still shows up here suggests it's not yet fully bypassed.
