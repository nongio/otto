# Matrix D — Expose mode

**Date:** 2026-04-18
**Otto:** main @75fdc64 + lay-rs damage refactor + matrix-c (always-on damage clipping)
**Workload:** ghostty + chromium (animated `anim.html`) → trigger expose via ydotool Page Up

Goal: address the `as_content() mirroring a layer that is not image-cached` warning that floods the log when expose is open.

## Result summary

| Variant | RCS % | GPU W | Pkg W | Freq MHz | Otto/chromium | Mirror warnings (10s) |
|---|---|---|---|---|---|---|
| [baseline-clip-only](baseline-clip-only/) (matrix-c base, OLD otto) | 64.80 | 3.70 | 12.57 | 709 | 61.8 / 3.2 | 52 |
| [control-non-expose-with-toggle](control-non-expose-with-toggle/) (NEW otto, no expose) | 66.34 | 5.24 | 14.81 | 891 | 63.8 / 2.8 | — |
| [with-image-cached-toggle](with-image-cached-toggle/) (NEW otto, expose, toggle ON) | 64.46 | **7.16** | **24.26** | 910 | 61.9 / 2.8 | 34 |

## Verdict — REVERTED

**Image-caching workspace_layers when expose opens is a net regression.**

- Mirror warnings reduced (52 → 34) — confirms the toggle works
- But GPU power +37%, package power +64%
- Reason: chromium animates inside the workspace → cache invalidates every frame → cost = (render to cache + sample cache) per frame, instead of (direct render) per frame

The lay-rs warning is correct that the unmcached mirror is expensive, but the fix isn't unconditional caching — it's caching only when the source is *static*.

## Next approach (deferred)

The right fix mirrors what macOS Mission Control does: **freeze the mirror at expose-entry snapshot**, optionally throttling to a low rate (5-10fps) so live previews still work.

Implementation sketch:
1. On expose entry: `workspace_layer.set_image_cached(true)` AND mark cache as "live but throttled"
2. lay-rs cache invalidation triggered at most every 100ms instead of every commit
3. On expose exit: `set_image_cached(false)` — back to direct render

This needs a lay-rs feature (cache invalidation throttling) — bigger than a one-line change. Skipping for now.

## Note on the 7th client GPU mystery

Per-client RCS for chromium stayed at 2.8-3.2% across all variants — chromium itself is not noticeably different inside vs outside expose, even though it's being mirrored. That's expected: chromium's GPU work is still drawing the same animation; the mirror cost shows up under Otto's pid, not chromium's.
