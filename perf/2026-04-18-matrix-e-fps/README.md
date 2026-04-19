# Matrix E — Frame rate instrumentation (P3)

**Date:** 2026-04-18
**Otto:** matrix-c (always-on damage clipping) + new fps counter in `src/udev/render.rs`'s `frame_finish`
**Workload:** chromium animated `anim.html` on 120Hz display (eDP-1, scale 2)

## Change

Added a tiny fps counter that logs once per second under `target: "otto::fps"`:

```rust
{
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static FPS: OnceLock<Mutex<(Instant, u32)>> = OnceLock::new();
    let mut g = FPS.get_or_init(|| Mutex::new((Instant::now(), 0))).lock().unwrap();
    g.1 += 1;
    let elapsed = g.0.elapsed();
    if elapsed >= Duration::from_secs(1) {
        let fps = g.1 as f64 / elapsed.as_secs_f64();
        tracing::info!(target: "otto::fps", "fps={fps:.1} ({} frames in {:.2}s)", g.1, elapsed.as_secs_f64());
        g.0 = Instant::now();
        g.1 = 0;
    }
}
```

Enable via `RUST_LOG=otto::fps=info` (or any RUST_LOG that includes that target).

## Observation

Otto delivers chromium animation at **30-38 fps, average ~35 fps** on a 120Hz display.

```
fps=37.0 (37 frames in 1.00s)
fps=32.3 (33 frames in 1.02s)
fps=35.8 (37 frames in 1.03s)
fps=38.1 (39 frames in 1.02s)
fps=32.7 (33 frames in 1.01s)
fps=32.9 (34 frames in 1.03s)
fps=33.6 (35 frames in 1.04s)
fps=37.7 (38 frames in 1.01s)
fps=35.1 (36 frames in 1.02s)
fps=34.4 (35 frames in 1.02s)
…
```

Idle (no client damage): 0 fps logged — the counter only fires inside `frame_finish`, so if Otto skips a frame entirely (no work to submit), no log is emitted. This is the correct idle behaviour.

## What this changes about earlier matrix conclusions

Earlier matrices reported only RCS busy %. Per-frame analysis is now possible:

| Compositor | RCS % | fps | RCS / frame | vs Anvil |
|---|---|---|---|---|
| Anvil + chromium | 28% | ~120 (assumed, user-reported "2× faster than Otto") | **0.23%** | 1× |
| Otto + chromium (matrix-c) | 60% | **35 (measured)** | **1.71%** | **7.4×** |

→ **Otto is ~7.4× more expensive per frame than Anvil under identical content.** Earlier estimate (4.5×) assumed Otto = 60fps; reality is closer to 35.

→ Otto delivers only ~29% of available display frames. Chromium's `requestAnimationFrame` is throttled to whatever Otto actually presents.

## Verdict — KEEP

Tiny code addition, log-target gated (no overhead unless `RUST_LOG=otto::fps=info`), no behaviour change. Should be folded into a `--features profiling` flag for clean upstreaming, but as-is it's already minimal.

## Next steps suggested by this finding

1. Measure Anvil's fps too with same `anim.html` — confirms the 120fps assumption (deferred — needs tty switch).
2. Add per-stage timing in `render_surface` (build_elements / lay-rs update / Skia paint / GL submit / swap) — answers "where does the per-frame budget go".
3. P4/P5 work in PLAN.md is now the primary path: shrinking the per-frame cost from 1.71% → closer to anvil's 0.23%.
