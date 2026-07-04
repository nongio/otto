//! Runtime debug tooling for the plane pipeline: /tmp touch-file toggles,
//! per-plane PNG dumps, and the 1 Hz frame-realization log.

use smithay::backend::renderer::element::RenderElementStates;

use super::types::SurfaceData;

/// Once per second: refresh the /tmp debug toggles into atomics (so the hot
/// path never stat()s files) and log how each plane element was realized.
/// Call after `render_frame` with the frame's element states.
pub(super) fn debug_tick(
    surface: &mut SurfaceData,
    states: &RenderElementStates,
    expose_active: bool,
) {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static LAST: OnceLock<Mutex<Instant>> = OnceLock::new();
    let mut last = LAST
        .get_or_init(|| Mutex::new(Instant::now() - Duration::from_secs(2)))
        .lock()
        .unwrap();
    if last.elapsed() < Duration::from_secs(1) {
        return;
    }
    *last = Instant::now();

    refresh_debug_toggles(surface);
    log_frame_realization(surface, states, expose_active);
}

/// Debug: `touch /tmp/otto-tint` tints everything GPU-composited red
/// (client textures via Smithay's DebugFlags::TINT, our plane fallback
/// blits via TINT_COMPOSITE); zero-copy plane scanout stays untinted.
/// `touch /tmp/otto-no-scanout` disables window promotion (A/B testing).
/// `touch /tmp/otto-dump-planes` requests a one-shot per-plane PNG dump.
fn refresh_debug_toggles(surface: &mut SurfaceData) {
    use crate::render_elements::scene_dmabuf_element::{DUMP_PLANES, NO_SCANOUT, TINT_COMPOSITE};
    use smithay::backend::renderer::DebugFlags;
    use std::sync::atomic::Ordering;

    let tint = std::path::Path::new("/tmp/otto-tint").exists();
    let flags = if tint {
        DebugFlags::TINT
    } else {
        DebugFlags::empty()
    };
    if surface.compositor.debug_flags() != flags {
        surface.compositor.set_debug_flags(flags);
        tracing::info!(target: "otto::planes", "composite tint {}", if tint { "ON" } else { "OFF" });
    }
    TINT_COMPOSITE.store(tint, Ordering::Relaxed);

    NO_SCANOUT.store(
        std::path::Path::new("/tmp/otto-no-scanout").exists(),
        Ordering::Relaxed,
    );
    if std::path::Path::new("/tmp/otto-dump-planes").exists() {
        let _ = std::fs::remove_file("/tmp/otto-dump-planes");
        DUMP_PLANES.store(true, Ordering::Relaxed);
    }
}

/// Log how each plane element was realized (ZeroCopy = on a hardware
/// plane, Rendering = GPU-composited into the primary swapchain, absent =
/// not part of this frame at all), plus a histogram over every element
/// smithay saw this frame — client buffers (direct scanout candidates)
/// show up there even though their ids can't be matched to a plane.
fn log_frame_realization(surface: &SurfaceData, states: &RenderElementStates, expose_active: bool) {
    let mut summary = String::new();
    macro_rules! log_state {
        ($el:expr, $name:literal) => {
            if let Some(el) = $el.as_ref() {
                use smithay::backend::renderer::element::Element as _;
                let s = states
                    .element_render_state(el.id().clone())
                    .map(|s| format!("{:?}", s.presentation_state))
                    .unwrap_or_else(|| "absent".into());
                summary.push_str(&format!("{}[{:?}]={} ", $name, el.id(), s));
            }
        };
    }
    log_state!(surface.scene_dmabuf_element, "bg");
    log_state!(surface.windows_dmabuf_element, "windows");
    log_state!(surface.expose_dmabuf_element, "expose");
    log_state!(surface.overlay_dmabuf_element, "overlay");
    log_state!(surface.switcher_dmabuf_element, "switcher");
    log_state!(surface.dock_dmabuf_element, "dock");
    let (mut zc, mut rend, mut skip) = (0, 0, 0);
    for s in states.states.values() {
        use smithay::backend::renderer::element::RenderElementPresentationState as P;
        match s.presentation_state {
            P::ZeroCopy => zc += 1,
            P::Rendering { .. } => rend += 1,
            P::Skipped => skip += 1,
        }
    }
    tracing::debug!(
        target: "otto::planes",
        "frame realization: {summary}expose_active={expose_active} shadow_only={} elements: total={} zerocopy={zc} rendering={rend} skipped={skip}",
        surface.shadow_only_windows.len(),
        states.states.len(),
    );
}

/// Debug: dump every plane buffer to PNG when requested
/// (`touch /tmp/otto-dump-planes`; the file is polled at 1 Hz and converted
/// into a one-shot flag). Shows exactly what each KMS plane scans out,
/// independent of the GPU-composited screencopy.
pub(super) fn maybe_dump_planes(surface: &SurfaceData) {
    if !crate::render_elements::scene_dmabuf_element::DUMP_PLANES
        .swap(false, std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = format!("{home}/Pictures/Screenshots");
    let _ = std::fs::create_dir_all(&dir);
    macro_rules! dump_plane {
        ($el:expr, $name:literal) => {
            if let Some(el) = &$el {
                el.save_to_png(&format!("{dir}/otto_plane_{}.png", $name));
            }
        };
    }
    dump_plane!(surface.scene_dmabuf_element, "bg");
    dump_plane!(surface.windows_dmabuf_element, "windows");
    dump_plane!(surface.expose_dmabuf_element, "expose");
    dump_plane!(surface.overlay_dmabuf_element, "overlay");
    dump_plane!(surface.switcher_dmabuf_element, "switcher");
    dump_plane!(surface.dock_dmabuf_element, "dock");
    tracing::info!(target: "otto::planes", "plane buffers dumped to {dir}");
}

/// Debug PNG saves — triggered by Shift+6..9 (debug-kms feature only).
#[cfg(feature = "debug-kms")]
pub(super) fn maybe_save_planes(surface: &SurfaceData) {
    use crate::input::keyboard::*;
    use std::sync::atomic::Ordering;
    macro_rules! dbg_save {
        ($flag:expr, $el:expr, $path:expr) => {
            if $flag.swap(false, Ordering::Relaxed) {
                if let Some(el) = &$el {
                    el.save_to_png(&$path);
                }
            }
        };
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let ss_dir = std::path::PathBuf::from(format!("{home}/Pictures/Screenshots"));
    macro_rules! ss_path {
        ($name:literal) => {
            ss_dir
                .join(format!("otto_plane_{}.png", $name))
                .to_string_lossy()
                .into_owned()
        };
    }
    dbg_save!(DBG_SAVE_BG, surface.scene_dmabuf_element, ss_path!("bg"));
    dbg_save!(DBG_SAVE_WIN, surface.windows_dmabuf_element, ss_path!("win"));
    dbg_save!(
        DBG_SAVE_EXPOSE,
        surface.expose_dmabuf_element,
        ss_path!("expose")
    );
    dbg_save!(
        DBG_SAVE_OVERLAY,
        surface.overlay_dmabuf_element,
        ss_path!("overlay")
    );
}
