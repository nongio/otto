use std::sync::Mutex;

use smithay::{
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
    utils::Rectangle,
    wayland::shm,
};

use crate::state::{Backend, Otto};

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
        let global = display.create_global::<Otto<BackendData>, ZwlrScreencopyManagerV1, ()>(2, ());
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

/// A pending screencopy frame waiting to be filled during the render loop.
pub struct PendingScreencopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
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
    let Some(client) = wl_output.client() else {
        return None;
    };
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

                state.pending_screencopy_frames.push(PendingScreencopy {
                    frame: resource.clone(),
                    buffer,
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
        _state: &mut Otto<BackendData>,
        _client: ClientId,
        _resource: &ZwlrScreencopyFrameV1,
        _data: &ScreencopyFrameData,
    ) {
    }
}

/// Called from the render loop after the output has been rendered to
/// a Skia surface. Reads pixels from the Skia surface into pending
/// screencopy shm buffers for this output.
pub fn complete_screencopy_for_output(
    pending: &mut Vec<PendingScreencopy>,
    output: &Output,
    skia_surface: &mut layers::skia::Surface,
) {
    let indices: Vec<usize> = pending
        .iter()
        .enumerate()
        .filter(|(_, p)| p.output == *output)
        .map(|(i, _)| i)
        .collect();

    for i in indices.into_iter().rev() {
        let p = pending.remove(i);
        let result = shm::with_buffer_contents(&p.buffer, |ptr, len, buf_data| {
            if buf_data.format != wl_shm::Format::Argb8888 {
                return false;
            }
            let expected = p.stride as usize * p.height as usize;
            if len < expected {
                return false;
            }

            let x_off = p
                .region
                .map(|r| {
                    let scale = output.current_scale().fractional_scale();
                    (r.loc.x as f64 * scale) as i32
                })
                .unwrap_or(0);
            let y_off = p
                .region
                .map(|r| {
                    let scale = output.current_scale().fractional_scale();
                    (r.loc.y as f64 * scale) as i32
                })
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

        match result {
            Ok(true) => {
                p.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                p.frame.ready(
                    (now.as_secs() >> 32) as u32,
                    now.as_secs() as u32,
                    now.subsec_nanos(),
                );
            }
            _ => {
                p.frame.failed();
            }
        }
    }
}
