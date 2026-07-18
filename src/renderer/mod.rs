//! Skia-based GPU renderer for the Otto compositor.
//!
//! This module provides a hardware-accelerated rendering backend that combines
//! Smithay's OpenGL renderer with Skia's 2D graphics library. This allows the
//! compositor to use both efficient buffer management from Smithay and advanced
//! drawing capabilities from Skia.
//!
//! # Architecture
//!
//! - `egl_context`: EGL surface wrappers for use in collections
//! - `sync`: GPU synchronization using EGL fences
//! - `skia_surface`: Skia surface creation and management
//! - `textures`: Texture types combining OpenGL and Skia
//! - `frame`: Frame rendering implementations for SkiaFrame
//!
//! The main `SkiaRenderer` in the parent module orchestrates these components.

pub mod egl_context;
pub mod frame;
pub mod skia_surface;
pub mod sync;
pub mod textures;

// Re-export commonly used types
pub use egl_context::EGLSurfaceWrapper;
pub use skia_surface::SkiaSurface;
pub use sync::SkiaSync;
pub use textures::{SkiaFrame, SkiaGLesFbo, SkiaTexture, SkiaTextureImage, SkiaTextureMapping};

use smithay::{
    backend::renderer::gles::{ffi, GlesError},
    utils::{Physical, Rectangle},
};

use crate::{
    skia_renderer::{SkiaRenderer, SkiaTarget},
    udev::UdevRenderer,
};

/// Trait for blitting the currently bound framebuffer to a destination dmabuf
pub trait BlitCurrentFrame {
    type Error: std::error::Error;

    /// Blit the current framebuffer content to a destination dmabuf
    ///
    /// This is useful for screenshare scenarios where you want to copy
    /// the already-rendered output to additional buffers.
    fn blit_current_frame(
        &mut self,
        dst_dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), Self::Error>;
}

impl crate::renderer::BlitCurrentFrame for SkiaRenderer {
    type Error = GlesError;

    #[profiling::function]
    fn blit_current_frame(
        &mut self,
        dst_dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), Self::Error> {
        use smithay::backend::renderer::Bind;

        // Get the currently bound source FBO
        let src_fbo = self.get_current_fbo()?.fbo;

        // Remember the caller's bound target: `bind(dst_dmabuf)` below switches
        // the renderer to the screencast buffer, and if we leave it there the
        // next DRM atomic commit exports its in-fence with the wrong target
        // current — EGL returns BAD_PARAMETER (`eglDupNativeFenceFDANDROID`),
        // which surfaces as ContextLost and used to crash the compositor.
        let prev_target = self.current_target.clone();

        // Bind the destination dmabuf to get its FBO
        self.bind(dst_dmabuf)?;
        let dst_target = SkiaTarget::Dmabuf(dst_dmabuf.weak());

        let dst_fbo = self
            .buffers
            .get(&dst_target)
            .ok_or(GlesError::FramebufferBindingError)?
            .fbo;

        // Direct FBO-to-FBO blit (GPU only)
        unsafe {
            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, src_fbo);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst_fbo);

            self.gl.BlitFramebuffer(
                src.loc.x,
                src.loc.y,
                src.loc.x + src.size.w,
                src.loc.y + src.size.h,
                dst.loc.x,
                dst.loc.y,
                dst.loc.x + dst.size.w,
                dst.loc.y + dst.size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );

            self.gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
            self.gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, 0);
        }

        // Restore the caller's target so the renderer is left exactly as we
        // found it (see `prev_target` above).
        self.current_target = prev_target;

        Ok(())
    }
}

impl BlitCurrentFrame for UdevRenderer<'_> {
    type Error = GlesError;

    #[profiling::function]
    fn blit_current_frame(
        &mut self,
        dst_dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
        src: Rectangle<i32, Physical>,
        dst: Rectangle<i32, Physical>,
    ) -> Result<(), Self::Error> {
        let renderer = self.as_mut();
        renderer.blit_current_frame(dst_dmabuf, src, dst)?;
        Ok(())
    }
}
