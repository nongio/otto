//! Capture through the standard protocol: `ext-image-capture-source-v1`
//! (sources from outputs and toplevels) and `ext-image-copy-capture-v1`
//! (sessions and frames). See `specs/security-model.md`.
//!
//! Who gets it: the clients that get the privileged interfaces, and agents'
//! connections. For an agent, an output is its workspace: a capture of the
//! output its workspace is on shows that workspace, shown or not, and never
//! the bars, the dialogs or the user's cursor. A toplevel source needs a
//! handle, and an agent's connection is told only of its own windows
//! (`ToplevelOwner`), so it captures those alone; a session on anything
//! else is stopped.
//!
//! How a frame is filled: an output capture for the user's own programs
//! rides the render loop, like wlr-screencopy (`super::screencopy`), so it
//! shows exactly what the screen does. An agent's workspace, and any
//! toplevel, are drawn off screen from their layer trees
//! (`super::workspace_capture`) and copied into the client's buffer at once.
//! Buffers are wl_shm, ARGB8888 or XRGB8888.

use smithay::{
    output::WeakOutput,
    reexports::{
        wayland_server::Client,
        wayland_server::{protocol::wl_shm, DisplayHandle},
    },
    utils::Size,
    wayland::{
        foreign_toplevel_list::ForeignToplevelWeakHandle,
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, ImageCaptureSourceState,
            OutputCaptureSourceHandler, OutputCaptureSourceState, ToplevelCaptureSourceHandler,
            ToplevelCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, Frame, ImageCopyCaptureHandler,
            ImageCopyCaptureState, Session, SessionRef,
        },
        shm,
    },
};

use super::{
    foreign_toplevel_shared::ToplevelOwner,
    screencopy::{CaptureBuffer, CaptureFrame, PendingScreencopy},
    Backend, ClientState, Otto,
};

/// The capture globals and the sessions open on them.
pub struct ImageCaptureStates {
    pub sources: ImageCaptureSourceState,
    pub outputs: OutputCaptureSourceState,
    pub toplevels: ToplevelCaptureSourceState,
    pub copy: ImageCopyCaptureState,
    /// Open sessions; dropping one tells its client it stopped.
    pub sessions: Vec<Session>,
}

impl ImageCaptureStates {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: OutputCaptureSourceHandler + ToplevelCaptureSourceHandler + ImageCopyCaptureHandler,
    {
        let offered = |client: &Client| {
            crate::sandbox::is_privileged_client(client)
                || ClientState::agent_seat_of(client).is_some()
        };
        Self {
            sources: ImageCaptureSourceState::new(),
            outputs: OutputCaptureSourceState::new_with_filter::<D, _>(display, offered),
            toplevels: ToplevelCaptureSourceState::new_with_filter::<D, _>(display, offered),
            copy: ImageCopyCaptureState::new_with_filter::<D, _>(display, offered),
            sessions: Vec::new(),
        }
    }
}

/// What a source stands for.
enum Target {
    Output(smithay::output::Output),
    Toplevel(smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle),
}

fn target_of(source: &ImageCaptureSource) -> Option<Target> {
    if let Some(output) = source.user_data().get::<WeakOutput>() {
        return output.upgrade().map(Target::Output);
    }
    source
        .user_data()
        .get::<ForeignToplevelWeakHandle>()?
        .upgrade()
        .map(Target::Toplevel)
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// The window behind an ext toplevel handle.
    fn window_of_handle(
        &self,
        handle: &smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle,
    ) -> Option<(
        smithay::reexports::wayland_server::backend::ObjectId,
        crate::shell::WindowElement,
    )> {
        let (id, _) = self
            .foreign_toplevels
            .iter()
            .find(|(_, handles)| handles.ext.as_ref().is_some_and(|ext| ext.matches(handle)))?;
        let window = self.workspaces.get_window_for_surface(id)?.clone();
        Some((id.clone(), window))
    }

    /// A toplevel's size in pixels: its geometry at its output's scale.
    fn toplevel_pixel_size(&self, window: &crate::shell::WindowElement) -> Option<(i32, i32)> {
        let geometry = self.workspaces.element_geometry(window)?;
        let scale = self
            .workspaces
            .output_for_window(window)
            .map(|output| output.current_scale().fractional_scale())
            .unwrap_or(1.0);
        let width = (geometry.size.w as f64 * scale).round() as i32;
        let height = (geometry.size.h as f64 * scale).round() as i32;
        (width > 0 && height > 0).then_some((width, height))
    }

    /// Draw `surface` into the frame's wl_shm buffer, which must hold
    /// `width` by `height` ARGB8888 pixels.
    fn fill_frame_from_surface(
        &self,
        frame: Frame,
        mut surface: layers::skia::Surface,
        width: i32,
        height: i32,
    ) {
        let buffer = frame.buffer();
        let stride = width as usize * 4;
        let written = shm::with_buffer_contents(&buffer, |ptr, len, data| {
            let right_format = matches!(
                data.format,
                wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888
            );
            if !right_format
                || data.width != width
                || data.height != height
                || data.stride as usize != stride
                || len < stride * height as usize
            {
                return false;
            }
            let info = layers::skia::ImageInfo::new(
                (width, height),
                layers::skia::ColorType::BGRA8888,
                layers::skia::AlphaType::Premul,
                None,
            );
            // SAFETY: the buffer is at least `stride * height` bytes, as
            // checked above, and mapped for as long as this closure runs.
            let dst =
                unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, stride * height as usize) };
            surface.read_pixels(&info, dst, stride, (0, 0))
        });
        if matches!(written, Ok(true)) {
            CaptureFrame::Ext(frame).finish(true);
        } else {
            frame.fail(CaptureFailureReason::BufferConstraints);
        }
    }
}

impl<BackendData: Backend + 'static> ImageCaptureSourceHandler for Otto<BackendData> {}

impl<BackendData: Backend + 'static> OutputCaptureSourceHandler for Otto<BackendData> {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.image_capture.outputs
    }

    fn output_source_created(
        &mut self,
        source: ImageCaptureSource,
        output: &smithay::output::Output,
    ) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl<BackendData: Backend + 'static> ToplevelCaptureSourceHandler for Otto<BackendData> {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.image_capture.toplevels
    }

    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle,
    ) {
        source
            .user_data()
            .insert_if_missing(|| toplevel.downgrade());
    }
}

impl<BackendData: Backend + 'static> ImageCopyCaptureHandler for Otto<BackendData> {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_capture.copy
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let (width, height) = match target_of(source)? {
            Target::Output(output) => {
                let mode = output.current_mode()?;
                (mode.size.w, mode.size.h)
            }
            Target::Toplevel(handle) => {
                let (_, window) = self.window_of_handle(&handle)?;
                self.toplevel_pixel_size(&window)?
            }
        };
        Some(BufferConstraints {
            size: Size::from((width, height)),
            shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888],
            dma: None,
        })
    }

    fn new_session(&mut self, session: Session) {
        // An agent's connection captures its workspace, as the output, and
        // its own windows; anything else is stopped at once.
        let agent = session
            .client()
            .and_then(|client| ClientState::agent_seat_of(&client).map(str::to_string));
        if let Some(seat) = agent {
            let allowed = match target_of(&session.source()) {
                Some(Target::Output(_)) => self.workspace_of_seat(&seat).is_ok(),
                Some(Target::Toplevel(handle)) => {
                    handle
                        .user_data()
                        .get::<ToplevelOwner>()
                        .and_then(|owner| owner.seat())
                        .as_deref()
                        == Some(seat.as_str())
                }
                None => false,
            };
            if !allowed {
                tracing::warn!(seat, "capture session refused: outside the agent's scope");
                session.stop();
                return;
            }
        }
        self.image_capture.sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        if self.is_session_locked() {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        let agent = session
            .client()
            .and_then(|client| ClientState::agent_seat_of(&client).map(str::to_string));
        let Some(target) = target_of(&session.source()) else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        match (target, agent) {
            // An agent's output is its workspace.
            (Target::Output(output), Some(seat)) => {
                let (roots, width, height) = match self.workspace_of_seat(&seat) {
                    Ok(target) if target.output == output.name() => {
                        let Some(mode) = output.current_mode() else {
                            frame.fail(CaptureFailureReason::Unknown);
                            return;
                        };
                        (
                            [target.view.wallpaper_group.id, target.view.windows_layer.id],
                            mode.size.w,
                            mode.size.h,
                        )
                    }
                    _ => {
                        frame.fail(CaptureFailureReason::Stopped);
                        return;
                    }
                };
                match self.render_layers(&roots, width, height) {
                    Ok(surface) => self.fill_frame_from_surface(frame, surface, width, height),
                    Err(err) => {
                        tracing::warn!(%err, "cannot draw the agent's workspace");
                        frame.fail(CaptureFailureReason::Unknown);
                    }
                }
            }
            // The user's own programs see the screen as it is drawn.
            (Target::Output(output), None) => {
                let Some(mode) = output.current_mode() else {
                    frame.fail(CaptureFailureReason::Unknown);
                    return;
                };
                let buffer = frame.buffer();
                let capture_buffer = match smithay::wayland::dmabuf::get_dmabuf(&buffer) {
                    Ok(dmabuf) => CaptureBuffer::Dmabuf(dmabuf.clone()),
                    Err(_) => CaptureBuffer::Shm(buffer),
                };
                let (width, height) = (mode.size.w as u32, mode.size.h as u32);
                self.pending_screencopy_frames.push(PendingScreencopy {
                    frame: CaptureFrame::Ext(frame),
                    buffer: capture_buffer,
                    output,
                    region: None,
                    width,
                    height,
                    stride: width * 4,
                    overlay_cursor: false,
                });
                self.backend_data.request_redraw();
            }
            (Target::Toplevel(handle), agent) => {
                let Some((id, window)) = self.window_of_handle(&handle) else {
                    frame.fail(CaptureFailureReason::Stopped);
                    return;
                };
                if let Some(seat) = agent {
                    if !self.agent_scope_window_ids(&seat).contains(&id) {
                        frame.fail(CaptureFailureReason::Stopped);
                        return;
                    }
                }
                let Some((width, height)) = self.toplevel_pixel_size(&window) else {
                    frame.fail(CaptureFailureReason::Unknown);
                    return;
                };
                let roots = [window.layer().id];
                match self.render_layers(&roots, width, height) {
                    Ok(surface) => self.fill_frame_from_surface(frame, surface, width, height),
                    Err(err) => {
                        tracing::warn!(%err, "cannot draw the window");
                        frame.fail(CaptureFailureReason::Unknown);
                    }
                }
            }
        }
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.image_capture
            .sessions
            .retain(|open| open.as_ref() != session);
    }
}
