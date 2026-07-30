use smithay::{
    backend::{
        allocator::format::FormatSet,
        drm::DrmNode,
        renderer::{
            multigpu::{gbm::GbmGlesBackend, GpuManager},
            ImportDma,
        },
    },
    reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1,
    wayland::dmabuf::DmabufFeedbackBuilder,
};

use crate::skia_renderer::SkiaRenderer;

use super::types::{DrmSurfaceDmabufFeedback, GbmDrmCompositor};

/// Intel "clear color" CCS modifiers (`RC_CCS_CC`) carry an extra clear-color
/// value plane (3 planes total). Otto's Skia dmabuf import samples them as pure
/// black (the 2-plane `RC_CCS` variants render fine). Dropping these from the
/// formats we advertise makes clients fall back to the 2-plane modifiers Otto
/// renders correctly — notably DXVK/Proton fullscreen swapchains (e.g. Cuphead),
/// which otherwise pick `Y_TILED_GEN12_RC_CCS_CC` and show a black screen.
const CLEAR_COLOR_MODIFIERS: &[u64] = &[
    0x0100_0000_0000_0008, // I915_FORMAT_MOD_Y_TILED_GEN12_RC_CCS_CC
    0x0100_0000_0000_000c, // I915_FORMAT_MOD_4_TILED_DG2_RC_CCS_CC
    0x0100_0000_0000_000f, // I915_FORMAT_MOD_4_TILED_MTL_RC_CCS_CC
];

/// Removes the clear-color CCS modifiers (see [`CLEAR_COLOR_MODIFIERS`]) from a
/// set of dmabuf formats before it is advertised to clients.
pub fn strip_clear_color_modifiers(formats: FormatSet) -> FormatSet {
    formats
        .into_iter()
        .filter(|f| !CLEAR_COLOR_MODIFIERS.contains(&u64::from(f.modifier)))
        .collect()
}

/// Constructs dmabuf feedback for a surface
///
/// Creates two feedback objects:
/// - `render_feedback`: For general rendering operations
/// - `scanout_feedback`: Optimized for direct scanout with format preferences
///
/// The scanout feedback is limited to formats that can also be rendered to,
/// ensuring a fallback render path exists if direct scanout fails.
pub fn get_surface_dmabuf_feedback(
    primary_gpu: DrmNode,
    render_node: DrmNode,
    gpus: &mut GpuManager<GbmGlesBackend<SkiaRenderer, smithay::backend::drm::DrmDeviceFd>>,
    composition: &GbmDrmCompositor,
) -> Option<DrmSurfaceDmabufFeedback> {
    let primary_formats =
        strip_clear_color_modifiers(gpus.single_renderer(&primary_gpu).ok()?.dmabuf_formats());
    let render_formats =
        strip_clear_color_modifiers(gpus.single_renderer(&render_node).ok()?.dmabuf_formats());

    let all_render_formats = primary_formats
        .iter()
        .chain(render_formats.iter())
        .copied()
        .collect::<FormatSet>();

    let surface = composition.surface();
    let planes = surface.planes().clone();

    // We limit the scan-out tranche to formats we can also render from
    // so that there is always a fallback render path available in case
    // the supplied buffer can not be scanned out directly
    let planes_formats = surface
        .plane_info()
        .formats
        .iter()
        .copied()
        .chain(planes.overlay.into_iter().flat_map(|p| p.formats))
        .collect::<FormatSet>()
        .intersection(&all_render_formats)
        .copied()
        .collect::<FormatSet>();

    let builder = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), primary_formats);
    let render_feedback = builder
        .clone()
        .add_preference_tranche(render_node.dev_id(), None, render_formats.clone())
        .build()
        .unwrap();

    let scanout_feedback = builder
        .add_preference_tranche(
            surface.device_fd().dev_id().unwrap(),
            Some(zwp_linux_dmabuf_feedback_v1::TrancheFlags::Scanout),
            planes_formats,
        )
        .add_preference_tranche(render_node.dev_id(), None, render_formats)
        .build()
        .unwrap();

    Some(DrmSurfaceDmabufFeedback {
        render_feedback,
        scanout_feedback,
    })
}
