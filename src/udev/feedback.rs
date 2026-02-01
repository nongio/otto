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
    let primary_formats = gpus.single_renderer(&primary_gpu).ok()?.dmabuf_formats();
    let render_formats = gpus.single_renderer(&render_node).ok()?.dmabuf_formats();

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
