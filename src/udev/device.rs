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
#[cfg(feature = "renderer_sync")]
use smithay::backend::renderer::sync::SyncPoint;

/// Size the fallback composite scene element to fit the largest mapped
/// output (output subtrees overlap at scene (0,0), so the scene never
/// needs to be wider than the biggest output — matches the scene-root
/// sizing in the workspaces layout pass). Free function over the two
/// fields so callers can invoke it while other fields of `Otto` are
/// mutably borrowed.
fn sync_scene_size_to_outputs(
    workspaces: &crate::workspaces::Workspaces,
    scene_element: &mut crate::render_elements::scene_element::SceneElement,
) {
    let (mut max_w, mut max_h) = (0f32, 0f32);
    for o in workspaces.outputs() {
        if let Some(mode) = o.current_mode() {
            max_w = max_w.max(mode.size.w as f32);
            max_h = max_h.max(mode.size.h as f32);
        }
    }
    if max_w > 0.0 && max_h > 0.0 {
        scene_element.set_size(max_w, max_h);
    }
}

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
            let modes = connector.modes();
            // A refresh-only profile pins the display's preferred resolution;
            // an explicit `resolution` overrides it.
            let preferred_size = modes
                .iter()
                .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
                .map(|m| m.size());
            let res_matches = |mode: &smithay::reexports::drm::control::Mode| {
                let size = mode.size();
                match profile.resolution {
                    Some(r) => size.0 as u32 == r.width && size.1 as u32 == r.height,
                    None => preferred_size.is_none_or(|p| size == p),
                }
            };
            // Most specific first: resolution + refresh, then resolution
            // alone, then the connector's preferred mode. The advertised
            // refresh always comes from the mode actually selected.
            profile
                .refresh_hz
                .and_then(|hz| {
                    modes
                        .iter()
                        .position(|m| res_matches(m) && (m.vrefresh() as f64 - hz).abs() <= 1.0)
                })
                .or_else(|| {
                    profile.resolution.and_then(|desired_res| {
                        let idx = modes.iter().position(res_matches);
                        if idx.is_none() {
                            warn!(
                                "Requested resolution {}x{} not available for {}, using preferred mode",
                                desired_res.width, desired_res.height, output_name
                            );
                        }
                        idx
                    })
                })
                .or_else(|| {
                    modes
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
        // Advertise the selected mode's real refresh — config refresh_hz only
        // influences mode selection above, it must never make clients see a
        // rate the hardware isn't running.
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

        let global = output.create_global::<Otto<UdevData>>(&self.display_handle);

        // If this connector was suspended (lid close), restore its saved
        // position and primary status: other outputs (e.g. virtual ones) kept
        // running meanwhile, and auto-placement would pack the panel after
        // them and leave primary — and the dock — on the wrong output.
        let suspended = self.workspaces.take_suspended_output(output_name);

        // Position: suspended restore first, then the config profile,
        // otherwise auto-place to the right of the existing outputs (logical
        // coordinates). There is no mirroring feature: a position that
        // overlaps an existing output is rejected in favour of auto-placement.
        let screen_scale = Config::with(|c| c.screen_scale);
        let logical_size = smithay::utils::Size::<i32, smithay::utils::Logical>::from((
            (wl_mode.size.w as f64 / screen_scale) as i32,
            (wl_mode.size.h as f64 / screen_scale) as i32,
        ));
        let position: smithay::utils::Point<i32, smithay::utils::Logical> = suspended
            .map(|(pos, _)| pos)
            .or_else(|| {
                config_profile
                    .as_ref()
                    .and_then(|p| p.position)
                    .map(|p| smithay::utils::Point::from((p.x, p.y)))
            })
            .filter(|&pos| {
                let rect = smithay::utils::Rectangle::new(pos, logical_size);
                let overlap = self.workspaces.outputs().any(|o| {
                    self.workspaces
                        .output_geometry(o)
                        .is_some_and(|g| g.overlaps(rect))
                });
                if overlap {
                    warn!(
                        "Configured position {:?} for {} overlaps an existing output; \
                         falling back to auto placement (outputs cannot overlap)",
                        pos, output_name
                    );
                }
                !overlap
            })
            .unwrap_or_else(|| {
                let x = self.workspaces.outputs().fold(0, |acc, o| {
                    acc + self
                        .workspaces
                        .output_geometry(o)
                        .map(|g| g.size.w)
                        .unwrap_or(0)
                });
                (x, 0).into()
            });
        output.set_preferred(wl_mode);
        output.change_current_state(
            Some(wl_mode),
            None,
            Some(smithay::output::Scale::Fractional(screen_scale)),
            Some(position),
        );

        let is_primary = suspended
            .map(|(_, was_primary)| was_primary)
            .unwrap_or_else(|| config_profile.as_ref().map(|p| p.primary).unwrap_or(false));
        self.workspaces
            .map_output_with_primary(&output, position, is_primary);

        // The flattened model's width/height describe the PRIMARY output
        // (dock sizing, layout fallbacks) — never overwrite them from a
        // secondary connector. The scene root is sized to the union of all
        // outputs by the layout pass in either branch.
        let is_model_source = self
            .workspaces
            .primary_output()
            .map(|o| o.name() == output_name)
            .unwrap_or(true);
        if is_model_source {
            self.workspaces
                .set_screen_dimension(wl_mode.size.w, wl_mode.size.h);
        } else {
            self.workspaces.relayout_outputs();
        }
        sync_scene_size_to_outputs(&self.workspaces, &mut self.scene_element);

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

        let surface_is_legacy = surface.is_legacy();
        let result = self.create_surface_compositor(
            node,
            surface,
            allocator,
            color_formats,
            render_formats,
            &output,
        );

        if let Some((compositor, overlay_count)) = result {
            // Plane decomposition needs an atomic driver (per-frame TEST_ONLY
            // assignment), enough overlay planes for the per-purpose buffers,
            // and the primary GPU (plane dmabufs are rendered with the primary
            // GPU's EGL context; a cross-device import per plane per frame is
            // unreliable). Anything else renders as a single scene element.
            let planes_enabled = !surface_is_legacy
                && overlay_count >= 3
                && device_render_node == self.backend_data.primary_gpu;
            if !planes_enabled {
                tracing::info!(
                    target: "otto::planes",
                    "plane decomposition disabled for {}: legacy={} overlays={} primary_gpu={}",
                    output.name(),
                    surface_is_legacy,
                    overlay_count,
                    device_render_node == self.backend_data.primary_gpu,
                );
            }
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
                #[cfg(feature = "metrics")]
                render_metrics: Some(self.render_metrics.clone()),
                avg_render_time_us: 2000.0, // start with 2ms estimate
                idle_countdown: 0,
                has_rendered_once: false,
                full_redraw_done: false,
                rendered_damage_gen: 0,
                cursor_was_in_output: false,
                prefetched_scene_damage: None,
                scene_dmabuf_element: None,
                backdrop_surface: None,
                backdrop_image: None,
                backdrop_preblurred: false,
                backdrop_dirty: false,
                last_fullscreen_scanout: None,
                expose_last_active: None,
                switcher_last_active: None,
                overlay_last_active: None,
                overlay_was_active: false,
                switcher_was_active: false,
                promote_candidates: Vec::new(),
                promote_since: None,
                was_force_composite: false,
                composite_hold_until: None,
                transition_dump_left: 0,
                windows_dmabuf_element: None,
                expose_dmabuf_element: None,
                overlay_dmabuf_element: None,
                switcher_dmabuf_element: None,
                dock_dmabuf_element: None,
                last_frame_mode: super::types::FrameMode::Planes,
                planes_enabled,
                shadow_only_windows: Vec::new(),
                #[cfg(feature = "renderer_sync")]
                pending_gpu_fence: SyncPoint::signaled(),
            };

            let device = self.backend_data.backends.get_mut(&node).unwrap();
            device.surfaces.insert(crtc, surface_data);

            self.schedule_initial_render(node, crtc, self.handle.clone());
        }
    }

    /// Creates a surface compositor and returns the available overlay plane
    /// count (after driver-specific filtering). The caller uses the count to
    /// decide whether the per-purpose plane decomposition is worth enabling
    /// for this output (`SurfaceData::planes_enabled`).
    fn create_surface_compositor(
        &mut self,
        node: DrmNode,
        surface: smithay::backend::drm::DrmSurface,
        allocator: GbmAllocator<DrmDeviceFd>,
        color_formats: &[smithay::backend::allocator::Fourcc],
        render_formats: smithay::backend::allocator::format::FormatSet,
        output: &Output,
    ) -> Option<(GbmDrmCompositor, usize)> {
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

        let overlay_count = planes.overlay.len();
        tracing::info!(
            target: "otto::planes",
            "DRM overlay planes available for {}: {} (primary={}, cursor={})",
            output.name(),
            overlay_count,
            planes.primary.len(),
            planes.cursor.len(),
        );
        // Log supported formats/modifiers for primary and overlay planes.
        for (i, p) in planes.primary.iter().enumerate() {
            let fmts: Vec<_> = p.formats.iter().map(|f| format!("{:?}+{:?}", f.code, f.modifier)).collect();
            tracing::debug!(target: "otto::planes", "primary[{i}] formats: {fmts:?}");
        }
        for (i, p) in planes.overlay.iter().enumerate() {
            let fmts: Vec<_> = p.formats.iter().map(|f| format!("{:?}+{:?}", f.code, f.modifier)).collect();
            tracing::debug!(target: "otto::planes", "overlay[{i}] formats: {fmts:?}");
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
        Some((compositor, overlay_count))
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
                // Re-pack the remaining outputs and shrink the scene root /
                // fallback composite back to their union.
                self.workspaces.relayout_outputs();
                sync_scene_size_to_outputs(&self.workspaces, &mut self.scene_element);
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

        // Check if any external monitor is connected (clamshell mode blocks suspend)
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

        if disable_laptop_panels && self.is_lid_closed {
            if has_external_monitor {
                tracing::info!(
                    "Lid closed with external monitor - disabling laptop panel (clamshell mode)"
                );
            } else {
                tracing::info!("Lid closed without external monitor - disabling laptop panel");
            }
        }

        // Collect outputs to disconnect/reconnect
        let mut to_disconnect = vec![];
        let mut to_reconnect = vec![];

        if disable_laptop_panels {
            // Lid is closed - disconnect laptop panels that are currently active
            for (&node, device) in &self.backend_data.backends {
                for &crtc in device.surfaces.keys() {
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

        // Suspend laptop panels that should be disabled.
        // Unlike a real connector disconnect, we only tear down the DRM surface
        // (stops rendering and drops the Wayland global) but keep all workspace
        // data intact so windows survive the lid-close/reopen cycle.
        let suspended_any_panel = !to_disconnect.is_empty();
        for (node, crtc) in to_disconnect {
            let device = match self.backend_data.backends.get_mut(&node) {
                Some(d) => d,
                None => continue,
            };

            // Find the output before removing the surface (which drops the Wayland global).
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

            device.surfaces.remove(&crtc);

            if let Some(output) = output {
                self.workspaces.suspend_output(&output);
            }
        }

        // Reconnect laptop panels that should be re-enabled
        for (node, connector, crtc) in to_reconnect {
            self.connector_connected(node, connector, crtc);
        }

        // Lid closed on a plain laptop ("auto" mode, no external monitor):
        // Otto owns the suspend decision — logind's lid handling is expected
        // to be `ignore` so we can gate it. Skip while a remote client is
        // consuming frames (RDP bridge / screenshare), so closing the lid
        // during a remote session keeps serving instead of going to sleep.
        // Edge-triggered on the panel teardown so a repeated call (or a
        // wake-up with the lid still closed) doesn't immediately re-suspend.
        if suspended_any_panel
            && self.is_lid_closed
            && matches!(lid_action, LidCloseAction::Auto)
            && !has_external_monitor
        {
            let screenshare_active = !self.screenshare_sessions.is_empty();
            let remote_streaming = self
                .virtual_outputs
                .iter()
                .any(|v| v.pipewire_stream.is_streaming());

            if screenshare_active || remote_streaming {
                tracing::info!(
                    screenshare_active,
                    remote_streaming,
                    "Lid closed - NOT suspending, remote session active"
                );
            } else {
                tracing::info!("Lid closed - suspending via logind (systemctl suspend)");
                if let Err(err) = std::process::Command::new("systemctl")
                    .arg("suspend")
                    .spawn()
                {
                    tracing::error!("Failed to invoke systemctl suspend: {err}");
                }
            }
        }
    }
}
