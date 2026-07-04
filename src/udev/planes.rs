//! Plane-element lifecycle for the udev backend: lazy swapchain allocation
//! for the per-purpose scene buffers, per-frame node wiring, and the
//! push-if-ready helper used when building the frame's element list.

use std::sync::Arc;

use layers::prelude::Engine;
use smithay::backend::allocator::{gbm::GbmDevice, Fourcc};
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::reexports::drm::control::crtc;

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
    strip_y: Option<i32>,
) {
    if field.is_some() {
        return;
    }
    let mut el = SceneDmabufElement::new(engine.clone(), size, label);
    el.opaque = opaque;
    if let Some(y) = strip_y {
        el.position = (0, y);
        el.set_viewport((0, y));
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
) {
    let (w, h) = mode_size;
    let render_node = surface.render_node;

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

    // Strip-sized planes: full output width, cropped bands of their
    // full-screen containers via the element viewport. Small buffers
    // mean dock/switcher animations no longer redraw a full-screen
    // plane, and the KMS watermark cost scales with plane size.
    let dock_strip_h = (h / 4).min(480);
    let switcher_strip_h = (h / 2).min(960);
    ensure_plane(
        &mut surface.dock_dmabuf_element,
        engine,
        gbm,
        render_node,
        crtc,
        (w, dock_strip_h),
        Fourcc::Argb8888,
        false,
        "dock",
        Some(h - dock_strip_h),
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
        Some((h - switcher_strip_h) / 2),
    );
}

/// Every frame: point each plane element at its output's scene node.
pub(super) fn wire_plane_nodes(surface: &SurfaceData, ows: &OutputWorkspaces) {
    if let Some(el) = &surface.scene_dmabuf_element {
        el.set_node_ref(ows.background_plane.id);
    }
    if let Some(el) = &surface.windows_dmabuf_element {
        el.set_node_ref(ows.windows_plane.id);
    }
    if let Some(el) = &surface.expose_dmabuf_element {
        el.set_node_ref(ows.expose_layer.id);
    }
    if let Some(el) = &surface.overlay_dmabuf_element {
        el.set_node_ref(ows.overlay_plane.id);
    }
    if let Some(el) = &surface.switcher_dmabuf_element {
        el.set_node_ref(ows.switcher_plane.id);
    }
    if let Some(el) = &surface.dock_dmabuf_element {
        el.set_node_ref(ows.dock_plane.id);
    }
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
