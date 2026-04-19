# Scenario: Otto + ghostty + chromium (animated canvas)

**Date:** 2026-04-18
**Otto:** main @75fdc64 + lay-rs damage refactor (working tree)
**Workload:** ghostty idle + chromium showing /tmp/anim.html (200 animated arcs at requestAnimationFrame rate)

## Result

| Metric | Value |
|---|---|
| Otto CPU | 5-10% |
| GPU RCS busy | **62.6%** avg (60-64) |
| GPU power | 5.91W |
| Pkg power | **14.2W** |
| GPU freq | 972 MHz (near max) |
| RC6 sleep | 36.2% |

**Per-client GPU**:
- otto **60.5%** RCS
- chromium 2.4% RCS

→ Otto's compositing is **~25× chromium's own GPU work** for the same frames.

## Top CPU hotspots (flat distribution)

| Symbol | % |
|---|---|
| `DrmRenderElements::damage_since` | 2.57 |
| `SkRect::round` | 2.57 |
| `__vdso_clock_gettime` | 2.44 |
| `PathGeoBuilder::createMeshAndPutBackReserve` | 2.42 |
| `Device::drawRRect` | 2.16 |
| `calloop::dispatch` | 2.10 |
| `GrOpFlushState::detachAppliedClip` | 2.10 |
| `SkMatrix::setConcat` | 1.88 |
| `SkMatrix::computeTypeMask` | 1.85 |
| `Arc::drop_slow` | 1.65 |

CPU profile is flat (no symbol > 3%). The bottleneck is GPU, not CPU.

## Conclusion

Under real animation, Otto consumes ~25× more GPU than chromium itself produces. This is the cost of full-scene Skia recomposition every frame. The damage refactor's job is to make Otto's GPU usage proportional to the actual changed area (chromium's surface, not the entire screen).
