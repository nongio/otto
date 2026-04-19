# Matrix B — Otto with `image_cached(true)` on workspace_layer

**Date:** 2026-04-18
**Otto:** main @75fdc64 + lay-rs damage refactor + `set_image_cached(true)` on workspace_layer (was `false` in Matrix A)
**Workload:** identical to Matrix A — same `anim.html`, same client mix

## Comparison vs Matrix A (image_cached=false)

| Scenario | Metric | Matrix A | **Matrix B** | Δ |
|---|---|---|---|---|
| [ghostty](ghostty/) | RCS busy % | 4.22 | **5.63** | +33% |
| | GPU power W | 0.25 | 0.31 | +24% |
| | Pkg power W | 5.0 | 13.5 (noisy) | — |
| [chrome-animated](chrome-animated/) | RCS busy % | 62.62 | **64.79** | +3.5% |
| | GPU power W | 5.91 | 5.65 | -4.4% |
| | Pkg power W | 14.16 | 14.68 | +3.7% |
| | Otto / chromium share | 60.5 / 2.4 | 62.6 / 2.4 | otto slightly higher |

## Conclusion

**Image-caching the workspace_layer does not help and slightly regresses idle terminal cost.** The reason is structural: the workspace_layer contains the chromium window, which changes every frame during animation. The cache invalidates per-frame, so we pay the cost of caching (rasterize-once + blit) without ever reusing the cached pixels.

CPU profile shifts:
- `subtree_has_visible_drawables` (was 6.8%) drops out of top 5 → caching does suppress lay-rs scene walks
- But `Any::type_id` and `UserDataMap::insert_if_missing` rise (~5% combined) — Smithay dispatch overhead becomes visible

Net: the savings on lay-rs traversal are eaten by other costs.

## Where image caching could pay off

- **Static layers**: background, layer_shell_bg_mirror, dock chrome, otto-bar
- **Layers whose children rarely change**: window decorations, popups
- **NOT on containers of animated client content** like workspace_layer

## Next steps

- Revert `workspace_layer.set_image_cached(true)` → back to `false`
- Selectively enable `image_cached(true)` on background_view / dock root only
- Re-test with that more targeted caching
