// Udev backend module - DRM/KMS compositor implementation
//
// This module implements the production backend for Otto using DRM/KMS for display
// and libinput for input handling.

pub mod device;
pub mod feedback;
pub mod gamma;
pub mod init;
pub mod render;
pub mod types;

// Re-export public API
pub use init::run_udev;

// Re-export public types
pub use types::{
    DeviceAddError, RenderOutcome, UdevData, UdevOutputId, UdevRenderer, SUPPORTED_FORMATS,
    SUPPORTED_FORMATS_8BIT_ONLY,
};

use crate::renderer::{SkiaTexture, SkiaTextureImage};
use crate::{
    skia_renderer::SkiaRenderer,
    state::{Backend, Otto},
};

#[cfg(feature = "fps_ticker")]
use smithay::backend::renderer::ImportMem;
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        drm::{DrmDevice, DrmDeviceFd, DrmNode},
        renderer::{
            multigpu::{gbm::GbmGlesBackend, MultiTexture},
            utils::import_surface,
            ImportDma,
        },
        session::{libseat::LibSeatSession, Session},
        udev::UdevBackend,
    },
    delegate_dmabuf, delegate_drm_lease,
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::{
        drm::control::{Device as ControlDevice, ModeTypeFlags},
        rustix::fs::OFlags,
        wayland_server::protocol::wl_surface,
    },
    utils::DeviceFd,
    wayland::{
        compositor,
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_lease::{
            DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState,
            LeaseRejected,
        },
    },
};

impl DmabufHandler for Otto<UdevData> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self
            .backend_data
            .gpus
            .single_renderer(&self.backend_data.primary_gpu)
            .and_then(|mut renderer| renderer.import_dmabuf(&dmabuf, None))
            .is_ok()
        {
            let _ = notifier.successful::<Otto<UdevData>>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(Otto<UdevData>);

impl Backend for UdevData {
    const HAS_RELATIVE_MOTION: bool = true;
    const HAS_GESTURES: bool = true;

    fn seat_name(&self) -> String {
        self.session.seat()
    }

    fn backend_name(&self) -> &'static str {
        "udev"
    }

    fn reset_buffers(&mut self, output: &Output) {
        if let Some(id) = output.user_data().get::<UdevOutputId>() {
            if let Some(gpu) = self.backends.get_mut(&id.device_id) {
                if let Some(surface) = gpu.surfaces.get_mut(&id.crtc) {
                    surface.compositor.reset_buffers();
                }
            }
        }
    }

    fn early_import(&mut self, surface: &wl_surface::WlSurface) {
        if let Err(err) = self.gpus.early_import(self.primary_gpu, surface) {
            tracing::warn!("Early buffer import failed: {}", err);
        }
        let mut r = self.gpus.single_renderer(&self.primary_gpu).unwrap();
        compositor::with_states(surface, |states| {
            if let Err(err) = import_surface(&mut r, states) {
                tracing::warn!("Early buffer import surface failed: {}", err);
            }
        });
    }

    fn texture_for_surface(
        &self,
        surface: &smithay::backend::renderer::utils::RendererSurfaceState,
    ) -> Option<SkiaTextureImage> {
        let id = self.context_id.as_ref().cloned()?;
        let tex = surface.texture::<MultiTexture>(id.clone());
        if let Some(multitexture) = tex {
            // Convert ContextId<MultiTexture> to ContextId<SkiaTexture>
            let skia_id: smithay::backend::renderer::ContextId<SkiaTexture> = id.map();
            let texture = multitexture.get::<GbmGlesBackend<SkiaRenderer, DrmDeviceFd>>(&skia_id);
            return texture.map(|t| t.into());
        }
        None
    }
    fn set_cursor(&mut self, _image: &CursorImageStatus) {
        // No-op: cursor rendering handled directly in render_surface
    }
    fn renderer_context(&mut self) -> Option<layers::skia::gpu::DirectContext> {
        let r = self.gpus.single_renderer(&self.primary_gpu).unwrap();
        let r = r.as_ref();
        r.context.clone()
    }

    fn gbm_device(
        &self,
    ) -> Option<smithay::backend::allocator::gbm::GbmDevice<smithay::backend::drm::DrmDeviceFd>>
    {
        // Get the GBM device from any available backend
        // The primary_gpu might be a render node, but backends are keyed by primary nodes
        tracing::debug!(
            "gbm_device() called: primary_gpu={:?}, backends.len()={}",
            self.primary_gpu,
            self.backends.len()
        );

        // Try to get from primary_gpu first
        if let Some(backend) = self.backends.get(&self.primary_gpu) {
            return Some(backend.gbm.clone());
        }

        // Fallback: get from any backend (usually there's only one)
        if let Some((_key, backend)) = self.backends.iter().next() {
            tracing::debug!("Using GBM device from first available backend");
            return Some(backend.gbm.clone());
        }

        None
    }

    fn render_format(&mut self) -> Option<(u32, u64)> {
        // Get the renderer and query its render formats
        let renderer = self.gpus.single_renderer(&self.primary_gpu).ok()?;
        let formats = renderer.dmabuf_formats();

        // Find ARGB8888 or XRGB8888 format (common render formats)
        let argb = smithay::backend::allocator::Fourcc::Argb8888;
        let xrgb = smithay::backend::allocator::Fourcc::Xrgb8888;

        // Prefer ARGB8888, fall back to XRGB8888
        let format = formats
            .iter()
            .find(|f| f.code == argb)
            .or_else(|| formats.iter().find(|f| f.code == xrgb))?;

        Some((format.code as u32, format.modifier.into()))
    }

    fn get_format_modifiers(&mut self, fourcc: smithay::backend::allocator::Fourcc) -> Vec<u64> {
        // Get all modifiers supported for the given format
        let renderer = match self.gpus.single_renderer(&self.primary_gpu) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        renderer
            .dmabuf_formats()
            .iter()
            .filter(|f| f.code == fourcc)
            .map(|f| f.modifier.into())
            .collect()
    }

    fn prefers_dmabuf_screenshare(&self) -> bool {
        // Udev backend supports DMA-BUF for zero-copy screenshare
        true
    }
}

impl DrmLeaseHandler for Otto<UdevData> {
    fn drm_lease_state(&mut self, node: DrmNode) -> &mut DrmLeaseState {
        self.backend_data
            .backends
            .get_mut(&node)
            .unwrap()
            .leasing_global
            .as_mut()
            .unwrap()
    }

    fn lease_request(
        &mut self,
        node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let backend = self
            .backend_data
            .backends
            .get(&node)
            .ok_or(LeaseRejected::default())?;

        let mut builder = DrmLeaseBuilder::new(&backend.drm);
        for conn in request.connectors {
            if let Some((_, crtc)) = backend
                .non_desktop_connectors
                .iter()
                .find(|(handle, _)| *handle == conn)
            {
                builder.add_connector(conn);
                builder.add_crtc(*crtc);
                let planes = backend
                    .drm
                    .planes(crtc)
                    .map_err(LeaseRejected::with_cause)?;
                let (primary_plane, primary_plane_claim) = planes
                    .primary
                    .iter()
                    .find_map(|plane| {
                        backend
                            .drm
                            .claim_plane(plane.handle, *crtc)
                            .map(|claim| (plane, claim))
                    })
                    .ok_or_else(LeaseRejected::default)?;
                builder.add_plane(primary_plane.handle, primary_plane_claim);
                if let Some((cursor, claim)) = planes.cursor.iter().find_map(|plane| {
                    backend
                        .drm
                        .claim_plane(plane.handle, *crtc)
                        .map(|claim| (plane, claim))
                }) {
                    builder.add_plane(cursor.handle, claim);
                }
            } else {
                tracing::warn!(
                    ?conn,
                    "Lease requested for desktop connector, denying request"
                );
                return Err(LeaseRejected::default());
            }
        }

        Ok(builder)
    }

    fn new_active_lease(&mut self, node: DrmNode, lease: DrmLease) {
        let backend = self.backend_data.backends.get_mut(&node).unwrap();
        backend.active_leases.push(lease);
    }

    fn lease_destroyed(&mut self, node: DrmNode, lease: u32) {
        let backend = self.backend_data.backends.get_mut(&node).unwrap();
        backend.active_leases.retain(|l| l.id() != lease);
    }
}

delegate_drm_lease!(Otto<UdevData>);

pub fn probe_displays() {
    #[allow(clippy::disallowed_macros)]
    {
        println!("Probing available displays and resolutions...\n");
    }

    let (mut session, _notifier) = match LibSeatSession::new() {
        Ok(ret) => ret,
        Err(err) => {
            tracing::error!("Could not initialize a session: {}", err);
            #[allow(clippy::disallowed_macros)]
            {
                eprintln!("Error: Could not initialize session - {}", err);
                eprintln!(
                    "Note: This command may require root privileges or proper seat permissions."
                );
            }
            return;
        }
    };

    let udev_backend = match UdevBackend::new(session.seat()) {
        Ok(ret) => ret,
        Err(err) => {
            tracing::error!("Failed to initialize udev backend: {:?}", err);
            #[allow(clippy::disallowed_macros)]
            {
                eprintln!("Error: Failed to initialize udev backend - {:?}", err);
            }
            return;
        }
    };

    let mut found_displays = false;

    for (device_id, path) in udev_backend.device_list() {
        let node = match DrmNode::from_dev_id(device_id) {
            Ok(node) => node,
            Err(err) => {
                tracing::warn!("Failed to get DRM node for device {}: {}", device_id, err);
                continue;
            }
        };

        #[allow(clippy::disallowed_macros)]
        {
            println!("Device: {} ({:?})", node, path);
        }

        let fd = match session.open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        ) {
            Ok(fd) => DrmDeviceFd::new(DeviceFd::from(fd)),
            Err(err) => {
                tracing::warn!("Failed to open DRM device: {}", err);
                continue;
            }
        };

        let (drm_device, _notifier) = match DrmDevice::new(fd.clone(), true) {
            Ok(device) => device,
            Err(err) => {
                tracing::warn!("Failed to create DRM device: {:?}", err);
                continue;
            }
        };

        let res_handles = match drm_device.resource_handles() {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!("Failed to get resource handles: {:?}", err);
                continue;
            }
        };

        for connector_handle in res_handles.connectors() {
            let connector_info = match drm_device.get_connector(*connector_handle, false) {
                Ok(info) => info,
                Err(err) => {
                    tracing::warn!("Failed to get connector info: {:?}", err);
                    continue;
                }
            };

            if connector_info.state()
                != smithay::reexports::drm::control::connector::State::Connected
            {
                continue;
            }

            found_displays = true;

            let connector_name = format!(
                "{}-{}",
                connector_info.interface().as_str(),
                connector_info.interface_id()
            );

            #[allow(clippy::disallowed_macros)]
            {
                println!("\n  Connector: {}", connector_name);
                println!("  Status: Connected");
                println!("  Available modes:");
            }

            for mode in connector_info.modes() {
                let refresh = (mode.vrefresh() as f32) / 1000.0;
                #[allow(clippy::disallowed_macros)]
                {
                    println!(
                        "    {}x{} @ {:.2} Hz{}",
                        mode.size().0,
                        mode.size().1,
                        refresh,
                        if mode.mode_type().contains(ModeTypeFlags::PREFERRED) {
                            " (preferred)"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
    }

    if !found_displays {
        #[allow(clippy::disallowed_macros)]
        {
            println!("\nNo connected displays found.");
        }
    }
}
