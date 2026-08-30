// Initialization logic for udev backend
//
// Handles session setup, GPU initialization, libinput configuration,
// and the main event loop for the udev backend.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

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

/// Whether a kernel log line reports a display-engine underrun. There is
/// no standard KMS event for this, so detection is per-driver log
/// phrasing: i915 says "FIFO underrun", amdgpu/DC says "underflow"
/// (HUBP/DCN), smaller drivers vary. The context words guard against
/// unrelated "underrun" sources (audio, serial).
fn looks_like_display_underrun(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    (l.contains("underrun") || l.contains("underflow"))
        && (l.contains("drm")
            || l.contains("i915")
            || l.contains("amdgpu")
            || l.contains("pipe")
            || l.contains("crtc")
            || l.contains("hubp")
            || l.contains("display"))
}

/// Configures the devices libinput already knows about, and records them so a
/// later `input.*` change can reach them again.
///
/// The events are consumed here, before the calloop source exists, so this is
/// the only chance to see the devices present at startup.
fn configure_libinput_devices(
    libinput: &mut Libinput,
    config: &Config,
    devices: &mut Vec<smithay::reexports::input::Device>,
) {
    use smithay::reexports::input::{
        event::{DeviceEvent, EventTrait},
        Event,
    };

    // Process initial devices
    libinput.dispatch().ok();

    for event in libinput.by_ref() {
        if let Event::Device(DeviceEvent::Added(added_event)) = event {
            let mut device = added_event.device();
            super::input_config::apply_device_config(&mut device, &config.input);
            devices.push(device);
        }
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
        syncobj_state: None,
        session,
        primary_gpu,
        gpus,
        backends: HashMap::new(),
        input_devices: Vec::new(),
        #[cfg(feature = "fps_ticker")]
        fps_texture: None,

        context_id: None, // Will be set after device initialization
        render_requested: AtomicBool::new(false),
        damage_generation: 0,
        underrun_penalty: 0,
        last_screencast_kick: None,
        last_kick_cursor_pos: None,
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
    let mut initial_devices = Vec::new();
    Config::with(|config| {
        configure_libinput_devices(&mut libinput_context, config, &mut initial_devices);
    });
    state.backend_data.input_devices = initial_devices;

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    /*
     * Bind all our objects that get driven by the event loop
     */
    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, data| {
            // Keep the device registry in step before the event is handled: a
            // device that appears now must be configured from the *current*
            // configuration, not from the snapshot Otto started with, and it
            // has to be reachable when a setting changes later.
            match &event {
                smithay::backend::input::InputEvent::DeviceAdded { device } => {
                    let mut device = device.clone();
                    crate::config::Config::with(|config| {
                        super::input_config::apply_device_config(&mut device, &config.input);
                    });
                    data.backend_data.input_devices.push(device);
                }
                smithay::backend::input::InputEvent::DeviceRemoved { device } => {
                    data.backend_data
                        .input_devices
                        .retain(|known| known != device);
                }
                _ => {}
            }

            let dh = data.backend_data.dh.clone();
            data.process_input_event(&dh, event);
            // Input may move the cursor or trigger visual changes — request a render.
            data.backend_data
                .render_requested
                .store(true, std::sync::atomic::Ordering::Release);
        })
        .unwrap();

    let handle = event_loop.handle();
    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), data| match event {
            SessionEvent::PauseSession => {
                libinput_context.suspend();
                info!("pausing session");

                // Keys held while we leave for another VT never report their
                // release here, so xkb would keep them latched for the rest of
                // the session — a stuck Ctrl arms drag-to-tile on every later
                // window drag. Drop them all now.
                data.release_all_keys();

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

                // Coming back from another VT, the primary plane has no content
                // and nothing in the scene has changed — so resetting the DRM
                // buffers below is only half the story. The scene engine
                // replays recorded pictures and reports damage for what moved,
                // and nothing moved while we were away, so it would hand back
                // an empty frame over an empty plane. Damage the whole scene so
                // the first frame after the switch is a complete one.
                let extent = data
                    .workspaces
                    .outputs()
                    .filter_map(|output| data.workspaces.output_geometry(output))
                    .fold(
                        None::<smithay::utils::Rectangle<i32, smithay::utils::Logical>>,
                        |acc, geo| {
                            Some(match acc {
                                Some(acc) => acc.merge(geo),
                                None => geo,
                            })
                        },
                    );
                if let Some(extent) = extent {
                    // Scene space is physical pixels; a scale of 1 would
                    // under-damage a HiDPI screen, so take the largest.
                    let scale = data
                        .workspaces
                        .outputs()
                        .map(|o| o.current_scale().fractional_scale() as f32)
                        .fold(1.0f32, f32::max);
                    data.layers_engine.add_damage(layers::skia::Rect::from_xywh(
                        extent.loc.x as f32 * scale,
                        extent.loc.y as f32 * scale,
                        extent.size.w as f32 * scale,
                        extent.size.h as f32 * scale,
                    ));
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

    // init dmabuf support with format list from our primary gpu.
    // Strip the clear-color CCS modifiers Otto can't sample (see
    // feedback::strip_clear_color_modifiers) so clients fall back to renderable ones.
    let dmabuf_formats = super::feedback::strip_clear_color_modifiers(renderer.dmabuf_formats());
    let default_feedback = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), dmabuf_formats)
        .build()
        .unwrap();
    let mut dmabuf_state = DmabufState::new();
    let global = dmabuf_state
        .create_global_with_default_feedback::<Otto<UdevData>>(&display_handle, &default_feedback);
    state.backend_data.dmabuf_state = Some((dmabuf_state, global));

    // Expose explicit sync (wp_linux_drm_syncobj) if supported by primary GPU
    {
        use smithay::backend::drm::NodeType;
        use smithay::wayland::drm_syncobj::{supports_syncobj_eventfd, DrmSyncobjState};

        if let Some(primary_node) = state
            .backend_data
            .primary_gpu
            .node_with_type(NodeType::Primary)
            .and_then(|x| x.ok())
        {
            if let Some(backend) = state.backend_data.backends.get(&primary_node) {
                let import_device = backend.drm.device_fd().clone();
                if supports_syncobj_eventfd(&import_device) {
                    let syncobj_state =
                        DrmSyncobjState::new::<Otto<UdevData>>(&display_handle, import_device);
                    state.backend_data.syncobj_state = Some(syncobj_state);
                    info!("Explicit sync (wp_linux_drm_syncobj) enabled");
                } else {
                    info!("Explicit sync not supported by GPU (syncobj_eventfd unavailable)");
                }
            }
        }
    }

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
    // Accessibility rides on the same D-Bus thread. Only this backend offers
    // it: a nested Otto would take the name from the session hosting it.
    let accessible = crate::config::Config::with(|config| config.accessibility.enabled);
    let a11y = accessible.then(|| state.a11y.take_dbus_parts()).flatten();

    match crate::screenshare::ScreenshareManager::start(&event_loop.handle(), a11y) {
        Ok(manager) => {
            // The shell's own accessible tree. Registered here rather than with
            // the state because it needs the command channel the service just
            // created — an assistive technology clicking a dock icon takes the
            // same path as any other request to focus an application.
            if accessible {
                let chrome = crate::a11y::chrome::ShellAccessibility::new(
                    manager.command_sender.clone(),
                    std::sync::Arc::downgrade(&state.workspaces.dock),
                    std::sync::Arc::downgrade(&state.workspaces.app_switcher),
                    state.workspaces.show_all.clone(),
                    state.workspaces.window_views.clone(),
                );
                crate::utils::Observable::add_listener(&mut state.workspaces, chrome.clone());
                // And the dock's own model, which lands later than the change
                // that caused it: without this the tree says an application
                // that has just started is not running, and goes on saying so.
                state.workspaces.dock.add_model_listener(chrome.clone());
                // Describe the desktop as it stands, rather than waiting for
                // something to change it: an assistive technology attaching to
                // an idle session would otherwise be told the shell is empty.
                state
                    .workspaces
                    .with_model(|model| crate::utils::Observer::notify(chrome.as_ref(), model));
                state.a11y.chrome = Some(chrome);
                tracing::info!("Shell accessibility published");
            }

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
        // Login mode drives the primary output only. A configured virtual
        // output would otherwise be created here and hand the greeter a second
        // screen — one that a remote client could attach to.
        let vout_configs = if crate::login::is_login_mode() {
            Vec::new()
        } else {
            crate::config::Config::with(|c| c.virtual_outputs.clone())
        };
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

        // Scripted-gesture driver (`/tmp/otto-gesture`). Idle and allocation-free
        // until that file appears, so it costs a file-existence check per tick.
        {
            let interval = std::time::Duration::from_millis(8);
            state
                .handle
                .insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(interval),
                    move |_, _, data: &mut Otto<super::types::UdevData>| {
                        crate::debug_gesture::tick(data);
                        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(interval)
                    },
                )
                .expect("failed to schedule synthetic gesture timer");
        }

        // Calloop timer driving off-VBlank rendering: virtual outputs (which
        // have no physical VBlank) and physical outputs with an active
        // screencast (which would otherwise only render on damage, starving
        // the capture when the desktop is idle). Always scheduled — both
        // kicks no-op when there's nothing to do, so it also covers a
        // screencast started at runtime with no virtual outputs configured.
        {
            let refresh_hz = state
                .virtual_outputs
                .iter()
                .filter_map(|v| v.output.current_mode())
                .map(|m| m.refresh as f64 / 1000.0)
                .fold(f64::INFINITY, f64::min);
            let refresh_hz = if refresh_hz.is_finite() {
                refresh_hz
            } else {
                30.0
            };
            let interval = std::time::Duration::from_micros((1_000_000.0 / refresh_hz) as u64);

            state
                .handle
                .insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(interval),
                    move |_, _, data: &mut Otto<super::types::UdevData>| {
                        data.render_virtual_outputs();
                        data.kick_screencast_outputs();
                        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(interval)
                    },
                )
                .expect("failed to schedule off-vblank render timer");
            tracing::info!("Off-VBlank render timer started at {:.1} Hz", refresh_hz);
        }
    }

    /*
     * And run our loop
     */

    // ── Adaptive plane budget: kernel underrun monitor ────────────────────
    // i915 logs "CPU pipe X FIFO underrun" once per episode when the display
    // engine starves fetching planes (the affected pipe scans out solid
    // garbage from mid-frame down — bright green on Intel). Follow the
    // kernel journal and shed plane usage when it happens: level 1 drops
    // window promotion, level 2 the whole plane decomposition. Display
    // bandwidth is shared across pipes, so the penalty is global.
    {
        use smithay::reexports::calloop::{
            generic::Generic, Interest, Mode as CalloopMode, PostAction,
        };
        use std::io::Read as _;
        use std::os::fd::AsRawFd;
        match std::process::Command::new("journalctl")
            .args(["-kf", "-o", "cat", "-n", "0"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                // Non-blocking: the source fires on readiness; drain fully.
                unsafe {
                    let fd = stdout.as_raw_fd();
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
                event_loop
                    .handle()
                    .insert_source(
                        Generic::new(stdout, Interest::READ, CalloopMode::Level),
                        move |_, stdout, data: &mut Otto<UdevData>| {
                            let mut buf = [0u8; 4096];
                            let mut hit = false;
                            loop {
                                // Safety: the fd stays owned by the source.
                                match unsafe { stdout.get_mut() }.read(&mut buf) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if String::from_utf8_lossy(&buf[..n])
                                            .lines()
                                            .any(looks_like_display_underrun)
                                        {
                                            hit = true;
                                        }
                                    }
                                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                                    Err(_) => break,
                                }
                            }
                            if hit {
                                data.raise_underrun_penalty();
                            }
                            Ok(PostAction::Continue)
                        },
                    )
                    .expect("failed to insert underrun monitor");
                // The follower lives for the whole session.
                std::mem::forget(child);
            }
            Err(e) => {
                warn!("underrun monitor unavailable (journalctl spawn failed): {e}");
            }
        }
    }

    // Perform an initial dispatch so that backends (including XWayland) can
    // finish asynchronous setup (e.g. setting DISPLAY) before autostart.
    if event_loop
        .dispatch(Some(Duration::from_millis(0)), &mut state)
        .is_err()
    {
        state.running.store(false, Ordering::SeqCst);
    }

    if state.running.load(Ordering::SeqCst) {
        state.autostart();
    }

    while state.running.load(Ordering::SeqCst) {
        // Use tight timing when animations are active or the idle countdown
        // is still running (recent input/damage keeps us at frame rate).
        let has_animations = state.scene_element.has_pending_animations();
        let has_active_countdown = state
            .backend_data
            .backends
            .values()
            .flat_map(|d| d.surfaces.values())
            .any(|s| s.idle_countdown > 0);
        let dispatch_timeout = if has_animations || has_active_countdown {
            Some(Duration::from_millis(1))
        } else {
            Some(Duration::from_secs(1))
        };

        let result = event_loop.dispatch(dispatch_timeout, &mut state);
        if result.is_err() {
            state.running.store(false, Ordering::SeqCst);
        } else {
            // If a redraw was requested (e.g. client commit, input), reset
            // idle countdowns. Only trigger an explicit render() when fully
            // idle (countdown was 0) to kick-start the render loop; otherwise
            // the existing reschedule timer will pick it up — calling render()
            // while a timer is pending causes double-renders and stuttering.
            let was_requested = state
                .backend_data
                .render_requested
                .swap(false, Ordering::AcqRel);
            if was_requested {
                // Idle is a PER-SURFACE property: with multiple outputs one
                // can be idle (no timer, no VBlank pending) while another is
                // mid-loop. Kick exactly the idle ones — resetting a busy
                // surface's countdown is enough, its pending timer/VBlank
                // consumes it. (Kicking only when ALL surfaces were idle
                // wedged multi-output: an idle surface's countdown got reset
                // to 3 with no render scheduled, nothing ever decremented it,
                // so `all(== 0)` never held again and input stopped waking
                // the render loop entirely.)
                let mut kick: Vec<(
                    smithay::backend::drm::DrmNode,
                    smithay::reexports::drm::control::crtc::Handle,
                )> = Vec::new();
                for (node, device) in state.backend_data.backends.iter_mut() {
                    for (crtc, surface) in device.surfaces.iter_mut() {
                        if surface.idle_countdown == 0 {
                            kick.push((*node, *crtc));
                        }
                        // Short tail after the last input/commit — enough to absorb
                        // one missed event gap without flapping fast/slow dispatch.
                        // (Was 30 ≈ 500 ms which kept the 1 kHz poll loop hot
                        // through entire animations with no benefit.)
                        surface.idle_countdown = 3;
                    }
                }
                for (node, crtc) in kick {
                    state.render(node, Some(crtc));
                }
            }
            // Debug hook: `echo ActionName > $OTTO_ACTION_FILE` executes a
            // builtin shortcut action as if its key was pressed. Shared with
            // the winit backend; see `poll_debug_action_file`.
            if state.poll_debug_action_file() {
                // Real key events request a redraw as a side effect; without
                // it the scheduled lay-rs transactions never tick and the
                // action stays invisible.
                state.backend_data.request_redraw();
            }
            // Tell any window that has moved where it is now. Diffed against
            // what was last sent, so a desktop at rest sends nothing.
            crate::surface_style::send_desktop_frames(&mut state);
            display_handle.flush_clients().unwrap();
        }
    }
}
