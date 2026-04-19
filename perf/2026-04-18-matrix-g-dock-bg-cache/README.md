# Matrix G — Dock background bar caching (P4 second attempt)

**Date:** 2026-04-18
**Otto:** matrix-f base + picture+image cache on dock_background_bar layer
**Workload:** ghostty + chromium animated

## Change

`src/workspaces/dock/view.rs` line 187 area — added `.picture_cached(true).image_cache(true)` to the dock_background_bar `LayerTreeBuilder`. This layer has:
- `shadow_radius(20.0)` (expensive shadow blur)
- `blend_mode(BlendMode::BackgroundBlur)` (reads what's behind and blurs it)

## Result vs matrix-f (chrome animated)

| Metric | Matrix F | **Matrix G** | Δ |
|---|---|---|---|
| RCS busy % | 57.45 | 58.64 | +1.2 |
| GPU power W | 5.56 | 5.04 | -0.5 |
| Pkg power W | 24.00 | 22.73 | -1.3 |
| GPU freq MHz | 772 | 732 | -40 |
| Otto / chromium | 54.0 / 3.3 | 55.5 / 3.1 | otto +1.5 |
| fps | ~35 | ~35 | same |

Mixed/within noise. RCS up, GPU power down, freq down. No clear directional win.

## Verdict — REVERTED

**The BackgroundBlur is the obstacle.** A layer with `BlendMode::BackgroundBlur` reads what's behind it before drawing. Caching the layer's rasterized output freezes the blur of whatever was behind at cache time. When chromium animates behind the dock, two bad things happen:

1. **Visual bug**: the cached blur reflects the OLD background (frozen blur).
2. **No perf win**: lay-rs has to invalidate the cache when the underlying changes, defeating the cache.

So caching shadow+blur layers fails for the same structural reason as caching workspace_layer (matrix B): the visible result depends on continuously-changing content underneath.

## Pattern emerging

Caching helps when:
- Layer content is **stable** (doesn't depend on what changes per frame)
- Layer has **expensive ops** that the cache can amortize (shadows, blurs of stored images, text, vector paths)

Caching hurts when:
- Layer reads what's behind it (BackgroundBlur) and the behind-content animates
- Layer is a parent of animated content (workspace_layer in matrix B)

## What's left to try (always-visible only)

Mostly nothing in the always-visible set. Otto's always-visible chrome:
- Dock icons (cached in matrix F — won)
- Dock background (BackgroundBlur — blocked here)
- Dock badge (hidden by default)
- otto-bar (separate process, only composited)
- Cursor (separate path)

Most other shadow/text candidates (workspace_selector items, window_selector thumbnails, context menus) are **conditional UI** — only visible during expose / right-click / hover. Caching them would help during those modes specifically.
