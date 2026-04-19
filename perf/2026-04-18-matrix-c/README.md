# Matrix C — Otto with always-on damage-region subtree culling

**Date:** 2026-04-18
**Otto:** main @75fdc64 + lay-rs damage refactor + scene_element.rs change
**Change tested:** in `src/render_elements/scene_element.rs`, the damage region was always passed to `render_node_tree` (previously only when occlusion culling was also enabled).

```diff
-let damage_ref = if occluded_ref.is_some() {
-    damage_region.as_ref()
-} else {
-    None
-};
+let damage_ref = damage_region.as_ref();
```

## Comparison vs Matrix A and B

| Scenario | Metric | A (default) | B (image_cached) | **C (clip always)** |
|---|---|---|---|---|
| [ghostty](ghostty/) | RCS busy % | 4.22 | 5.63 | **4.22** |
| | GPU power W | 0.25 | 0.31 | **0.18** |
| | Pkg power W | 5.0 | 13.5 (noisy) | 8.2 (noisy) |
| | Freq MHz | 57 | 61 | **37** |
| [chrome-animated](chrome-animated/) | RCS busy % | 62.62 | 64.79 | **60.74** |
| | GPU power W | 5.91 | 5.65 | **5.53** |
| | Pkg power W | 14.16 | 14.68 | 24.05 (noisy) |
| | Freq MHz | 972 | 949 | **775** |
| | Otto / chromium share | 60.5 / 2.4 | 62.6 / 2.4 | **57.6 / 3.1** |

## Hot path changes

By category (see `perf/README.md` § "CPU vs GPU work"):

| Category | Matrix A top contributors | Matrix C top contributors |
|---|---|---|
| **Otto / lay-rs scene mgmt** | `subtree_has_visible_drawables` 6.79% + `Arc::into_any_arc` 6.32% = **~13%** | `RenderLayer::update_with_model_and_layout` 2.21% + `taffy::round_layout_inner` 2.07% + `set_node_layout_size` 1.53% = **~6%** |
| **Skia CPU work** | `SkSurface::recordingContext` 7.34% + `SkRect::round` 2.57% + … | `skgpu::KeyBuilder::addBits` 1.42% + … |
| **Mesa / Gallium driver** | unresolved hex symbols ~17% | unresolved hex symbols (similar) |
| **Wayland / Smithay** | `AtomicDrmSurface::page_flip` 6.32%, `MultiCache::get` 1.75% | similar |

**Where the win came from**: Otto's lay-rs scene-management cost dropped from ~13% to ~6% of CPU samples. Skia CPU prep also dropped (the scene tree no longer asked Skia to record nodes that were outside the damage region). Mesa/Smithay portions essentially unchanged.

The new top-of-list (`taffy::round_layout_inner` etc.) suggests **taffy is recomputing layout every frame on the same input**. That's a different problem from the scene walk and is the obvious next target.

## GPU side of the win

| Metric (chrome scenario) | A | C | Δ |
|---|---|---|---|
| RCS busy % | 62.6 | 60.7 | -1.9 pts |
| GPU power (W) | 5.91 | 5.53 | -6.4% |
| GPU freq (MHz) | 972 | 775 | **-20%** |
| RC6 sleep % | 36.2 | 36.7 | similar |

The standout GPU number is **frequency**: the driver no longer needed to peg the GPU at near-max to keep up. RCS busy dropped only slightly (-1.9 pts) because the work is now done at lower clock — total work *throughput* dropped roughly proportionally to the freq drop.

## Conclusion

Always-on damage-region clipping in `scene_element.rs` is a **modest but unambiguous win**:

- Eliminates the lay-rs scene-tree visibility walk (`subtree_has_visible_drawables`) from the hot path
- Reduces Otto's compositor share by ~3 points absolute (60.5% → 57.6%)
- Lowers GPU frequency on both scenarios (chrome 972 → 775 MHz, ghostty 57 → 37 MHz)
- Lowers GPU power on both (chrome 5.91 → 5.53W, ghostty 0.25 → 0.18W)

The original conditional (gating damage clipping behind occlusion culling) was overly conservative — the comment in the code noted "correctness is preserved", and that holds in practice.

## New bottleneck

With scene-walk culling done, the top hot path is now **layout** — `taffy::round_layout_inner` 2.1%, `set_node_layout_size` 1.5%, `RenderLayer::update_with_model_and_layout` 2.2%. Even when nothing changes, lay-rs appears to be re-running taffy layout. This is the next target.

## Recommendation

**Land the change** — the conditional `if occluded_ref.is_some()` gate around damage_ref should be removed. The savings are real and the comment in the original code already acknowledged it was safe.
