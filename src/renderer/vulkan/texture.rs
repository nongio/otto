//! Textures, render targets and read-back mappings of the Vulkan renderer.

// Rust guideline compliant 2026-02-21

use std::{fmt, sync::Arc, sync::Mutex};

use layers::skia;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{Texture, TextureMapping},
        vulkan::image::VulkanImage,
    },
    utils::{Buffer, Rectangle},
};

use super::retire::{retire, RetireQueue, RetiredHandle};
use crate::renderer::{SkiaSurface, SkiaTextureImage};

/// What holds a texture's pixels.
pub(crate) enum TextureMemory {
    /// A client dmabuf imported as a `VkImage`.
    Dmabuf { _image: VulkanImage },
    /// A Skia-owned texture uploaded from memory, and the surface that writes it.
    ///
    /// The texture's [`SkiaVkTexture::image`] borrows the same GPU texture, so
    /// an `update_memory` through the surface shows up in the image.
    Memory(skia::Surface),
}

/// The memory behind a texture, shared by its clones.
///
/// Dropped with the last clone, it retires the memory to the renderer, which
/// destroys it once the image is unreachable and the GPU is done with it.
pub(crate) struct TextureBacking {
    memory: Mutex<Option<TextureMemory>>,
    image: skia::Image,
    /// The memory texture's rows were uploaded bottom to top.
    pub flipped: bool,
    retire: RetireQueue,
}

impl TextureBacking {
    /// Backs `image` with `memory`.
    pub fn new(
        memory: TextureMemory,
        image: skia::Image,
        flipped: bool,
        retire: RetireQueue,
    ) -> Self {
        Self {
            memory: Mutex::new(Some(memory)),
            image,
            flipped,
            retire,
        }
    }

    /// The surface that writes a memory texture, or `None` for a dmabuf.
    pub fn memory_surface(&self) -> Option<std::sync::MutexGuard<'_, Option<TextureMemory>>> {
        let guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        matches!(*guard, Some(TextureMemory::Memory(_))).then_some(guard)
    }

    fn kind(&self) -> &'static str {
        match *self.memory.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(TextureMemory::Dmabuf { .. }) => "dmabuf",
            Some(TextureMemory::Memory(_)) => "memory",
            None => "retired",
        }
    }
}

impl Drop for TextureBacking {
    fn drop(&mut self) {
        let memory = self
            .memory
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(memory) = memory {
            retire(
                &self.retire,
                RetiredHandle::Image(self.image.clone()),
                Box::new(memory),
            );
        }
    }
}

/// A texture the Vulkan renderer samples.
///
/// Cloning is cheap: the Skia image is reference counted and the backing is
/// shared.
#[derive(Clone)]
pub struct SkiaVkTexture {
    /// The Skia image to draw.
    pub image: skia::Image,
    /// The pixel format has an alpha channel.
    pub has_alpha: bool,
    /// The sampled alpha byte is format padding, not coverage; drawn forced opaque.
    pub padding_alpha: bool,
    /// The DRM fourcc of the pixels.
    pub format: Option<Fourcc>,
    /// The buffer damage the client reported with the commit this import
    /// belongs to. The surface layer's draw content reports it as its own
    /// damage, which is what makes a commit repaint only the changed band.
    pub damage: Option<Vec<Rectangle<i32, Buffer>>>,
    pub(crate) backing: Arc<TextureBacking>,
}

// SAFETY: Skia objects are only touched on the render thread; Smithay
// requires `Send` of texture ids so it can keep them in surface state, which
// never leaves that thread in Otto.
unsafe impl Send for SkiaVkTexture {}
// SAFETY: as above.
unsafe impl Send for TextureBacking {}
// SAFETY: as above.
unsafe impl Sync for TextureBacking {}

impl fmt::Debug for SkiaVkTexture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkiaVkTexture")
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("format", &self.format)
            .field("backing", &self.backing.kind())
            .finish()
    }
}

impl Texture for SkiaVkTexture {
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

impl From<SkiaVkTexture> for SkiaTextureImage {
    fn from(value: SkiaVkTexture) -> Self {
        SkiaTextureImage {
            // No GL name on Vulkan; the image id changes whenever the
            // texture is re-wrapped, which is what the id is hashed for.
            tid: value.image.unique_id(),
            image: value.image,
            has_alpha: value.has_alpha,
            padding_alpha: value.padding_alpha,
            format: value.format,
            damage: value.damage,
        }
    }
}

/// What a render target draws into.
#[derive(Clone)]
pub(crate) enum TargetKind {
    /// A dmabuf wrapped as a render target; handed back to the foreign
    /// queue family after every frame so KMS or another device can read it.
    Dmabuf(VulkanImage),
    /// A Skia-owned offscreen surface.
    Offscreen,
}

/// A framebuffer of the Vulkan renderer.
#[derive(Clone)]
pub struct SkiaVkTarget {
    /// The Skia surface to draw into, with the context it belongs to.
    pub skia_surface: SkiaSurface,
    pub(crate) format: Fourcc,
    pub(crate) kind: TargetKind,
}

impl SkiaVkTarget {
    /// The target is a wrapped dmabuf.
    pub fn is_dmabuf(&self) -> bool {
        matches!(self.kind, TargetKind::Dmabuf(_))
    }
}

impl fmt::Debug for SkiaVkTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkiaVkTarget")
            .field("width", &self.skia_surface.surface.width())
            .field("height", &self.skia_surface.surface.height())
            .field("format", &self.format)
            .field("dmabuf", &self.is_dmabuf())
            .finish()
    }
}

impl Texture for SkiaVkTarget {
    fn width(&self) -> u32 {
        self.skia_surface.surface.width() as u32
    }

    fn height(&self) -> u32 {
        self.skia_surface.surface.height() as u32
    }

    fn format(&self) -> Option<Fourcc> {
        Some(self.format)
    }
}

/// Pixels read back from a framebuffer or texture.
#[derive(Debug, Clone)]
pub struct SkiaVkMapping {
    pub(crate) format: Fourcc,
    pub(crate) region: Rectangle<i32, Buffer>,
    pub(crate) data: Vec<u8>,
}

impl Texture for SkiaVkMapping {
    fn width(&self) -> u32 {
        self.region.size.w as u32
    }

    fn height(&self) -> u32 {
        self.region.size.h as u32
    }

    fn format(&self) -> Option<Fourcc> {
        Some(self.format)
    }
}

impl TextureMapping for SkiaVkMapping {
    fn flipped(&self) -> bool {
        false
    }

    fn format(&self) -> Fourcc {
        self.format
    }
}
