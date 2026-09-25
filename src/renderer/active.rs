//! The renderers the udev backend can run on, chosen at startup.
//!
//! [`RendererApi`] names everything the udev backend needs from a renderer:
//! the Smithay multi-GPU api, the per-device renderer, its texture and error
//! types, and the multi-GPU renderer the render loop draws with. The udev
//! backend, its render elements and screenshare are generic over it.
//! [`GlApi`] runs Skia on GL through Smithay's `GbmGlesBackend`; with the
//! `vulkan` feature, [`VulkanApi`] runs Skia on Vulkan. `main` picks one
//! from `--renderer` or `[rendering] renderer` in the config. winit and x11
//! use the GL renderer directly.

// Rust guideline compliant 2026-02-21

use std::any::Any;

use layers::skia::gpu::DirectContext;
use smithay::backend::{
    allocator::{dmabuf::Dmabuf, format::FormatSet, gbm::GbmDevice},
    drm::{DrmDeviceFd, DrmNode},
    renderer::{
        multigpu::{
            gbm::GbmGlesBackend, ApiDevice, Error as MultiError, GpuManager, GraphicsApi,
            MultiRenderer, MultiTexture,
        },
        Bind, ExportMem, ImportAll, ImportDma, ImportDmaWl, ImportMem, ImportMemWl, Renderer,
        RendererSuper, Texture,
    },
    SwapBuffersError,
};

use crate::{
    renderer::{BlitCurrentFrame, FrameSurface, SkiaSurface, SkiaTextureImage},
    skia_renderer::SkiaRenderer,
};

/// Multi-GPU renderer of the udev backend on the renderer `A`.
pub type UdevRenderer<'a, A> = <A as RendererApi>::Multi<'a>;

/// Error of the multi-GPU renderer on the renderer `A`.
pub type UdevRendererError<A> =
    MultiError<<A as RendererApi>::GraphicsApi, <A as RendererApi>::GraphicsApi>;

/// A renderer the udev backend can run on.
///
/// Implemented by zero-sized markers; the udev backend is generic over it
/// (`UdevData<A>`), so each renderer gets its own monomorphised backend and
/// the choice between them is made once, in `main`.
pub trait RendererApi: Sized + 'static {
    /// Smithay multi-GPU api creating one [`Self::Renderer`] per DRM node.
    type GraphicsApi: GraphicsApi<
            Device: ApiDevice<Renderer = Self::Renderer>,
            Error: Into<SwapBuffersError> + Send + Sync + 'static,
        > + 'static;
    /// Renderer of one device.
    type Renderer: SkiaDeviceRenderer
        + RendererSuper<TextureId = Self::Texture, Error = Self::Error>
        + Renderer
        + Bind<Dmabuf>
        + ImportDma
        + ImportDmaWl
        + ImportMem
        + ImportMemWl
        + ExportMem<TextureMapping: 'static>
        + 'static;
    /// Texture of [`Self::Renderer`].
    type Texture: Texture + Clone + Send + Into<SkiaTextureImage> + 'static;
    /// Error of [`Self::Renderer`].
    type Error: std::error::Error + Into<SwapBuffersError> + Send + Sync + 'static;
    /// Error of adding a DRM node to [`Self::GraphicsApi`].
    type AddNodeError: std::error::Error + Send + Sync + 'static;
    /// Multi-GPU renderer the render loop draws with.
    ///
    /// Always `MultiRenderer<'a, 'a, Self::GraphicsApi, Self::GraphicsApi>`;
    /// naming it here states the traits the udev backend relies on once,
    /// instead of on every function that renders.
    type Multi<'a>: Renderer<TextureId = MultiTexture, Error = UdevRendererError<Self>>
        + FrameSurface
        + ImportAll
        + ImportDma
        + ImportMem
        + ImportMemWl
        + ExportMem
        + Bind<Dmabuf>
        + AsRef<Self::Renderer>
        + AsMut<Self::Renderer>
        + BlitCurrentFrame<Error = Self::Error>;

    /// Name of the renderer, as `--renderer` and the config spell it.
    const NAME: &'static str;
    /// Whether outputs may use the per-purpose KMS plane decomposition.
    ///
    /// False on Vulkan: its plane slots are not fenced for scanout yet, so
    /// every output composites into the primary plane.
    const PLANES_SUPPORTED: bool;

    /// Creates the multi-GPU api the udev backend starts with.
    fn graphics_api() -> Self::GraphicsApi;

    /// The render node that renders for the DRM device `node`, or `node`
    /// itself when none is found.
    fn render_node_of(node: DrmNode, gbm: &GbmDevice<DrmDeviceFd>) -> DrmNode;

    /// Adds the device `node`, allocating with `gbm`, to `api`.
    ///
    /// # Errors
    ///
    /// Fails when no renderer can be created for the device.
    fn add_node(
        api: &mut Self::GraphicsApi,
        node: DrmNode,
        gbm: GbmDevice<DrmDeviceFd>,
    ) -> Result<(), Self::AddNodeError>;

    /// Removes the device `node` from `api`.
    fn remove_node(api: &mut Self::GraphicsApi, node: &DrmNode);

    /// The renderer of the single device `node`.
    ///
    /// # Errors
    ///
    /// Fails when `node` has no renderer.
    fn single_renderer<'a>(
        gpus: &'a mut GpuManager<Self::GraphicsApi>,
        node: &DrmNode,
    ) -> Result<Self::Multi<'a>, UdevRendererError<Self>>;

    /// A renderer drawing on `render` for a buffer scanned out by `target`.
    ///
    /// # Errors
    ///
    /// Fails when either node has no renderer.
    fn renderer<'a>(
        gpus: &'a mut GpuManager<Self::GraphicsApi>,
        render: &DrmNode,
        target: &DrmNode,
        copy_format: smithay::backend::allocator::Fourcc,
    ) -> Result<Self::Multi<'a>, UdevRendererError<Self>>;

    /// Lets clients hand over EGL buffers (`wl_drm`), where the renderer can
    /// import them.
    ///
    /// # Errors
    ///
    /// Fails when the renderer cannot import EGL buffers.
    #[cfg(feature = "egl")]
    fn bind_wl_display(
        renderer: &mut Self::Multi<'_>,
        display: &smithay::reexports::wayland_server::DisplayHandle,
    ) -> Result<(), smithay::backend::egl::Error>;
}

/// What the udev backend asks of a Skia renderer beyond Smithay's traits.
pub trait SkiaDeviceRenderer {
    /// Error of creating a plane surface.
    type SurfaceError: std::fmt::Debug;

    /// Wraps `dmabuf` as a Skia surface for a KMS plane slot.
    ///
    /// Returns the surface and a token that frees its GPU resources; drop
    /// the token after the surface.
    ///
    /// # Errors
    ///
    /// Fails when the renderer cannot render into the dmabuf's format.
    fn create_plane_surface(
        &mut self,
        dmabuf: &Dmabuf,
    ) -> Result<(SkiaSurface, Box<dyn Any>), Self::SurfaceError>;

    /// Formats the renderer can render into.
    fn render_formats(&self) -> FormatSet;

    /// Skia context the renderer draws with.
    fn skia_context(&self) -> Option<DirectContext>;

    /// Surface of the frame rendered last.
    fn current_surface(&mut self) -> Option<&SkiaSurface>;

    /// Records that this frame renders into `dmabuf` for scanout, so
    /// [`Self::flush_planes`] can fence that write for the KMS commit.
    fn note_scanout_write(&mut self, dmabuf: &Dmabuf);

    /// Makes the plane buffers rendered this frame safe to scan out.
    fn flush_planes(&mut self);
}

impl SkiaDeviceRenderer for SkiaRenderer {
    type SurfaceError = smithay::backend::renderer::gles::GlesError;

    fn create_plane_surface(
        &mut self,
        dmabuf: &Dmabuf,
    ) -> Result<(SkiaSurface, Box<dyn Any>), Self::SurfaceError> {
        let (surface, release) = self.create_surface_from_dmabuf(dmabuf)?;
        Ok((surface, Box::new(release)))
    }

    fn render_formats(&self) -> FormatSet {
        self.dmabuf_render_formats()
    }

    fn skia_context(&self) -> Option<DirectContext> {
        self.context.clone()
    }

    fn current_surface(&mut self) -> Option<&SkiaSurface> {
        self.current_skia_renderer()
    }

    fn note_scanout_write(&mut self, dmabuf: &Dmabuf) {
        self.note_scanout_write(dmabuf);
    }

    fn flush_planes(&mut self) {
        self.flush_planes_for_scanout();
    }
}

/// Skia on GL, through Smithay's `GbmGlesBackend`.
#[derive(Debug, Clone, Copy)]
pub struct GlApi;

/// Multi-GPU api of [`GlApi`].
type GlGraphicsApi = GbmGlesBackend<SkiaRenderer, DrmDeviceFd>;

impl RendererApi for GlApi {
    type GraphicsApi = GlGraphicsApi;
    type Renderer = SkiaRenderer;
    type Texture = crate::renderer::SkiaTexture;
    type Error = smithay::backend::renderer::gles::GlesError;
    type AddNodeError = smithay::backend::egl::Error;
    type Multi<'a> = MultiRenderer<'a, 'a, GlGraphicsApi, GlGraphicsApi>;

    const NAME: &'static str = "gl";
    const PLANES_SUPPORTED: bool = true;

    fn graphics_api() -> Self::GraphicsApi {
        GbmGlesBackend::with_context_priority(smithay::backend::egl::context::ContextPriority::High)
    }

    /// Asked of EGL, which also resolves devices without a render node of
    /// their own.
    fn render_node_of(node: DrmNode, gbm: &GbmDevice<DrmDeviceFd>) -> DrmNode {
        use smithay::backend::egl::{EGLDevice, EGLDisplay};

        // SAFETY: the display is only used to query its device.
        unsafe { EGLDisplay::new(gbm.clone()) }
            .ok()
            .and_then(|display| EGLDevice::device_for_display(&display).ok())
            .and_then(|device| device.try_get_render_node().ok().flatten())
            .unwrap_or(node)
    }

    fn add_node(
        api: &mut Self::GraphicsApi,
        node: DrmNode,
        gbm: GbmDevice<DrmDeviceFd>,
    ) -> Result<(), Self::AddNodeError> {
        api.add_node(node, gbm)
    }

    fn remove_node(api: &mut Self::GraphicsApi, node: &DrmNode) {
        api.remove_node(node);
    }

    fn single_renderer<'a>(
        gpus: &'a mut GpuManager<Self::GraphicsApi>,
        node: &DrmNode,
    ) -> Result<Self::Multi<'a>, UdevRendererError<Self>> {
        gpus.single_renderer(node)
    }

    fn renderer<'a>(
        gpus: &'a mut GpuManager<Self::GraphicsApi>,
        render: &DrmNode,
        target: &DrmNode,
        copy_format: smithay::backend::allocator::Fourcc,
    ) -> Result<Self::Multi<'a>, UdevRendererError<Self>> {
        gpus.renderer(render, target, copy_format)
    }

    #[cfg(feature = "egl")]
    fn bind_wl_display(
        renderer: &mut Self::Multi<'_>,
        display: &smithay::reexports::wayland_server::DisplayHandle,
    ) -> Result<(), smithay::backend::egl::Error> {
        use smithay::backend::renderer::ImportEgl;

        renderer.bind_wl_display(display)
    }
}

#[cfg(feature = "vulkan")]
pub use vulkan::VulkanApi;

#[cfg(feature = "vulkan")]
mod vulkan {
    use std::any::Any;

    use layers::skia::gpu::DirectContext;
    use smithay::backend::{
        allocator::{dmabuf::Dmabuf, format::FormatSet, gbm::GbmDevice},
        drm::{DrmDeviceFd, DrmNode, NodeType},
        renderer::multigpu::{GpuManager, MultiRenderer},
    };

    use super::{RendererApi, SkiaDeviceRenderer, UdevRendererError};
    use crate::{
        renderer::{
            vulkan::{SkiaVkError, SkiaVkRenderer, SkiaVkTexture},
            SkiaSurface,
        },
        udev::vulkan_api::{GbmVulkanBackend, GbmVulkanError},
    };

    /// Skia on Vulkan, through [`GbmVulkanBackend`].
    #[derive(Debug, Clone, Copy)]
    pub struct VulkanApi;

    impl SkiaDeviceRenderer for SkiaVkRenderer {
        type SurfaceError = SkiaVkError;

        fn create_plane_surface(
            &mut self,
            dmabuf: &Dmabuf,
        ) -> Result<(SkiaSurface, Box<dyn Any>), Self::SurfaceError> {
            let (surface, release) = self.create_surface_from_dmabuf(dmabuf)?;
            Ok((surface, Box::new(release)))
        }

        fn render_formats(&self) -> FormatSet {
            self.dmabuf_render_formats()
        }

        fn skia_context(&self) -> Option<DirectContext> {
            self.context.clone()
        }

        fn current_surface(&mut self) -> Option<&SkiaSurface> {
            self.current_skia_renderer()
        }

        /// The Vulkan renderer waits for its plane writes on the CPU in
        /// `flush_planes`, so there is nothing to record per buffer.
        fn note_scanout_write(&mut self, _dmabuf: &Dmabuf) {}

        fn flush_planes(&mut self) {
            self.flush_planes_for_scanout();
        }
    }

    impl RendererApi for VulkanApi {
        type GraphicsApi = GbmVulkanBackend;
        type Renderer = SkiaVkRenderer;
        type Texture = SkiaVkTexture;
        type Error = SkiaVkError;
        type AddNodeError = GbmVulkanError;
        type Multi<'a> = MultiRenderer<'a, 'a, GbmVulkanBackend, GbmVulkanBackend>;

        const NAME: &'static str = "vulkan";
        const PLANES_SUPPORTED: bool = false;

        fn graphics_api() -> Self::GraphicsApi {
            GbmVulkanBackend::default()
        }

        fn render_node_of(node: DrmNode, _gbm: &GbmDevice<DrmDeviceFd>) -> DrmNode {
            node.node_with_type(NodeType::Render)
                .and_then(Result::ok)
                .unwrap_or(node)
        }

        fn add_node(
            api: &mut Self::GraphicsApi,
            node: DrmNode,
            gbm: GbmDevice<DrmDeviceFd>,
        ) -> Result<(), Self::AddNodeError> {
            api.add_node(node, gbm)
        }

        fn remove_node(api: &mut Self::GraphicsApi, node: &DrmNode) {
            api.remove_node(node);
        }

        fn single_renderer<'a>(
            gpus: &'a mut GpuManager<Self::GraphicsApi>,
            node: &DrmNode,
        ) -> Result<Self::Multi<'a>, UdevRendererError<Self>> {
            gpus.single_renderer(node)
        }

        fn renderer<'a>(
            gpus: &'a mut GpuManager<Self::GraphicsApi>,
            render: &DrmNode,
            target: &DrmNode,
            copy_format: smithay::backend::allocator::Fourcc,
        ) -> Result<Self::Multi<'a>, UdevRendererError<Self>> {
            gpus.renderer(render, target, copy_format)
        }

        #[cfg(feature = "egl")]
        fn bind_wl_display(
            renderer: &mut Self::Multi<'_>,
            display: &smithay::reexports::wayland_server::DisplayHandle,
        ) -> Result<(), smithay::backend::egl::Error> {
            use smithay::backend::renderer::ImportEgl;

            renderer.bind_wl_display(display)
        }
    }
}
