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
    /// - `pixel_geometry`: Subpixel layout of the panel the framebuffer is
    ///   scanned out to; see [`surface_props`]
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
        pixel_geometry: skia::PixelGeometry,
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
            Some(&surface_props(pixel_geometry)),
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
    ///
    /// The surface declares no subpixel layout: textures back plane slots,
    /// which are blended by the display engine (see [`surface_props`]).
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
        let gl_format = match color_type {
            skia::ColorType::RGBA1010102 | skia::ColorType::BGRA1010102 => {
                skia::gpu::gl::Format::RGB10_A2
            }
            skia::ColorType::RGB888x => skia::gpu::gl::Format::RGB8,
            _ => skia::gpu::gl::Format::RGBA8,
        };
        let gl_info = skia::gpu::gl::TextureInfo {
            target: ffi::TEXTURE_2D,
            id: texid.into(),
            format: gl_format.into(),
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
            Some(&surface_props(skia::PixelGeometry::Unknown)),
        )
        .unwrap();

        Self {
            gr_context,
            surface,
        }
    }

    /// Subpixel layout the surface was created with.
    pub fn pixel_geometry(&self) -> skia::PixelGeometry {
        self.surface.props().pixel_geometry()
    }

    /// Returns a clone of the underlying Skia surface.
    pub fn surface(&self) -> skia::Surface {
        self.surface.clone()
    }
}

/// Surface properties for a render target with `pixel_geometry`.
///
/// The geometry is what turns subpixel (LCD) text on: Skia draws glyphs with
/// per-channel coverage only into a surface that declares one. That is only
/// correct where the pixels under the text are opaque by the time the target
/// is shown, which holds for an output's primary framebuffer and nothing
/// else the renderer draws into: plane slots and offscreen buffers are
/// blended afterwards, by the display engine or by a later draw, and a
/// per-channel coverage blended through a single alpha shows as colour
/// fringes. Those pass [`skia::PixelGeometry::Unknown`], which draws
/// grayscale text.
pub fn surface_props(pixel_geometry: skia::PixelGeometry) -> skia::SurfaceProps {
    skia::SurfaceProps::new(Default::default(), pixel_geometry)
}

/// Skia's name for a panel's subpixel layout.
///
/// `subpixel` is in the panel's native orientation, which is the orientation
/// of the buffers it scans out: the output transform is applied while they
/// are drawn. A rotated canvas draws grayscale text by itself, since Skia
/// keeps subpixel glyphs to axis-aligned matrices.
pub fn pixel_geometry_for(subpixel: smithay::output::Subpixel) -> skia::PixelGeometry {
    use smithay::output::Subpixel;
    match subpixel {
        Subpixel::HorizontalRgb => skia::PixelGeometry::RGBH,
        Subpixel::HorizontalBgr => skia::PixelGeometry::BGRH,
        Subpixel::VerticalRgb => skia::PixelGeometry::RGBV,
        Subpixel::VerticalBgr => skia::PixelGeometry::BGRV,
        Subpixel::None | Subpixel::Unknown => skia::PixelGeometry::Unknown,
    }
}
