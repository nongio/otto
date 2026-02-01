use std::{
    cell::RefCell,
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};

use crate::{
    render::*,
    render_elements::workspace_render_elements::WorkspaceRenderElements,
    shell::WindowElement,
    skia_renderer::{SkiaRenderer, SkiaTextureImage},
    state::{post_repaint, take_presentation_feedback, Backend, Otto},
};
#[cfg(feature = "egl")]
use smithay::backend::renderer::ImportEgl;

use smithay::{
    backend::{
        allocator::{
            dmabuf::{Dmabuf, DmabufAllocator},
            gbm::{GbmAllocator, GbmBufferFlags},
            vulkan::{ImageUsageFlags, VulkanAllocator},
        },
        egl::{EGLContext, EGLDisplay},
        renderer::{damage::OutputDamageTracker, Bind, ImportDma, ImportMemWl},
        vulkan::{version::Version, Instance, PhysicalDevice},
        x11::{WindowBuilder, X11Backend, X11Event, X11Surface},
    },
    delegate_dmabuf,
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        ash::ext,
        calloop::EventLoop,
        gbm,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{protocol::wl_surface, Display},
    },
    utils::{DeviceFd, IsAlive, Logical, Physical, Point, Rectangle, Scale},
    wayland::presentation::Refresh,
    wayland::{
        compositor,
        dmabuf::{
            DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
            ImportNotifier,
        },
    },
};
use tracing::{error, info, trace, warn};

#[allow(dead_code)]
#[derive(Debug, Default)]
struct OldGeometry(RefCell<Option<Rectangle<i32, Logical>>>);
impl OldGeometry {
    #[allow(dead_code)]
    pub fn save(&self, geo: Rectangle<i32, Logical>) {
        *self.0.borrow_mut() = Some(geo);
    }
    #[allow(dead_code)]
    pub fn restore(&self) -> Option<Rectangle<i32, Logical>> {
        self.0.borrow_mut().take()
    }
}
#[cfg(feature = "xwayland")]
impl<BackendData: Backend> XWaylandShellHandler for Otto<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

pub const OUTPUT_NAME: &str = "x11";

pub struct X11Data {
    render: bool,
    mode: Mode,
    // FIXME: If GlesRenderer is dropped before X11Surface, then the MakeCurrent call inside Gles2Renderer will
    // fail because the X11Surface is keeping gbm alive.
    renderer: SkiaRenderer,
    damage_tracker: OutputDamageTracker,
    surface: X11Surface,
    dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    _dmabuf_default_feedback: DmabufFeedback,
    #[cfg(feature = "fps_ticker")]
    fps: fps_ticker::Fps,
}

impl DmabufHandler for Otto<X11Data> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self
            .backend_data
            .renderer
            .import_dmabuf(&dmabuf, None)
            .is_ok()
        {
            let _ = notifier.successful::<Otto<X11Data>>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(Otto<X11Data>);

impl Backend for X11Data {
    fn seat_name(&self) -> String {
        "x11".to_owned()
    }
    fn backend_name(&self) -> &'static str {
        "x11"
    }
    fn reset_buffers(&mut self, _output: &Output) {
        self.surface.reset_buffers();
    }
    fn early_import(&mut self, _surface: &wl_surface::WlSurface) {}
    fn set_cursor(&mut self, _image: &CursorImageStatus) {}
    fn texture_for_surface(
        &self,
        _surface: &smithay::backend::renderer::utils::RendererSurfaceState,
    ) -> Option<SkiaTextureImage> {
        None
    }
    fn renderer_context(&mut self) -> Option<layers::skia::gpu::DirectContext> {
        None
    }
    fn request_redraw(&mut self) {
        self.render = true;
    }
}

pub fn run_x11() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut display_handle = display.handle();

    let backend = X11Backend::new().expect("Failed to initilize X11 backend");
    let handle = backend.handle();

    // Obtain the DRM node the X server uses for direct rendering.
    let (node, fd) = handle
        .drm_node()
        .expect("Could not get DRM node used by X server");

    // Create the gbm device for buffer allocation.
    let device = gbm::Device::new(DeviceFd::from(fd)).expect("Failed to create gbm device");
    // Initialize EGL using the GBM device.
    let egl = unsafe { EGLDisplay::new(device.clone()).expect("Failed to create EGLDisplay") };
    // Create the OpenGL context
    let context = EGLContext::new(&egl).expect("Failed to create EGLContext");

    let window = WindowBuilder::new()
        .title("Anvil")
        .build(&handle)
        .expect("Failed to create first window");

    let skip_vulkan = std::env::var("ANVIL_NO_VULKAN")
        .map(|x| {
            x == "1"
                || x.to_lowercase() == "true"
                || x.to_lowercase() == "yes"
                || x.to_lowercase() == "y"
        })
        .unwrap_or(false);

    let vulkan_allocator = if !skip_vulkan {
        Instance::new(Version::VERSION_1_2, None)
            .ok()
            .and_then(|instance| {
                PhysicalDevice::enumerate(&instance)
                    .ok()
                    .and_then(|devices| {
                        devices
                            .filter(|phd| phd.has_device_extension(ext::physical_device_drm::NAME))
                            .find(|phd| {
                                phd.primary_node().unwrap() == Some(node)
                                    || phd.render_node().unwrap() == Some(node)
                            })
                    })
            })
            .and_then(|physical_device| {
                VulkanAllocator::new(
                    &physical_device,
                    ImageUsageFlags::COLOR_ATTACHMENT | ImageUsageFlags::SAMPLED,
                )
                .ok()
            })
    } else {
        None
    };

    let surface = match vulkan_allocator {
        // Create the surface for the window.
        Some(vulkan_allocator) => handle
            .create_surface(
                &window,
                DmabufAllocator(vulkan_allocator),
                context
                    .dmabuf_render_formats()
                    .iter()
                    .map(|format| format.modifier),
            )
            .expect("Failed to create X11 surface"),
        None => handle
            .create_surface(
                &window,
                DmabufAllocator(GbmAllocator::new(device, GbmBufferFlags::RENDERING)),
                context
                    .dmabuf_render_formats()
                    .iter()
                    .map(|format| format.modifier),
            )
            .expect("Failed to create X11 surface"),
    };

    #[cfg_attr(not(feature = "egl"), allow(unused_mut))]
    let mut renderer =
        unsafe { SkiaRenderer::new(context) }.expect("Failed to initialize renderer");

    #[cfg(feature = "egl")]
    if renderer.bind_wl_display(&display.handle()).is_ok() {
        info!("EGL hardware-acceleration enabled");
    }

    let dmabuf_formats = renderer.dmabuf_formats();
    let dmabuf_default_feedback = DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats)
        .build()
        .unwrap();
    let mut dmabuf_state = DmabufState::new();
    let dmabuf_global = dmabuf_state.create_global_with_default_feedback::<Otto<X11Data>>(
        &display.handle(),
        &dmabuf_default_feedback,
    );

    let size = {
        let s = window.size();

        (s.w as i32, s.h as i32).into()
    };

    let mode = Mode {
        size,
        refresh: 60_000,
    };

    #[cfg(feature = "fps_ticker")]
    let fps_image = image::io::Reader::with_format(
        std::io::Cursor::new(FPS_NUMBERS_PNG),
        image::ImageFormat::Png,
    )
    .decode()
    .unwrap();
    #[cfg(feature = "fps_ticker")]
    let fps_texture = renderer
        .import_memory(
            &fps_image.to_rgba8(),
            Fourcc::Abgr8888,
            (fps_image.width() as i32, fps_image.height() as i32).into(),
            false,
        )
        .expect("Unable to upload FPS texture");
    #[cfg(feature = "fps_ticker")]
    let mut fps_element = FpsElement::new(fps_texture);
    let output = Output::new(
        OUTPUT_NAME.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "X11".into(),
            serial_number: None,
        },
    );
    let _global = output.create_global::<Otto<X11Data>>(&display.handle());
    output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
    output.set_preferred(mode);

    let damage_tracker = OutputDamageTracker::from_output(&output);

    let data = X11Data {
        render: true,
        mode,
        surface,
        renderer,
        damage_tracker,
        dmabuf_state,
        _dmabuf_global: dmabuf_global,
        _dmabuf_default_feedback: dmabuf_default_feedback,
        #[cfg(feature = "fps_ticker")]
        fps: fps_ticker::Fps::default(),
    };

    let mut state = Otto::init(display, event_loop.handle(), data, true);
    state
        .shm_state
        .update_formats(state.backend_data.renderer.shm_formats());

    state.workspaces.map_output(&output, (0, 0));

    let output_clone = output.clone();

    event_loop
        .handle()
        .insert_source(backend, move |event, _, data| match event {
            X11Event::CloseRequested { .. } => {
                data.running.store(false, Ordering::SeqCst);
            }
            X11Event::Resized { new_size, .. } => {
                let output = &output_clone;
                let size = { (new_size.w as i32, new_size.h as i32).into() };

                data.backend_data.mode = Mode {
                    size,
                    refresh: 60_000,
                };
                output.delete_mode(output.current_mode().unwrap());
                output.change_current_state(Some(data.backend_data.mode), None, None, None);
                output.set_preferred(data.backend_data.mode);
                crate::shell::fixup_positions(
                    &mut data.workspaces,
                    data.pointer.current_location(),
                );

                data.backend_data.render = true;
            }
            X11Event::PresentCompleted { .. } | X11Event::Refresh { .. } => {
                data.backend_data.render = true;
            }
            X11Event::Input { event, .. } => data.process_input_event_windowed(event, OUTPUT_NAME),
            X11Event::Focus { focused: false, .. } => {
                data.release_all_keys();
            }
            _ => {}
        })
        .expect("Failed to insert X11 Backend into event loop");

    #[cfg(feature = "xwayland")]
    state.start_xwayland();

    info!("Initialization completed, starting the main loop.");

    // Removed unused PointerElement - cursor now rendered directly using CursorManager

    while state.running.load(Ordering::SeqCst) {
        if state.backend_data.render {
            profiling::scope!("render_frame");

            let backend_data = &mut state.backend_data;
            // We need to borrow everything we want to refer to inside the renderer callback otherwise rustc is unhappy.
            let cursor_status = &state.cursor_status;
            #[cfg(feature = "fps_ticker")]
            let fps = backend_data.fps.avg().round() as u32;
            #[cfg(feature = "fps_ticker")]
            fps_element.update_fps(fps);

            let (buffer, age) = backend_data
                .surface
                .buffer()
                .expect("gbm device was destroyed");
            if let Err(err) = backend_data.renderer.bind(buffer) {
                error!("Error while binding buffer: {}", err);
                profiling::finish_frame!();
                continue;
            }

            #[cfg(feature = "debug")]
            if let Some(renderdoc) = state.renderdoc.as_mut() {
                renderdoc.start_frame_capture(
                    backend_data.renderer.egl_context().get_context_handle(),
                    std::ptr::null(),
                );
            }

            let mut cursor_guard = cursor_status.lock().unwrap();
            let elements: Vec<WorkspaceRenderElements<'_, SkiaRenderer>> = Vec::new();

            // draw the cursor as relevant
            // reset the cursor if the surface is no longer alive
            let mut reset = false;
            if let CursorImageStatus::Surface(ref surface) = *cursor_guard {
                reset = !surface.alive();
            }
            if reset {
                *cursor_guard = CursorImageStatus::default_named();
            }
            let cursor_visible = !matches!(*cursor_guard, CursorImageStatus::Surface(_));

            let scale = Scale::from(output.current_scale().fractional_scale());
            let cursor_hotspot = if let CursorImageStatus::Surface(ref surface) = *cursor_guard {
                compositor::with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .hotspot
                })
            } else {
                (0, 0).into()
            };
            let cursor_pos = state.pointer.current_location() - cursor_hotspot.to_f64();
            let _cursor_pos_scaled: Point<i32, Physical> =
                cursor_pos.to_physical(scale).to_i32_round();

            // Cursor rendering removed - to be implemented similar to winit/udev using CursorManager
            // elements.extend(pointer_element.render_elements(...));

            // draw the dnd icon if any
            // if let Some(surface) = state.dnd_icon.as_ref() {
            //     if surface.alive() {
            //         elements.extend(AsRenderElements::<SkiaRenderer>::render_elements(
            //             &smithay::desktop::space::SurfaceTree::from_surface(surface),
            //             &mut backend_data.renderer,
            //             cursor_pos_scaled,
            //             scale,
            //             1.0,
            //         ));
            //     }
            // }

            #[cfg(feature = "fps_ticker")]
            elements.push(WorkspaceRenderElements::Fps(fps_element.clone()));

            let all_window_elements: Vec<&WindowElement> =
                state.workspaces.spaces_elements().collect();
            let render_res = render_output(
                &output,
                &all_window_elements,
                elements,
                state.dnd_icon.as_ref(),
                &mut backend_data.renderer,
                &mut backend_data.damage_tracker,
                age.into(),
            );

            match render_res {
                Ok(render_output_result) => {
                    trace!("Finished rendering");
                    let submitted = if let Err(err) = backend_data.surface.submit() {
                        backend_data.surface.reset_buffers();
                        warn!("Failed to submit buffer: {}. Retrying", err);
                        false
                    } else {
                        true
                    };

                    // Send frame events so that client start drawing their next frame
                    let time = state.clock.now();
                    let all_window_elements: Vec<&WindowElement> =
                        state.workspaces.spaces_elements().collect();
                    post_repaint(
                        &output,
                        &render_output_result.states,
                        &all_window_elements,
                        None,
                        time,
                    );

                    if render_output_result.damage.is_some() {
                        let all_window_elements: Vec<&WindowElement> =
                            state.workspaces.spaces_elements().collect();
                        let mut output_presentation_feedback = take_presentation_feedback(
                            &output,
                            &all_window_elements,
                            &render_output_result.states,
                        );
                        output_presentation_feedback.presented(
                            time,
                            output
                                .current_mode()
                                .map(|mode| {
                                    Refresh::fixed(Duration::from_nanos(
                                        1_000_000_000_000 / mode.refresh as u64,
                                    ))
                                })
                                .unwrap_or(Refresh::Unknown),
                            0,
                            wp_presentation_feedback::Kind::Vsync,
                        )
                    }

                    #[cfg(feature = "debug")]
                    if render_output_result.damage.is_some() {
                        if let Some(renderdoc) = state.renderdoc.as_mut() {
                            renderdoc.end_frame_capture(
                                state
                                    .backend_data
                                    .renderer
                                    .egl_context()
                                    .get_context_handle(),
                                std::ptr::null(),
                            );
                        }
                    } else if let Some(renderdoc) = state.renderdoc.as_mut() {
                        renderdoc.discard_frame_capture(
                            state
                                .backend_data
                                .renderer
                                .egl_context()
                                .get_context_handle(),
                            std::ptr::null(),
                        );
                    }

                    state.backend_data.render = !submitted;
                }
                Err(err) => {
                    #[cfg(feature = "debug")]
                    if let Some(renderdoc) = state.renderdoc.as_mut() {
                        renderdoc.discard_frame_capture(
                            backend_data.renderer.egl_context().get_context_handle(),
                            std::ptr::null(),
                        );
                    }

                    backend_data.surface.reset_buffers();
                    error!("Rendering error: {}", err);
                    // TODO: convert RenderError into SwapBuffersError and skip temporary (will retry) and panic on ContextLost or recreate
                }
            }

            #[cfg(feature = "fps_ticker")]
            state.backend_data.fps.tick();
            window.set_cursor_visible(cursor_visible);
            profiling::finish_frame!();
        }

        let result = event_loop.dispatch(Some(Duration::from_millis(16)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
        } else {
            state.workspaces.refresh_space();
            state.popups.cleanup();
            display_handle.flush_clients().unwrap();
        }
    }
}
