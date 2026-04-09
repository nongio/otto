use smithay::utils::{Physical, Rectangle};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Reason a frame was triggered — used for attribution in reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameTrigger {
    PointerMove,
    SurfaceCommit,
    Animation,
    WindowManagement,
    Other,
}

impl FrameTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::PointerMove => "pointer_move",
            Self::SurfaceCommit => "surface_commit",
            Self::Animation => "animation",
            Self::WindowManagement => "window_management",
            Self::Other => "other",
        }
    }
}

#[derive(Debug)]
pub struct RenderMetrics {
    backend_name: &'static str,
    frame_count: AtomicU64,
    total_render_time_ns: AtomicU64,
    total_pixels: AtomicU64,
    damaged_pixels: AtomicU64,
    damage_rect_count: AtomicU64,
    last_log_time: std::sync::Mutex<Option<Instant>>,

    // GPU timer query results
    gpu_frame_count: AtomicU64,
    total_gpu_time_ns: AtomicU64,
    max_gpu_time_ns: AtomicU64,

    // Frame trigger attribution
    trigger_pointer_move: AtomicU64,
    trigger_surface_commit: AtomicU64,
    trigger_animation: AtomicU64,
    trigger_window_management: AtomicU64,
    trigger_other: AtomicU64,
}

impl RenderMetrics {
    pub fn new(backend_name: &'static str) -> Self {
        Self {
            backend_name,
            frame_count: AtomicU64::new(0),
            total_render_time_ns: AtomicU64::new(0),
            total_pixels: AtomicU64::new(0),
            damaged_pixels: AtomicU64::new(0),
            damage_rect_count: AtomicU64::new(0),
            last_log_time: std::sync::Mutex::new(None),
            gpu_frame_count: AtomicU64::new(0),
            total_gpu_time_ns: AtomicU64::new(0),
            max_gpu_time_ns: AtomicU64::new(0),
            trigger_pointer_move: AtomicU64::new(0),
            trigger_surface_commit: AtomicU64::new(0),
            trigger_animation: AtomicU64::new(0),
            trigger_window_management: AtomicU64::new(0),
            trigger_other: AtomicU64::new(0),
        }
    }

    pub fn start_frame(&self) -> FrameTimer<'_> {
        FrameTimer {
            start: Instant::now(),
            metrics: self,
        }
    }

    pub fn record_damage(&self, output_size: (i32, i32), damage: &[Rectangle<i32, Physical>]) {
        let total = (output_size.0 * output_size.1) as u64;
        let damaged: u64 = damage
            .iter()
            .map(|rect| (rect.size.w * rect.size.h) as u64)
            .sum();

        self.total_pixels.fetch_add(total, Ordering::Relaxed);
        self.damaged_pixels.fetch_add(damaged, Ordering::Relaxed);
        self.damage_rect_count
            .fetch_add(damage.len() as u64, Ordering::Relaxed);
    }

    fn record_frame_time(&self, duration: Duration) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        self.total_render_time_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record GPU time for a frame (from GL timer query result, in nanoseconds).
    pub fn record_gpu_time_ns(&self, gpu_ns: u64) {
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        self.total_gpu_time_ns.fetch_add(gpu_ns, Ordering::Relaxed);
        // Update max (relaxed CAS loop)
        let mut current = self.max_gpu_time_ns.load(Ordering::Relaxed);
        while gpu_ns > current {
            match self.max_gpu_time_ns.compare_exchange_weak(
                current,
                gpu_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Record what triggered this frame.
    pub fn record_trigger(&self, trigger: FrameTrigger) {
        match trigger {
            FrameTrigger::PointerMove => &self.trigger_pointer_move,
            FrameTrigger::SurfaceCommit => &self.trigger_surface_commit,
            FrameTrigger::Animation => &self.trigger_animation,
            FrameTrigger::WindowManagement => &self.trigger_window_management,
            FrameTrigger::Other => &self.trigger_other,
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn maybe_log_stats(&self, force: bool) {
        let mut last_log = self.last_log_time.lock().unwrap();
        let should_log = if force {
            true
        } else if let Some(last) = *last_log {
            last.elapsed() >= Duration::from_secs(5)
        } else {
            true
        };

        if !should_log {
            return;
        }

        let frame_count = self.frame_count.load(Ordering::Relaxed);
        if frame_count == 0 {
            return;
        }

        let total_render_ns = self.total_render_time_ns.load(Ordering::Relaxed);
        let total_pixels = self.total_pixels.load(Ordering::Relaxed);
        let damaged_pixels = self.damaged_pixels.load(Ordering::Relaxed);
        let damage_rect_count = self.damage_rect_count.load(Ordering::Relaxed);

        let avg_render_ms = (total_render_ns as f64 / frame_count as f64) / 1_000_000.0;
        let damage_ratio = if total_pixels > 0 {
            (damaged_pixels as f64 / total_pixels as f64) * 100.0
        } else {
            0.0
        };
        let avg_rects = damage_rect_count as f64 / frame_count as f64;

        let gpu_count = self.gpu_frame_count.load(Ordering::Relaxed);
        let gpu_info = if gpu_count > 0 {
            let avg_gpu_ms = (self.total_gpu_time_ns.load(Ordering::Relaxed) as f64
                / gpu_count as f64)
                / 1_000_000.0;
            let max_gpu_ms = self.max_gpu_time_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            format!(", gpu avg {avg_gpu_ms:.2}ms max {max_gpu_ms:.2}ms")
        } else {
            String::new()
        };

        tracing::info!(
            "RENDER METRICS [{}]: {} frames, avg {:.2}ms/frame{}, damage {:.1}% ({}/{} px), avg {:.1} rects/frame",
            self.backend_name,
            frame_count,
            avg_render_ms,
            gpu_info,
            damage_ratio,
            damaged_pixels,
            total_pixels,
            avg_rects
        );

        self.reset();
        *last_log = Some(Instant::now());
    }

    pub fn reset(&self) {
        self.frame_count.store(0, Ordering::Relaxed);
        self.total_render_time_ns.store(0, Ordering::Relaxed);
        self.total_pixels.store(0, Ordering::Relaxed);
        self.damaged_pixels.store(0, Ordering::Relaxed);
        self.damage_rect_count.store(0, Ordering::Relaxed);
        self.gpu_frame_count.store(0, Ordering::Relaxed);
        self.total_gpu_time_ns.store(0, Ordering::Relaxed);
        self.max_gpu_time_ns.store(0, Ordering::Relaxed);
        // Note: trigger counters are NOT reset — they accumulate for the report lifetime.
    }

    pub fn get_stats(&self) -> MetricsSnapshot {
        let frame_count = self.frame_count.load(Ordering::Relaxed);
        let total_render_ns = self.total_render_time_ns.load(Ordering::Relaxed);
        let total_pixels = self.total_pixels.load(Ordering::Relaxed);
        let damaged_pixels = self.damaged_pixels.load(Ordering::Relaxed);
        let damage_rect_count = self.damage_rect_count.load(Ordering::Relaxed);

        MetricsSnapshot {
            frame_count,
            avg_render_time_ms: if frame_count > 0 {
                (total_render_ns as f64 / frame_count as f64) / 1_000_000.0
            } else {
                0.0
            },
            damage_ratio: if total_pixels > 0 {
                (damaged_pixels as f64 / total_pixels as f64) * 100.0
            } else {
                0.0
            },
            total_pixels,
            damaged_pixels,
            avg_damage_rects: if frame_count > 0 {
                damage_rect_count as f64 / frame_count as f64
            } else {
                0.0
            },
        }
    }

    /// Get GPU timing stats, or `None` if no GPU timer data was recorded.
    pub fn get_gpu_stats(&self) -> Option<GpuTimingSnapshot> {
        let count = self.gpu_frame_count.load(Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        let total_ns = self.total_gpu_time_ns.load(Ordering::Relaxed);
        let max_ns = self.max_gpu_time_ns.load(Ordering::Relaxed);
        Some(GpuTimingSnapshot {
            gpu_frame_count: count,
            avg_gpu_time_ms: (total_ns as f64 / count as f64) / 1_000_000.0,
            max_gpu_time_ms: max_ns as f64 / 1_000_000.0,
        })
    }

    /// Get frame trigger breakdown as `(label, count)` pairs.
    pub fn get_trigger_stats(&self) -> Vec<(&'static str, u64)> {
        vec![
            (
                FrameTrigger::PointerMove.label(),
                self.trigger_pointer_move.load(Ordering::Relaxed),
            ),
            (
                FrameTrigger::SurfaceCommit.label(),
                self.trigger_surface_commit.load(Ordering::Relaxed),
            ),
            (
                FrameTrigger::Animation.label(),
                self.trigger_animation.load(Ordering::Relaxed),
            ),
            (
                FrameTrigger::WindowManagement.label(),
                self.trigger_window_management.load(Ordering::Relaxed),
            ),
            (
                FrameTrigger::Other.label(),
                self.trigger_other.load(Ordering::Relaxed),
            ),
        ]
    }
}

pub struct FrameTimer<'a> {
    start: Instant,
    metrics: &'a RenderMetrics,
}

impl Drop for FrameTimer<'_> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.metrics.record_frame_time(duration);
    }
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub frame_count: u64,
    pub avg_render_time_ms: f64,
    pub damage_ratio: f64,
    pub total_pixels: u64,
    pub damaged_pixels: u64,
    pub avg_damage_rects: f64,
}

impl MetricsSnapshot {
    pub fn print_summary(&self, label: &str) {
        println!("\n=== {} ===", label);
        println!("Frames rendered: {}", self.frame_count);
        println!("Avg render time: {:.3}ms", self.avg_render_time_ms);
        println!("Damage ratio: {:.1}%", self.damage_ratio);
        println!(
            "Pixels: {}/{} damaged",
            self.damaged_pixels, self.total_pixels
        );
        println!("Avg damage rects: {:.1}", self.avg_damage_rects);
        println!("================\n");
    }
}

#[derive(Debug, Clone)]
pub struct GpuTimingSnapshot {
    pub gpu_frame_count: u64,
    pub avg_gpu_time_ms: f64,
    pub max_gpu_time_ms: f64,
}

/// Manages a GL timer query for measuring GPU execution time of a single frame.
///
/// Usage:
/// ```ignore
/// let timer = GpuTimer::begin(&gl);
/// // ... render ...
/// timer.end(&gl); // inserts end query
/// // On a later frame, call try_collect() to read the result without blocking.
/// ```
pub struct GpuTimer {
    query_id: u32,
    ended: bool,
}

impl GpuTimer {
    /// Start a GPU timer query. Returns `None` if timer queries aren't supported.
    ///
    /// # Safety
    /// Must be called with a current GL context.
    pub unsafe fn begin(gl: &smithay::backend::renderer::gles::ffi::Gles2) -> Option<Self> {
        let mut query_id: u32 = 0;
        gl.GenQueries(1, &mut query_id);
        if query_id == 0 {
            return None;
        }
        gl.BeginQuery(0x88BF /* GL_TIME_ELAPSED */, query_id);
        Some(Self {
            query_id,
            ended: false,
        })
    }

    /// End the timer query. Must be called before `try_collect`.
    ///
    /// # Safety
    /// Must be called with a current GL context.
    pub unsafe fn end(&mut self, gl: &smithay::backend::renderer::gles::ffi::Gles2) {
        if !self.ended {
            gl.EndQuery(0x88BF /* GL_TIME_ELAPSED */);
            self.ended = true;
        }
    }

    /// Try to read the query result without blocking.
    /// Returns `Some(nanoseconds)` if available, `None` if not yet ready.
    ///
    /// # Safety
    /// Must be called with a current GL context.
    pub unsafe fn try_collect(
        self,
        gl: &smithay::backend::renderer::gles::ffi::Gles2,
    ) -> Option<u64> {
        if !self.ended {
            gl.DeleteQueries(1, &self.query_id);
            return None;
        }

        let mut available: i32 = 0;
        gl.GetQueryObjectuiv(
            self.query_id,
            0x8867, /* GL_QUERY_RESULT_AVAILABLE */
            &mut available as *mut i32 as *mut u32,
        );
        if available == 0 {
            // Not ready yet — caller should retry next frame.
            // Don't delete the query; return None and let the caller manage.
            // Actually, we consume self, so we must handle cleanup.
            gl.DeleteQueries(1, &self.query_id);
            return None;
        }

        let mut result: u64 = 0;
        // GL_QUERY_RESULT for 64-bit: use GetQueryObjectui64v if available,
        // otherwise fall back to 32-bit.
        gl.GetQueryObjectuiv(
            self.query_id,
            0x8866, /* GL_QUERY_RESULT */
            &mut result as *mut u64 as *mut u32,
        );
        gl.DeleteQueries(1, &self.query_id);
        Some(result)
    }
}
