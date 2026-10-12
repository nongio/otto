//! Per-phase render timing stats, gated behind the `perf-counters` feature.
//!
//! Splits the udev compose path into four phases so we can see whether the
//! cost lives in the lay-rs engine tick, the smithay element walk, the Skia
//! scene traversal, or the Skia GPU flush:
//!
//! 1. `engine_update` — `SceneElement::update` (lay-rs animation/layout tick)
//! 2. `render_frame` — `DrmCompositor::render_frame` (smithay walk + GPU dispatch)
//! 3. `scene_draw` — `render_node_tree` inside `SceneElement::draw`
//!    (CPU traversal that records Skia draw ops)
//! 4. `skia_flush` — `SkiaFrame::finish`'s `flush_and_submit_surface`
//!    (Skia resolves and submits GL commands; on EGL this can include
//!    implicit waits)
//!
//! Phases (3) and (4) are recorded from shared rendering code and only
//! drained/logged from the udev path. The recording is a single relaxed
//! atomic add per phase per frame.

#[cfg(feature = "perf-counters")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "perf-counters")]
use std::sync::Mutex;
#[cfg(feature = "perf-counters")]
use std::time::{Duration, Instant};

#[cfg(feature = "perf-counters")]
struct Phase {
    total_ns: AtomicU64,
    count: AtomicU64,
    max_ns: AtomicU64,
}

#[cfg(feature = "perf-counters")]
impl Phase {
    const fn new() -> Self {
        Self {
            total_ns: AtomicU64::new(0),
            count: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    fn record(&self, ns: u64) {
        self.total_ns.fetch_add(ns, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        // Lock-free max via CAS.
        let mut current = self.max_ns.load(Ordering::Relaxed);
        while ns > current {
            match self.max_ns.compare_exchange_weak(
                current,
                ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }

    /// Atomically read and reset the accumulators.
    fn drain(&self) -> (u64, u64, u64) {
        let total = self.total_ns.swap(0, Ordering::Relaxed);
        let count = self.count.swap(0, Ordering::Relaxed);
        let max = self.max_ns.swap(0, Ordering::Relaxed);
        (count, total, max)
    }
}

#[cfg(feature = "perf-counters")]
static ENGINE_UPDATE: Phase = Phase::new();
#[cfg(feature = "perf-counters")]
static RENDER_FRAME: Phase = Phase::new();
#[cfg(feature = "perf-counters")]
static SCENE_DRAW: Phase = Phase::new();
#[cfg(feature = "perf-counters")]
static SKIA_FLUSH: Phase = Phase::new();

/// Per-plane render timings, keyed by `SceneDmabufElement::label`
/// ("bg", "windows", "expose", "overlay", "dock", "switcher").
///
/// Under plane decomposition this is where the Skia work actually happens —
/// `render_frame` only hands Smithay already-rendered dmabufs, so the four
/// phases above show a near-empty compose path and say nothing about which
/// buffer is expensive. Recorded only when a plane really re-rendered
/// (`render()` returned true), so a skipped plane doesn't dilute the mean.
///
/// A `Mutex<Vec<_>>` rather than atomics: the label set is small and only
/// discovered at runtime, and this is locked a handful of times per frame.
/// Fields per label: (label, count, total_ns, max_ns, flush_ns).
///
/// `flush_ns` accumulates just the `flush_and_submit_surface(.., SyncCpu::Yes)`
/// call, which blocks the CPU until that plane's GPU work retires. Subtracting
/// it from `total_ns` gives the CPU-side draw-recording cost, so the two
/// questions — "is the GPU work expensive?" vs "are we merely waiting for it?"
/// — can be told apart. Only the second is recoverable by handing the plane an
/// IN_FENCE_FD instead of blocking (see the note at the call site).
#[cfg(feature = "perf-counters")]
type PlaneStat = (&'static str, u64, u64, u64, u64);

#[cfg(feature = "perf-counters")]
static PLANES: Mutex<Vec<PlaneStat>> = Mutex::new(Vec::new());

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_plane_render(label: &'static str, elapsed: Duration) {
    let ns = elapsed.as_nanos() as u64;
    let mut planes = PLANES.lock().unwrap();
    match planes.iter_mut().find(|(l, ..)| *l == label) {
        Some((_, count, total, max, _)) => {
            *count += 1;
            *total += ns;
            *max = (*max).max(ns);
        }
        None => planes.push((label, 1, ns, ns, 0)),
    }
}

/// Record the blocking-flush portion of a plane render. Called from inside the
/// render, before `record_plane_render` closes out the same frame's entry.
#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_plane_flush(label: &'static str, elapsed: Duration) {
    let ns = elapsed.as_nanos() as u64;
    let mut planes = PLANES.lock().unwrap();
    match planes.iter_mut().find(|(l, ..)| *l == label) {
        Some((_, _, _, _, flush)) => *flush += ns,
        None => planes.push((label, 0, 0, 0, ns)),
    }
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_plane_render(_label: &'static str, _elapsed: std::time::Duration) {}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_plane_flush(_label: &'static str, _elapsed: std::time::Duration) {}

/// The single per-frame CPU wait for all plane buffers
/// (`SkiaRenderer::flush_planes_for_scanout`). This replaced one blocking
/// wait per plane, so it should be compared against the *sum* of the old
/// per-plane flush times, not against any single one.
#[cfg(feature = "perf-counters")]
static PLANE_SYNC: Phase = Phase::new();

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_plane_sync(elapsed: Duration) {
    PLANE_SYNC.record(elapsed.as_nanos() as u64);
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_plane_sync(_elapsed: std::time::Duration) {}

#[cfg(feature = "perf-counters")]
static LAST_LOG: Mutex<Option<Instant>> = Mutex::new(None);

#[cfg(feature = "perf-counters")]
const LOG_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_engine_update(elapsed: Duration) {
    ENGINE_UPDATE.record(elapsed.as_nanos() as u64);
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_engine_update(_elapsed: std::time::Duration) {}

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_render_frame(elapsed: Duration) {
    RENDER_FRAME.record(elapsed.as_nanos() as u64);
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_render_frame(_elapsed: std::time::Duration) {}

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_scene_draw(elapsed: Duration) {
    SCENE_DRAW.record(elapsed.as_nanos() as u64);
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_scene_draw(_elapsed: std::time::Duration) {}

#[cfg(feature = "perf-counters")]
#[inline]
pub fn record_skia_flush(elapsed: Duration) {
    SKIA_FLUSH.record(elapsed.as_nanos() as u64);
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn record_skia_flush(_elapsed: std::time::Duration) {}

/// If at least `LOG_INTERVAL` has passed since the previous log, drain the
/// counters and emit a single `tracing::debug!` line summarising each phase
/// over that window. Cheap to call every frame.
#[cfg(feature = "perf-counters")]
pub fn log_if_due() {
    let mut last = LAST_LOG.lock().unwrap();
    let now = Instant::now();
    let due = match *last {
        Some(t) => now.duration_since(t) >= LOG_INTERVAL,
        None => {
            *last = Some(now);
            false
        }
    };
    if !due {
        return;
    }
    *last = Some(now);

    let (eu_n, eu_total, eu_max) = ENGINE_UPDATE.drain();
    let (rf_n, rf_total, rf_max) = RENDER_FRAME.drain();
    let (sd_n, sd_total, sd_max) = SCENE_DRAW.drain();
    let (sf_n, sf_total, sf_max) = SKIA_FLUSH.drain();
    let (ps_n, ps_total, ps_max) = PLANE_SYNC.drain();

    let mean_us = |n: u64, total: u64| -> f32 {
        if n == 0 {
            0.0
        } else {
            (total as f32 / n as f32) / 1_000.0
        }
    };
    let max_us = |max: u64| -> f32 { max as f32 / 1_000.0 };

    tracing::debug!(
        target: "otto::perf.compose",
        engine_update_n = eu_n,
        engine_update_mean_us = mean_us(eu_n, eu_total),
        engine_update_max_us = max_us(eu_max),
        render_frame_n = rf_n,
        render_frame_mean_us = mean_us(rf_n, rf_total),
        render_frame_max_us = max_us(rf_max),
        scene_draw_n = sd_n,
        scene_draw_mean_us = mean_us(sd_n, sd_total),
        scene_draw_max_us = max_us(sd_max),
        skia_flush_n = sf_n,
        skia_flush_mean_us = mean_us(sf_n, sf_total),
        skia_flush_max_us = max_us(sf_max),
        plane_sync_n = ps_n,
        plane_sync_mean_us = mean_us(ps_n, ps_total),
        plane_sync_max_us = max_us(ps_max),
        "compose phase timings (per-second window)",
    );

    // One line per plane that re-rendered in this window. Separate lines
    // rather than fields on the line above: the label set is dynamic, and a
    // plane that never redrew should be absent rather than logged as zero.
    let drained: Vec<_> = {
        let mut planes = PLANES.lock().unwrap();
        let snapshot = planes.clone();
        planes.clear();
        snapshot
    };
    for (label, count, total, max, flush) in drained {
        // draw = everything that is not the blocking flush: Skia op recording,
        // swapchain acquire, surface setup.
        let draw = total.saturating_sub(flush);
        tracing::debug!(
            target: "otto::perf.compose",
            plane = label,
            n = count,
            mean_us = mean_us(count, total),
            draw_mean_us = mean_us(count, draw),
            flush_mean_us = mean_us(count, flush),
            max_us = max_us(max),
            "plane render timings (per-second window)",
        );
    }
}

#[cfg(not(feature = "perf-counters"))]
#[inline]
pub fn log_if_due() {}
