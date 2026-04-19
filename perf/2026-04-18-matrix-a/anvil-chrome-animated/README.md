# Scenario: Anvil + chromium (animated canvas) — control comparison

**Date:** 2026-04-18
**Compositor:** Anvil (Smithay reference, `--tty-udev`)
**Workload:** Identical to `chrome-animated/` — chromium showing `/tmp/anim.html` (200 animated arcs at requestAnimationFrame rate)

## Result

| Metric | Value |
|---|---|
| GPU RCS busy | **27.98%** avg (steady) |
| GPU power | 2.10W |
| Pkg power | 11.56W |
| GPU freq | 717 MHz |
| RC6 sleep | 69% |

**Per-client GPU**:
- anvil 14.1%
- chromium 10.8%

## Top CPU hotspots

1. (unresolved Mesa) — 8.37%
2. `DrmCompositor::render_frame` — 6.97%
3. `wl_output drop_in_place` — 6.69%

CPU profile is small (anvil is mostly GPU work via direct GLES blit).

## Direct comparison vs Otto

Same chromium running `anim.html`:

| Metric | Anvil | Otto | Otto/Anvil |
|---|---|---|---|
| GPU RCS busy | 28.0% | 62.6% | **2.24×** |
| GPU power | 2.1W | 5.9W | 2.81× |
| Pkg power | 11.6W | 14.2W | 1.22× |
| Compositor share | 14.1% | **60.5%** | **4.3×** |

## Framerate observation (user)

User reported the canvas animation **visually ran ~2× faster on anvil than on Otto**. The display is 120Hz; chromium's `requestAnimationFrame` callback is bound by the compositor's frame delivery rate via `wp-presentation-time` / `wl_surface.frame` callbacks.

Implication: Otto isn't keeping up with 120Hz and is delivering frames at ~60Hz (or chromium throttles itself to 60 due to missed deadlines). Per-frame cost worsens accordingly:

| | RCS % | est. fps | RCS per frame |
|---|---|---|---|
| Anvil | 28% | ~120 | 0.23%/frame |
| Otto | 63% | ~60 | **1.05%/frame** |

→ **Otto is ~4.5× more expensive *per frame*** than anvil under identical content.

## Conclusion

Anvil at full 120Hz consumes less than half the GPU that Otto at 60Hz does. The compositor share alone (Otto 60.5% vs anvil 14.1%) confirms lay-rs/Skia recomposition is the primary cost — anvil's per-surface GLES blits scale linearly with damage; Otto's per-frame Skia paint is largely independent of what changed.

Future experiment: instrument actual fps (via `wp-presentation-time` feedback) to confirm the 60 vs 120 hypothesis precisely.
