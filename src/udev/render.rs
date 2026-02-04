// Rendering module - Surface rendering and frame management
//
// This module contains the core rendering logic for the udev backend:
// - Frame submission and presentation feedback
// - Surface rendering pipeline
// - Direct scanout optimization
// - Screenshare integration

#[cfg(feature = "metrics")]
use std::sync::Arc;
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
    render_elements::{output_render_elements::OutputRenderElements, scene_element::SceneElement},
    shell::{WindowElement, WindowRenderElement},
    state::{post_repaint, take_presentation_feedback, SurfaceDmabufFeedback},
};

use smithay::{
    backend::{
        drm::{DrmAccessError, DrmError, DrmEventMetadata, DrmNode},
        renderer::element::{AsRenderElements, Kind},
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

use super::types::{RenderOutcome, SurfaceData, UdevData, UdevOutputId, UdevRenderer};
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
            o.user_data().get::<UdevOutputId>()
                == Some(&UdevOutputId {
                    device_id: surface.device_id,
                    crtc,
                })
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
            // What are we trying to solve by introducing a delay here:
            //
            // Basically it is all about latency of client provided buffers.
            // A client driven by frame callbacks will wait for a frame callback
            // to repaint and submit a new buffer. As we send frame callbacks
            // as part of the repaint in the compositor the latency would always
            // be approx. 2 frames. By introducing a delay before we repaint in
            // the compositor we can reduce the latency to approx. 1 frame + the
            // remaining duration from the repaint to the next VBlank.
            //
            // With the delay it is also possible to further reduce latency if
            // the client is driven by presentation feedback. As the presentation
            // feedback is directly sent after a VBlank the client can submit a
            // new buffer during the repaint delay that can hit the very next
            // VBlank, thus reducing the potential latency to below one frame.
            //
            // Choosing a good delay is a topic on its own so we just implement
            // a simple strategy here. We just split the duration between two
            // VBlanks into two steps, one for the client repaint and one for the
            // compositor repaint. Theoretically the repaint in the compositor should
            // be faster so we give the client a bit more time to repaint. On a typical
            // modern system the repaint in the compositor should not take more than 2ms
            // so this should be safe for refresh rates up to at least 120 Hz. For 120 Hz
            // this results in approx. 3.33ms time for repainting in the compositor.
            // A too big delay could result in missing the next VBlank in the compositor.
            //
            // A more complete solution could work on a sliding window analyzing past repaints
            // and do some prediction for the next repaint.
            let repaint_delay =
                Duration::from_millis(((1_000_000f32 / output_refresh as f32) * 0.6f32) as u64);

            let timer = if self.backend_data.primary_gpu != surface.render_node {
                // However, if we need to do a copy, that might not be enough.
                // (And without actual comparision to previous frames we cannot really know.)
                // So lets ignore that in those cases to avoid thrashing performance.
                trace!("scheduling repaint timer immediately on {:?}", crtc);
                Timer::immediate()
            } else {
                trace!(
                    "scheduling repaint timer with delay {:?} on {:?}",
                    repaint_delay,
                    crtc
                );
                // Timer::from_duration(repaint_delay)
                Timer::immediate()
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
    }

    pub(super) fn render_surface(&mut self, node: DrmNode, crtc: crtc::Handle) {
        profiling::scope!("render_surface", &format!("{crtc:?}"));

        // Tick gamma transitions before rendering
        self.tick_gamma_transitions();

        // Get screenshare sessions before borrowing backend_data
        // let _has_screenshare = !self.screenshare_sessions.is_empty();

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
            o.user_data().get::<UdevOutputId>()
                == Some(&UdevOutputId {
                    device_id: surface.device_id,
                    crtc,
                })
        }) {
            output.clone()
        } else {
            // somehow we got called with an invalid output
            return;
        };

        // let output_scale = output.current_scale().fractional_scale();
        // let integer_scale = output_scale.round() as u32;
        let _config_scale = Config::with(|c| c.screen_scale);

        let scene_has_damage = self.scene_element.update();
        let all_window_elements: Vec<&WindowElement> = self.workspaces.spaces_elements().collect();

        // Determine if direct scanout should be allowed:
        // - Current workspace must be in fullscreen mode and not animating
        // - Disable during expose gesture
        // - Disable during workspace swipe gesture
        let allow_direct_scanout =
            self.workspaces.is_fullscreen_and_stable() && !self.swipe_gesture.is_active();

        // Only fetch the fullscreen window if direct scanout is allowed
        let fullscreen_window = if allow_direct_scanout {
            self.workspaces.get_fullscreen_window()
        } else {
            None
        };

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
            self.scene_element.clone(),
            scene_has_damage,
            fullscreen_window.as_ref(),
        );

        let reschedule = match &result {
            Ok(outcome) => !outcome.rendered,
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

                    tracing::debug!(
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

        if reschedule {
            let output_refresh = match output.current_mode() {
                Some(mode) => mode.refresh,
                None => return,
            };
            // If reschedule is true we either hit a temporary failure or more likely rendering
            // did not cause any damage on the output. In this case we just re-schedule a repaint
            // after approx. one frame to re-test for damage.
            let reschedule_duration =
                Duration::from_millis((1_000_000f32 / output_refresh as f32) as u64);
            trace!(
                "reschedule repaint timer with delay {:?} on {:?}",
                reschedule_duration,
                crtc,
            );
            let timer = Timer::from_duration(reschedule_duration);
            self.handle
                .insert_source(timer, move |_, _, data| {
                    data.render(node, Some(crtc));
                    TimeoutAction::Drop
                })
                .expect("failed to schedule frame timer");
        } else {
            let elapsed = start.elapsed();
            tracing::trace!(?elapsed, "rendered surface");
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
}

#[allow(clippy::too_many_arguments)]
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
    scene_element: SceneElement,
    scene_has_damage: bool,
    fullscreen_window: Option<&WindowElement>,
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

    // Track direct scanout mode transitions
    let is_direct_scanout = fullscreen_window.is_some();
    let mode_changed = is_direct_scanout != surface.was_direct_scanout;
    surface.was_direct_scanout = is_direct_scanout;

    // Reset buffers when transitioning between direct scanout and normal mode
    // This ensures clean state when switching rendering paths
    if mode_changed {
        surface.compositor.reset_buffers();
    }

    // If fullscreen_window is Some, direct scanout is allowed (checked by caller)
    let (output_elements, clear_color, should_draw) =
        if let Some(fullscreen_win) = fullscreen_window {
            // In fullscreen mode: render only the fullscreen window + cursor
            // Skip the scene element entirely for direct scanout
            let mut elements: Vec<OutputRenderElements<'a, _, WindowRenderElement<_>>> = Vec::new();

            // Add pointer elements first (rendered at bottom, but cursor plane may handle separately)
            elements.extend(
                workspace_render_elements
                    .into_iter()
                    .map(OutputRenderElements::from),
            );

            // Add the fullscreen window's render elements wrapped in Wrap
            use smithay::backend::renderer::element::Wrap;
            let window_elements_rendered: Vec<WindowRenderElement<_>> =
                fullscreen_win.render_elements(renderer, (0, 0).into(), scale, 1.0);
            elements.extend(
                window_elements_rendered
                    .into_iter()
                    .map(|e| OutputRenderElements::Window(Wrap::from(e))),
            );

            // Always render in fullscreen mode since the window surface may have damage
            // Use black clear color - the window fills the screen anyway
            (elements, CLEAR_COLOR, true)
        } else {
            // Normal mode: render the full scene
            workspace_render_elements.push(WorkspaceRenderElements::Scene(scene_element));

            // Render if scene has damage, dnd icon needs drawing, or cursor is visible
            let cursor_needs_draw = pointer_in_output;
            let should_draw = scene_has_damage || dnd_needs_draw || cursor_needs_draw;
            if !should_draw {
                return Ok(RenderOutcome::skipped());
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

    let render_frame_result = surface
        .compositor
        .render_frame(
            renderer,
            &output_elements,
            clear_color,
            smithay::backend::drm::compositor::FrameFlags::empty(),
        )
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => err.into(),
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                smithay::backend::renderer::damage::Error::Rendering(err),
            ) => err.into(),
            other => {
                tracing::error!("Unexpected render frame error: {:?}", other);
                SwapBuffersError::ContextLost(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Render frame error: {:?}", other),
                )))
            }
        })?;

    #[cfg(feature = "renderer_sync")]
    {
        use smithay::backend::drm::compositor::PrimaryPlaneElement;
        if let PrimaryPlaneElement::Swapchain(element) = render_frame_result.primary_element {
            let _ = element.sync.wait();
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

    // In direct scanout mode, only send frame callbacks to the fullscreen window
    // This prevents off-workspace windows from generating damage that causes glitches
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
    );

    if rendered {
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
            smithay::backend::drm::compositor::FrameFlags::empty(),
        )
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => err.into(),
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                smithay::backend::renderer::damage::Error::Rendering(err),
            ) => err.into(),
            other => SwapBuffersError::ContextLost(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Render frame error: {:?}", other),
            ))),
        })?;
    surface.compositor.queue_frame(None)?;
    surface.compositor.reset_buffers();

    Ok(())
}
