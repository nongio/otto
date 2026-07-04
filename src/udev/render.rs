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

use super::types::{FrameMode, RenderOutcome, SurfaceData, UdevData, UdevOutputId, UdevRenderer};
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

        // ---- Topmost-window scanout selection ----
        // Computed here, BEFORE the `device`/`surface` mutable borrow of
        // `self.backend_data`, because the demotion re-import below calls
        // `self.update_window_view` (full `&mut self`). Capture (screenshot
        // via screencopy / recording via screenshare) must see a fully
        // composited primary, so it disables scanout.
        let capture_active =
            !self.pending_screencopy_frames.is_empty() || !self.screenshare_sessions.is_empty();
        // Fullscreen direct scanout: a stable single fullscreen window is
        // scanned out on the primary plane on its own — all chrome planes
        // are dropped for those frames. Disabled during capture and the
        // 3-finger swipe (the finger-drag moves the workspace with no
        // animation flag, so a fixed plane would not follow it).
        let allow_fullscreen_scanout = self.workspaces.is_fullscreen_and_stable()
            && !self.swipe_gesture.is_active()
            && !capture_active;
        let fullscreen_window = if allow_fullscreen_scanout {
            self.workspaces.get_fullscreen_window()
        } else {
            None
        };
        let scanout_desired: Vec<smithay::reexports::wayland_server::backend::ObjectId> =
            if capture_active || self.swipe_gesture.is_active() || fullscreen_window.is_some() {
                Vec::new()
            } else {
                self.workspaces.get_scanout_candidates()
            };
        // Demotion: windows that LEAVE the scanout set had a stale (or no)
        // lay-rs content import while promoted; re-import them now (after the
        // set update unhides their content_layer) so the first composited
        // frame shows the current buffer, not a stale one.
        let new_scanout_ids: std::collections::HashSet<
            smithay::reexports::wayland_server::backend::ObjectId,
        > = scanout_desired.iter().cloned().collect();
        let prev_scanout_ids = self.workspaces.scanout_window_ids();
        let departed_windows: Vec<WindowElement> = self
            .workspaces
            .spaces_elements()
            .filter(|w| prev_scanout_ids.contains(&w.id()) && !new_scanout_ids.contains(&w.id()))
            .cloned()
            .collect();
        self.workspaces.set_scanout_windows(&scanout_desired);
        for w in &departed_windows {
            self.update_window_view(w);
        }

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
        let scene_has_damage = if !departed_windows.is_empty() {
            // A window was demoted from scanout this frame: its buffer was
            // re-imported (above) *after* the Phase-1 scene prefetch, so that
            // content transaction is still unflushed. Re-run the engine update
            // now to apply it before compositing, and force a draw — otherwise
            // the first composited frame after demotion shows the stale,
            // shadow-only scene (a one-frame flicker).
            self.scene_element.update();
            true
        } else {
            prefetched_scene_damage.unwrap_or_else(|| self.scene_element.update())
        };
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
            // The background may only direct-scan the PRIMARY plane: as a
            // full-output opaque buffer it must never float to an overlay
            // above the primary swapchain (it would hide every element that
            // fell back to GPU compositing there).
            if let Some(el) = surface.scene_dmabuf_element.as_mut() {
                el.kind = smithay::backend::renderer::element::Kind::Unspecified;
            }
            // Solid-black test element (visual debug plane).
            alloc_plane!(surface.test_dmabuf_element,       Fourcc::Argb8888, true,  "test");
            alloc_plane!(surface.windows_dmabuf_element,    Fourcc::Argb8888, false, "windows");
            alloc_plane!(surface.expose_dmabuf_element,     Fourcc::Argb8888, false, "expose");
            alloc_plane!(surface.overlay_dmabuf_element,    Fourcc::Argb8888, false, "overlay");

            // Strip-sized planes: full output width, cropped bands of their
            // full-screen containers via the element viewport. Small buffers
            // mean dock/switcher animations no longer redraw a full-screen
            // plane, and the KMS watermark cost scales with plane size.
            let dock_strip_h = (mode.size.h / 4).min(480);
            let switcher_strip_h = (mode.size.h / 2).min(960);
            macro_rules! alloc_strip {
                ($field:expr, $h:expr, $y:expr, $label:literal) => {
                    if $field.is_none() {
                        let mut el = SceneDmabufElement::new(
                            self.layers_engine.clone(),
                            (mode.size.w, $h),
                            $label,
                        );
                        el.opaque = false;
                        el.position = (0, $y);
                        el.set_viewport((0, $y));
                        match el.ensure_swapchain(device_gbm.clone(), Fourcc::Argb8888, surface.render_node) {
                            Ok(()) => $field = Some(el),
                            Err(e) => tracing::warn!("strip plane alloc failed for {crtc:?}: {e}"),
                        }
                    }
                };
            }
            alloc_strip!(
                surface.dock_dmabuf_element,
                dock_strip_h,
                mode.size.h - dock_strip_h,
                "dock"
            );
            alloc_strip!(
                surface.switcher_dmabuf_element,
                switcher_strip_h,
                (mode.size.h - switcher_strip_h) / 2,
                "switcher"
            );
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
            if let Some(el) = &surface.switcher_dmabuf_element {
                el.set_node_ref(ows.switcher_plane.id);
            }
            if let Some(el) = &surface.dock_dmabuf_element {
                el.set_node_ref(ows.dock_plane.id);
            }
        }

        // Classify every window into its visibility state so post_repaint can
        // pick a per-window frame-callback throttle. `occluded_ids` is empty
        // for v1 — we rely on the fullscreen detection inside the classifier
        // for the main "background app behind a maximized window" case.
        let expose_active =
            self.workspaces.is_expose_transitioning() || self.workspaces.get_show_all();
        tracing::debug!(
            target: "otto::planes",
            "expose inputs: transitioning={} show_all={} gesture={} animating={}",
            self.workspaces.is_expose_transitioning(),
            self.workspaces.get_show_all(),
            self.workspaces
                .show_all_gesture
                .load(std::sync::atomic::Ordering::Relaxed),
            self.workspaces
                .is_animating
                .load(std::sync::atomic::Ordering::Relaxed),
        );

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

        // Apply the scanout set (selection + content_layer transitions were
        // done in `set_scanout_windows`, before the `surface` borrow).
        surface.shadow_only_windows = scanout_desired.clone();

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
            fullscreen_window.as_ref(),
            self.workspaces.app_switcher.alive(),
            !self.workspaces.dock.is_hidden(),
            {
                // Overlay chrome is on demand: an empty full-screen ARGB
                // buffer must not waste a hardware plane slot. It is pushed
                // only when something in its subtree is actually visible.
                use smithay::desktop::layer_map_for_output;
                use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
                let w = &self.workspaces;
                let layer_shell_active = layer_map_for_output(&output)
                    .layers()
                    .any(|l| matches!(l.layer(), WlrLayer::Top | WlrLayer::Overlay));
                let popups = !w.popup_overlay.layer.children().is_empty();
                // Selector and DnD layers are never `hidden()` — they are
                // empty containers until used, so check content/state instead.
                let selector = w.is_expose_transitioning()
                    || w.get_show_all()
                    || w.is_animating.load(std::sync::atomic::Ordering::Relaxed);
                let osd = w.osd.is_visible();
                let tiling = w.tiling_overlay.is_visible();
                let dnd = self.dnd_icon.is_some();
                let active = layer_shell_active || popups || selector || osd || tiling || dnd;
                if active {
                    tracing::debug!(
                        target: "otto::planes",
                        "overlay active: shell={layer_shell_active} popups={popups} selector={selector} osd={osd} tiling={tiling} dnd={dnd}",
                    );
                }
                active
            },
            self.workspaces.output_workspaces.get(&output.name()),
            screencopy_pending,
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
    fullscreen_window: Option<&WindowElement>,
    switcher_active: bool,
    dock_visible: bool,
    overlay_active: bool,
    output_workspaces: Option<&crate::workspaces::OutputWorkspaces>,
    screencopy_pending: bool,
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
            // Fullscreen scanout must always draw: the promoted buffer's
            // commits produce no scene damage, and gating on it would drop
            // video frames.
            let should_draw = scene_has_damage
                || dnd_needs_draw
                || cursor_needs_draw
                || screencopy_pending
                || fullscreen_window.is_some();
            if !should_draw {
                return Ok(RenderOutcome::skipped());
            }

            // NOTE: when a screencopy client is waiting we still build the
            // normal plane-element set — only the scanout FrameFlags are
            // dropped (below) so every element GPU-composites into the
            // primary swapchain and `blit_current_frame` captures exactly
            // the on-screen stack. Re-rendering the scene tree as one
            // `Scene` element is NOT equivalent: plane subtrees render in
            // isolation and ignore ancestor visibility (e.g. the hidden
            // workspaces_layer while expose is shown), so a tree re-render
            // diverges from what the planes display.
            if let Some(fs_win) = fullscreen_window {
                // ── Fullscreen direct scanout ─────────────────────────────
                // The client buffer IS the frame: push only the window's
                // surface tree (cursor elements were pushed above) and skip
                // every chrome plane — dock and topbar are hidden in
                // fullscreen anyway. The buffer spans the output and is
                // opaque, so Smithay direct-scans it on the primary plane
                // (ALLOW_PRIMARY_PLANE_SCANOUT_ANY); if that fails it
                // GPU-composites the same element, which stays z-correct.
                use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
                if let Some(wl_surface) = fs_win.wl_surface() {
                    let geo_loc = fs_win.geometry().loc.to_f64().to_physical(scale);
                    let pos: Point<i32, Physical> = (
                        -(geo_loc.x.round() as i32),
                        -(geo_loc.y.round() as i32),
                    )
                        .into();
                    let elems: Vec<WorkspaceRenderElements<_>> =
                        render_elements_from_surface_tree(
                            renderer,
                            &wl_surface,
                            pos,
                            scale,
                            1.0,
                            Kind::ScanoutCandidate,
                        );
                    workspace_render_elements.extend(elems);
                }
            } else {
            // Push plane elements top→bottom. Smithay's `DrmCompositor::render_frame`
            // assigns overlay planes front-first and tries every element tagged
            // `Kind::ScanoutCandidate` on an overlay before falling back to
            // GPU-compositing that element into the primary plane. Our
            // `SceneDmabufElement` already reports `ScanoutCandidate`, so we just
            // push in z-order and let Smithay do plane assignment + fallback.
            // Push-only: planes are rendered explicitly bottom-up further
            // down (the backdrop composite needs the lower planes rendered
            // before the overlay), and engine damage is cleared only once
            // per frame — a render inside the push would re-render planes a
            // second time.
            macro_rules! push_plane {
                ($el:expr) => {
                    if let Some(el) = $el.as_ref() {
                        // Pushed even when nothing new was rendered this frame:
                        // the existing dmabuf stays on the plane and Smithay sees
                        // an unchanged commit_counter → empty damage → no page-flip.
                        if el.current_dmabuf().is_some() {
                            workspace_render_elements
                                .push(WorkspaceRenderElements::SceneDmabuf(el.clone()));
                        }
                    }
                };
            }

            // ── Cross-plane backdrop (vibrancy) ──────────────────────────
            //
            // The blur-bearing planes (overlay UI: dock, switcher, menus, OSD
            // — and expose) render into their own buffers, so their
            // `BackgroundBlur` layers can't see the planes below. Build a
            // DOWNSCALED composite of the lower planes and hand it to them via
            // lay-rs' external-backdrop API (`render_node_tree`'s backdrop
            // parameter seeds it behind the blur shapes with DstOver).
            // Downscaled because the blur re-downscales its input anyway: a
            // low-res backdrop is imperceptible after blurring but far cheaper
            // to build, hold and sample than a full-res snapshot.
            //
            // Two-stage build in one small surface: draw bg → snapshot (the
            // expose backdrop), then draw the middle plane on top → snapshot
            // (the overlay backdrop). Rebuilt only when a lower plane recorded
            // damage this frame; the fresh snapshot's unique_id is what makes
            // the consumers re-render.
            const BACKDROP_SCALE: f32 = 0.25;

            // Render the bottom plane first so the composite reflects this frame.
            if let Some(el) = &surface.scene_dmabuf_element {
                el.render(renderer.as_mut());
            }

            let bg_damaged = surface
                .scene_dmabuf_element
                .as_ref()
                .and_then(|el| el.subtree_damage())
                .is_some();
            let middle_el = if expose_active {
                surface.expose_dmabuf_element.as_ref()
            } else {
                surface.windows_dmabuf_element.as_ref()
            };
            let middle_damaged = middle_el.and_then(|el| el.subtree_damage()).is_some();
            let rebuild = surface.backdrop_image.is_none() || bg_damaged || middle_damaged;

            if rebuild {
                let bg_img = surface
                    .scene_dmabuf_element
                    .as_ref()
                    .and_then(|el| el.snapshot());
                let ctx = surface
                    .scene_dmabuf_element
                    .as_ref()
                    .and_then(|el| el.gr_context());
                if let (Some(bg_img), Some(mut ctx)) = (bg_img, ctx) {
                    let (out_w, out_h) = output
                        .current_mode()
                        .map(|m| (m.size.w, m.size.h))
                        .unwrap_or((bg_img.width(), bg_img.height()));
                    let (bw, bh) = (
                        ((out_w as f32 * BACKDROP_SCALE) as i32).max(1),
                        ((out_h as f32 * BACKDROP_SCALE) as i32).max(1),
                    );
                    if surface.backdrop_surface.is_none() {
                        let image_info = layers::skia::ImageInfo::new(
                            (bw, bh),
                            layers::skia::ColorType::RGBA8888,
                            layers::skia::AlphaType::Premul,
                            None,
                        );
                        surface.backdrop_surface = layers::skia::gpu::surfaces::render_target(
                            &mut ctx,
                            layers::skia::gpu::Budgeted::No,
                            &image_info,
                            None,
                            layers::skia::gpu::SurfaceOrigin::TopLeft,
                            None,
                            false,
                            false,
                        )
                        .map(|surface| crate::udev::types::BackdropSurface {
                            surface,
                            context: ctx.clone(),
                        });
                    }
                    if let Some(bs) = surface.backdrop_surface.as_mut() {
                        let dst = layers::skia::Rect::from_xywh(0.0, 0.0, bw as f32, bh as f32);
                        let sampling = layers::skia::SamplingOptions::new(
                            layers::skia::FilterMode::Linear,
                            layers::skia::MipmapMode::None,
                        );
                        let paint = layers::skia::Paint::default();
                        // Stage 1: bg only — the expose backdrop.
                        {
                            let canvas = bs.surface.canvas();
                            canvas.clear(layers::skia::Color4f::new(0.0, 0.0, 0.0, 1.0));
                            canvas.draw_image_rect_with_sampling_options(
                                &bg_img, None, dst, sampling, &paint,
                            );
                        }
                        if expose_active {
                            if let Some(expose) = &surface.expose_dmabuf_element {
                                bs.context.flush_and_submit();
                                let bg_small = bs.surface.image_snapshot();
                                expose.set_backdrop(Some((bg_small, BACKDROP_SCALE)));
                            }
                        }
                        // Render the middle plane now (expose renders with its
                        // fresh backdrop; windows has no blur).
                        if let Some(el) = middle_el {
                            el.render(renderer.as_mut());
                        }
                        // Stage 2: + middle plane — the overlay backdrop.
                        if let Some(middle_img) = middle_el.and_then(|el| el.snapshot()) {
                            let canvas = bs.surface.canvas();
                            canvas.draw_image_rect_with_sampling_options(
                                &middle_img, None, dst, sampling, &paint,
                            );
                        }
                        bs.context.flush_and_submit();
                        surface.backdrop_image = Some(bs.surface.image_snapshot());
                    }
                } else if let Some(el) = middle_el {
                    // No bg snapshot/context yet (first frames) — still render
                    // the middle plane so the stack stays warm.
                    el.render(renderer.as_mut());
                }
            } else if let Some(el) = middle_el {
                el.render(renderer.as_mut());
            }

            // All blur-bearing upper planes consume the same composite; each
            // re-renders only when the snapshot's unique_id changes or its own
            // subtree is damaged.
            let upper_backdrop = surface
                .backdrop_image
                .clone()
                .map(|img| (img, BACKDROP_SCALE));
            if let Some(el) = &surface.overlay_dmabuf_element {
                el.set_backdrop(upper_backdrop.clone());
                if overlay_active {
                    el.render(renderer.as_mut());
                }
            }
            if let Some(el) = &surface.switcher_dmabuf_element {
                el.set_backdrop(upper_backdrop.clone());
                if switcher_active {
                    el.render(renderer.as_mut());
                }
            }
            if let Some(el) = &surface.dock_dmabuf_element {
                el.set_backdrop(upper_backdrop);
                if dock_visible {
                    el.render(renderer.as_mut());
                }
            }

            // Push top→bottom: dock, switcher (only while alive — an empty
            // transparent strip would waste a plane), then overlay chrome.
            if dock_visible {
                push_plane!(surface.dock_dmabuf_element);
            }
            if switcher_active {
                push_plane!(surface.switcher_dmabuf_element);
            }
            if overlay_active {
                push_plane!(surface.overlay_dmabuf_element);
            }

            if expose_active {
                // Expose replaces the windows plane while it's visible.
                push_plane!(surface.expose_dmabuf_element);
                // Keep the windows swapchain warm for the expose→windows
                // transition so closing expose doesn't flash a cold frame.
                if let Some(el) = &surface.windows_dmabuf_element {
                    el.render(renderer.as_mut());
                }
            } else {
                // Top-window direct scanout: the client's Wayland buffer goes
                // to Smithay as a `ScanoutCandidate`. Smithay tries to bind
                // it to an overlay; if that fails it composites the client
                // buffer into primary (one GPU blit — still cheaper than
                // doubly-walking the scene graph).
                use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
                for win_id in &surface.shadow_only_windows {
                    if let Some(win) = window_elements.iter().find(|w| w.id() == *win_id) {
                        if let Some(wl_surface) = win.wl_surface() {
                            // render_position() is the visible-content origin
                            // (physical px). render_elements_from_surface_tree
                            // expects the wl_surface buffer origin, which for
                            // CSD windows is shifted back by geometry.loc.
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
                            {
                                use smithay::backend::renderer::element::Element as _;
                                for e in &elems {
                                    tracing::debug!(
                                        target: "otto::planes",
                                        "topwin elem {:?} geo={:?}",
                                        e.id(),
                                        e.geometry(scale.into()),
                                    );
                                }
                            }
                            tracing::debug!(
                                target: "otto::planes",
                                "topwin scanout push: {} elements at ({buf_x},{buf_y})",
                                elems.len(),
                            );
                            workspace_render_elements.extend(elems);
                        }
                    }
                }

                let has_windows = output_workspaces.map_or(false, |ows| {
                    let ws = &ows.workspace_views[ows.current_workspace];
                    !ws.windows_list.read().unwrap().is_empty()
                });
                if has_windows {
                    push_plane!(surface.windows_dmabuf_element);
                }
            }

            // Background on primary plane (bottom).
            push_plane!(surface.scene_dmabuf_element);

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

            // Debug: dump every plane buffer to PNG when the trigger file
            // exists (`touch /tmp/otto-dump-planes`), then remove it. Shows
            // exactly what each KMS plane scans out, independent of the
            // GPU-composited screencopy path.
            if std::path::Path::new("/tmp/otto-dump-planes").exists() {
                let _ = std::fs::remove_file("/tmp/otto-dump-planes");
                let dir = "/home/riccardo/Pictures/Screenshots";
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

            } // end planes branch

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

    // Reset the swapchain when the element mode changes so the transition
    // frame itself renders with full damage (see `FrameMode`).
    let frame_mode = if screencopy_pending {
        FrameMode::Composite
    } else if fullscreen_window.is_some() || !surface.shadow_only_windows.is_empty() {
        FrameMode::DirectScanout
    } else {
        FrameMode::Planes
    };
    if frame_mode != surface.last_frame_mode {
        surface.last_frame_mode = frame_mode;
        surface.compositor.reset_buffers();
    }

    // Debug: dump the final element order handed to render_frame (front→back).
    if output_elements.len() > 4 {
        use smithay::backend::renderer::element::Element as _;
        let order: Vec<String> = output_elements
            .iter()
            .map(|e| format!("{:?}", e.id()))
            .collect();
        tracing::debug!(target: "otto::planes", "element order: {}", order.join(" | "));
    }

    // Screencopy frames composite everything into the primary swapchain so
    // the capture blit sees the full image; the cursor keeps its own plane
    // so captures exclude the pointer.
    let frame_flags = if screencopy_pending {
        smithay::backend::drm::compositor::FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT
    } else {
        smithay::backend::drm::compositor::FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT
            | smithay::backend::drm::compositor::FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY
            | smithay::backend::drm::compositor::FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT
    };
    let render_frame_result = surface
        .compositor
        .render_frame(renderer, &output_elements, clear_color, frame_flags)
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

    // Debug: once per second, log how each plane element was realized
    // (ZeroCopy = on a hardware plane, Rendering = GPU-composited into the
    // primary swapchain, missing = not part of this frame at all).
    {
        use std::sync::{Mutex, OnceLock};
        use std::time::{Duration, Instant};
        static LAST: OnceLock<Mutex<Instant>> = OnceLock::new();
        let mut last = LAST
            .get_or_init(|| Mutex::new(Instant::now() - Duration::from_secs(2)))
            .lock()
            .unwrap();
        if last.elapsed() >= Duration::from_secs(1) {
            *last = Instant::now();

            // Debug: `touch /tmp/otto-tint` tints everything GPU-composited
            // red (client textures via Smithay's DebugFlags::TINT, our plane
            // fallback blits via TINT_COMPOSITE). Zero-copy plane scanout
            // stays untinted. Remove the file to switch the tint off.
            {
                use smithay::backend::renderer::DebugFlags;
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
                crate::render_elements::scene_dmabuf_element::TINT_COMPOSITE
                    .store(tint, std::sync::atomic::Ordering::Relaxed);
            }

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
            // Histogram over every element smithay saw this frame — client
            // buffers (direct scanout candidates) show up here even though
            // we can't match their ids to a plane element.
            let (mut zc, mut rend, mut skip) = (0, 0, 0);
            for s in states.states.values() {
                use smithay::backend::renderer::element::RenderElementPresentationState as P;
                match s.presentation_state {
                    P::ZeroCopy => zc += 1,
                    P::Rendering { .. } => rend += 1,
                    P::Skipped => skip += 1,
                }
            }
            tracing::info!(
                target: "otto::planes",
                "frame realization: {summary}expose_active={expose_active} shadow_only={} elements: total={} zerocopy={zc} rendering={rend} skipped={skip}",
                surface.shadow_only_windows.len(),
                states.states.len(),
            );
        }
    }
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

    // In fullscreen scanout only the fullscreen window gets frame callbacks —
    // other windows generating damage would only cause pointless wakeups.
    let post_repaint_elements: Vec<&WindowElement> = if let Some(fs_win) = fullscreen_window {
        vec![fs_win]
    } else {
        window_elements.to_vec()
    };

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
