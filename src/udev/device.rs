// Device management for udev backend
//
// Handles lifecycle of DRM devices: addition, removal, and change events.
// Also manages connector connection/disconnection.

use std::{collections::HashMap, path::Path};

use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{exporter::gbm::GbmFramebufferExporter, DrmDevice, DrmDeviceFd, DrmEvent, DrmNode},
        egl::{EGLDevice, EGLDisplay},
        session::Session,
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        drm::{
            control::{
                connector::{self, SubPixel},
                crtc, Device as ControlDevice, ModeTypeFlags,
            },
            Device,
        },
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
    wayland::drm_lease::DrmLeaseState,
};
use smithay_drm_extras::drm_scanner::DrmScanEvent;
use tracing::{debug, error, info, warn};

use crate::{config::Config, state::Otto};

use super::{
    feedback::get_surface_dmabuf_feedback,
    types::{
        BackendData, DeviceAddError, GbmDrmCompositor, SurfaceData, UdevData, UdevOutputId,
        SUPPORTED_FORMATS, SUPPORTED_FORMATS_8BIT_ONLY,
    },
};

impl Otto<UdevData> {
    /// Handles addition of a new DRM device
    pub(super) fn device_added(
        &mut self,
        node: DrmNode,
        path: &Path,
    ) -> Result<(), DeviceAddError> {
        // Try to open the device
        let fd = self
            .backend_data
            .session
            .open(
                path,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
            )
            .map_err(DeviceAddError::DeviceOpen)?;

        let fd = DrmDeviceFd::new(DeviceFd::from(fd));

        let (drm, notifier) =
            DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::DrmDevice)?;
        let gbm = GbmDevice::new(fd).map_err(DeviceAddError::GbmDevice)?;

        let registration_token = self
            .handle
            .insert_source(
                notifier,
                move |event, metadata, data: &mut Otto<_>| match event {
                    DrmEvent::VBlank(crtc) => {
                        profiling::scope!("vblank", &format!("{crtc:?}"));
                        data.frame_finish(node, crtc, metadata);
                    }
                    DrmEvent::Error(error) => {
                        error!("{:?}", error);
                    }
                },
            )
            .unwrap();

        let render_node =
            EGLDevice::device_for_display(&unsafe { EGLDisplay::new(gbm.clone()).unwrap() })
                .ok()
                .and_then(|x| x.try_get_render_node().ok().flatten())
                .unwrap_or(node);

        self.backend_data
            .gpus
            .as_mut()
            .add_node(render_node, gbm.clone())
            .map_err(DeviceAddError::AddNode)?;

        self.backend_data.backends.insert(
            node,
            BackendData {
                registration_token,
                gbm,
                drm,
                drm_scanner: smithay_drm_extras::drm_scanner::DrmScanner::new(),
                non_desktop_connectors: Vec::new(),
                render_node,
                surfaces: HashMap::new(),
                leasing_global: DrmLeaseState::new::<Otto<UdevData>>(&self.display_handle, &node)
                    .map_err(|err| {
                        warn!(?err, "Failed to initialize drm lease global for: {}", node);
                        err
                    })
                    .ok(),
                active_leases: Vec::new(),
            },
        );

        self.device_changed(node);

        Ok(())
    }

    /// Handles device changes (connector hotplug, etc.)
    pub(super) fn device_changed(&mut self, node: DrmNode) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let scan_result = match device.drm_scanner.scan_connectors(&device.drm) {
            Ok(scan_result) => scan_result,
            Err(err) => {
                warn!(?err, "Failed to scan connectors");
                return;
            }
        };

        for event in scan_result {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_connected(node, connector, crtc);
                }
                DrmScanEvent::Disconnected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_disconnected(node, connector, crtc);
                }
                _ => {}
            }
        }

        // fixup window coordinates
        crate::shell::fixup_positions(&mut self.workspaces, self.pointer.current_location());
    }

    /// Handles removal of a DRM device
    pub(super) fn device_removed(&mut self, node: DrmNode) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let crtcs: Vec<_> = device
            .drm_scanner
            .crtcs()
            .map(|(info, crtc)| (info.clone(), crtc))
            .collect();

        for (connector, crtc) in crtcs {
            self.connector_disconnected(node, connector, crtc);
        }

        debug!("Surfaces dropped");

        // drop the backends on this side
        if let Some(mut backend_data) = self.backend_data.backends.remove(&node) {
            if let Some(mut leasing_global) = backend_data.leasing_global.take() {
                leasing_global.disable_global::<Otto<UdevData>>();
            }

            self.backend_data
                .gpus
                .as_mut()
                .remove_node(&backend_data.render_node);

            self.handle.remove(backend_data.registration_token);

            debug!("Dropping device");
        }

        crate::shell::fixup_positions(&mut self.workspaces, self.pointer.current_location());
    }

    /// Handles connector connection events
    pub(super) fn connector_connected(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let mut renderer = self
            .backend_data
            .gpus
            .single_renderer(&device.render_node)
            .unwrap();
        let render_formats = renderer
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .clone();

        let output_name = format!(
            "{}-{}",
            connector.interface().as_str(),
            connector.interface_id()
        );
        info!(?crtc, "Trying to setup connector {}", output_name,);

        let non_desktop = device
            .drm
            .get_properties(connector.handle())
            .ok()
            .and_then(|props| {
                let (info, value) = props
                    .into_iter()
                    .filter_map(|(handle, value)| {
                        let info = device.drm.get_property(handle).ok()?;
                        Some((info, value))
                    })
                    .find(|(info, _)| info.name().to_str() == Ok("non-desktop"))?;

                info.value_type().convert_value(value).as_boolean()
            })
            .unwrap_or(false);

        // EDID info is no longer available in smithay-drm-extras
        // Using connector info instead
        let (make, model) = (
            format!("{:?}", connector.interface()),
            format!("{:?}", connector.interface()),
        );

        if non_desktop {
            info!(
                "Connector {} is non-desktop, setting up for leasing",
                output_name
            );
            device
                .non_desktop_connectors
                .push((connector.handle(), crtc));
            if let Some(lease_state) = device.leasing_global.as_mut() {
                lease_state.add_connector::<Otto<UdevData>>(
                    connector.handle(),
                    output_name,
                    format!("{} {}", make, model),
                );
            }
        } else {
            self.setup_desktop_connector(
                node,
                connector,
                crtc,
                &output_name,
                &make,
                &model,
                render_formats,
            );
        }
    }

    /// Sets up a desktop (normal display) connector
    #[allow(clippy::too_many_arguments)]
    fn setup_desktop_connector(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
        output_name: &str,
        make: &str,
        model: &str,
        render_formats: smithay::backend::allocator::format::FormatSet,
    ) {
        let device_render_node = {
            let device = self.backend_data.backends.get(&node).unwrap();
            device.render_node
        };

        let device = self.backend_data.backends.get_mut(&node).unwrap();

        // Try to get mode from config first
        let config_profile = Config::with(|config| {
            let descriptor = crate::config::DisplayDescriptor {
                connector: output_name,
                vendor: Some(make),
                model: Some(model),
                kind: None,
            };
            config.displays.resolve(output_name, &descriptor)
        });

        let mode_id = if let Some(ref profile) = config_profile {
            // Try to find matching resolution from config
            if let Some(desired_res) = profile.resolution {
                connector
                    .modes()
                    .iter()
                    .position(|mode| {
                        let size = mode.size();
                        size.0 as u32 == desired_res.width && size.1 as u32 == desired_res.height
                    })
                    .or_else(|| {
                        warn!(
                            "Requested resolution {}x{} not available for {}, using preferred mode",
                            desired_res.width, desired_res.height, output_name
                        );
                        connector
                            .modes()
                            .iter()
                            .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                    })
                    .unwrap_or(0)
            } else {
                connector
                    .modes()
                    .iter()
                    .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                    .unwrap_or(0)
            }
        } else {
            connector
                .modes()
                .iter()
                .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                .unwrap_or(0)
        };

        let drm_mode = connector.modes()[mode_id];
        info!(
            "Selected mode for {}: {}x{} @ {}Hz",
            output_name,
            drm_mode.size().0,
            drm_mode.size().1,
            drm_mode.vrefresh()
        );

        let mut wl_mode = WlMode::from(drm_mode);
        // Use config refresh rate, or use DRM mode's refresh rate, or fallback to 60Hz
        if let Some(ref profile) = config_profile {
            if let Some(refresh_hz) = profile.refresh_hz {
                wl_mode.refresh = (refresh_hz * 1000.0) as i32;
            }
        }
        // If still zero after config check, use DRM mode's refresh or 60Hz fallback
        if wl_mode.refresh == 0 {
            let drm_refresh_mhz = drm_mode.vrefresh() as i32 * 1000;
            wl_mode.refresh = if drm_refresh_mhz > 0 {
                drm_refresh_mhz
            } else {
                60 * 1000
            };
        }

        let surface = match device
            .drm
            .create_surface(crtc, drm_mode, &[connector.handle()])
        {
            Ok(surface) => surface,
            Err(err) => {
                warn!("Failed to create drm surface: {}", err);
                return;
            }
        };

        let subpixel = match connector.subpixel() {
            SubPixel::Unknown => Subpixel::Unknown,
            SubPixel::None => Subpixel::None,
            SubPixel::NotImplemented => Subpixel::Unknown,
            SubPixel::HorizontalRgb => Subpixel::HorizontalRgb,
            SubPixel::HorizontalBgr => Subpixel::HorizontalBgr,
            SubPixel::VerticalRgb => Subpixel::VerticalRgb,
            SubPixel::VerticalBgr => Subpixel::VerticalBgr,
            _ => Subpixel::Unknown,
        };
        let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
        let output = Output::new(
            output_name.to_string(),
            PhysicalProperties {
                size: (phys_w as i32, phys_h as i32).into(),
                subpixel,
                make: make.to_string(),
                model: model.to_string(),
                serial_number: String::new(),
            },
        );

        // FIXME handle multimonitor setup
        let root = self.scene_element.root_layer().unwrap();
        let w = wl_mode.size.w as f32;
        let h = wl_mode.size.h as f32;
        self.workspaces
            .set_screen_dimension(wl_mode.size.w, wl_mode.size.h);
        let scene_size = layers::types::Size::points(w, h);
        root.set_size(scene_size, None);
        self.scene_element.set_size(w, h);
        self.layers_engine.scene_set_size(w, h);

        let global = output.create_global::<Otto<UdevData>>(&self.display_handle);

        let x = self.workspaces.outputs().fold(0, |acc, o| {
            acc + self.workspaces.output_geometry(o).unwrap().size.w
        });
        let position = (x, 0).into();
        output.set_preferred(wl_mode);
        let screen_scale = Config::with(|c| c.screen_scale);
        output.change_current_state(
            Some(wl_mode),
            None,
            Some(smithay::output::Scale::Fractional(screen_scale)),
            Some(position),
        );

        let is_primary = config_profile.as_ref().map(|p| p.primary).unwrap_or(false);
        self.workspaces.map_output_with_primary(&output, position, is_primary);

        output.user_data().insert_if_missing(|| UdevOutputId {
            crtc,
            device_id: node,
            is_laptop_panel: crate::utils::is_laptop_panel(output_name),
        });

        #[cfg(feature = "fps_ticker")]
        let fps_element = self
            .backend_data
            .fps_texture
            .clone()
            .map(crate::drawing::FpsElement::new);

        let allocator = GbmAllocator::new(
            device.gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );

        let color_formats = if std::env::var("ANVIL_DISABLE_10BIT").is_ok() {
            SUPPORTED_FORMATS_8BIT_ONLY
        } else {
            SUPPORTED_FORMATS
        };

        let compositor = self.create_surface_compositor(
            node,
            surface,
            allocator,
            color_formats,
            render_formats,
            &output,
        );

        if let Some(compositor) = compositor {
            let dmabuf_feedback = get_surface_dmabuf_feedback(
                self.backend_data.primary_gpu,
                device_render_node,
                &mut self.backend_data.gpus,
                &compositor,
            );

            let surface_data = SurfaceData {
                dh: self.display_handle.clone(),
                device_id: node,
                render_node: device_render_node,
                global: Some(global),
                compositor,
                #[cfg(feature = "fps_ticker")]
                fps: fps_ticker::Fps::default(),
                #[cfg(feature = "fps_ticker")]
                fps_element,
                dmabuf_feedback,
                was_direct_scanout: false,
                #[cfg(feature = "metrics")]
                render_metrics: Some(self.render_metrics.clone()),
            };

            let device = self.backend_data.backends.get_mut(&node).unwrap();
            device.surfaces.insert(crtc, surface_data);

            self.schedule_initial_render(node, crtc, self.handle.clone());
        }
    }

    /// Creates a surface compositor (either Surface or Compositor mode)
    fn create_surface_compositor(
        &mut self,
        node: DrmNode,
        surface: smithay::backend::drm::DrmSurface,
        allocator: GbmAllocator<DrmDeviceFd>,
        color_formats: &[smithay::backend::allocator::Fourcc],
        render_formats: smithay::backend::allocator::format::FormatSet,
        output: &Output,
    ) -> Option<GbmDrmCompositor> {
        let device = self.backend_data.backends.get_mut(&node)?;

        let driver = match device.drm.get_driver() {
            Ok(driver) => driver,
            Err(err) => {
                warn!("Failed to query drm driver: {}", err);
                return None;
            }
        };

        let mut planes = surface.planes().clone();

        // Using an overlay plane on a nvidia card breaks
        if driver
            .name()
            .to_string_lossy()
            .to_lowercase()
            .contains("nvidia")
            || driver
                .description()
                .to_string_lossy()
                .to_lowercase()
                .contains("nvidia")
        {
            planes.overlay = vec![];
        }

        tracing::debug!("Max cursor size: {:?}", device.drm.cursor_size());
        let compositor = match smithay::backend::drm::compositor::DrmCompositor::new(
            output,
            surface,
            Some(planes),
            allocator,
            GbmFramebufferExporter::new(device.gbm.clone(), device.render_node.into()),
            color_formats.iter().copied(),
            render_formats,
            device.drm.cursor_size(),
            Some(device.gbm.clone()),
        ) {
            Ok(compositor) => compositor,
            Err(err) => {
                warn!("Failed to create drm compositor: {}", err);
                return None;
            }
        };
        Some(compositor)
    }

    /// Handles connector disconnection events
    pub(super) fn connector_disconnected(
        &mut self,
        node: DrmNode,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        if let Some(pos) = device
            .non_desktop_connectors
            .iter()
            .position(|(handle, _)| *handle == connector.handle())
        {
            let _ = device.non_desktop_connectors.remove(pos);
            if let Some(leasing_state) = device.leasing_global.as_mut() {
                leasing_state.withdraw_connector(connector.handle());
            }
        } else {
            device.surfaces.remove(&crtc);

            let output = self
                .workspaces
                .outputs()
                .find(|o| {
                    o.user_data()
                        .get::<UdevOutputId>()
                        .map(|id| id.device_id == node && id.crtc == crtc)
                        .unwrap_or(false)
                })
                .cloned();

            if let Some(output) = output {
                self.workspaces.unmap_output(&output);
            }
        }
    }

    /// Updates display power state based on lid switch and configuration
    ///
    /// This implements clamshell mode: when lid is closed and external monitors
    /// are connected, the laptop panel is disabled. When the lid opens, it's re-enabled.
    pub fn update_display_power_state(&mut self) {
        use crate::config::{Config, LidCloseAction};

        // Check config - if lid management is disabled, do nothing
        let (manage_lid, lid_action) = Config::with(|config| {
            (
                config.power_management.manage_lid_switch,
                config.power_management.on_lid_close,
            )
        });

        if !manage_lid {
            tracing::debug!("Lid switch management disabled in config");
            return;
        }

        // Determine if we should disable laptop panels based on lid state and config
        let disable_laptop_panels = match lid_action {
            LidCloseAction::Auto => {
                // Normal laptop behavior: only disable if lid is closed
                self.is_lid_closed
            }
            LidCloseAction::DisableInternalScreen => {
                // Display manager mode: always act like lid is closed
                // (screen turns off, system stays running)
                true
            }
        };

        if disable_laptop_panels && self.is_lid_closed {
            // Check if any external monitor is connected (for logging purposes)
            let mut has_external_monitor = false;
            for device in self.backend_data.backends.values() {
                for (connector, _crtc) in device.drm_scanner.crtcs() {
                    let connector_name = format!(
                        "{}-{}",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );
                    if !crate::utils::is_laptop_panel(&connector_name) {
                        has_external_monitor = true;
                        break;
                    }
                }
                if has_external_monitor {
                    break;
                }
            }

            if has_external_monitor {
                tracing::info!(
                    "Lid closed with external monitor - disabling laptop panel (clamshell mode)"
                );
            } else {
                tracing::info!("Lid closed without external monitor - disabling laptop panel (system will suspend via systemd-logind)");
            }
        }

        // Collect outputs to disconnect/reconnect
        let mut to_disconnect = vec![];
        let mut to_reconnect = vec![];

        if disable_laptop_panels {
            // Lid is closed - disconnect laptop panels that are currently active
            for (&node, device) in &self.backend_data.backends {
                for (&crtc, _surface) in &device.surfaces {
                    // Find the output for this surface
                    let output = self.workspaces.outputs().find(|o| {
                        o.user_data()
                            .get::<UdevOutputId>()
                            .map(|id| id.device_id == node && id.crtc == crtc && id.is_laptop_panel)
                            .unwrap_or(false)
                    });

                    if let Some(output) = output {
                        let output_name = output.name();
                        tracing::info!("Disabling laptop panel: {}", output_name);
                        to_disconnect.push((node, crtc));
                    }
                }
            }
        } else {
            // Lid is open - reconnect laptop panels that are not currently active
            for (&node, device) in &self.backend_data.backends {
                for (connector, crtc) in device.drm_scanner.crtcs() {
                    let connector_name = format!(
                        "{}-{}",
                        connector.interface().as_str(),
                        connector.interface_id()
                    );

                    // Check if this is a laptop panel
                    if !crate::utils::is_laptop_panel(&connector_name) {
                        continue;
                    }

                    // Check if this panel is already connected
                    let already_connected = device.surfaces.contains_key(&crtc);

                    if !already_connected {
                        tracing::info!("Re-enabling laptop panel: {}", connector_name);
                        to_reconnect.push((node, connector.clone(), crtc));
                    }
                }
            }
        }

        // Disconnect laptop panels that should be disabled
        let disconnect_with_connectors: Vec<_> = to_disconnect
            .into_iter()
            .filter_map(|(node, crtc)| {
                self.backend_data.backends.get(&node).and_then(|device| {
                    device
                        .drm_scanner
                        .crtcs()
                        .find(|(_, c)| *c == crtc)
                        .map(|(connector, _)| (node, connector.clone(), crtc))
                })
            })
            .collect();

        for (node, connector, crtc) in disconnect_with_connectors {
            self.connector_disconnected(node, connector, crtc);
        }

        // Reconnect laptop panels that should be re-enabled
        for (node, connector, crtc) in to_reconnect {
            self.connector_connected(node, connector, crtc);
        }
    }
}
