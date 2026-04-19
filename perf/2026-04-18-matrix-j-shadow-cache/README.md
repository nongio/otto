# Matrix J — Window shadow caching (split from window cache)

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f base + restructure window layer caching
**Workload:** chromium animated (3 iterations)

## Change

`src/workspaces/window_view/view.rs:74` — moved `image_cached(true)` from `window_layer` (outer) to `shadow_layer` (child).

```diff
-layer.set_image_cached(true);
+shadow_layer.set_image_cached(true);
```

The window's structure:
```
window_layer (outer)         ← was cached, NOW uncached
├── shadow_layer (child)     ← was uncached, NOW cached
└── content_layer (child)    ← chromium buffer, uncached
```

## Result vs prior baselines (3 iterations each)

| Build | RCS avg | GPU W avg | Pkg W avg | GPU freq | Otto share | fps |
|---|---|---|---|---|---|---|
| Matrix C (start) | 60.7% | 5.5 | 14.0 | 775 | 57.6 | 35 |
| Matrix F (+ dock icon cache) | 58.7% | 5.1 | 20.3 | 800 | 55.5 | 35 |
| **Matrix J (+ shadow-only cache)** | **52.7%** | 4.2 | 19.5 | 675 | 48.7 | 35 |
| Anvil (reference) | 28% | 2.1 | 11.6 | 717 | 14.1 | ~120 |

**vs Matrix C: -8.0pts RCS, -8.9pts Otto share, -0.4 pts on share, -100 MHz GPU freq.**
**vs Matrix F: -6.0pts RCS, -6.8pts Otto share, -125 MHz GPU freq.**

Per-frame cost (RCS / fps):
- Matrix C: 1.73% per frame
- Matrix F: 1.68% per frame
- **Matrix J: 1.51% per frame** ← 13% reduction from C
- Anvil: 0.23% per frame ← still 6.6× away

## Why it works

The user observed: "when chrome commits its texture we have the overhead of drawing the scene." This change addresses that directly.

**Before** (image_cached on window_layer):
- Chrome commits a new texture
- content_layer marks damage
- Damage propagates up to window_layer
- window_layer's cached image invalidates (because content changed)
- The whole layer subtree is re-rendered into a new cached image, INCLUDING the shadow blur
- Shadow blur is the expensive part; recomputed every chrome frame

**After** (image_cached on shadow_layer only):
- Chrome commits a new texture
- content_layer marks damage  
- Damage propagates up to window_layer
- window_layer is uncached → just renders children directly
- shadow_layer's cache is independent → stays valid (window didn't move/resize/activate)
- Skia just samples the cached shadow + draws the new content

The shadow only invalidates when the window changes shape/position/activation state, not when chromium repaints.

## Verdict — KEEP

Largest single-change win in the matrix so far. Logically clean: cache the stable thing (shadow), don't cache the changing thing (window content).

## What remains

Otto is still ~6.6× anvil per frame. The remaining costs:
- Per-frame full-scene Skia paint (lay-rs traversal happens but doesn't skip whole layers — matrix-c clipping helped but not all the way)
- Mesa/Gallium GPU command submission (~17% in raw perf, mostly unavoidable for any GPU compositor)
- Smithay protocol dispatch (~5%, unavoidable)

Next high-impact targets per PLAN.md:
- P5: damage-proportional repaint (architectural — would close most of the remaining gap)
- Audit other layers using the same "outer cache that contains animated content" anti-pattern
