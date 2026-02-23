// Initialization logic for udev backend
//
// Handles session setup, GPU initialization, libinput configuration,
// and the main event loop for the udev backend.

use std::{collections::HashMap, sync::atomic::Ordering, time::Duration};

use smithay::{
    backend::{
        drm::{DrmNode, NodeType},
        egl::context::ContextPriority,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            multigpu::{gbm::GbmGlesBackend, GpuManager},
            ImportDma, ImportMemWl, Renderer,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    reexports::{calloop::EventLoop, input::Libinput, wayland_server::Display},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufState},
};
use tracing::{error, info, warn};

use crate::{
    config::Config,
    state::{Backend, Otto},
};

use super::{
    feedback::get_surface_dmabuf_feedback,
    types::{DeviceAddError, UdevData},
};

/// Configures all libinput devices based on Otto's configuration
fn configure_libinput_devices(libinput: &mut Libinput, config: &Config) {
    use smithay::reexports::input::{
        event::{DeviceEvent, EventTrait},
        Event,
    };

    // Process initial devices
    libinput.dispatch().ok();

    for event in libinput.by_ref() {
        if let Event::Device(DeviceEvent::Added(added_event)) = event {
            let mut device = added_event.device();
            apply_device_config(&mut device, config);
        }
    }
}

/// Applies configuration to an individual input device
fn apply_device_config(device: &mut smithay::reexports::input::Device, config: &Config) {
    // Only configure pointer devices (touchpads)
    if !device.has_capability(smithay::reexports::input::DeviceCapability::Pointer) {
        return;
    }

    // Check if it's a touchpad
    if device.config_tap_finger_count() > 0 {
        // Configure tap-to-click
        if device
            .config_tap_set_enabled(config.input.tap_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.tap_enabled,
                "Set tap-to-click"
            );
        }

        // Configure tap-and-drag
        if device
            .config_tap_set_drag_enabled(config.input.tap_drag_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.tap_drag_enabled,
                "Set tap-and-drag"
            );
        }

        // Configure tap drag lock
        if device
            .config_tap_set_drag_lock_enabled(config.input.tap_drag_lock_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.tap_drag_lock_enabled,
                "Set tap drag lock"
            );
        }

        // Configure click method
        use crate::config::TouchpadClickMethod;
        use smithay::reexports::input::ClickMethod;

        let click_method = match config.input.touchpad_click_method {
            TouchpadClickMethod::Clickfinger => ClickMethod::Clickfinger,
            TouchpadClickMethod::ButtonAreas => ClickMethod::ButtonAreas,
        };

        if device.config_click_set_method(click_method).is_ok() {
            tracing::debug!(
                device = device.name(),
                method = ?config.input.touchpad_click_method,
                "Set click method"
            );
        }

        // Configure disable-while-typing
        if device
            .config_dwt_set_enabled(config.input.touchpad_dwt_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.touchpad_dwt_enabled,
                "Set disable-while-typing"
            );
        }

        // Configure natural scrolling for touchpad
        if device
            .config_scroll_set_natural_scroll_enabled(config.input.touchpad_natural_scroll_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.touchpad_natural_scroll_enabled,
                "Set natural scroll"
            );
        }

        // Configure left-handed mode
        if device
            .config_left_handed_set(config.input.touchpad_left_handed)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.touchpad_left_handed,
                "Set left-handed mode"
            );
        }

        // Configure middle button emulation
        if device
            .config_middle_emulation_set_enabled(config.input.touchpad_middle_emulation_enabled)
            .is_ok()
        {
            tracing::debug!(
                device = device.name(),
                enabled = config.input.touchpad_middle_emulation_enabled,
                "Set middle button emulation"
            );
        }

        info!(
            device = device.name(),
            "Configured touchpad with tap={}, drag={}, natural_scroll={}",
            config.input.tap_enabled,
            config.input.tap_drag_enabled,
            config.input.touchpad_natural_scroll_enabled
        );
    }
}

/// Main entry point for the udev backend
///
/// Initializes the session, GPU, input devices, and runs the main event loop.
pub fn run_udev() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let mut display_handle = display.handle();

    /*
     * Initialize session
     */
    let (session, notifier) = match LibSeatSession::new() {
        Ok(ret) => ret,
        Err(err) => {
            error!("Could not initialize a session: {}", err);
            return;
        }
    };

    /*
     * Initialize the compositor
     */
    let primary_gpu = if let Ok(var) = std::env::var("ANVIL_DRM_DEVICE") {
        DrmNode::from_path(var).expect("Invalid drm device path")
    } else {
        primary_gpu(session.seat())
            .unwrap()
            .and_then(|x| {
                DrmNode::from_path(x)
                    .ok()?
                    .node_with_type(NodeType::Render)?
                    .ok()
            })
            .unwrap_or_else(|| {
                all_gpus(session.seat())
                    .unwrap()
                    .into_iter()
                    .find_map(|x| DrmNode::from_path(x).ok())
                    .expect("No GPU!")
            })
    };
    info!("Using {} as primary gpu.", primary_gpu);

    let gpus =
        GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High)).unwrap();

    // // Context ID will be obtained after devices are initialized
    let data = UdevData {
        dh: display_handle.clone(),
        dmabuf_state: None,
        session,
        primary_gpu,
        gpus,
        backends: HashMap::new(),
        #[cfg(feature = "fps_ticker")]
        fps_texture: None,

        context_id: None, // Will be set after device initialization
    };
    let mut state = Otto::init(display, event_loop.handle(), data, true);

    /*
     * Initialize the udev backend
     */
    let udev_backend = match UdevBackend::new(&state.seat_name) {
        Ok(ret) => ret,
        Err(err) => {
            error!(error = ?err, "Failed to initialize udev backend");
            return;
        }
    };

    /*
     * Initialize libinput backend
     */
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        state.backend_data.session.clone().into(),
    );
    libinput_context.udev_assign_seat(&state.seat_name).unwrap();

    // Configure input devices based on config
    Config::with(|config| {
        configure_libinput_devices(&mut libinput_context, config);
    });

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    /*
     * Bind all our objects that get driven by the event loop
     */
    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, data| {
            let dh = data.backend_data.dh.clone();
            data.process_input_event(&dh, event)
        })
        .unwrap();

    let handle = event_loop.handle();
    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), data| match event {
            SessionEvent::PauseSession => {
                libinput_context.suspend();
                info!("pausing session");

                for backend in data.backend_data.backends.values_mut() {
                    backend.drm.pause();
                    backend.active_leases.clear();
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.suspend();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                info!("resuming session");

                if let Err(err) = libinput_context.resume() {
                    error!("Failed to resume libinput context: {:?}", err);
                }
                for (node, backend) in data
                    .backend_data
                    .backends
                    .iter_mut()
                    .map(|(handle, backend)| (*handle, backend))
                {
                    let _ = backend.drm.activate(false);
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.resume::<Otto<UdevData>>();
                    }
                    for surface in backend.surfaces.values_mut() {
                        if let Err(err) = surface.compositor.surface().reset_state() {
                            warn!("Failed to reset drm surface state: {}", err);
                        }
                        // reset the buffers after resume to trigger a full redraw
                        // this is important after a vt switch as the primary plane
                        // has no content and damage tracking may prevent a redraw
                        // otherwise
                        surface.compositor.reset_buffers();
                    }
                    handle.insert_idle(move |data| data.render(node, None));
                }
            }
        })
        .unwrap();

    for (device_id, path) in udev_backend.device_list() {
        if let Err(err) = DrmNode::from_dev_id(device_id)
            .map_err(DeviceAddError::DrmNode)
            .and_then(|node| state.device_added(node, path))
        {
            error!("Skipping device {device_id}: {err}");
        }
    }

    // Now that devices are added, set the context_id
    if let Ok(renderer) = state.backend_data.gpus.single_renderer(&primary_gpu) {
        state.backend_data.context_id = Some(renderer.context_id());
    }

    state.shm_state.update_formats(
        state
            .backend_data
            .gpus
            .single_renderer(&primary_gpu)
            .unwrap()
            .shm_formats(),
    );

    #[cfg_attr(not(feature = "egl"), allow(unused_mut))]
    let mut renderer = state
        .backend_data
        .gpus
        .single_renderer(&primary_gpu)
        .unwrap();

    #[cfg(feature = "fps_ticker")]
    {
        use crate::drawing::{FpsElement, FPS_NUMBERS_PNG};

        let fps_image = image::io::Reader::with_format(
            std::io::Cursor::new(FPS_NUMBERS_PNG),
            image::ImageFormat::Png,
        )
        .decode()
        .unwrap();
        let fps_texture = renderer
            .import_memory(
                &fps_image.to_rgba8(),
                Fourcc::Abgr8888,
                (fps_image.width() as i32, fps_image.height() as i32).into(),
                false,
            )
            .expect("Unable to upload FPS texture");

        for backend in state.backend_data.backends.values_mut() {
            for surface in backend.surfaces.values_mut() {
                surface.fps_element = Some(FpsElement::new(fps_texture.clone()));
            }
        }
        state.backend_data.fps_texture = Some(fps_texture);
    }

    #[cfg(feature = "egl")]
    {
        use smithay::backend::renderer::ImportEgl;

        info!(
            ?primary_gpu,
            "Trying to initialize EGL Hardware Acceleration",
        );
        match renderer.bind_wl_display(&display_handle) {
            Ok(_) => info!("EGL hardware-acceleration enabled"),
            Err(err) => info!(?err, "Failed to initialize EGL hardware-acceleration"),
        }
    }

    // init dmabuf support with format list from our primary gpu
    let dmabuf_formats = renderer.dmabuf_formats();
    let default_feedback = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), dmabuf_formats)
        .build()
        .unwrap();
    let mut dmabuf_state = DmabufState::new();
    let global = dmabuf_state
        .create_global_with_default_feedback::<Otto<UdevData>>(&display_handle, &default_feedback);
    state.backend_data.dmabuf_state = Some((dmabuf_state, global));

    let gpus = &mut state.backend_data.gpus;
    state
        .backend_data
        .backends
        .values_mut()
        .for_each(|backend_data| {
            // Update the per drm surface dmabuf feedback
            backend_data.surfaces.values_mut().for_each(|surface_data| {
                surface_data.dmabuf_feedback = surface_data.dmabuf_feedback.take().or_else(|| {
                    get_surface_dmabuf_feedback(
                        primary_gpu,
                        surface_data.render_node,
                        gpus,
                        &surface_data.compositor,
                    )
                });
            });
        });

    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, data| match event {
            UdevEvent::Added { device_id, path } => {
                if let Err(err) = DrmNode::from_dev_id(device_id)
                    .map_err(DeviceAddError::DrmNode)
                    .and_then(|node| data.device_added(node, &path))
                {
                    error!("Skipping device {device_id}: {err}");
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.device_changed(node)
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.device_removed(node)
                }
            }
        })
        .unwrap();

    /*
     * Start XWayland if supported
     */
    #[cfg(feature = "xwayland")]
    state.start_xwayland();

    /*
     * Start the screenshare D-Bus service
     */
    match crate::screenshare::ScreenshareManager::start(&event_loop.handle()) {
        Ok(manager) => {
            state.screenshare_manager = Some(manager);
            tracing::info!("Screenshare D-Bus service started");
        }
        Err(e) => {
            tracing::warn!("Failed to start screenshare D-Bus service: {}", e);
        }
    }

    /*
     * Create virtual outputs from config
     */
    {
        let vout_configs = crate::config::Config::with(|c| c.virtual_outputs.clone());
        if !vout_configs.is_empty() {
            let gbm_device = state.backend_data.gbm_device();
            let format_modifiers = state
                .backend_data
                .get_format_modifiers(smithay::backend::allocator::Fourcc::Argb8888);

            for vout_config in &vout_configs {
                let output = crate::virtual_output::VirtualOutputState::build_output(vout_config);
                let global = output.create_global::<Otto<UdevData>>(&display_handle);

                let position: smithay::utils::Point<i32, smithay::utils::Logical> = vout_config
                    .position
                    .map(|p| (p.x, p.y).into())
                    .unwrap_or_else(|| (0, 0).into());
                state.workspaces.map_output(&output, position);

                match crate::virtual_output::VirtualOutputState::start(
                    output,
                    global,
                    vout_config,
                    gbm_device.clone(),
                    format_modifiers.clone(),
                ) {
                    Ok((vout_state, node_id)) => {
                        tracing::info!(
                            "Virtual output '{}' started (PipeWire node {}). \
                             Connect with: pw-play --target {}",
                            vout_config.name,
                            node_id,
                            node_id
                        );
                        state.virtual_outputs.push(vout_state);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to create virtual output '{}': {}",
                            vout_config.name,
                            e
                        );
                    }
                }
            }
        }

        // Add a calloop timer to drive virtual output rendering independently of physical VBlanks.
        // This ensures frames keep flowing even when physical outputs are idle.
        if !state.virtual_outputs.is_empty() {
            let refresh_hz = state
                .virtual_outputs
                .iter()
                .filter_map(|v| v.output.current_mode())
                .map(|m| m.refresh as f64 / 1000.0)
                .fold(f64::INFINITY, f64::min);
            let refresh_hz = if refresh_hz.is_finite() { refresh_hz } else { 60.0 };
            let interval =
                std::time::Duration::from_micros((1_000_000.0 / refresh_hz) as u64);

            state
                .handle
                .insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(interval),
                    move |_, _, data: &mut Otto<super::types::UdevData>| {
                        data.render_virtual_outputs();
                        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(interval)
                    },
                )
                .expect("failed to schedule virtual output render timer");
            tracing::info!(
                "Virtual output render timer started at {:.1} Hz",
                refresh_hz
            );
        }
    }

    /*
     * And run our loop
     */

    // FIXME: check if we can delay this
    while state.running.load(Ordering::SeqCst) {
        let result = event_loop.dispatch(Some(Duration::from_millis(16)), &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
        } else {
            display_handle.flush_clients().unwrap();
            // Log rendering metrics periodically
            #[cfg(feature = "metrics")]
            state.render_metrics.maybe_log_stats(false);
        }
    }
}
