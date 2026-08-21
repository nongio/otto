//! Plane-element lifecycle for the udev backend: lazy swapchain allocation
//! for the per-purpose scene buffers, per-frame node wiring, and the
//! push-if-ready helper used when building the frame's element list.

use std::sync::Arc;

use layers::prelude::Engine;
use smithay::backend::allocator::{gbm::GbmDevice, Fourcc};
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::reexports::drm::control::crtc;

use crate::config::DockPosition;
use crate::render_elements::scene_dmabuf_element::SceneDmabufElement;
use crate::render_elements::workspace_render_elements::WorkspaceRenderElements;
use crate::workspaces::OutputWorkspaces;

use super::types::{SurfaceData, UdevRenderer};

/// Allocate one plane element (full-screen, or a strip when `strip` is set)
/// into `field` if it doesn't exist yet. Idempotent.
#[allow(clippy::too_many_arguments)]
fn ensure_plane(
    field: &mut Option<SceneDmabufElement>,
    engine: &Arc<Engine>,
    gbm: &GbmDevice<DrmDeviceFd>,
    render_node: DrmNode,
    crtc: crtc::Handle,
    size: (i32, i32),
    format: Fourcc,
    opaque: bool,
    label: &'static str,
    strip_origin: Option<(i32, i32)>,
) {
    if field.is_some() {
        return;
    }
    let mut el = SceneDmabufElement::new(engine.clone(), size, label);
    el.opaque = opaque;
    if let Some(origin) = strip_origin {
        el.position = origin;
        el.set_viewport(origin);
    }
    match el.ensure_swapchain(gbm.clone(), format, render_node) {
        Ok(()) => *field = Some(el),
        Err(e) => tracing::warn!("plane alloc failed for {label} on {crtc:?}: {e}"),
    }
}

/// Lazily set up the dmabuf-backed scene elements for this surface.
/// Skipped entirely by the caller when the plane decomposition is disabled
/// for this output — the swapchains would only waste GPU memory.
pub(super) fn ensure_plane_elements(
    surface: &mut SurfaceData,
    engine: &Arc<Engine>,
    gbm: &GbmDevice<DrmDeviceFd>,
    crtc: crtc::Handle,
    mode_size: (i32, i32),
    dock_position: DockPosition,
) {
    let (w, h) = mode_size;
    let render_node = surface.render_node;

    // The dock plane is a strip along the edge the dock lives on, so moving the
    // dock invalidates it: drop it and let it be rebuilt against the new edge.
    if surface.dock_plane_position != Some(dock_position) {
        surface.dock_dmabuf_element = None;
        surface.dock_plane_position = Some(dock_position);
    }

    ensure_plane(
        &mut surface.scene_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, h),
        Fourcc::Xrgb8888,
        true,
        "bg",
        None,
    );
    // The background may only direct-scan the PRIMARY plane: as a
    // full-output opaque buffer it must never float to an overlay
    // above the primary swapchain (it would hide every element that
    // fell back to GPU compositing there).
    if let Some(el) = surface.scene_dmabuf_element.as_mut() {
        el.kind = smithay::backend::renderer::element::Kind::Unspecified;
        // The background is the one plane whose buffer stays on screen while
        // exposé hides its content's ancestor — a render in that state writes
        // black (see `SceneDmabufElement::honor_ancestor_visibility`).
        el.honor_ancestor_visibility = true;
    }
    ensure_plane(
        &mut surface.windows_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, h),
        Fourcc::Argb8888,
        false,
        "windows",
        None,
    );
    ensure_plane(
        &mut surface.expose_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, h),
        Fourcc::Argb8888,
        false,
        "expose",
        None,
    );
    ensure_plane(
        &mut surface.overlay_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, h),
        Fourcc::Argb8888,
        false,
        "overlay",
        None,
    );

    // The promoted-window plane starts at the output's mode size — the
    // largest a window's subtree can be before it is cropped anyway — and is
    // re-allocated to the window's own bounds by `wire_window_plane` as soon
    // as one is promoted. Allocating up front keeps the gbm/format inputs on
    // the element so later resizes need no plumbing.
    ensure_plane(
        &mut surface.window_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, h),
        Fourcc::Argb8888,
        false,
        "window",
        None,
    );

    // Strip-sized planes: full output width, cropped bands of their
    // full-screen containers via the element viewport. Small buffers
    // mean dock/switcher animations no longer redraw a full-screen
    // plane, and the KMS watermark cost scales with plane size.
    let switcher_strip_h = (h / 2).min(960);
    // The dock strip follows the dock: a bottom band, or a side column.
    let (dock_size, dock_origin) = match dock_position {
        DockPosition::Bottom => {
            let strip_h = (h / 4).min(480);
            ((w, strip_h), (0, h - strip_h))
        }
        // Wider than the bottom band is tall: a side dock's tooltips and
        // context menus open *across* the strip, and anything that reaches past
        // its edge is cropped away.
        DockPosition::Left => {
            let strip_w = (w / 2).min(960);
            ((strip_w, h), (0, 0))
        }
        DockPosition::Right => {
            let strip_w = (w / 2).min(960);
            ((strip_w, h), (w - strip_w, 0))
        }
    };
    ensure_plane(
        &mut surface.dock_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        dock_size,
        Fourcc::Argb8888,
        false,
        "dock",
        Some(dock_origin),
    );
    ensure_plane(
        &mut surface.switcher_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, switcher_strip_h),
        Fourcc::Argb8888,
        false,
        "switcher",
        Some((0, (h - switcher_strip_h) / 2)),
    );
}

/// Every frame: point each plane element at its output's scene node and
/// refresh the output's scene origin (its static position in scene space —
/// nonzero for any output right of / below the first). Re-set every frame so
/// a layout change on hotplug propagates without extra plumbing.
pub(super) fn wire_plane_nodes(surface: &SurfaceData, ows: &OutputWorkspaces) {
    let pos = ows.output_layer.render_position();
    let origin = (pos.x as i32, pos.y as i32);
    if let Some(el) = &surface.scene_dmabuf_element {
        el.set_node_ref(ows.background_plane.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.windows_dmabuf_element {
        el.set_node_ref(ows.windows_plane.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.expose_dmabuf_element {
        el.set_node_ref(ows.expose_layer.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.overlay_dmabuf_element {
        el.set_node_ref(ows.overlay_plane.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.switcher_dmabuf_element {
        el.set_node_ref(ows.switcher_plane.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.dock_dmabuf_element {
        el.set_node_ref(ows.dock_plane.id);
        el.set_scene_origin(origin);
    }
    if let Some(el) = &surface.window_dmabuf_element {
        el.set_node_ref(ows.promoted_plane.id);
        el.set_scene_origin(origin);
    }
}

/// Size and place the promoted-window plane for this frame.
///
/// Unlike every other plane, this one's buffer is the promoted window's own
/// bounds (shadow included) and its origin moves with the window, so both are
/// refreshed here before the element renders. Returns whether the plane has a
/// window to draw at all — the caller skips rendering and pushing it otherwise,
/// so an idle output does not carry a window-sized buffer it never shows.
///
/// A resize drops the swapchain, so this MUST run before the element renders
/// in the same frame; otherwise the plane sits out a frame and the window
/// blinks out of the stack.
pub(super) fn wire_window_plane(
    surface: &mut SurfaceData,
    workspaces: &crate::workspaces::Workspaces,
    output_name: &str,
    mode_size: (i32, i32),
) -> bool {
    let bounds = workspaces.promoted_plane_bounds(output_name);
    let Some((origin, size)) = bounds else {
        surface.window_plane_active = false;
        return false;
    };
    // Crop to the output. The subtree's bounds include the shadow's safe area
    // on every side, so even a modest window overhangs the screen edges — and
    // a plane whose destination is not fully inside the CRTC is rejected, which
    // would silently drop the window back into GPU composite.
    let (mw, mh) = mode_size;
    let x = origin.x.max(0);
    let y = origin.y.max(0);
    let w = (origin.x + size.0).min(mw) - x;
    let h = (origin.y + size.1).min(mh) - y;
    if w <= 0 || h <= 0 {
        surface.window_plane_active = false;
        return false;
    }
    let (origin, size) = ((x, y), (w, h));
    let Some(el) = surface.window_dmabuf_element.as_mut() else {
        surface.window_plane_active = false;
        return false;
    };
    let resized = el.resize(size);
    el.set_origin(origin);
    // Re-entering the frame after sitting out (or after a resize dropped every
    // slot) leaves the buffer holding a stale window — repaint all of it.
    if resized || !surface.window_plane_active {
        el.request_full_render();
    }
    surface.window_plane_active = true;
    true
}

/// Push a plane element into the frame's element list if it has a dmabuf.
/// Pushed even when nothing new was rendered this frame: the existing
/// dmabuf stays on the plane and Smithay sees an unchanged commit_counter
/// → empty damage → no page-flip.
pub(super) fn push_ready<'a>(
    el: &Option<SceneDmabufElement>,
    out: &mut Vec<WorkspaceRenderElements<'a, UdevRenderer<'a>>>,
) {
    if let Some(el) = el.as_ref() {
        if el.current_dmabuf().is_some() {
            out.push(WorkspaceRenderElements::SceneDmabuf(el.clone()));
        }
    }
}

/// Reclaim the GPU memory of a plane whose UI has been closed for
/// [`PLANE_RELEASE_AFTER`]: a full-screen swapchain (up to 4 slots of
/// ~22 MB at 3K) otherwise stays allocated forever after its first use.
/// Dropping the element is enough — buffer allocation is lazy and
/// [`ensure_plane_elements`] recreates it on the next active frame (the
/// recreated element renders cold, which the first frame of a reopening
/// UI does anyway). Frames only run when something draws, so release
/// happens on the first frame at least this long after the UI closed.
pub(super) const PLANE_RELEASE_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) fn maybe_release_plane(
    el: &mut Option<SceneDmabufElement>,
    active: bool,
    last_active: &mut Option<std::time::Instant>,
    label: &str,
) {
    if active {
        *last_active = Some(std::time::Instant::now());
        return;
    }
    if el.is_some() {
        let idle_since = last_active.get_or_insert_with(std::time::Instant::now);
        if idle_since.elapsed() >= PLANE_RELEASE_AFTER {
            *el = None;
            tracing::debug!(target: "otto::planes", "released {label} plane swapchain");
        }
    }
}
