//! Skia surface wrappers for GPU-backed rendering.
//!
//! This module provides wrappers around Skia's GPU surfaces, which are backed
//! by OpenGL framebuffers or textures. Each surface has its own GPU context
//! for rendering operations.

use layers::skia;
use smithay::backend::renderer::gles::ffi;

/// A GPU-backed Skia rendering surface.
///
/// Wraps a Skia DirectContext and Surface, allowing drawing operations
/// to be performed with the Skia canvas API. The surface is backed by
/// either an OpenGL framebuffer or texture.
#[derive(Clone)]
pub struct SkiaSurface {
    pub gr_context: skia::gpu::DirectContext,
    pub surface: skia::Surface,
}

impl SkiaSurface {
    /// Returns a reference to the Skia canvas for drawing operations.
    pub fn canvas(&mut self) -> &skia::Canvas {
        self.surface.canvas()
    }

    /// Creates a new Skia surface backed by an OpenGL framebuffer.
    ///
    /// # Parameters
    ///
    /// - `width`, `height`: Surface dimensions in pixels
    /// - `sample_count`: MSAA sample count (1 for no antialiasing)
    /// - `stencil_bits`: Number of stencil buffer bits
    /// - `fboid`: OpenGL framebuffer object ID
    /// - `color_type`: Pixel format (e.g., RGBA8888)
    /// - `context`: Optional existing GPU context to share (creates new if None)
    /// - `origin`: Coordinate system origin (TopLeft or BottomLeft)
    /// - `gl_internal_format`: OpenGL internal format constant
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_fbo(
        width: impl Into<i32>,
        height: impl Into<i32>,
        sample_count: impl Into<usize>,
        stencil_bits: impl Into<usize>,
        fboid: impl Into<u32>,
        color_type: skia::ColorType,
        context: Option<&skia::gpu::DirectContext>,
        origin: skia::gpu::SurfaceOrigin,
        gl_internal_format: u32,
    ) -> Self {
        let fb_info = {
            skia::gpu::gl::FramebufferInfo {
                fboid: fboid.into(),
                format: gl_internal_format,
                ..Default::default()
            }
        };
        let backend_render_target = skia::gpu::backend_render_targets::make_gl(
            (width.into(), height.into()),
            sample_count.into(),
            stencil_bits.into(),
            fb_info,
        );

        let mut gr_context: skia::gpu::DirectContext = if let Some(context) = context {
            context.clone()
        } else {
            let interface = skia::gpu::gl::Interface::new_native().unwrap();
            skia::gpu::direct_contexts::make_gl(interface, None).unwrap()
        };
        gr_context.reset(None);
        let surface = skia::gpu::surfaces::wrap_backend_render_target(
            &mut gr_context,
            &backend_render_target,
            origin,
            color_type,
            None,
            Some(&skia::SurfaceProps::new(
                Default::default(),
                skia::PixelGeometry::BGRH, // for font rendering optimisations
            )),
        )
        .unwrap();

        Self {
            gr_context,
            surface,
        }
    }

    /// Creates a new Skia surface backed by an OpenGL texture.
    ///
    /// # Parameters
    ///
    /// - `width`, `height`: Surface dimensions in pixels
    /// - `sample_cnt`: MSAA sample count (1 for no antialiasing)
    /// - `texid`: OpenGL texture ID
    /// - `color_type`: Pixel format (e.g., RGBA8888)
    /// - `context`: Optional existing GPU context to share (creates new if None)
    /// - `origin`: Coordinate system origin (TopLeft or BottomLeft)
    ///
    /// # Safety
    ///
    /// The texture ID must be valid and compatible with the current GL context.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_texture(
        width: impl Into<i32>,
        height: impl Into<i32>,
        sample_cnt: impl Into<usize>,
        texid: impl Into<u32>,
        color_type: skia::ColorType,
        context: Option<&skia::gpu::DirectContext>,
        origin: skia::gpu::SurfaceOrigin,
    ) -> Self {
        let sample_cnt = sample_cnt.into();
        let gl_info = skia::gpu::gl::TextureInfo {
            target: ffi::TEXTURE_2D,
            id: texid.into(),
            format: skia::gpu::gl::Format::RGBA8.into(),
            ..Default::default()
        };
        let backend_texture = unsafe {
            skia::gpu::backend_textures::make_gl(
                (width.into(), height.into()),
                skia::gpu::Mipmapped::No,
                gl_info,
                "",
            )
        };
        let mut gr_context: skia::gpu::DirectContext = if let Some(context) = context {
            context.clone()
        } else {
            let interface = skia::gpu::gl::Interface::new_native().unwrap();
            skia::gpu::direct_contexts::make_gl(interface, None).unwrap()
        };
        gr_context.reset(None);
        let surface = skia::gpu::surfaces::wrap_backend_texture(
            &mut gr_context,
            &backend_texture,
            origin,
            sample_cnt,
            color_type,
            None,
            Some(&skia::SurfaceProps::new(
                Default::default(),
                skia::PixelGeometry::BGRH, // for font rendering optimisations
            )),
        )
        .unwrap();

        Self {
            gr_context,
            surface,
        }
    }

    /// Returns a clone of the underlying Skia surface.
    pub fn surface(&self) -> skia::Surface {
        self.surface.clone()
    }
}
