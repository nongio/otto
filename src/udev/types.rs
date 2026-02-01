use std::collections::hash_map::HashMap;
#[cfg(feature = "metrics")]
use std::sync::Arc;

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::DrmCompositor, exporter::gbm::GbmFramebufferExporter, DrmDevice,
            DrmDeviceFd, DrmNode,
        },
        renderer::{
            multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer, MultiTexture},
            ContextId,
        },
        session::libseat::LibSeatSession,
    },
    desktop::utils::OutputPresentationFeedback,
    reexports::{
        calloop::RegistrationToken,
        drm::control::{connector, crtc},
        wayland_server::{backend::GlobalId, DisplayHandle},
    },
    utils::{Physical, Rectangle},
    wayland::{
        dmabuf::{DmabufFeedback, DmabufGlobal, DmabufState},
        drm_lease::DrmLeaseState,
    },
};
use smithay_drm_extras::drm_scanner::DrmScanner;

use crate::skia_renderer::SkiaRenderer;

// Supported pixel formats for rendering
// We pick ARGB2101010 (10-bit) or ARGB8888 (8-bit) as they are widely supported.
pub const SUPPORTED_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];

pub const SUPPORTED_FORMATS_8BIT_ONLY: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

/// Multi-GPU renderer type for udev backend
pub type UdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<SkiaRenderer, DrmDeviceFd>,
    GbmGlesBackend<SkiaRenderer, DrmDeviceFd>,
>;

/// DRM compositor using GBM allocation
pub type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

/// Unique identifier for a udev output (device + CRTC)
#[derive(Debug, PartialEq)]
pub struct UdevOutputId {
    pub device_id: DrmNode,
    pub crtc: crtc::Handle,
}

/// Main udev backend data
pub struct UdevData {
    pub session: LibSeatSession,
    pub(super) dh: DisplayHandle,
    pub(super) dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    pub(super) primary_gpu: DrmNode,
    pub(super) gpus: GpuManager<GbmGlesBackend<SkiaRenderer, DrmDeviceFd>>,
    pub(super) backends: HashMap<DrmNode, BackendData>,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps_texture: Option<smithay::backend::renderer::multigpu::MultiTexture>,
    pub context_id: Option<ContextId<MultiTexture>>,
}

/// Per-device backend data
pub struct BackendData {
    pub(super) surfaces: HashMap<crtc::Handle, SurfaceData>,
    pub(super) non_desktop_connectors: Vec<(connector::Handle, crtc::Handle)>,
    pub(super) leasing_global: Option<DrmLeaseState>,
    pub(super) active_leases: Vec<smithay::wayland::drm_lease::DrmLease>,
    pub(super) gbm: GbmDevice<DrmDeviceFd>,
    pub(super) drm: DrmDevice,
    pub(super) drm_scanner: DrmScanner,
    pub(super) render_node: DrmNode,
    pub(super) registration_token: RegistrationToken,
}

/// Per-surface rendering data
pub struct SurfaceData {
    pub(super) dh: DisplayHandle,
    pub(super) device_id: DrmNode,
    pub(super) render_node: DrmNode,
    pub(super) global: Option<GlobalId>,
    pub(super) compositor: GbmDrmCompositor,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps: fps_ticker::Fps,
    #[cfg(feature = "fps_ticker")]
    pub(super) fps_element:
        Option<crate::drawing::FpsElement<smithay::backend::renderer::multigpu::MultiTexture>>,
    pub(super) dmabuf_feedback: Option<DrmSurfaceDmabufFeedback>,
    /// Track whether we were in direct scanout mode on the previous frame
    /// Used to reset buffers when transitioning between modes
    pub(super) was_direct_scanout: bool,
    /// Rendering metrics
    #[cfg(feature = "metrics")]
    pub(super) render_metrics: Option<Arc<crate::render_metrics::RenderMetrics>>,
}

impl Drop for SurfaceData {
    fn drop(&mut self) {
        if let Some(global) = self.global.take() {
            self.dh
                .remove_global::<crate::state::Otto<UdevData>>(global);
        }
    }
}

/// Dmabuf feedback for a DRM surface (render vs scanout)
pub struct DrmSurfaceDmabufFeedback {
    pub render_feedback: DmabufFeedback,
    pub scanout_feedback: DmabufFeedback,
}

/// Error type for device addition
#[derive(Debug, thiserror::Error)]
pub enum DeviceAddError {
    #[error("Failed to open device using libseat: {0}")]
    DeviceOpen(smithay::backend::session::libseat::Error),
    #[error("Failed to initialize drm device: {0}")]
    DrmDevice(smithay::backend::drm::DrmError),
    #[error("Failed to initialize gbm device: {0}")]
    GbmDevice(std::io::Error),
    #[error("Failed to access drm node: {0}")]
    DrmNode(smithay::backend::drm::CreateDrmNodeError),
    #[error("Failed to add device to GpuManager: {0}")]
    AddNode(smithay::backend::egl::Error),
}

/// Outcome of a render operation
pub struct RenderOutcome {
    pub rendered: bool,
    /// Damage regions from the render.
    pub damage: Option<Vec<Rectangle<i32, Physical>>>,
}

impl RenderOutcome {
    pub fn skipped() -> Self {
        Self {
            rendered: false,
            damage: None,
        }
    }
}
