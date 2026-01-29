//! Texture types and utilities for GPU-backed rendering.
//!
//! This module provides texture wrappers that combine OpenGL textures with
//! Skia images, enabling efficient GPU-based rendering with the Skia API.

use std::cell::RefCell;

use layers::skia;
use smithay::{
    backend::{
        allocator::Fourcc,
        egl::ffi::egl::types::EGLImage,
        renderer::{gles::GlesTexture, Texture, TextureMapping},
    },
    utils::{Buffer, Physical, Rectangle, Size},
};

use super::skia_surface::SkiaSurface;

/// FBO information for Skia rendering targets.
///
/// Contains OpenGL framebuffer object details needed to create
/// Skia surfaces backed by FBOs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkiaGLesFbo {
    pub fbo: u32,
    pub tex_id: u32,
    pub format: Fourcc,
    pub origin: skia::gpu::SurfaceOrigin,
}

/// A GPU texture that can be used with both OpenGL and Skia.
///
/// Combines a GlesTexture (for OpenGL operations) with a Skia Image
/// (for Skia drawing operations). This allows textures to be efficiently
/// shared between the compositor's GL pipeline and Skia rendering.
#[derive(Debug, Clone)]
pub struct SkiaTexture {
    pub texture: GlesTexture,
    pub image: skia::Image,
    pub has_alpha: bool,
    pub format: Option<Fourcc>,
    pub egl_images: Option<Vec<EGLImage>>,
    pub is_external: bool,
    pub damage: Option<Vec<Rectangle<i32, Buffer>>>,
}

unsafe impl Send for SkiaTexture {}

impl Texture for SkiaTexture {
    fn width(&self) -> u32 {
        self.image.width() as u32
    }

    fn height(&self) -> u32 {
        self.image.height() as u32
    }

    fn format(&self) -> Option<Fourcc> {
        self.format
    }
}

/// A lightweight texture representation using only the texture ID and Skia image.
///
/// This is used when the full GlesTexture is not needed, reducing memory overhead.
#[derive(Debug, Clone)]
pub struct SkiaTextureImage {
    pub tid: u32,
    pub image: skia::Image,
    pub has_alpha: bool,
    pub format: Option<Fourcc>,
    pub damage: Option<Vec<Rectangle<i32, Buffer>>>,
}

impl From<SkiaTexture> for SkiaTextureImage {
    fn from(value: SkiaTexture) -> Self {
        SkiaTextureImage {
            tid: value.texture.tex_id(),
            image: value.image,
            has_alpha: value.has_alpha,
            format: value.format,
            damage: value.damage,
        }
    }
}

/// A mapped texture region for CPU access to GPU texture data.
///
/// Allows reading pixel data from a GPU texture by mapping it to CPU-accessible
/// memory. The mapping is lazy - data is only copied when accessed.
#[derive(Debug, Clone)]
pub struct SkiaTextureMapping {
    pub fourcc_format: Fourcc,
    pub flipped: bool,
    pub width: u32,
    pub height: u32,
    pub fbo_info: SkiaGLesFbo,
    pub region: Rectangle<i32, Buffer>,
    pub image: RefCell<Option<skia::Image>>,
    pub data: RefCell<Option<Vec<u8>>>,
}

impl TextureMapping for SkiaTextureMapping {
    fn flipped(&self) -> bool {
        self.flipped
    }

    fn format(&self) -> Fourcc {
        self.fourcc_format
    }
}

impl Texture for SkiaTextureMapping {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn format(&self) -> Option<Fourcc> {
        Some(self.fourcc_format)
    }
}

/// Rendering frame for a single output.
///
/// Represents an active rendering frame with access to the Skia surface
/// for drawing operations. The frame is tied to a specific renderer instance.
pub struct SkiaFrame<'frame> {
    pub(crate) size: Size<i32, Physical>,
    pub skia_surface: SkiaSurface,
    pub(crate) renderer: &'frame mut crate::skia_renderer::SkiaRenderer,
    pub(crate) id: usize,
}
