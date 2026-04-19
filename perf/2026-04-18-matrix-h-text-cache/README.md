# Matrix H — Text label caching (P4c)

**Date:** 2026-04-18
**Otto:** matrix-c + matrix-f + `picture_cached(true).image_cache(true)` on `workspace_selector_desktop_label` (workspace_selector.rs:451)
**Workload:** ghostty + chromium animated, also tested in expose mode

## Change

Added picture+image caching to the workspace name text label rendered by `draw_text_content`. This label has no shadow, no blur — pure text. It's only visible during expose mode.

## Result vs matrix-f (chrome animated, NON-expose)

Three back-to-back iterations of each build to control for measurement noise.

| Build | RCS avg (3 iter) | GPU W avg | Pkg W avg | Otto share |
|---|---|---|---|---|
| Matrix F (no text cache) | **58.7%** (57.2 / 58.2 / 60.7) | 5.1W | 20.3W | 54-57.5 |
| **Matrix H (with text cache)** | **65.5%** (65.4 / 65.2 / 66.1) | 5.4W | 14.9W | 62.7-63.5 |
| Δ | **+6.8 pts** (worse) | similar | — | +7.5 pts |

In expose mode (where the label IS visible): RCS 65.5%, no further improvement from the cache.

## Surprising finding

The cached layer is **only visible during expose**. We measured during normal chrome animation, where the workspace selector label was hidden. Yet adding `picture_cached(true)` to this hidden layer cost ~7 pts of compositor RCS.

Plausible explanations (lay-rs internals):
- Cached layers may be evaluated/setup even when hidden (cost paid regardless)
- Cache invalidation tracking adds overhead per frame for every cached layer
- The picture_cached flag may force extra layer-tree traversal to manage the cache

This is a **lay-rs cost model issue worth investigating** — caching should never penalise hidden layers.

## Verdict — REVERTED

The text cache regresses the chrome animated scenario substantially. Reverted.

## Implications

The general "cache anything that has shadows/text" rule needs refinement:
- ✅ Cache helps: stable layer with expensive ops, **and** the layer is actively visible
- ❌ Cache hurts: hidden layer (this matrix), animated content underneath (matrix B, G), or backdrop blur reading animated background (matrix G)

For Otto's always-visible text, the candidate set is essentially empty — text is mostly:
- otto-bar (separate process)
- Conditional UI (expose, hover, context menu)

So text caching's measurable impact during normal use is nil unless we figure out why hidden cached layers cost extra.

## Next investigation candidates

1. **Why does picture_cached on a hidden layer cost ~7pts?** — needs lay-rs source review.
2. **Conditional text caching**: enable cache only when the layer becomes visible (toggle on enter expose, off on exit). Would test the principle without the always-on cost.
