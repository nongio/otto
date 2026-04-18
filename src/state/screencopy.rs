use std::sync::Mutex;

use smithay::{
    backend::allocator::{dmabuf::Dmabuf, Fourcc},
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            backend::{ClientId, GlobalId},
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Physical, Rectangle, Size},
    wayland::{dmabuf::get_dmabuf, shm},
};

use crate::{
    renderer::BlitCurrentFrame,
    state::{Backend, Otto},
    udev::UdevRenderer,
};

#[derive(Debug)]
pub struct ScreencopyManagerState {
    #[allow(dead_code)]
    global: GlobalId,
}

impl ScreencopyManagerState {
    pub fn new<BackendData>(display: &DisplayHandle) -> Self
    where
        BackendData: Backend + 'static,
        Otto<BackendData>: GlobalDispatch<ZwlrScreencopyManagerV1, ()>,
        Otto<BackendData>: Dispatch<ZwlrScreencopyManagerV1, ()>,
        Otto<BackendData>: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData>,
    {
        // v3 advertises linux_dmabuf so capable clients can negotiate a GPU
        // dmabuf and receive frames via the screenshare blit path (zero CPU
        // copy). v1/v2 clients fall back to the SHM read_pixels path.
        let global = display.create_global::<Otto<BackendData>, ZwlrScreencopyManagerV1, ()>(3, ());
        Self { global }
    }
}

#[derive(Debug)]
pub struct ScreencopyFrameData {
    pub output: Output,
    pub overlay_cursor: bool,
    pub region: Option<Rectangle<i32, smithay::utils::Logical>>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    #[allow(dead_code)]
    state: Mutex<FrameState>,
}

#[derive(Debug)]
enum FrameState {
    AwaitingCopy,
    Copying,
}

/// What the client gave us as the destination buffer.
///
/// `Dmabuf` clients ride the same GPU blit path as PipeWire screenshare —
/// zero CPU copy. `Shm` clients fall back to the synchronous `read_pixels`
/// path; this is the legacy slow path kept only for compatibility with
/// `grim` and other v1/v2 tools.
pub enum CaptureBuffer {
    Shm(WlBuffer),
    Dmabuf(Dmabuf),
}

/// A pending screencopy frame waiting to be filled during the render loop.
pub struct PendingScreencopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: CaptureBuffer,
    pub output: Output,
    pub region: Option<Rectangle<i32, smithay::utils::Logical>>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

fn find_output_for_wl<BackendData: Backend>(
    state: &Otto<BackendData>,
    wl_output: &WlOutput,
) -> Option<Output> {
    let client = wl_output.client()?;
    state
        .workspaces
        .outputs()
        .find(|o| o.client_outputs(&client).any(|co| co == *wl_output))
        .cloned()
}

impl<BackendData> GlobalDispatch<ZwlrScreencopyManagerV1, (), Otto<BackendData>>
    for ScreencopyManagerState
where
    BackendData: Backend + 'static,
    Otto<BackendData>: Dispatch<ZwlrScreencopyManagerV1, ()>,
    Otto<BackendData>: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData>,
{
    fn bind(
        _state: &mut Otto<BackendData>,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData> Dispatch<ZwlrScreencopyManagerV1, (), Otto<BackendData>>
    for ScreencopyManagerState
where
    BackendData: Backend + 'static,
    Otto<BackendData>: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData>,
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output: wl_output,
            } => {
                let Some(output) = find_output_for_wl(state, &wl_output) else {
                    return;
                };
                init_frame(state, data_init, frame, output, overlay_cursor != 0, None);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output: wl_output,
                x,
                y,
                width,
                height,
            } => {
                let Some(output) = find_output_for_wl(state, &wl_output) else {
                    return;
                };
                let region = Rectangle::new((x, y).into(), (width, height).into());
                init_frame(
                    state,
                    data_init,
                    frame,
                    output,
                    overlay_cursor != 0,
                    Some(region),
                );
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

fn init_frame<BackendData: Backend + 'static>(
    _state: &mut Otto<BackendData>,
    data_init: &mut DataInit<'_, Otto<BackendData>>,
    frame_new: New<ZwlrScreencopyFrameV1>,
    output: Output,
    overlay_cursor: bool,
    region: Option<Rectangle<i32, smithay::utils::Logical>>,
) where
    Otto<BackendData>: Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData>,
{
    let mode = output.current_mode().unwrap();
    let scale = output.current_scale().fractional_scale();
    let (width, height) = if let Some(region) = region {
        (
            (region.size.w as f64 * scale) as u32,
            (region.size.h as f64 * scale) as u32,
        )
    } else {
        (mode.size.w as u32, mode.size.h as u32)
    };
    let stride = width * 4;

    let frame_data = ScreencopyFrameData {
        output,
        overlay_cursor,
        region,
        width,
        height,
        stride,
        state: Mutex::new(FrameState::AwaitingCopy),
    };
    let frame = data_init.init(frame_new, frame_data);

    frame.buffer(wl_shm::Format::Argb8888, width, height, stride);
    if frame.version() >= 3 {
        // Advertise dmabuf so capable clients (PipeWire portal, OBS,
        // wf-recorder) can hand us a GPU buffer and skip the SHM readback.
        frame.linux_dmabuf(Fourcc::Argb8888 as u32, width, height);
        frame.buffer_done();
    }
}

impl<BackendData> Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameData, Otto<BackendData>>
    for ScreencopyManagerState
where
    BackendData: Backend + 'static,
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        resource: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrameData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer }
            | zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                let mut frame_state = data.state.lock().unwrap();
                if !matches!(*frame_state, FrameState::AwaitingCopy) {
                    resource.failed();
                    return;
                }
                *frame_state = FrameState::Copying;
                drop(frame_state);

                // Pick the GPU dmabuf path when the client gave us a
                // dmabuf-backed buffer; otherwise fall back to legacy SHM.
                let capture_buffer = match get_dmabuf(&buffer) {
                    Ok(dmabuf) => CaptureBuffer::Dmabuf(dmabuf.clone()),
                    Err(_) => CaptureBuffer::Shm(buffer),
                };

                state.pending_screencopy_frames.push(PendingScreencopy {
                    frame: resource.clone(),
                    buffer: capture_buffer,
                    output: data.output.clone(),
                    region: data.region,
                    width: data.width,
                    height: data.height,
                    stride: data.stride,
                });
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Otto<BackendData>,
        _client: ClientId,
        resource: &ZwlrScreencopyFrameV1,
        _data: &ScreencopyFrameData,
    ) {
        // Drop any pending entries belonging to this destroyed frame so the
        // render loop doesn't keep doing GPU readback for a dead resource.
        state
            .pending_screencopy_frames
            .retain(|p| p.frame != *resource);
    }
}

/// Called from the render loop after the output has been rendered.
/// Dmabuf clients ride the screenshare GPU blit path (zero CPU copy);
/// SHM clients fall back to the legacy synchronous read_pixels path.
pub fn complete_screencopy_for_output(
    pending: &mut Vec<PendingScreencopy>,
    output: &Output,
    renderer: &mut UdevRenderer<'_>,
) {
    let indices: Vec<usize> = pending
        .iter()
        .enumerate()
        .filter(|(_, p)| p.output == *output)
        .map(|(i, _)| i)
        .collect();

    if indices.is_empty() {
        return;
    }

    for i in indices.into_iter().rev() {
        let p = pending.remove(i);
        let success = match &p.buffer {
            CaptureBuffer::Dmabuf(dmabuf) => copy_to_dmabuf(renderer, &p, dmabuf, output),
            CaptureBuffer::Shm(buffer) => copy_to_shm(renderer, &p, buffer, output),
        };

        if success {
            p.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            p.frame.ready(
                (now.as_secs() >> 32) as u32,
                now.as_secs() as u32,
                now.subsec_nanos(),
            );
        } else {
            p.frame.failed();
        }
    }
}

/// Compute the (source on framebuffer, destination on output buffer)
/// rectangles for a capture, in physical pixels.
fn capture_rects(
    p: &PendingScreencopy,
    output: &Output,
) -> (Rectangle<i32, Physical>, Rectangle<i32, Physical>) {
    let dst_size: Size<i32, Physical> = (p.width as i32, p.height as i32).into();
    let dst = Rectangle::from_size(dst_size);
    let src = if let Some(region) = p.region {
        let scale = output.current_scale().fractional_scale();
        Rectangle::new(
            (
                (region.loc.x as f64 * scale) as i32,
                (region.loc.y as f64 * scale) as i32,
            )
                .into(),
            dst_size,
        )
    } else {
        dst
    };
    (src, dst)
}

fn copy_to_dmabuf(
    renderer: &mut UdevRenderer<'_>,
    p: &PendingScreencopy,
    dmabuf: &Dmabuf,
    output: &Output,
) -> bool {
    let (src, dst) = capture_rects(p, output);
    let mut dmabuf = dmabuf.clone();
    match renderer.blit_current_frame(&mut dmabuf, src, dst) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(?err, "screencopy dmabuf blit failed");
            false
        }
    }
}

fn copy_to_shm(
    renderer: &mut UdevRenderer<'_>,
    p: &PendingScreencopy,
    buffer: &WlBuffer,
    output: &Output,
) -> bool {
    let Some(skia_renderer) = renderer.as_mut().current_skia_renderer() else {
        return false;
    };
    let mut skia_surface = skia_renderer.surface.clone();
    let scale = output.current_scale().fractional_scale();

    let result = shm::with_buffer_contents(buffer, |ptr, len, buf_data| {
        if buf_data.format != wl_shm::Format::Argb8888 {
            return false;
        }
        let expected = p.stride as usize * p.height as usize;
        if len < expected {
            return false;
        }

        let x_off = p
            .region
            .map(|r| (r.loc.x as f64 * scale) as i32)
            .unwrap_or(0);
        let y_off = p
            .region
            .map(|r| (r.loc.y as f64 * scale) as i32)
            .unwrap_or(0);

        let info = layers::skia::ImageInfo::new(
            (p.width as i32, p.height as i32),
            layers::skia::ColorType::BGRA8888,
            layers::skia::AlphaType::Premul,
            None,
        );

        let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, expected) };

        skia_surface.read_pixels(&info, dst, p.stride as usize, (x_off, y_off))
    });

    matches!(result, Ok(true))
}
