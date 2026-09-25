//! The renderer the udev backend runs on, chosen at compile time.
//!
//! Skia on GL through Smithay's `GbmGlesBackend` by default, Skia on Vulkan
//! with the `vulkan` feature. The udev backend, the render elements and
//! screenshare name these aliases rather than a renderer, so they compile
//! against either. winit and x11 use the GL renderer directly in every build.

// Rust guideline compliant 2026-02-21

#[cfg(not(feature = "vulkan"))]
mod imp {
    use smithay::backend::{
        drm::DrmDeviceFd,
        egl::context::ContextPriority,
        renderer::{gles::GlesError, multigpu::gbm::GbmGlesBackend},
    };

    pub use crate::{
        renderer::{
            SkiaFrame as Frame, SkiaSurface as Surface, SkiaSync as Sync, SkiaTexture as Texture,
        },
        skia_renderer::{PlaneTextureRelease, SkiaRenderer as Renderer},
    };

    /// Multi-GPU api of the udev renderer.
    pub type GraphicsApi = GbmGlesBackend<Renderer, DrmDeviceFd>;
    /// Error of the udev renderer.
    pub type Error = GlesError;
    /// Error of adding a DRM node to the [`GraphicsApi`].
    pub type AddNodeError = smithay::backend::egl::Error;

    /// Creates the multi-GPU api the udev backend starts with.
    pub fn graphics_api() -> GraphicsApi {
        GbmGlesBackend::with_context_priority(ContextPriority::High)
    }
}

pub use imp::*;
