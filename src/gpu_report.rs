//! GPU performance report generator.
//!
//! Collects system information, GL context details, output configuration,
//! and runtime render metrics, then writes a human-readable report to a file
//! that users can attach to bug reports.

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use smithay::output::Output;
use tracing::{error, info};

/// Information about the GL/GPU context, collected once at renderer init.
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub gl_vendor: String,
    pub gl_renderer: String,
    pub gl_version: String,
    pub gl_extensions: Vec<String>,
}

impl GpuInfo {
    /// Collect GL strings from the current GL context.
    ///
    /// # Safety
    /// Must be called with a current GL context.
    pub unsafe fn from_gl(gl: &smithay::backend::renderer::gles::ffi::Gles2) -> Self {
        use std::ffi::CStr;
        use std::os::raw::c_char;

        let get_string = |name: u32| -> String {
            let ptr = gl.GetString(name) as *const c_char;
            if ptr.is_null() {
                "<unavailable>".to_string()
            } else {
                CStr::from_ptr(ptr)
                    .to_str()
                    .unwrap_or("<invalid utf8>")
                    .to_string()
            }
        };

        let gl_version = get_string(smithay::backend::renderer::gles::ffi::VERSION);
        let gl_vendor = get_string(smithay::backend::renderer::gles::ffi::VENDOR);
        let gl_renderer = get_string(smithay::backend::renderer::gles::ffi::RENDERER);

        let ext_ptr =
            gl.GetString(smithay::backend::renderer::gles::ffi::EXTENSIONS) as *const c_char;
        let gl_extensions = if ext_ptr.is_null() {
            vec![]
        } else {
            let ext_str = CStr::from_ptr(ext_ptr).to_str().unwrap_or("").to_string();
            ext_str.split(' ').map(|s| s.to_string()).collect()
        };

        Self {
            gl_vendor,
            gl_renderer,
            gl_version,
            gl_extensions,
        }
    }
}

fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Simple UTC breakdown (no external crate needed)
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Days since 1970-01-01 → calendar date (simplified leap-year calc)
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_date(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let months: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &m in &months {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Build the full report string.
pub fn build_report(
    backend_name: &str,
    gpu_info: &GpuInfo,
    outputs: &[OutputInfo],
    #[cfg(feature = "metrics")] metrics: Option<&crate::render_metrics::RenderMetrics>,
) -> String {
    let mut report = String::with_capacity(4096);

    // Header
    let timestamp = format_timestamp();
    let _ = writeln!(report, "Otto GPU Performance Report");
    let _ = writeln!(report, "Generated: {timestamp}");
    let _ = writeln!(report, "========================================\n");

    // System info
    let _ = writeln!(report, "## System");
    if let Ok(contents) = fs::read_to_string("/etc/os-release") {
        for line in contents.lines() {
            if line.starts_with("PRETTY_NAME=") {
                let name = line.trim_start_matches("PRETTY_NAME=").trim_matches('"');
                let _ = writeln!(report, "OS: {name}");
                break;
            }
        }
    }
    if let Ok(release) = fs::read_to_string("/proc/version") {
        let kernel = release
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(report, "Kernel: {kernel}");
    }
    let _ = writeln!(report);

    // GPU / GL info
    let _ = writeln!(report, "## GPU / OpenGL");
    let _ = writeln!(report, "Backend: {backend_name}");
    let _ = writeln!(report, "GL Vendor: {}", gpu_info.gl_vendor);
    let _ = writeln!(report, "GL Renderer: {}", gpu_info.gl_renderer);
    let _ = writeln!(report, "GL Version: {}", gpu_info.gl_version);

    // DRM driver info (if available via sysfs)
    if let Some(drm_info) = read_drm_driver_info() {
        let _ = writeln!(report, "DRM Driver: {drm_info}");
    }
    let _ = writeln!(report);

    // Outputs
    let _ = writeln!(report, "## Outputs");
    if outputs.is_empty() {
        let _ = writeln!(report, "(none detected)");
    } else {
        for o in outputs {
            let _ = writeln!(
                report,
                "- {}: {}x{} @ {:.2}Hz, scale {:.2}, make={}, model={}",
                o.name, o.width, o.height, o.refresh_rate, o.scale, o.make, o.model,
            );
        }
    }
    let _ = writeln!(report);

    // Render metrics
    #[cfg(feature = "metrics")]
    if let Some(m) = metrics {
        let snap = m.get_stats();
        let _ = writeln!(report, "## Render Metrics (since last reset)");
        let _ = writeln!(report, "Frames rendered: {}", snap.frame_count);
        let _ = writeln!(
            report,
            "Avg CPU render time: {:.3} ms",
            snap.avg_render_time_ms
        );
        let _ = writeln!(report, "Damage ratio: {:.1}%", snap.damage_ratio);
        let _ = writeln!(
            report,
            "Pixels damaged/total: {}/{}",
            snap.damaged_pixels, snap.total_pixels
        );
        let _ = writeln!(
            report,
            "Avg damage rects/frame: {:.1}",
            snap.avg_damage_rects
        );

        if let Some(gpu_snap) = m.get_gpu_stats() {
            let _ = writeln!(report);
            let _ = writeln!(report, "## GPU Timing (GL timer queries)");
            let _ = writeln!(report, "GPU frames measured: {}", gpu_snap.gpu_frame_count);
            let _ = writeln!(report, "Avg GPU time: {:.3} ms", gpu_snap.avg_gpu_time_ms);
            let _ = writeln!(report, "Max GPU time: {:.3} ms", gpu_snap.max_gpu_time_ms);
        }

        let _ = writeln!(report);
        let _ = writeln!(report, "## Frame Trigger Breakdown");
        let triggers = m.get_trigger_stats();
        for (reason, count) in &triggers {
            let _ = writeln!(report, "- {reason}: {count}");
        }
        let _ = writeln!(report);
    }

    // GL Extensions (at the end, they're verbose)
    let _ = writeln!(
        report,
        "## GL Extensions ({} total)",
        gpu_info.gl_extensions.len()
    );
    for ext in &gpu_info.gl_extensions {
        let _ = writeln!(report, "  {ext}");
    }

    report
}

/// Information about a connected output, extracted for the report.
pub struct OutputInfo {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub refresh_rate: f64,
    pub scale: f64,
    pub make: String,
    pub model: String,
}

impl OutputInfo {
    pub fn from_output(output: &Output) -> Self {
        let (width, height, refresh) = output
            .current_mode()
            .map(|m| (m.size.w, m.size.h, m.refresh as f64 / 1000.0))
            .unwrap_or((0, 0, 0.0));
        let scale = output.current_scale().fractional_scale();
        let phys = output.physical_properties();
        OutputInfo {
            name: output.name(),
            width,
            height,
            refresh_rate: refresh,
            scale,
            make: phys.make,
            model: phys.model,
        }
    }
}

/// Try to read DRM driver name from sysfs.
fn read_drm_driver_info() -> Option<String> {
    for entry in fs::read_dir("/sys/class/drm/").ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_str()?;
        if name_str.starts_with("card") && !name_str.contains('-') {
            let driver_link = entry.path().join("device/driver");
            if let Ok(target) = fs::read_link(&driver_link) {
                if let Some(driver_name) = target.file_name().and_then(|n| n.to_str()) {
                    return Some(driver_name.to_string());
                }
            }
        }
    }
    None
}

/// Write the report to `$XDG_STATE_HOME/otto/gpu-report-<timestamp>.txt`
/// (falls back to `~/.local/state/otto/`).
pub fn write_report(report: &str) -> Result<PathBuf, std::io::Error> {
    let state_dir = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{home}/.local/state")
    });
    let dir = PathBuf::from(state_dir).join("otto");
    fs::create_dir_all(&dir)?;

    let timestamp = format_timestamp().replace(':', "-");
    let filename = format!("gpu-report-{timestamp}.txt");
    let path = dir.join(filename);

    fs::write(&path, report)?;
    info!("GPU report saved to {}", path.display());
    Ok(path)
}

/// Entry point called from the keybinding handler.
pub fn generate_and_save(
    backend_name: &str,
    gpu_info: &GpuInfo,
    outputs: &[OutputInfo],
    #[cfg(feature = "metrics")] metrics: Option<&crate::render_metrics::RenderMetrics>,
) {
    let report = build_report(
        backend_name,
        gpu_info,
        outputs,
        #[cfg(feature = "metrics")]
        metrics,
    );

    match write_report(&report) {
        Ok(path) => info!("GPU performance report written to {}", path.display()),
        Err(err) => error!(?err, "Failed to write GPU performance report"),
    }
}
