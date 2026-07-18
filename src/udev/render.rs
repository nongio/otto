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
use crate::state::Backend;
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
                tracing::debug!(target: "otto::fps", "fps={fps:.1} ({} frames in {:.2}s)", g.1, elapsed.as_secs_f64());
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

        // Debug (`/tmp/otto-slow`): a VBlank line proves page flips complete.
        if std::path::Path::new("/tmp/otto-slow").exists() {
            tracing::info!(target: "otto::planes", "SLOW vblank on {crtc:?}");
        }
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
            if scene_has_damage {
                // Damage ticks are counted globally: the flag is consumed by
                // whichever output ticks first, so other surfaces detect the
                // event by lagging behind `damage_generation`.
                self.backend_data.damage_generation += 1;
            }
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
    /// Kernel reported a display FIFO underrun: the display engine could
    /// not fetch the currently-configured planes. Reduce the plane budget
    /// one step (1 = no window promotion, 2 = full GPU composite) and
    /// re-render everything so the lighter configuration flips in now.
    /// Sticky for the session — underruns tend to recur under the same
    /// plane load, and each occurrence corrupts a visible frame.
    pub(super) fn raise_underrun_penalty(&mut self) {
        if self.backend_data.underrun_penalty >= 2 {
            return;
        }
        self.backend_data.underrun_penalty += 1;
        let level = self.backend_data.underrun_penalty;
        tracing::warn!(
            "display FIFO underrun — reducing plane budget to level {} ({})",
            level,
            if level == 1 {
                "window promotion disabled"
            } else {
                "plane decomposition disabled, full GPU composite"
            }
        );
        for device in self.backend_data.backends.values_mut() {
            for surface in device.surfaces.values_mut() {
                for el in [
                    &surface.scene_dmabuf_element,
                    &surface.windows_dmabuf_element,
                    &surface.expose_dmabuf_element,
                    &surface.overlay_dmabuf_element,
                    &surface.switcher_dmabuf_element,
                    &surface.dock_dmabuf_element,
                ]
                .into_iter()
                .flatten()
                {
                    el.request_full_render();
                }
                surface.idle_countdown = 3;
            }
        }
        let nodes: Vec<_> = self.backend_data.backends.keys().copied().collect();
        for node in nodes {
            self.render(node, None);
        }
    }

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
        let allow_fullscreen_scanout = std::env::var_os("DISABLE_DIRECT_SCANOUT").is_none()
            && self.workspaces.is_fullscreen_and_stable()
            && !self.swipe_gesture.is_active()
            && !capture_active;
        let fullscreen_window = if allow_fullscreen_scanout {
            self.workspaces.get_fullscreen_window()
        } else {
            None
        };
        // XWayland fullscreen windows take the same direct-scanout path as
        // native clients: the black-scanout they used to show was the
        // clear-color CCS modifiers (stripped since — see
        // feedback::strip_clear_color_modifiers) plus the missing
        // explicit-sync acquire blocker (added in shell::new_surface).
        // Whether this output uses the plane decomposition at all (set once at
        // surface creation from overlay count / atomic / GPU identity).
        // Underrun penalty level 2 fully disables the plane decomposition:
        // the display engine proved it cannot fetch this many planes
        // (see `UdevData::underrun_penalty`).
        let planes_enabled = self
            .backend_data
            .backends
            .get(&node)
            .and_then(|d| d.surfaces.get(&crtc))
            .map(|s| s.planes_enabled)
            .unwrap_or(false)
            && self.backend_data.underrun_penalty < 2;
        // Any running minimize/unminimize genie forces the full-GPU scene
        // composite for the frame (and drops scanout promotion): the genie's
        // image filter paints far outside the per-plane damage rects, so the
        // plane pipeline's partial redraws corrupt the animation. The mode is
        // held ~150ms past the animation: the settle work (reparent into the
        // drawer, rescale, unhide) lands from an async task across several
        // engine updates, and flipping back to planes mid-settle scans out a
        // stale frame.
        let minimize_now = self.workspaces.has_minimizing_window();
        let minimize_active = if let Some(surf) = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|d| d.surfaces.get_mut(&crtc))
        {
            if minimize_now {
                surf.composite_hold_until =
                    Some(Instant::now() + std::time::Duration::from_millis(150));
                true
            } else {
                surf.composite_hold_until
                    .is_some_and(|t| Instant::now() < t)
            }
        } else {
            minimize_now
        };
        // Promotion candidates are per-output — resolve this CRTC's output
        // before the surface borrow below.
        let scanout_output = self.workspaces.outputs().find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device_id == node && id.crtc == crtc)
                .unwrap_or(false)
        });
        let scanout_output_name = scanout_output.map(|o| o.name());
        let raw_scanout_desired: Vec<smithay::reexports::wayland_server::backend::ObjectId> =
            if !planes_enabled
                || self.backend_data.underrun_penalty >= 1
                || capture_active
                || minimize_active
                || self.swipe_gesture.is_active()
                || fullscreen_window.is_some()
            {
                Vec::new()
            } else if let Some(output) = scanout_output {
                self.workspaces.get_scanout_candidates(output)
            } else {
                Vec::new()
            };
        // Promotion hysteresis (see `SurfaceData::promote_candidates`):
        // removals apply this frame, additions only after the candidate set
        // has been stable for the full window.
        const PROMOTE_STABLE: std::time::Duration = std::time::Duration::from_millis(500);
        let current_scanout = self.workspaces.scanout_window_ids();
        let has_additions = raw_scanout_desired
            .iter()
            .any(|id| !current_scanout.contains(id));
        let scanout_desired: Vec<smithay::reexports::wayland_server::backend::ObjectId> =
            if !has_additions {
                raw_scanout_desired
            } else if let Some(surf) = self
                .backend_data
                .backends
                .get_mut(&node)
                .and_then(|d| d.surfaces.get_mut(&crtc))
            {
                if surf.promote_candidates != raw_scanout_desired {
                    surf.promote_candidates = raw_scanout_desired.clone();
                    surf.promote_since = Some(Instant::now());
                }
                if surf
                    .promote_since
                    .is_some_and(|t| t.elapsed() >= PROMOTE_STABLE)
                {
                    raw_scanout_desired
                } else {
                    // Additions still settling — keep only the members that
                    // are already promoted AND still eligible.
                    raw_scanout_desired
                        .into_iter()
                        .filter(|id| current_scanout.contains(id))
                        .collect()
                }
            } else {
                raw_scanout_desired
            };
        // Demotion: windows that LEAVE the scanout set had a stale (or no)
        // lay-rs content import while promoted; re-import them now (after the
        // set update unhides their content_layer) so the first composited
        // frame shows the current buffer, not a stale one.
        let new_scanout_ids: std::collections::HashSet<
            smithay::reexports::wayland_server::backend::ObjectId,
        > = scanout_desired.iter().cloned().collect();
        let prev_scanout_ids = self.workspaces.scanout_window_ids();
        // Resolve departures through windows_map, NOT the Space: a window that
        // starts minimizing is unmapped from every Space *before* this frame
        // (hit-test exclusion), so a Space lookup misses it and skips the
        // re-import — the genie animation would then run on the stale/blank
        // content left over from promotion.
        let mut departed_windows: Vec<WindowElement> = prev_scanout_ids
            .iter()
            .filter(|id| !new_scanout_ids.contains(id))
            .filter_map(|id| self.workspaces.get_window_for_surface(id).cloned())
            .collect();
        // Fullscreen direct scanout never renders the window into the scene, so
        // when it ends (e.g. an expose gesture switches to render-all) the
        // composited scene and the expose mirror have no texture for it. Treat
        // the window leaving fullscreen scanout like a demotion: re-import +
        // scene damage so its content is drawn before it's shown.
        let fullscreen_now_id = fullscreen_window.as_ref().map(|w| w.id());
        let fullscreen_departed = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|d| d.surfaces.get_mut(&crtc))
            .and_then(|surf| {
                let prev = surf.last_fullscreen_scanout.take();
                surf.last_fullscreen_scanout = fullscreen_now_id.clone();
                prev.filter(|p| fullscreen_now_id.as_ref() != Some(p))
            });
        if let Some(fid) = fullscreen_departed {
            if let Some(w) = self.workspaces.get_window_for_surface(&fid).cloned() {
                if !departed_windows.iter().any(|d| d.id() == w.id()) {
                    departed_windows.push(w);
                }
            }
        }
        if let Some(name) = scanout_output_name.as_deref() {
            self.workspaces
                .set_scanout_windows_for_output(name, &scanout_desired);
        }
        for w in &departed_windows {
            self.update_window_view(w);
        }

        // A workspace swipe — and the settle/snap animation after the finger
        // lifts — scrolls the scene content across the output without producing
        // per-plane subtree damage, so the plane pipeline keeps scanning out a
        // stale frame (a visible flicker, most obvious with a single full-output
        // window). Force the scrolling planes to redraw every frame while either
        // the gesture or the follow-up animation is running.
        let swipe_active = self.swipe_gesture.is_active()
            || self
                .workspaces
                .is_animating
                .load(std::sync::atomic::Ordering::Relaxed);

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

        // A demoted window's content was hidden/blanked in the windows plane
        // while it was promoted; on demotion force a full-buffer redraw so the
        // windows plane repaints the whole region with the re-imported content
        // instead of trusting partial engine damage (which can miss the freshly
        // unhidden layer and leave a hole where the scanned-out buffer was).
        if !departed_windows.is_empty() {
            if let Some(el) = &surface.windows_dmabuf_element {
                el.request_full_render();
            }
        }

        // Swipe: redraw the scrolling planes every frame (see note above).
        if swipe_active {
            for el in [
                &surface.scene_dmabuf_element,
                &surface.windows_dmabuf_element,
                &surface.expose_dmabuf_element,
            ] {
                if let Some(el) = el {
                    el.request_full_render();
                }
            }
        }

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
            if self.scene_element.update() {
                self.backend_data.damage_generation += 1;
            }
            true
        } else {
            match prefetched_scene_damage {
                Some(damage) => damage,
                None => {
                    let damage = self.scene_element.update();
                    if damage {
                        self.backend_data.damage_generation += 1;
                    }
                    damage
                }
            }
        };
        let all_window_elements: Vec<&WindowElement> = self.workspaces.spaces_elements().collect();

        // Lazily set up the dmabuf-backed scene elements for this surface.
        // Uses `device_gbm` (cloned out of `device` before the surface
        // mut-borrow) so we don't conflict with the existing mutable borrows.
        // Skipped entirely when the plane decomposition is disabled for this
        // output — the swapchains would only waste GPU memory.
        if let (true, Some(mode)) = (planes_enabled, output.current_mode()) {
            super::planes::ensure_plane_elements(
                surface,
                &self.layers_engine,
                &device_gbm,
                crtc,
                (mode.size.w, mode.size.h),
            );
        }

        // Every frame: point each plane element at its output's node.
        if let Some(ows) = self.workspaces.output_workspaces.get(&output.name()) {
            super::planes::wire_plane_nodes(surface, ows);
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

        let occluded_ids = self.workspaces.occluded_window_ids();
        let window_throttle_states = crate::state::window_throttle::classify_windows(
            &self.workspaces,
            &all_window_elements,
            &occluded_ids,
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
        // An active screencast of this output is treated exactly like a
        // pending screencopy: it forces a full GPU composite (no scanout) and,
        // crucially, forces `should_draw` so an idle desktop still renders —
        // otherwise the screenshare tap (and any RDP bridge on top) is starved
        // of frames and the remote client spins forever. The off-VBlank timer
        // (`kick_screencast_outputs`) supplies the render trigger when idle;
        // this makes that trigger actually paint.
        let screencast_active = self
            .screenshare_sessions
            .values()
            .any(|s| s.streams.contains_key(&output.name()));
        let screencopy_pending = screencast_active
            || self
                .pending_screencopy_frames
                .iter()
                .any(|p| p.output == output);

        // Apply the scanout set (selection + content_layer transitions were
        // done in `set_scanout_windows`, before the `surface` borrow).
        surface.shadow_only_windows = scanout_desired.clone();

        // Dock/switcher chrome exists only on the primary output. Beyond
        // correctness, NOT pushing these strip planes on secondary outputs
        // matters for display bandwidth: extra full-width planes on a 4K
        // output can exceed the display engine's fetch budget and cause
        // pipe FIFO underruns (bottom of the frame scans out as garbage).
        let chrome_output = self
            .workspaces
            .primary_output()
            .map(|p| p.name() == output.name())
            .unwrap_or(false);
        let switcher_active = chrome_output && self.workspaces.app_switcher.alive();
        let overlay_active =
            self.workspaces.is_overlay_ui_active(&output) || self.dnd_icon.is_some();
        {
            use super::planes::maybe_release_plane;
            maybe_release_plane(
                &mut surface.expose_dmabuf_element,
                expose_active,
                &mut surface.expose_last_active,
                "expose",
            );
            maybe_release_plane(
                &mut surface.switcher_dmabuf_element,
                switcher_active,
                &mut surface.switcher_last_active,
                "switcher",
            );
            maybe_release_plane(
                &mut surface.overlay_dmabuf_element,
                overlay_active,
                &mut surface.overlay_last_active,
                "overlay",
            );
        }
        // Re-activation edge: the plane's buffer still shows whatever was
        // rendered before it left the frame (a destroyed tooltip, a closed
        // switcher) — the removal damage was cleared on frames where the
        // plane didn't render. Force a full redraw so re-pushing the plane
        // can't flash ghost content.
        if overlay_active && !surface.overlay_was_active {
            if let Some(el) = &surface.overlay_dmabuf_element {
                el.request_full_render();
            }
        }
        surface.overlay_was_active = overlay_active;
        if switcher_active && !surface.switcher_was_active {
            if let Some(el) = &surface.switcher_dmabuf_element {
                el.request_full_render();
            }
        }
        surface.switcher_was_active = switcher_active;
        // Debug: `touch /tmp/otto-full-redraw` forces every plane element on
        // every output to fully re-render and flip a fresh buffer on its
        // next frame (needs a frame trigger, e.g. moving the cursor).
        // Remove the file and touch it again to re-trigger.
        {
            let want = std::path::Path::new("/tmp/otto-full-redraw").exists();
            if want && !surface.full_redraw_done {
                surface.full_redraw_done = true;
                tracing::info!(target: "otto::planes", "debug full redraw requested");
                for el in [
                    &surface.scene_dmabuf_element,
                    &surface.windows_dmabuf_element,
                    &surface.expose_dmabuf_element,
                    &surface.overlay_dmabuf_element,
                    &surface.switcher_dmabuf_element,
                    &surface.dock_dmabuf_element,
                ]
                .into_iter()
                .flatten()
                {
                    el.request_full_render();
                }
            } else if !want {
                surface.full_redraw_done = false;
            }
        }
        // Composite→planes edge: the genie frames rendered through the
        // full-scene element, which consumed and cleared all engine damage
        // while the plane buffers sat idle. Without a full redraw the first
        // planes frame scans out the stale pre-composite content (ghost of
        // the just-minimized window).
        if surface.was_force_composite && !minimize_active {
            for el in [
                &surface.scene_dmabuf_element,
                &surface.windows_dmabuf_element,
                &surface.expose_dmabuf_element,
                &surface.overlay_dmabuf_element,
                &surface.switcher_dmabuf_element,
                &surface.dock_dmabuf_element,
            ]
            .into_iter()
            .flatten()
            {
                el.request_full_render();
            }
            if std::path::Path::new("/tmp/otto-dump-transition").exists() {
                surface.transition_dump_left = 8;
            }
        }
        surface.was_force_composite = minimize_active;

        let output_scene_element = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .map(|ows| self.scene_element.for_output_layer(&ows.output_layer))
            .unwrap_or_else(|| self.scene_element.clone());

        // A surface lagging the global damage generation must render even if
        // its own tick reported no damage — the damage flag was consumed on
        // another output's tick (see `UdevData::damage_generation`).
        let frame_gen = self.backend_data.damage_generation;
        let scene_has_damage = scene_has_damage || surface.rendered_damage_gen < frame_gen;

        let result = render_output_frame(
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
            switcher_active,
            chrome_output && !self.workspaces.dock.is_hidden(),
            overlay_active,
            {
                // The windows plane must stay up while a workspace switch is
                // in flight: `current_workspace` flips to the TARGET workspace
                // the moment the release animation starts, so gating on the
                // current workspace alone drops the plane mid-transition when
                // switching toward an empty workspace — the source windows
                // vanish while still animating out. The swipe drag and the
                // follow-up animation both keep it pushed.
                let current_has_windows = self
                    .workspaces
                    .output_workspaces
                    .get(&output.name())
                    .map(|ows| ows.current_workspace_has_windows())
                    .unwrap_or(false);
                current_has_windows
                    || self.swipe_gesture.is_active()
                    || self
                        .workspaces
                        .is_animating
                        .load(std::sync::atomic::Ordering::Relaxed)
            },
            screencopy_pending,
            self.workspaces
                .scanout_commit_pending
                .swap(false, std::sync::atomic::Ordering::Relaxed),
            planes_enabled,
            minimize_active,
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
                    // A ContextLost here is nearly always spurious — a stray
                    // EGL BAD_PARAMETER from the screenshare blit leaving the
                    // context in an odd state, not a genuinely lost GL context.
                    // Panicking took the WHOLE compositor (every window, the
                    // user's session) down over one bad auxiliary frame. Drop
                    // this frame and reschedule instead; the next frame rebinds
                    // the primary from scratch. If the context really is gone
                    // the next frame fails the same way and just logs again —
                    // still far better than killing the session.
                    SwapBuffersError::ContextLost(err) => {
                        warn!("Rendering context lost ({err}); dropping frame and continuing");
                        true
                    }
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
                        // Rebase the global pointer to this output's space —
                        // same correction as in render_output_frame.
                        let pointer_location =
                            self.pointer.current_location() - output.current_location().to_f64();

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
                if was_rendered {
                    surface.has_rendered_once = true;
                }
                if result.is_ok() {
                    surface.rendered_damage_gen = frame_gen;
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

        // Multi-output damage lifecycle: engine damage can only be cleared
        // once EVERY surface has rendered the current damage generation —
        // clearing earlier starves the other outputs' plane renders (windows
        // frozen on their first frame). Surfaces that are behind and idle get
        // scheduled here; busy ones consume the lag via their own loop.
        let gen = self.backend_data.damage_generation;
        let mut lagging: Vec<(DrmNode, crtc::Handle)> = Vec::new();
        let mut all_caught_up = true;
        for (n, d) in self.backend_data.backends.iter_mut() {
            for (c, s) in d.surfaces.iter_mut() {
                if s.rendered_damage_gen < gen {
                    all_caught_up = false;
                    if s.idle_countdown == 0 {
                        // Marks the surface as scheduled — the same invariant
                        // the input kick in init.rs relies on.
                        s.idle_countdown = 3;
                        lagging.push((*n, *c));
                    }
                }
            }
        }
        if all_caught_up {
            self.layers_engine.clear_damage();
        } else {
            for (n, c) in lagging {
                self.handle
                    .insert_source(Timer::immediate(), move |_, _, data| {
                        data.render(n, Some(c));
                        TimeoutAction::Drop
                    })
                    .expect("failed to schedule lagging-output render");
            }
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
    /// Keep physical outputs that have an active screencast rendering, even
    /// when their desktop is idle. A physical output only renders on damage,
    /// so a static screen would starve the screenshare tap (and any RDP
    /// bridge on top of it) of frames — the remote client would sit on a
    /// blank "loading" screen until something happened to move. Forcing a
    /// full frame per tick (via `reset_buffers`) mirrors how virtual outputs
    /// already stream continuously. No-op when nothing is being cast.
    pub(super) fn kick_screencast_outputs(&mut self) {
        if self.screenshare_sessions.is_empty() {
            return;
        }
        // Rate-limit hard: each kick drops the primary swapchain (a
        // full-screen buffer reallocation on the next frame). Content
        // activity damages and renders at full rate on its own; the kick
        // only refreshes a static screen. Exception: cursor motion — the
        // cursor moves on a hardware plane without damaging the scene, but
        // the remote feed only shows it where a blit embedded it, so a
        // moved cursor kicks immediately.
        const KICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        let cursor_pos = self.pointer.current_location();
        let cursor_moved = self.backend_data.last_kick_cursor_pos != Some(cursor_pos);
        if !cursor_moved
            && self
                .backend_data
                .last_screencast_kick
                .is_some_and(|t| t.elapsed() < KICK_INTERVAL)
        {
            return;
        }
        self.backend_data.last_screencast_kick = Some(std::time::Instant::now());
        self.backend_data.last_kick_cursor_pos = Some(cursor_pos);
        // Collect the connectors with an active cast (dedup across sessions).
        let mut connectors: Vec<String> = Vec::new();
        for session in self.screenshare_sessions.values() {
            for connector in session.streams.keys() {
                if !connectors.contains(connector) {
                    connectors.push(connector.clone());
                }
            }
        }
        for connector in connectors {
            let Some(output) = self
                .workspaces
                .outputs()
                .find(|o| o.name() == connector)
                .cloned()
            else {
                continue;
            };
            // Virtual outputs render via `render_virtual_outputs`; skip them.
            if crate::virtual_output::is_virtual_output(&output) {
                continue;
            }
            let Some((node, crtc)) = output
                .user_data()
                .get::<super::types::UdevOutputId>()
                .map(|id| (id.device_id, id.crtc))
            else {
                continue;
            };
            // Force a full frame so the screenshare blit runs without damage.
            self.backend_data.reset_buffers(&output);
            self.render(node, Some(crtc));
        }
    }

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

            // Composite this output the way the KMS plane path decomposes it:
            // one isolated subtree per plane, stacked in z-order. A single
            // `for_output_layer(output_layer)` re-render is NOT equivalent —
            // plane subtrees ignore ancestor visibility (the hidden
            // `workspaces_layer` while expose is shown), so the tree render
            // went black during expose and expose gestures.
            // Top→bottom, matching the physical push order in
            // `render_output_frame`; the windows subtree is dropped while
            // expose is up, exactly like the windows plane.
            let expose_active =
                self.workspaces.is_expose_transitioning() || self.workspaces.get_show_all();
            let scene_stack: Vec<crate::render_elements::scene_element::SceneElement> = self
                .workspaces
                .output_workspaces
                .get(&output_name)
                .map(|ows| {
                    let pos = ows.output_layer.render_position();
                    let origin = (pos.x, pos.y);
                    let mut stack = vec![
                        scene_element.for_plane_subtree(&ows.dock_plane, origin),
                        scene_element.for_plane_subtree(&ows.switcher_plane, origin),
                        scene_element.for_plane_subtree(&ows.overlay_plane, origin),
                        scene_element.for_plane_subtree(&ows.expose_layer, origin),
                    ];
                    if !expose_active {
                        stack.push(scene_element.for_plane_subtree(&ows.windows_plane, origin));
                    }
                    stack.push(scene_element.for_plane_subtree(&ows.background_plane, origin));
                    stack
                })
                .unwrap_or_else(|| vec![scene_element.clone()]);

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
                            elements.extend(
                                scene_stack
                                    .iter()
                                    .cloned()
                                    .map(WorkspaceRenderElements::Scene),
                            );
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
                                    ss_elements.extend(
                                        scene_stack
                                            .iter()
                                            .cloned()
                                            .map(WorkspaceRenderElements::Scene),
                                    );
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
pub(super) fn render_output_frame<'a>(
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
    windows_plane_has_content: bool,
    screencopy_pending: bool,
    scanout_commit: bool,
    planes_enabled: bool,
    force_composite: bool,
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

    // The pointer location is global (multi-output space) but this frame's
    // coordinates are output-local — rebase so the in-output test and the
    // cursor element position are relative to this output's top-left.
    let pointer_location = pointer_location - output.current_location().to_f64();

    let pointer_in_output = output_geometry
        .to_f64()
        .contains(pointer_location.to_physical(scale));
    // One farewell frame when the pointer crosses to another output: render
    // without the cursor element so the cursor plane is cleared — otherwise
    // this output keeps scanning out the stale cursor at its last position.
    let cursor_left_output = surface.cursor_was_in_output && !pointer_in_output;
    surface.cursor_was_in_output = pointer_in_output;

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
        let cursor_needs_draw = pointer_in_output || cursor_left_output;
            // Fullscreen scanout must always draw: the promoted buffer's
            // commits produce no scene damage, and gating on it would drop
            // video frames. `scanout_commit` is the same signal for promoted
            // (non-fullscreen) windows, set per-commit by the shell.
            // A surface that has never rendered must always draw: the global
            // scene-damage flag may already have been consumed by another
            // output's render, and skipping would leave this display black.
            let should_draw = scene_has_damage
                || !surface.has_rendered_once
                || dnd_needs_draw
                || cursor_needs_draw
                || screencopy_pending
                || fullscreen_window.is_some()
                || scanout_commit;
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
            // Render the bottom plane first (planes mode only) — the backdrop
            // composite needs it, and its dmabuf's existence decides whether
            // the plane stack has a floor at all.
            let bg_plane_ready = if planes_enabled && fullscreen_window.is_none() && !force_composite {
                if let Some(el) = &surface.scene_dmabuf_element {
                    el.render(renderer.as_mut());
                    el.current_dmabuf().is_some()
                } else {
                    false
                }
            } else {
                false
            };

            // Whether the full-scene element is part of this frame. It clears
            // engine damage itself in draw(); clearing here as well would wipe
            // the damage region its subtree culling depends on.
            let mut scene_element_pushed = false;

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
            } else if !planes_enabled || !bg_plane_ready {
                // Single-element path: the plane decomposition is disabled for
                // this output (plane-poor / non-atomic / secondary-GPU
                // hardware), a full-GPU composite is explicitly forced (e.g.
                // during the minimize genie, whose image filter paints far
                // outside the per-plane damage rects), or the bg plane
                // unexpectedly has no dmabuf (allocation or Skia-surface
                // failure) — without a floor the plane stack would show only
                // the clear color.
                if planes_enabled && !force_composite {
                    static WARNED: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        tracing::warn!(
                            target: "otto::planes",
                            "bg plane has no dmabuf — falling back to scene composite"
                        );
                    }
                }
                workspace_render_elements
                    .push(WorkspaceRenderElements::Scene(output_scene_element.clone()));
                scene_element_pushed = true;
            } else {
            // Push plane elements top→bottom. Smithay's `DrmCompositor::render_frame`
            // assigns overlay planes front-first and tries every element tagged
            // `Kind::ScanoutCandidate` on an overlay before falling back to
            // GPU-compositing that element into the primary plane. Our
            // `SceneDmabufElement` already reports `ScanoutCandidate`, so we just
            // push in z-order and let Smithay do plane assignment + fallback.
            // Push-only (`planes::push_ready`): planes are rendered
            // explicitly bottom-up further down (the backdrop composite
            // needs the lower planes rendered before the overlay), and
            // engine damage is cleared only once per frame — a render
            // inside the push would re-render planes a second time.
            use super::planes::push_ready;

            // Cross-plane backdrop (vibrancy): rebuild the downscaled
            // composite when needed, render the middle plane, and hand the
            // composite to the blur-bearing upper planes (see
            // `udev::backdrop` for the full design notes).
            super::backdrop::update_backdrop_and_upper_planes(
                surface,
                renderer,
                output,
                expose_active,
                overlay_active,
                switcher_active,
                dock_visible,
            );

            // Push top→bottom: dock, switcher (only while alive — an empty
            // transparent strip would waste a plane), then overlay chrome.
            if dock_visible {
                push_ready(&surface.dock_dmabuf_element, &mut workspace_render_elements);
            }
            if switcher_active {
                push_ready(&surface.switcher_dmabuf_element, &mut workspace_render_elements);
            }
            if overlay_active {
                push_ready(&surface.overlay_dmabuf_element, &mut workspace_render_elements);
            }

            if expose_active {
                // Expose replaces the windows plane while it's visible.
                push_ready(&surface.expose_dmabuf_element, &mut workspace_render_elements);
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
                //
                // Only the ROOT surface is pushed. Decoration subsurfaces
                // (SSD titlebar/buttons/borders) keep rendering in the
                // windows plane — they never overlap the root surface's
                // rect, so outside the client element the windows plane
                // simply shows through. Pushing the whole tree instead made
                // every promoted SSD window explode into many overlapping
                // candidates that lost the plane auction and dragged the
                // full stack into GPU composite.
                use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
                use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
                for win_id in &surface.shadow_only_windows {
                    if let Some(win) = window_elements.iter().find(|w| w.id() == *win_id) {
                        if let Some(wl_surface) = win.wl_surface() {
                            // render_position() is the visible-content origin
                            // (physical px). The element expects the wl_surface
                            // buffer origin, which for CSD windows is shifted
                            // back by geometry.loc.
                            let pos = win.base_layer().render_position();
                            let geo_loc = win.geometry().loc.to_f64().to_physical(scale);
                            let buf_x = pos.x as f64 - geo_loc.x;
                            let buf_y = pos.y as f64 - geo_loc.y;
                            let elem = smithay::wayland::compositor::with_states(
                                &wl_surface,
                                |states| {
                                    // Same location math as the tree walk in
                                    // render_elements_from_surface_tree: the
                                    // root element sits at origin + its view
                                    // offset.
                                    let mut location: Point<f64, Physical> =
                                        (buf_x, buf_y).into();
                                    match states
                                        .data_map
                                        .get::<RendererSurfaceStateUserData>()
                                        .and_then(|d| d.lock().unwrap().view())
                                    {
                                        Some(view) => {
                                            location +=
                                                view.offset.to_f64().to_physical(scale);
                                        }
                                        // Unmapped — nothing to scan out.
                                        None => return Ok(None),
                                    }
                                    WaylandSurfaceRenderElement::from_surface(
                                        renderer,
                                        &wl_surface,
                                        states,
                                        location,
                                        1.0,
                                        Kind::ScanoutCandidate,
                                    )
                                },
                            );
                            match elem {
                                Ok(Some(e)) => {
                                    {
                                        use smithay::backend::renderer::element::Element as _;
                                        tracing::debug!(
                                            target: "otto::planes",
                                            "topwin scanout push at ({buf_x},{buf_y}) geo={:?} src={:?}",
                                            e.geometry(scale),
                                            e.src(),
                                        );
                                        // Debug (`/tmp/otto-slow`): commit counter must advance
                                        // with every client frame — a constant value here means
                                        // the surface's damage bag never ticks and the plane
                                        // keeps scanning the first buffer forever.
                                        if std::path::Path::new("/tmp/otto-slow").exists() {
                                            tracing::info!(
                                                target: "otto::planes",
                                                "SLOW topwin {win_id:?} commit={:?}",
                                                e.current_commit(),
                                            );
                                        }
                                    }
                                    workspace_render_elements
                                        .push(WorkspaceRenderElements::from(e));
                                }
                                Ok(None) => {}
                                Err(err) => {
                                    tracing::warn!(
                                        target: "otto::planes",
                                        "topwin surface import failed: {err}"
                                    );
                                }
                            }
                        }
                    }
                }

                if windows_plane_has_content {
                    push_ready(&surface.windows_dmabuf_element, &mut workspace_render_elements);
                }
            }

            // Background on primary plane (bottom).
            push_ready(&surface.scene_dmabuf_element, &mut workspace_render_elements);

            #[cfg(feature = "debug-kms")]
            super::debug::maybe_save_planes(surface);
            super::debug::maybe_dump_planes(surface);

            if surface.transition_dump_left > 0 {
                let idx = 8 - surface.transition_dump_left;
                surface.transition_dump_left -= 1;
                super::debug::dump_transition_frame(surface, idx);
            }

            } // end planes branch

            // Engine damage is NOT cleared here: with multiple outputs the
            // other surfaces still need this frame's damage rects for their
            // own plane renders. The caller (`render_surface`) clears it once
            // every surface has caught up to the current damage generation.
            let _ = scene_element_pushed;

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
    let frame_mode = if screencopy_pending || force_composite {
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

    // Debug (`/tmp/otto-slow`): frame-by-frame slideshow — sleep 100ms per
    // frame and log a frame counter with the mode and element set, so a
    // human-visible glitch can be matched 1:1 to a logged frame.
    if std::path::Path::new("/tmp/otto-slow").exists() {
        static FRAME_NO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = FRAME_NO.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        use smithay::backend::renderer::element::Element as _;
        let order: Vec<String> = output_elements.iter().map(|e| format!("{:?}", e.id())).collect();
        tracing::info!(
            target: "otto::planes",
            "SLOW frame {n}: mode={frame_mode:?} shadow_only={:?} elements=[{}]",
            surface.shadow_only_windows,
            order.join(" | ")
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Debug: dump the final element order handed to render_frame (front→back).
    // Level-checked first — the strings must not be built on every frame when
    // the target is quiet.
    if tracing::enabled!(target: "otto::planes", tracing::Level::DEBUG) && output_elements.len() > 4
    {
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

    // Debug (`/tmp/otto-slow`): log the frame outcome — pairs with the
    // pre-render SLOW frame line so a frozen screen can be attributed to
    // either "no flip queued" (rendered=false) or a post-queue problem.
    if std::path::Path::new("/tmp/otto-slow").exists() {
        tracing::info!(target: "otto::planes", "SLOW result: rendered={rendered}");
    }

    // 1 Hz: refresh /tmp debug toggles and log per-plane realization.
    super::debug::debug_tick(surface, &states, expose_active);

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
    // All windows get callbacks, always: the throttle classifier already
    // demotes everything behind a fullscreen window to the 2 Hz Occluded
    // bucket, and dropping below that starves Chromium's buffer-eviction
    // heuristic (blank canvas on restore).
    post_repaint(
        output,
        &states,
        window_elements,
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
            take_presentation_feedback(output, window_elements, &states);
        surface
            .compositor
            .queue_frame(Some(output_presentation_feedback))?;
    }

    Ok(RenderOutcome {
        rendered,
        damage: damage_for_return,
    })
}

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
