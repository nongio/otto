# Matrix F — Dock icon picture+image caching (P4 first attempt)

**Date:** 2026-04-18
**Otto:** matrix-c + fps counter (matrix-e) + dock icon cache change
**Workload:** ghostty + chromium animated

## Change

`src/workspaces/dock/render.rs` line 194:
```diff
-.picture_cached(false)
-.image_cache(false)
+.picture_cached(true)
+.image_cache(true)
```

The dock icon layer's `draw_app_icon` content function does:
- Draws the icon image
- Plus a shadow with `drop_shadow_only(blur=5.0)` — this is the expensive part

So this isn't an "image-only" layer — caching the rasterized result skips the shadow blur on every frame.

## Result vs matrix-c (chrome animated, otto/chromium share)

| Metric | Matrix C | **Matrix F** | Δ |
|---|---|---|---|
| RCS busy % | 60.74 | **57.45** | **-3.3 pts** |
| GPU power W | 5.53 | 5.56 | same |
| GPU freq MHz | 775 | 772 | same |
| RC6 % | 36.7 | **40.51** | **+3.8 pts** (more sleep) |
| Otto compositor share | 57.6 | **54.0** | **-3.6 pts** |
| Chromium share | 3.1 | 3.3 | same |
| fps | (was 35) | 30-40 (~35) | same |

Hot-symbol changes:
- Matrix C top: `RenderLayer::update_with_model_and_layout` 2.21%, `taffy::round_layout_inner` 2.07%, `set_node_layout_size` 1.53%
- Matrix F top: `malloc` 2.81%, `IndexMap::get_index_of` 1.52%, `set_node_layout_size` 1.36%, `Hasher::write` 1.29%

Skia-specific symbols (`SkRect::round`, `SkMatrix::setConcat` etc) dropped further out of top 5 — confirms less Skia work per frame.

## Verdict — KEEP

Modest but unambiguous improvement. The change is one line, low risk, and the principle (cache layers that produce expensive paints, especially shadows + blurs) is generally applicable. Consider auditing other layers that draw shadows/blurs.

## Note on user feedback

User pointed out that **image-only layers don't benefit from caching** (the cache stores roughly what the image already provides). The dock icon avoided this trap because it composites image+shadow. When trying caching elsewhere, look for layers with computed effects (shadows, blurs, paths, text), not pure image samplers.

## Next candidates (text layers per user hint)

Otto-rendered text layers that could benefit:
- Dock app labels (`setup_label` in dock/render.rs:246) — rendered with text + shadow + balloon path. Only visible on hover, so impact during normal use is small.
- Workspace selector labels (workspace_selector.rs:451 `draw_text_content`) — only visible during expose.
- Window labels in expose mode (window_selector.rs:635-668).

Most text in Otto is conditional UI (hover, expose). The big always-visible text source is `otto-bar`, but that's a separate process and renders into a wl_surface — Otto only composites the resulting buffer.
