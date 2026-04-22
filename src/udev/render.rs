// Rendering module - Surface rendering and frame management
//
// This module contains the core rendering logic for the udev backend:
// - Frame submission and presentation feedback
// - Surface rendering pipeline
// - Direct scanout optimization
// - Screenshare integration

use std::{
    io,
    time::{Duration, Instant},
};

use crate::{
    config::Config,
    cursor::{CursorManager, CursorTextureCache},
    drawing::*,
    render::*,
    render_elements::workspace_render_elements::WorkspaceRenderElements,
    render_elements::output_render_elements::OutputRenderElements,
    shell::{WindowElement, WindowRenderElement},
    state::{post_repaint, take_presentation_feedback, SurfaceDmabufFeedback},
};

use smithay::{
    backend::{
        drm::{DrmAccessError, DrmError, DrmEventMetadata, DrmNode},
        renderer::{
            damage::OutputDamageTracker,
            element::Kind,
            Bind,
        },
        SwapBuffersError,
    },
    output::Output,
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            LoopHandle,
        },
        drm::control::crtc,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::protocol::wl_surface,
    },
    utils::{Clock, IsAlive, Logical, Monotonic, Physical, Point, Rectangle, Scale},
    wayland::presentation::Refresh,
};
use tracing::{debug, trace, warn};

use super::types::{GbmDrmCompositor, RenderOutcome, RenderTier, SurfaceData, UdevData, UdevOutputId, UdevRenderer};
use crate::state::Otto;

// Type alias for the framebuffer returned when binding a Dmabuf with UdevRenderer
// type UdevFramebuffer<'a> = smithay::backend::renderer::multigpu::MultiFramebuffer<
//     'a,
//     smithay::backend::renderer::multigpu::gbm::GbmGlesBackend<
//         crate::skia_renderer::SkiaRenderer,
//         smithay::backend::drm::DrmDeviceFd,
//     >,
//     smithay::backend::renderer::multigpu::gbm::GbmGlesBackend<
//         crate::skia_renderer::SkiaRenderer,
//         smithay::backend::drm::DrmDeviceFd,
//     >,
// >;

impl Otto<UdevData> {
    pub(super) fn frame_finish(
        &mut self,
        dev_id: DrmNode,
        crtc: crtc::Handle,
        metadata: &mut Option<DrmEventMetadata>,
    ) {
        profiling::scope!("frame_finish", &format!("{crtc:?}"));
        // P3 fps counter — log once per second
        {
            use std::sync::Mutex;
            use std::sync::OnceLock;
            static FPS: OnceLock<Mutex<(Instant, u32)>> = OnceLock::new();
            let mut g = FPS
                .get_or_init(|| Mutex::new((Instant::now(), 0)))
                .lock()
                .unwrap();
            g.1 += 1;
            let elapsed = g.0.elapsed();
            if elapsed >= Duration::from_secs(1) {
                let fps = g.1 as f64 / elapsed.as_secs_f64();
                tracing::info!(target: "otto::fps", "fps={fps:.1} ({} frames in {:.2}s)", g.1, elapsed.as_secs_f64());
                g.0 = Instant::now();
                g.1 = 0;
            }
        }

        let device_backend = match self.backend_data.backends.get_mut(&dev_id) {
            Some(backend) => backend,
            None => {
                tracing::error!("Trying to finish frame on non-existent backend {}", dev_id);
                return;
            }
        };

        let surface = match device_backend.surfaces.get_mut(&crtc) {
            Some(surface) => surface,
            None => {
                tracing::error!("Trying to finish frame on non-existent crtc {:?}", crtc);
                return;
            }
        };

        let output = if let Some(output) = self.workspaces.outputs().find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device_id == surface.device_id && id.crtc == crtc)
                .unwrap_or(false)
        }) {
            output.clone()
        } else {
            // somehow we got called with an invalid output
            return;
        };

        let schedule_render =
            match surface.compositor.frame_submitted() {
                Ok(user_data) => {
                    if let Some(mut feedback) = user_data.flatten() {
                        let tp = metadata.as_ref().and_then(|metadata| match metadata.time {
                            smithay::backend::drm::DrmEventTime::Monotonic(tp) => Some(tp),
                            smithay::backend::drm::DrmEventTime::Realtime(_) => None,
                        });
                        let seq = metadata
                            .as_ref()
                            .map(|metadata| metadata.sequence)
                            .unwrap_or(0);

                        let (clock, flags) = if let Some(tp) = tp {
                            (
                                tp.into(),
                                wp_presentation_feedback::Kind::Vsync
                                    | wp_presentation_feedback::Kind::HwClock
                                    | wp_presentation_feedback::Kind::HwCompletion,
                            )
                        } else {
                            (self.clock.now(), wp_presentation_feedback::Kind::Vsync)
                        };

                        feedback.presented(
                            clock,
                            output
                                .current_mode()
                                .map(|mode| {
                                    Refresh::fixed(Duration::from_nanos(
                                        1_000_000_000_000 / mode.refresh as u64,
                                    ))
                                })
                                .unwrap_or(Refresh::Unknown),
                            seq as u64,
                            flags,
                        );
                    }

                    true
                }
                Err(err) => {
                    use smithay::backend::drm::compositor::FrameError;

                    // Log as debug for DeviceInactive (expected during suspend), warn for others
                    let is_device_inactive =
                        matches!(&err, FrameError::DrmError(DrmError::DeviceInactive));

                    if is_device_inactive {
                        debug!(
                            "Device inactive during rendering (expected during suspend): {:?}",
                            err
                        );
                    } else {
                        warn!("Error during rendering: {:?}", err);
                    }

                    match err {
                        FrameError::DrmError(DrmError::DeviceInactive) => {
                            // If the device has been deactivated do not reschedule, this will be done
                            // by session resume
                            false
                        }
                        FrameError::DrmError(DrmError::Access(DrmAccessError {
                            source, ..
                        })) if source.kind() == io::ErrorKind::PermissionDenied => true,
                        _ => {
                            panic!("Rendering loop lost: {}", err);
                        }
                    }
                }
            };

        if schedule_render {
            let output_refresh = match output.current_mode() {
                Some(mode) => mode.refresh,
                None => return,
            };

            // ── Frame-pipeline Phase 1: CPU scene update ─────────────────────
            //
            // The GPU is still scanning out the frame we just acknowledged, so
            // the CPU is free.  We tick the scene graph now, at VBlank time,
            // and cache the result.  The upcoming draw phase (Phase 2) will
            // consume the cached value instead of calling update() inline,
            // which shortens the critical path on the GPU-submit side and
            // keeps the two stages maximally overlapped.
            //
            // `scene_element` and `backend_data` are distinct fields of `self`,
            // so Rust's field-projection rules allow concurrent access here.
            let scene_has_damage = self.scene_element.update();
            surface.prefetched_scene_damage = Some(scene_has_damage);

            // ── Frame-pipeline Phase 2: schedule the draw at the deadline ─────
            //
            // We want to submit the next page flip as close to the upcoming
            // VBlank as possible (minimises input latency) while still leaving
            // enough margin for the GPU command recording to finish on time.
            //
            // Target deadline = frame_period − 2×avg_render_time
            //   • 2× gives a safety margin for render-time variance.
            //   • Clamped to at least 1 ms to avoid busy-spinning.
            //
            // For multi-GPU paths a buffer copy is needed after rendering; we
            // have no reliable estimate for the copy duration, so we fire
            // immediately and accept the slightly-wider timing window.
            let is_multi_gpu = self.backend_data.primary_gpu != surface.render_node;
            let timer = if is_multi_gpu {
                Timer::immediate()
            } else {
                // output_refresh is in millihertz (mHz); convert to µs/frame.
                let frame_period_us = 1_000_000_000f32 / output_refresh as f32;
                let avg_us = surface.avg_render_time_us;
                let draw_delay_us = (frame_period_us - avg_us * 2.0).max(1_000.0);
                trace!(
                    draw_delay_us,
                    frame_period_us,
                    avg_us,
                    "scheduling draw phase on {:?}",
                    crtc
                );
                Timer::from_duration(Duration::from_micros(draw_delay_us as u64))
            };

            self.handle
                .insert_source(timer, move |_, _, data| {
                    data.render(dev_id, Some(crtc));
                    TimeoutAction::Drop
                })
                .expect("failed to schedule frame timer");
        }
    }

    pub(super) fn render(&mut self, node: DrmNode, crtc: Option<crtc::Handle>) {
        let device_backend = match self.backend_data.backends.get_mut(&node) {
            Some(backend) => backend,
            None => {
                tracing::error!("Trying to render on non-existent backend {}", node);
                return;
            }
        };

        if let Some(crtc) = crtc {
            self.render_surface(node, crtc);
        } else {
            let crtcs: Vec<_> = device_backend.surfaces.keys().copied().collect();
            for crtc in crtcs {
                self.render_surface(node, crtc);
            }
        };

        // Render virtual outputs once per primary GPU cycle
        if node == self.backend_data.primary_gpu {
            self.render_virtual_outputs();
        }
    }

    #[allow(clippy::mutable_key_type)] // ObjectId as HashMap key — see window_throttle.rs
    pub(super) fn render_surface(&mut self, node: DrmNode, crtc: crtc::Handle) {
        profiling::scope!("render_surface", &format!("{crtc:?}"));

        // Tick gamma transitions before rendering
        self.tick_gamma_transitions();

        // ── Frame-pipeline: consume pre-computed scene update ─────────────────
        //
        // When the VBlank callback (`frame_finish`) ran at the start of this
        // frame period it already called `scene_element.update()` and stored
        // the damage flag in `surface.prefetched_scene_damage`.  We take that
        // value here so the CPU work (scene-graph evaluation, layout, animation
        // ticking) stays on the VBlank side of the pipeline and only the GPU
        // work (render-element building, command recording, page-flip) runs on
        // the deadline side.
        //
        // We extract the cached value *before* the long-lived `surface` borrow
        // below so that Rust's borrow checker does not see two simultaneous
        // `&mut` borrows of `backend_data.backends`.
        //
        // If no pre-computed value is available — e.g. on the very first frame,
        // after an idle wakeup, or when the render was triggered by a path that
        // bypasses `frame_finish` — we fall back to calling `update()` inline
        // (after the `surface` borrow is established, using field-split rules).
        let prefetched_scene_damage = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|d| d.surfaces.get_mut(&crtc))
            .and_then(|s| s.prefetched_scene_damage.take());

        // Get screenshare sessions before borrowing backend_data
        // let _has_screenshare = !self.screenshare_sessions.is_empty();

        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        // Clone the GbmDevice handle (cheap — Arc-internal) before borrowing
        // `device.surfaces` mutably. Used by the dmabuf-backed scene element
        // setup below; needs to escape `device`'s mutable borrow.
        let device_gbm = device.gbm.clone();

        let surface = if let Some(surface) = device.surfaces.get_mut(&crtc) {
            surface
        } else {
            return;
        };

        // ── Deferred GPU fence: wait for the *previous* frame's GPU work ─────
        //
        // We stored the EGL fence from the last render_frame() call instead of
        // blocking on it immediately.  By the time we arrive here the GPU has
        // had the entire inter-frame period (scene update, input dispatch, timer
        // scheduling) to finish, so in the common case this wait is a no-op.
        // If the GPU is still busy we block here — same guarantee as before, but
        // with much better pipelining.
        #[cfg(feature = "renderer_sync")]
        {
            let fence = std::mem::take(&mut surface.pending_gpu_fence);
            if let Err(err) = fence.wait() {
                tracing::warn!(?err, "Deferred GPU fence wait failed");
            }
        }

        let start = Instant::now();

        let render_node = surface.render_node;
        let primary_gpu = self.backend_data.primary_gpu;
        let mut renderer = if primary_gpu == render_node {
            self.backend_data.gpus.single_renderer(&render_node)
        } else {
            let format = surface.compositor.format();

            self.backend_data
                .gpus
                .renderer(&primary_gpu, &render_node, format)
        }
        .unwrap();

        let output = if let Some(output) = self.workspaces.outputs().find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device_id == surface.device_id && id.crtc == crtc)
                .unwrap_or(false)
        }) {
            output.clone()
        } else {
            // somehow we got called with an invalid output
            return;
        };

        // let output_scale = output.current_scale().fractional_scale();
        // let integer_scale = output_scale.round() as u32;
        let _config_scale = Config::with(|c| c.screen_scale);

        // Use the pre-computed damage flag, or tick the scene inline if the
        // pre-fetch was not available.  `scene_element` and `backend_data` are
        // distinct fields, so field-projection rules allow the mutable borrow
        // of `self.scene_element` here even though `surface` (derived from
        // `self.backend_data`) is also live.
        let scene_has_damage =
            prefetched_scene_damage.unwrap_or_else(|| self.scene_element.update());
        let all_window_elements: Vec<&WindowElement> = self.workspaces.spaces_elements().collect();

        // Lazily set up the dmabuf-backed scene element for this surface when
        // scanout is allowed. Uses `device_gbm` (cloned out of `device`
        // before the surface mut-borrow) so we don't conflict with the
        // existing mutable borrows.
        // Allocate plane elements unconditionally — planes are always needed,
        // not just when a fullscreen window is present.
        if let Some(mode) = output.current_mode() {
            use crate::render_elements::scene_dmabuf_element::SceneDmabufElement;
            use smithay::backend::allocator::Fourcc;

            macro_rules! alloc_plane {
                ($field:expr, $format:expr, $opaque:expr, $label:literal) => {
                    if $field.is_none() {
                        let mut el = SceneDmabufElement::new(
                            self.layers_engine.clone(),
                            (mode.size.w, mode.size.h),
                            $label,
                        );
                        el.opaque = $opaque;
                        match el.ensure_swapchain(device_gbm.clone(), $format, surface.render_node) {
                            Ok(()) => $field = Some(el),
                            Err(e) => tracing::warn!("plane alloc failed for {crtc:?}: {e}"),
                        }
                    }
                };
            }

            alloc_plane!(surface.scene_dmabuf_element,      Fourcc::Xrgb8888, true,  "bg");
            // Solid-black test element (visual debug plane).
            alloc_plane!(surface.test_dmabuf_element,       Fourcc::Argb8888, true,  "test");

            // Probe buffers for KMS tier grading.  Each gets its own GBM swapchain so
            // all four carry distinct gem handles (kernel rejects duplicates on separate
            // planes).  [0]=primary  [1]=overlay0  [2]=overlay1  [3]=overlay2  [4]=overlay3.
            //
            // TEST: probes [1..4] use half-screen size to simulate realistic overlay
            // buffer sizes and check whether the KMS watermark budget allows Tier4.
            while surface.probe_dmabufs.len() < 5 {
                let is_primary_probe = surface.probe_dmabufs.is_empty();
                let size = if is_primary_probe {
                    (mode.size.w, mode.size.h)
                } else {
                    (mode.size.w / 2, mode.size.h / 2)
                };
                let mut el = SceneDmabufElement::new(
                    self.layers_engine.clone(),
                    size,
                    "probe",
                );
                el.opaque = true;
                match el.ensure_swapchain(device_gbm.clone(), Fourcc::Argb8888, surface.render_node) {
                    Ok(()) => surface.probe_dmabufs.push(el),
                    Err(e) => {
                        tracing::warn!("probe buf alloc failed for {crtc:?}: {e}");
                        break;
                    }
                }
            }
            alloc_plane!(surface.windows_dmabuf_element,    Fourcc::Argb8888, false, "windows");
            alloc_plane!(surface.expose_dmabuf_element,     Fourcc::Argb8888, false, "expose");
            alloc_plane!(surface.overlay_dmabuf_element,    Fourcc::Argb8888, false, "overlay");
        }

        // Every frame: point each plane element at its output's node.
        if let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) {
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
        }

        // Classify every window into its visibility state so post_repaint can
        // pick a per-window frame-callback throttle. `occluded_ids` is empty
        // for v1 — we rely on the fullscreen detection inside the classifier
        // for the main "background app behind a maximized window" case.
        let expose_active =
            self.workspaces.is_expose_transitioning() || self.workspaces.get_show_all();

        if let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) {
            ows.debug_plane_indicator.set_hidden(!expose_active);
        }

        let window_throttle_states = crate::state::window_throttle::classify_windows(
            &self.workspaces,
            &all_window_elements,
            &std::collections::HashSet::new(),
            expose_active,
        );

        // ── Shadow-only / direct scanout window selection ─────────────────────
        //
        // When tier ≥ Tier3 and no overlay UI or expose is active, the topmost
        // non-animating window(s) are put in "shadow-only" mode: their
        // `content_layer` is hidden in lay-rs so the shadow still renders in
        // `windows_dmabuf_element`, while the client buffer is pushed directly as
        // a `ScanoutCandidate` element (zero GPU copy). Ordering by the workspace
        // windows_list (bottom→top) picks the topmost window at index rev().next().
        let screencopy_pending = self
            .pending_screencopy_frames
            .iter()
            .any(|p| p.output == output);

        {
            use crate::udev::types::RenderTier;
            let use_topwin_plane = surface.current_tier
                .map(|t| matches!(t, RenderTier::Tier3 | RenderTier::Tier4))
                .unwrap_or(false);

            let overlay_ui_active = {
                use smithay::desktop::layer_map_for_output;
                use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
                let app_switcher = self.workspaces.app_switcher.alive();
                let osd = self.workspaces.osd.is_visible();
                let has_layer_shell = layer_map_for_output(&output)
                    .layers()
                    .any(|l| matches!(l.layer(), WlrLayer::Overlay | WlrLayer::Top));
                app_switcher || osd || has_layer_shell
            };

            let desired: Vec<smithay::reexports::wayland_server::backend::ObjectId> =
                if use_topwin_plane && !expose_active && !overlay_ui_active && !screencopy_pending {
                    self.workspaces
                        .output_workspaces
                        .get(&output.name())
                        .map(|ows| {
                            let ws = &ows.workspace_views[ows.current_workspace];
                            let list = ws.windows_list.read().unwrap();
                            // Pick topmost non-animating window(s). Start with 1;
                            // extend to non-overlapping later.
                            list.iter()
                                .rev()
                                .filter(|id| {
                                    self.workspaces
                                        .get_window_view(id)
                                        .map(|wv| !wv.is_minimizing() && !wv.is_unmapped())
                                        .unwrap_or(false)
                                })
                                .take(1)
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

            // Apply set_content_hidden transitions only on changes.
            let current = surface.shadow_only_windows.clone();
            for id in current.iter().filter(|id| !desired.contains(id)) {
                if let Some(wv) = self.workspaces.get_window_view(id) {
                    wv.set_content_hidden(false);
                }
            }
            for id in desired.iter().filter(|id| !current.contains(*id)) {
                if let Some(wv) = self.workspaces.get_window_view(id) {
                    wv.set_content_hidden(true);
                }
            }
            surface.shadow_only_windows = desired;
        }

        let output_scene_element = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .map(|ows| self.scene_element.for_output_layer(&ows.output_layer))
            .unwrap_or_else(|| self.scene_element.clone());

        let result = render_surface(
            surface,
            &mut renderer,
            &all_window_elements,
            &output,
            self.pointer.current_location(),
            &self.cursor_manager,
            &self.cursor_texture_cache,
            self.dnd_icon.as_ref(),
            &self.clock,
            scene_has_damage,
            &window_throttle_states,
            &mut self.pending_screencopy_frames,
            expose_active,
            self.workspaces.output_workspaces.get(&output.name()),
            screencopy_pending,
            output_scene_element,
        );

        let reschedule = match &result {
            Ok(outcome) => {
                if outcome.rendered {
                    // Frame was submitted — VBlank callback will drive the next render.
                    false
                } else {
                    // No damage — defer idle decision to after borrow is released.
                    true
                }
            }
            Err(err) => {
                // Log as debug for DeviceInactive (expected during suspend), warn for others
                let is_device_inactive = matches!(
                    err,
                    SwapBuffersError::TemporaryFailure(e)
                        if matches!(e.downcast_ref::<DrmError>(), Some(&DrmError::DeviceInactive))
                );

                if is_device_inactive {
                    debug!(
                        "Device inactive during rendering (expected during suspend): {:?}",
                        err
                    );
                } else {
                    warn!("Error during rendering: {:?}", err);
                }

                match err {
                    SwapBuffersError::AlreadySwapped => false,
                    SwapBuffersError::TemporaryFailure(err) => match err.downcast_ref::<DrmError>()
                    {
                        Some(DrmError::DeviceInactive) => true,
                        Some(DrmError::Access(DrmAccessError { source, .. })) => {
                            source.kind() == io::ErrorKind::PermissionDenied
                        }
                        _ => false,
                    },
                    SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
                }
            }
        };

        // Render to screenshare buffers if rendering succeeded
        if let Ok(outcome) = &result {
            if outcome.rendered && !self.screenshare_sessions.is_empty() {
                let scale = Scale::from(output.current_scale().fractional_scale());

                // Get the source framebuffer that was just rendered to
                // Blit to PipeWire buffers on main thread
                for session in self.screenshare_sessions.values() {
                    // Check if we should render cursor for this session
                    // CURSOR_MODE_HIDDEN (1) = don't render cursor
                    // CURSOR_MODE_EMBEDDED (2) = render cursor into video
                    // CURSOR_MODE_METADATA (4) = send cursor as metadata (not in video) - NOT IMPLEMENTED, treat as hidden
                    const CURSOR_MODE_EMBEDDED: u32 = 2;
                    let should_render_cursor = session.cursor_mode == CURSOR_MODE_EMBEDDED;

                    tracing::trace!(
                        "Screenshare session {}: cursor_mode={}, should_render={}",
                        session.session_id,
                        session.cursor_mode,
                        should_render_cursor
                    );

                    // Build cursor elements for screenshare if needed
                    let cursor_elements: Vec<WorkspaceRenderElements<_>> = if should_render_cursor {
                        let output_geometry =
                            Rectangle::new((0, 0).into(), output.current_mode().unwrap().size);
                        let output_scale = output.current_scale().fractional_scale();
                        let pointer_location = self.pointer.current_location();

                        let pointer_in_output = output_geometry
                            .to_f64()
                            .contains(pointer_location.to_physical(scale));

                        if pointer_in_output {
                            use crate::cursor::RenderCursor;
                            use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
                            use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;

                            let mut elements = Vec::new();

                            match self
                                .cursor_manager
                                .get_render_cursor(output_scale.round() as i32)
                            {
                                RenderCursor::Hidden => {}
                                RenderCursor::Surface { hotspot, surface } => {
                                    let cursor_pos_scaled = (pointer_location.to_physical(scale)
                                        - hotspot.to_f64().to_physical(scale))
                                    .to_i32_round();
                                    let cursor_elems: Vec<WorkspaceRenderElements<_>> =
                                        render_elements_from_surface_tree(
                                            &mut renderer,
                                            &surface,
                                            cursor_pos_scaled,
                                            scale,
                                            1.0,
                                            Kind::Cursor,
                                        );
                                    elements.extend(cursor_elems);
                                }
                                RenderCursor::Named {
                                    icon,
                                    scale: _,
                                    cursor,
                                } => {
                                    let elapsed_millis = self.clock.now().as_millis();
                                    let (idx, image) = cursor.frame(elapsed_millis);
                                    let texture = self.cursor_texture_cache.get(
                                        icon,
                                        output_scale.round() as i32,
                                        &cursor,
                                        idx,
                                    );
                                    let hotspot_physical =
                                        Point::from((image.xhot as f64, image.yhot as f64));
                                    let cursor_pos_scaled: Point<i32, Physical> =
                                        (pointer_location.to_physical(scale) - hotspot_physical)
                                            .to_i32_round();
                                    let elem = MemoryRenderBufferRenderElement::from_buffer(
                                        &mut renderer,
                                        cursor_pos_scaled.to_f64(),
                                        &texture,
                                        None,
                                        None,
                                        None,
                                        Kind::Cursor,
                                    )
                                    .expect("Failed to create cursor render element");
                                    elements.push(WorkspaceRenderElements::from(elem));
                                }
                            }

                            elements
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    for (connector, stream) in &session.streams {
                        if connector == &output.name() {
                            let buffer_pool = stream.pipewire_stream.buffer_pool();
                            let mut pool = buffer_pool.lock().unwrap();

                            if let Some(available) = pool.available.pop_front() {
                                let size = output
                                    .current_mode()
                                    .map(|m| m.size)
                                    .unwrap_or_else(|| (1920, 1080).into());

                                // Force full frame for first render (when last_rendered_fd is None)
                                let is_first_frame = pool.last_rendered_fd.is_none();
                                let buffer_changed = pool.last_rendered_fd != Some(available.fd);

                                pool.last_rendered_fd = Some(available.fd);

                                // Use damage only if not first frame and same buffer
                                let damage_to_use = if is_first_frame || buffer_changed {
                                    None // Full frame for first render or buffer change
                                } else {
                                    outcome.damage.as_deref()
                                };

                                if is_first_frame {
                                    tracing::debug!(
                                        "First frame for stream on {}, forcing full blit",
                                        connector
                                    );
                                }

                                // Blit from source framebuffer and render cursor on top
                                let blit_result = crate::screenshare::fullscreen_to_dmabuf(
                                    &mut renderer,
                                    &mut available.dmabuf.clone(),
                                    size,
                                    damage_to_use,
                                    &cursor_elements,
                                    scale,
                                );

                                if let Err(e) = blit_result {
                                    tracing::debug!("Screenshare blit failed: {}", e);
                                } else {
                                    // Only increment sequence on successful blit
                                    stream.pipewire_stream.increment_frame_sequence();
                                }

                                pool.to_queue.insert(available.fd, available.pw_buffer);
                                drop(pool);
                                // Trigger to queue the buffer we just rendered
                                stream.pipewire_stream.trigger_frame();
                            } else {
                                // No buffer available - trigger to dequeue any released buffers
                                drop(pool);
                                stream.pipewire_stream.trigger_frame();
                                tracing::trace!("No available buffers for screenshare on {}, triggering dequeue", connector);
                            }
                        }
                    }
                } // Close for session loop
            }
        }

        {
            self.workspaces.refresh_space();
            self.popups.cleanup();
            self.update_dnd();
        }

        // Update the running average of render time and idle countdown (EMA with α=0.1)
        let render_time_us = start.elapsed().as_micros() as f32;
        let has_animations = self.scene_element.has_pending_animations();
        let was_rendered = result.as_ref().map(|o| o.rendered).unwrap_or(false);
        if let Some(device) = self.backend_data.backends.get_mut(&node) {
            if let Some(surface) = device.surfaces.get_mut(&crtc) {
                surface.avg_render_time_us =
                    surface.avg_render_time_us * 0.9 + render_time_us * 0.1;
                // Reset countdown on any activity: animations, actual frame
                // submitted, or a render triggered by input/client commit.
                // Short tail — see commentary in init.rs dispatch loop.
                if has_animations || was_rendered {
                    surface.idle_countdown = 3;
                }
            }
        }

        // Apply idle countdown: if reschedule was requested (no-damage path)
        // but no animations, count down before going idle.
        let reschedule = if reschedule && !has_animations {
            let remaining = self
                .backend_data
                .backends
                .get_mut(&node)
                .and_then(|d| d.surfaces.get_mut(&crtc))
                .map(|s| {
                    s.idle_countdown = s.idle_countdown.saturating_sub(1);
                    s.idle_countdown
                })
                .unwrap_or(0);
            remaining > 0
        } else {
            reschedule
        };

        if reschedule {
            let output_refresh = match output.current_mode() {
                Some(mode) => mode.refresh,
                None => return,
            };
            // Schedule the next render early enough to guarantee we finish before
            // the next VBlank.  We subtract 2× the average render time as a
            // safety margin (accounts for variance, scheduling jitter, etc.).
            // Clamped to at least 1 ms to avoid busy-spinning.
            //
            // `output_refresh` is in millihertz (mHz), so the frame period in
            // microseconds is 1_000_000_000 / refresh_mHz (not 1_000_000).
            let frame_period_us = 1_000_000_000f32 / output_refresh as f32;
            let avg_us = self
                .backend_data
                .backends
                .get(&node)
                .and_then(|d| d.surfaces.get(&crtc))
                .map(|s| s.avg_render_time_us)
                .unwrap_or(2000.0);
            let delay_us = (frame_period_us - avg_us * 2.0).max(1000.0);
            let timer = Timer::from_duration(Duration::from_micros(delay_us as u64));
            self.handle
                .insert_source(timer, move |_, _, data| {
                    data.render(node, Some(crtc));
                    TimeoutAction::Drop
                })
                .expect("failed to schedule frame timer");
        } else {
            let _elapsed = start.elapsed();
            //tracing::trace!(?elapsed, "rendered surface");
        }

        profiling::finish_frame!();
    }

    pub(super) fn schedule_initial_render(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        evt_handle: LoopHandle<'static, Otto<UdevData>>,
    ) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let surface = if let Some(surface) = device.surfaces.get_mut(&crtc) {
            surface
        } else {
            return;
        };

        let node = surface.render_node;
        let result = {
            let mut renderer = self.backend_data.gpus.single_renderer(&node).unwrap();
            initial_render(surface, &mut renderer)
        };

        if let Err(err) = result {
            match err {
                SwapBuffersError::AlreadySwapped => {}
                SwapBuffersError::TemporaryFailure(err) => {
                    // TODO dont reschedule after 3(?) retries
                    warn!("Failed to submit page_flip: {}", err);
                    let handle = evt_handle.clone();
                    evt_handle
                        .insert_idle(move |data| data.schedule_initial_render(node, crtc, handle));
                }
                SwapBuffersError::ContextLost(err) => panic!("Rendering loop lost: {}", err),
            }
        }
    }

    /// Render all virtual outputs into their PipeWire buffers.
    ///
    /// Called once per primary GPU render cycle. For each virtual output we:
    /// 1. Pop an available DMA-BUF buffer from the PipeWire pool.
    /// 2. Bind it as the render target.
    /// 3. Call `render_output()` directly into the PipeWire buffer.
    /// 4. Queue the buffer back and trigger PipeWire.
    pub(super) fn render_virtual_outputs(&mut self) {
        if self.virtual_outputs.is_empty() {
            return;
        }

        let primary_gpu = self.backend_data.primary_gpu;
        let all_window_elements: Vec<&WindowElement> = self.workspaces.spaces_elements().collect();
        let scene_element = self.scene_element.clone();

        for i in 0..self.virtual_outputs.len() {
            let mut renderer = match self.backend_data.gpus.single_renderer(&primary_gpu) {
                Ok(r) => r,
                Err(e) => {
                    warn!("render_virtual_outputs: failed to get renderer: {e}");
                    continue;
                }
            };

            // Clone output (cheap Arc clone) so we can hold &output alongside &mut damage_tracker
            let output_clone = self.virtual_outputs[i].output.clone();
            let output_name = output_clone.name();

            // Per-output scene element — renders only this output's sub-tree
            let output_scene_element = self
                .workspaces
                .output_workspaces
                .get(&output_name)
                .map(|ows| scene_element.for_output_layer(&ows.output_layer))
                .unwrap_or_else(|| scene_element.clone());

            // Build cursor elements if pointer is over this output
            let scale = Scale::from(output_clone.current_scale().fractional_scale());
            let output_mode_size = output_clone
                .current_mode()
                .map(|m| m.size)
                .unwrap_or_default();
            let output_geometry = Rectangle::new((0, 0).into(), output_mode_size);
            let pointer_location = self.pointer.current_location();
            // Virtual output's logical position in the scene
            let vout_geo = self.workspaces.output_geometry(&output_clone);
            let local_pointer: Point<f64, Logical> = vout_geo
                .map(|geo| {
                    (
                        pointer_location.x - geo.loc.x as f64,
                        pointer_location.y - geo.loc.y as f64,
                    )
                        .into()
                })
                .unwrap_or(pointer_location);
            let pointer_in_output = output_geometry
                .to_f64()
                .contains(local_pointer.to_physical(scale));

            // Helper closure — builds fresh cursor elements (can't clone render elements)
            let build_cursor_elements = |renderer: &mut _| -> Vec<WorkspaceRenderElements<_>> {
                if !pointer_in_output {
                    return Vec::new();
                }
                use crate::cursor::RenderCursor;
                use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
                use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
                let output_scale = output_clone.current_scale().fractional_scale();
                let mut elems = Vec::new();
                match self
                    .cursor_manager
                    .get_render_cursor(output_scale.round() as i32)
                {
                    RenderCursor::Hidden => {}
                    RenderCursor::Surface { hotspot, surface } => {
                        let cursor_pos_scaled = (local_pointer.to_physical(scale)
                            - hotspot.to_f64().to_physical(scale))
                        .to_i32_round();
                        let cursor_elems: Vec<WorkspaceRenderElements<_>> =
                            render_elements_from_surface_tree(
                                renderer,
                                &surface,
                                cursor_pos_scaled,
                                scale,
                                1.0,
                                Kind::Cursor,
                            );
                        elems.extend(cursor_elems);
                    }
                    RenderCursor::Named {
                        icon,
                        scale: _,
                        cursor,
                    } => {
                        let elapsed_millis = self.clock.now().as_millis();
                        let (idx, image) = cursor.frame(elapsed_millis);
                        let texture = self.cursor_texture_cache.get(
                            icon,
                            output_scale.round() as i32,
                            &cursor,
                            idx,
                        );
                        let hotspot_physical = Point::from((image.xhot as f64, image.yhot as f64));
                        let cursor_pos_scaled: Point<i32, Physical> =
                            (local_pointer.to_physical(scale) - hotspot_physical).to_i32_round();
                        if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                            renderer,
                            cursor_pos_scaled.to_f64(),
                            &texture,
                            None,
                            None,
                            None,
                            Kind::Cursor,
                        ) {
                            elems.push(WorkspaceRenderElements::from(elem));
                        }
                    }
                }
                elems
            };

            // --- Render into this virtual output's own PipeWire stream ---
            let pool_arc = self.virtual_outputs[i].pipewire_stream.buffer_pool();
            let maybe_buf = {
                let mut pool = pool_arc.lock().unwrap();
                pool.available.pop_front().inspect(|buf| {
                    pool.to_queue.insert(buf.fd, buf.pw_buffer);
                })
            };
            if let Some(available) = maybe_buf {
                let mut dmabuf = available.dmabuf.clone();
                {
                    // Scope the damage_tracker borrow so it ends before pipewire_stream access
                    let damage_tracker = &mut self.virtual_outputs[i].damage_tracker;
                    match renderer.bind(&mut dmabuf) {
                        Ok(mut framebuffer) => {
                            let mut elements = build_cursor_elements(&mut renderer);
                            elements
                                .push(WorkspaceRenderElements::Scene(output_scene_element.clone()));
                            let _ = crate::render::render_output(
                                &output_clone,
                                &all_window_elements,
                                elements,
                                None,
                                &mut renderer,
                                &mut framebuffer,
                                damage_tracker,
                                0,
                            );
                        }
                        Err(e) => {
                            warn!("render_virtual_outputs: bind failed for '{output_name}': {e}");
                        }
                    }
                }
                self.virtual_outputs[i]
                    .pipewire_stream
                    .increment_frame_sequence();
            }
            self.virtual_outputs[i].pipewire_stream.trigger_frame();

            // --- Tap screenshare sessions targeting this virtual output ---
            for session in self.screenshare_sessions.values() {
                for (connector, stream) in &session.streams {
                    if *connector == output_name {
                        let ss_pool = stream.pipewire_stream.buffer_pool();
                        let maybe_ss_buf = {
                            let mut pool = ss_pool.lock().unwrap();
                            pool.available.pop_front().inspect(|buf| {
                                pool.to_queue.insert(buf.fd, buf.pw_buffer);
                            })
                        };
                        if let Some(ss_buf) = maybe_ss_buf {
                            let mut ss_dmabuf = ss_buf.dmabuf.clone();
                            let mut temp_tracker = OutputDamageTracker::from_output(&output_clone);
                            match renderer.bind(&mut ss_dmabuf) {
                                Ok(mut fb) => {
                                    let mut ss_elements = build_cursor_elements(&mut renderer);
                                    ss_elements.push(WorkspaceRenderElements::Scene(
                                        output_scene_element.clone(),
                                    ));
                                    let _ = crate::render::render_output(
                                        &output_clone,
                                        &all_window_elements,
                                        ss_elements,
                                        None,
                                        &mut renderer,
                                        &mut fb,
                                        &mut temp_tracker,
                                        0,
                                    );
                                    stream.pipewire_stream.increment_frame_sequence();
                                }
                                Err(e) => {
                                    warn!("render_virtual_outputs: screenshare bind failed for '{output_name}': {e}");
                                }
                            }
                            stream.pipewire_stream.trigger_frame();
                        } else {
                            stream.pipewire_stream.trigger_frame();
                            trace!(
                                "render_virtual_outputs: no screenshare buffer for '{output_name}'"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::mutable_key_type)] // ObjectId as HashMap key — see window_throttle.rs
pub(super) fn render_surface<'a>(
    surface: &'a mut SurfaceData,
    renderer: &mut UdevRenderer<'a>,
    window_elements: &[&WindowElement],
    output: &Output,
    pointer_location: Point<f64, Logical>,
    cursor_manager: &CursorManager,
    cursor_texture_cache: &CursorTextureCache,
    dnd_icon: Option<&wl_surface::WlSurface>,
    clock: &Clock<Monotonic>,
    scene_has_damage: bool,
    window_throttle_states: &std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        crate::state::window_throttle::WindowThrottleState,
    >,
    pending_screencopy: &mut Vec<crate::state::screencopy::PendingScreencopy>,
    expose_active: bool,
    output_workspaces: Option<&crate::workspaces::OutputWorkspaces>,
    screencopy_pending: bool,
    output_scene_element: crate::render_elements::scene_element::SceneElement,
) -> Result<RenderOutcome, SwapBuffersError> {
    // Start frame timing
    #[cfg(feature = "metrics")]
    let _frame_timer = surface
        .render_metrics
        .as_ref()
        .map(|m: &Arc<_>| m.start_frame());

    let output_geometry = Rectangle::new((0, 0).into(), output.current_mode().unwrap().size);
    let scale = Scale::from(output.current_scale().fractional_scale());

    let mut workspace_render_elements: Vec<WorkspaceRenderElements<_>> = Vec::new();

    let output_scale = output.current_scale().fractional_scale();
    let dnd_needs_draw = dnd_icon.map(|surface| surface.alive()).unwrap_or(false);

    let pointer_in_output = output_geometry
        .to_f64()
        .contains(pointer_location.to_physical(scale));

    if pointer_in_output {
        use crate::cursor::RenderCursor;
        use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;

        match cursor_manager.get_render_cursor(output_scale.round() as i32) {
            RenderCursor::Hidden => {}
            RenderCursor::Surface { hotspot, surface } => {
                let cursor_pos_scaled = (pointer_location.to_physical(scale)
                    - hotspot.to_f64().to_physical(scale))
                .to_i32_round();
                let elements: Vec<WorkspaceRenderElements<_>> = render_elements_from_surface_tree(
                    renderer,
                    &surface,
                    cursor_pos_scaled,
                    scale,
                    1.0,
                    Kind::Cursor,
                );
                workspace_render_elements.extend(elements);
            }
            RenderCursor::Named {
                icon,
                scale: _,
                cursor,
            } => {
                let elapsed_millis = clock.now().as_millis();
                let (idx, image) = cursor.frame(elapsed_millis);
                let texture =
                    cursor_texture_cache.get(icon, output_scale.round() as i32, &cursor, idx);
                use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
                let hotspot_physical = Point::from((image.xhot as f64, image.yhot as f64));
                let cursor_pos_scaled: Point<i32, Physical> =
                    (pointer_location.to_physical(scale) - hotspot_physical).to_i32_round();
                let elem = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    cursor_pos_scaled.to_f64(),
                    &texture,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                )
                .expect("Failed to create cursor render element");
                workspace_render_elements.push(WorkspaceRenderElements::from(elem));
            }
        }
    }

    #[cfg(feature = "fps_ticker")]
    if let Some(element) = surface.fps_element.as_mut() {
        element.update_fps(surface.fps.avg().round() as u32);
        surface.fps.tick();
        workspace_render_elements.push(WorkspaceRenderElements::Fps(element.clone()));
    }

    let (output_elements, clear_color, should_draw) = {
        let cursor_needs_draw = pointer_in_output;
            let should_draw = scene_has_damage || dnd_needs_draw || cursor_needs_draw || screencopy_pending;
            if !should_draw {
                return Ok(RenderOutcome::skipped());
            }

            // When a screencopy client is waiting, render the full scene as one
            // GPU-composited pass so `blit_current_frame` captures everything.
            if screencopy_pending {
                workspace_render_elements
                    .push(WorkspaceRenderElements::Scene(output_scene_element));
            } else {
            // Push plane elements top→bottom so Smithay assigns overlay planes
            // from the front of the list. Scene element goes last as primary
            // plane / GPU-composite fallback for any plane that fails assignment.
            macro_rules! render_plane {
                ($el:expr) => {
                    if let Some(el) = &$el {
                        el.render(renderer.as_mut());
                    }
                };
            }
            macro_rules! push_plane {
                ($el:expr) => {
                    if let Some(el) = $el {
                        el.render(renderer.as_mut());
                        // Push even when render() returned false (no new damage):
                        // the existing dmabuf stays on the plane and Smithay sees
                        // an unchanged commit_counter → empty damage → no page-flip.
                        if el.current_dmabuf().is_some() {
                            workspace_render_elements
                                .push(WorkspaceRenderElements::SceneDmabuf(el));
                        }
                    }
                };
            }
            // Warm the solid-black test element (visual debug plane).
            render_plane!(surface.test_dmabuf_element.clone());
            // Warm all probe buffers so they have a dmabuf ready for the tier probe.
            for probe in &surface.probe_dmabufs {
                probe.render(renderer.as_mut());
            }

            let tier = surface.current_tier.unwrap_or(RenderTier::Fallback);
            let use_bg_plane      = matches!(tier, RenderTier::Tier2 | RenderTier::Tier3 | RenderTier::Tier4);
            let use_windows_plane = matches!(tier, RenderTier::Tier2 | RenderTier::Tier3 | RenderTier::Tier4);
            let use_topwin_plane  = matches!(tier, RenderTier::Tier3 | RenderTier::Tier4);
            let use_ui_dock_plane = matches!(tier, RenderTier::Tier1 | RenderTier::Tier2 | RenderTier::Tier3 | RenderTier::Tier4);

            macro_rules! dbg_plane_load {
                ($flag:ident) => {{
                    #[cfg(feature = "debug-kms")]
                    { crate::input::keyboard::$flag.load(std::sync::atomic::Ordering::Relaxed) }
                    #[cfg(not(feature = "debug-kms"))]
                    { false }
                }};
            }
            if use_ui_dock_plane || dbg_plane_load!(DBG_PLANE_OVERLAY) {
                push_plane!(surface.overlay_dmabuf_element.clone());
            }

            // expose and windows are mutually exclusive planes.
            if expose_active {
                // Keep the windows plane warm while expose is active so it has a
                // fresh dmabuf ready the moment expose closes (no cold-start re-render).
                render_plane!(surface.windows_dmabuf_element.clone());
                if use_windows_plane || dbg_plane_load!(DBG_PLANE_EXPOSE) {
                    push_plane!(surface.expose_dmabuf_element.clone());
                }
            } else {
                // Direct surface scanout for shadow-only windows (set by Otto::render_surface).
                // Each window's content_layer is hidden in lay-rs so the shadow still renders
                // into windows_dmabuf below; the client buffer scans out directly on top.
                if use_topwin_plane || dbg_plane_load!(DBG_PLANE_TOP_WIN) {
                    use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
                    for win_id in &surface.shadow_only_windows {
                        if let Some(win) = window_elements.iter().find(|w| w.id() == *win_id) {
                            if let Some(wl_surface) = win.wl_surface() {
                                // render_position() is the visible-content origin (physical px).
                                // render_elements_from_surface_tree expects the wl_surface buffer
                                // origin, which for CSD windows is shifted back by geometry.loc.
                                let pos = win.base_layer().render_position();
                                let geo_loc = win.geometry().loc.to_f64().to_physical(scale);
                                let buf_x = pos.x as i32 - geo_loc.x.round() as i32;
                                let buf_y = pos.y as i32 - geo_loc.y.round() as i32;
                                let elems: Vec<WorkspaceRenderElements<_>> =
                                    render_elements_from_surface_tree(
                                        renderer,
                                        &wl_surface,
                                        Point::<i32, Physical>::from((buf_x, buf_y)),
                                        scale,
                                        1.0,
                                        Kind::ScanoutCandidate,
                                    );
                                workspace_render_elements.extend(elems);
                            }
                        }
                    }
                }
                // Always push windows plane when there are windows — shadow-only windows
                // still need their shadows rendered here.
                let has_windows = output_workspaces.map_or(false, |ows| {
                    let ws = &ows.workspace_views[ows.current_workspace];
                    !ws.windows_list.read().unwrap().is_empty()
                });
                if has_windows && (use_windows_plane || dbg_plane_load!(DBG_PLANE_WIN)) {
                    push_plane!(surface.windows_dmabuf_element.clone());
                }
            }

            if use_bg_plane || dbg_plane_load!(DBG_PLANE_BG) {
                push_plane!(surface.scene_dmabuf_element.clone());
            }

            // Debug PNG saves — triggered by keys 6-0 (debug-kms feature only).
            #[cfg(feature = "debug-kms")]
            {
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
                let ss_dir = std::path::Path::new("/home/riccardo/Pictures/Screenshots");
                macro_rules! ss_path {
                    ($name:literal) => { ss_dir.join(format!("otto_plane_{}.png", $name)).to_string_lossy().into_owned() };
                }
                dbg_save!(DBG_SAVE_BG,      surface.scene_dmabuf_element,     ss_path!("bg"));
                dbg_save!(DBG_SAVE_WIN,     surface.windows_dmabuf_element,    ss_path!("win"));
                dbg_save!(DBG_SAVE_EXPOSE,  surface.expose_dmabuf_element,     ss_path!("expose"));
                dbg_save!(DBG_SAVE_OVERLAY, surface.overlay_dmabuf_element,    ss_path!("overlay"));
            }

            } // end !screencopy_pending

            // Clear engine damage after all plane renders so `subtree_damage()`
            // returns `None` next frame when nothing has changed. Without this
            // call `per_node_damage` is never cleared and every plane redraws
            // every frame even on an otherwise idle desktop.
            if let Some(el) = &surface.scene_dmabuf_element {
                el.clear_engine_damage();
            }

            let output_render_elements: Vec<OutputRenderElements<'a, _, WindowRenderElement<_>>> =
                workspace_render_elements
                    .into_iter()
                    .map(OutputRenderElements::from)
                    .collect::<Vec<_>>();
            let (output_elements, clear_color) = output_elements(
                output,
                window_elements.iter().copied(),
                output_render_elements,
                dnd_icon,
                renderer,
            );
            (output_elements, clear_color, true)
    };

    if !should_draw {
        return Ok(RenderOutcome::skipped());
    }

    // KMS tier probe — uses TEST_ONLY so no pixels are written and no flip happens.
    // Runs once (when current_tier == None). By this point render_plane! has already
    // rendered test_dmabuf_element (solid black, Argb8888, never committed to KMS),
    // giving us a distinct gem handle for the Tier2 two-plane test.
    // After the first probe the result is final; no re-probing is needed or safe
    // (overlay_ui_buf may enter active KMS scanout on subsequent frames).
    if surface.current_tier.is_none() {
        if let Some(t) = grade_render_tier(
            &surface.compositor,
            &surface.probe_dmabufs,
            output,
        ) {
            tracing::info!(
                output = %output.name(),
                tier = %t,
                overlays = surface.compositor.surface().planes().overlay.len(),
                "KMS render tier"
            );
            surface.current_tier = Some(t);
        }
    }

    let render_frame_result = surface
        .compositor
        .render_frame(
            renderer,
            &output_elements,
            clear_color,
            smithay::backend::drm::compositor::FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT
                | smithay::backend::drm::compositor::FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY
                | smithay::backend::drm::compositor::FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT,
        )
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => err.into(),
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                smithay::backend::renderer::damage::Error::Rendering(err),
            ) => err.into(),
            other => {
                tracing::error!("Unexpected render frame error: {:?}", other);
                SwapBuffersError::ContextLost(Box::new(std::io::Error::other(format!(
                    "Render frame error: {:?}",
                    other
                ))))
            }
        })?;

    #[cfg(feature = "renderer_sync")]
    {
        // Store this frame's GPU fence for deferred waiting.  The fence will be
        // consumed at the start of the *next* render_surface() call, giving the
        // GPU the entire inter-frame period to finish while the CPU handles
        // scene updates, input processing, etc.
        use smithay::backend::drm::compositor::PrimaryPlaneElement;
        if let PrimaryPlaneElement::Swapchain(element) = render_frame_result.primary_element {
            surface.pending_gpu_fence = element.sync.clone();
        }
    }

    let rendered = !render_frame_result.is_empty;
    let states = render_frame_result.states;
    let damage: Option<Vec<Rectangle<i32, Physical>>> = None; // DRM compositor doesn't provide damage info

    // Record damage metrics if available
    #[cfg(feature = "metrics")]
    if let Some(ref metrics) = surface.render_metrics {
        let mode = output.current_mode().unwrap();
        let output_size = (mode.size.w, mode.size.h);

        if let Some(ref damage_rects) = damage {
            // Have actual damage information
            metrics.as_ref().record_damage(output_size, damage_rects);
        } else if rendered {
            // No damage info available (DRM compositor mode), but frame was rendered
            // Record full frame as damage as approximation
            let full_screen = vec![Rectangle::new(
                (0, 0).into(),
                (mode.size.w, mode.size.h).into(),
            )];
            metrics.as_ref().record_damage(output_size, &full_screen);
        }
    }

    let damage_for_return = damage.clone();

    let post_repaint_elements: Vec<&WindowElement> = window_elements.to_vec();

    post_repaint(
        output,
        &states,
        &post_repaint_elements,
        surface
            .dmabuf_feedback
            .as_ref()
            .map(|feedback| SurfaceDmabufFeedback {
                render_feedback: &feedback.render_feedback,
                scanout_feedback: &feedback.scanout_feedback,
            }),
        clock.now(),
        window_throttle_states,
    );

    if rendered {
        // Gated: only enter the screencopy path when a client has actually
        // asked for a frame on this output. Internally branches between the
        // GPU dmabuf blit (reusing the screenshare path) and SHM read_pixels.
        if !pending_screencopy.is_empty() {
            crate::state::screencopy::complete_screencopy_for_output(
                pending_screencopy,
                output,
                renderer,
            );
        }

        let output_presentation_feedback =
            take_presentation_feedback(output, &post_repaint_elements, &states);
        surface
            .compositor
            .queue_frame(Some(output_presentation_feedback))?;
    }

    Ok(RenderOutcome {
        rendered,
        damage: damage_for_return,
    })
}

/// Reparent scanout windows when the desired candidate set changes.
///


pub(super) fn initial_render(
    surface: &mut SurfaceData,
    renderer: &mut UdevRenderer<'_>,
) -> Result<(), SwapBuffersError> {
    surface
        .compositor
        .render_frame::<_, WorkspaceRenderElements<_>>(
            renderer,
            &[],
            CLEAR_COLOR,
            smithay::backend::drm::compositor::FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT
                | smithay::backend::drm::compositor::FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY
                | smithay::backend::drm::compositor::FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT,
        )
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => err.into(),
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                smithay::backend::renderer::damage::Error::Rendering(err),
            ) => err.into(),
            other => SwapBuffersError::ContextLost(Box::new(std::io::Error::other(format!(
                "Render frame error: {:?}",
                other
            )))),
        })?;
    surface.compositor.queue_frame(None)?;
    surface.compositor.reset_buffers();

    Ok(())
}

/// Probe the hardware with `DRM_MODE_ATOMIC_TEST_ONLY` to determine the highest
/// render tier achievable for this output.  Returns `None` when a previous-frame
/// dmabuf is not yet available (typically the very first frame).
/// Probes KMS overlay support via TEST_ONLY atomic commits.
///
/// Each test mirrors the realistic plane arrangement Otto would commit for that tier:
///   Tier2: primary[0] + overlay0[1] + overlay1[2]
///   Tier1: primary[0] + overlay0[1]
///
/// The primary plane is always included — i915 requires the full pipe state for its
/// DBUF/watermark budget check (`skl_compute_wm`); omitting it causes EINVAL.
/// `probe_dmabufs` provides 5 distinct gem handles (separate GBM swapchains).
fn grade_render_tier(
    compositor: &GbmDrmCompositor,
    probe_dmabufs: &[crate::render_elements::scene_dmabuf_element::SceneDmabufElement],
    output: &Output,
) -> Option<RenderTier> {
    if probe_dmabufs.len() < 2 {
        return None; // not enough probe bufs yet
    }
    let mode = output.current_mode()?;
    let full = Rectangle::new((0, 0).into(), mode.size);
    // TEST: overlay probes are half-screen — use matching half-size dst so there
    // is no scaling (scaling needs more DBUF, not less). We want to know whether
    // the smaller footprint lets the watermark budget accept more planes.
    let half: Rectangle<i32, Physical> = Rectangle::new(
        (0, 0).into(),
        (mode.size.w / 2, mode.size.h / 2).into(),
    );
    let overlay_planes = compositor.surface().planes().overlay.clone();

    let buf0 = probe_dmabufs[0].current_dmabuf()?;
    let buf1 = probe_dmabufs[1].current_dmabuf()?;

    // Tier4: primary (full) + 4 overlays (half each).
    if overlay_planes.len() >= 4 && probe_dmabufs.len() >= 5 {
        if let (Some(buf2), Some(buf3), Some(buf4)) = (
            probe_dmabufs[2].current_dmabuf(),
            probe_dmabufs[3].current_dmabuf(),
            probe_dmabufs[4].current_dmabuf(),
        ) {
            let overlays = [
                (overlay_planes[0].handle, &buf1, half),
                (overlay_planes[1].handle, &buf2, half),
                (overlay_planes[2].handle, &buf3, half),
                (overlay_planes[3].handle, &buf4, half),
            ];
            if compositor.test_overlay_planes(Some((&buf0, full)), &overlays) {
                return Some(RenderTier::Tier4);
            }
            tracing::debug!("grade_render_tier: Tier4 rejected");
        }
    }

    // Tier3: primary (full) + 3 overlays (half each).
    if overlay_planes.len() >= 3 && probe_dmabufs.len() >= 4 {
        if let (Some(buf2), Some(buf3)) = (
            probe_dmabufs[2].current_dmabuf(),
            probe_dmabufs[3].current_dmabuf(),
        ) {
            let overlays = [
                (overlay_planes[0].handle, &buf1, half),
                (overlay_planes[1].handle, &buf2, half),
                (overlay_planes[2].handle, &buf3, half),
            ];
            if compositor.test_overlay_planes(Some((&buf0, full)), &overlays) {
                return Some(RenderTier::Tier3);
            }
            tracing::debug!("grade_render_tier: Tier3 rejected");
        }
    }

    // Tier2: primary (full) + 2 overlays (half each).
    if overlay_planes.len() >= 2 && probe_dmabufs.len() >= 3 {
        if let Some(buf2) = probe_dmabufs[2].current_dmabuf() {
            let overlays = [
                (overlay_planes[0].handle, &buf1, half),
                (overlay_planes[1].handle, &buf2, half),
            ];
            if compositor.test_overlay_planes(Some((&buf0, full)), &overlays) {
                return Some(RenderTier::Tier2);
            }
            tracing::debug!("grade_render_tier: Tier2 rejected");
        }
    }

    // Tier1: primary (full) + 1 overlay (half).
    if let Some(plane) = overlay_planes.first() {
        let overlays = [(plane.handle, &buf1, half)];
        if compositor.test_overlay_planes(Some((&buf0, full)), &overlays) {
            return Some(RenderTier::Tier1);
        }
        tracing::debug!("grade_render_tier: Tier1 rejected");
    }

    Some(RenderTier::Fallback)
}
