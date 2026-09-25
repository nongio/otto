//! Pixel formats the Vulkan renderer can draw with.
//!
//! Skia names Vulkan formats with its own enum, and pairs each with a colour
//! type. Only the formats in this table can be imported, rendered into or
//! read back; anything else (YUV video buffers among them) is reported as
//! unsupported and never advertised.

// Rust guideline compliant 2026-02-21

use layers::skia::{self, gpu::vk as skvk};
use smithay::{backend::allocator::Fourcc, reexports::ash::vk};

/// How a DRM fourcc maps onto Skia on Vulkan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SkiaFormat {
    /// The Vulkan format of an image holding this fourcc.
    pub vk: vk::Format,
    /// Skia's name for [`Self::vk`].
    pub skia_vk: skvk::Format,
    /// The colour type Skia reads and writes the image with.
    pub color_type: skia::ColorType,
    /// The fourcc has no alpha channel (an `X` format).
    pub opaque: bool,
}

impl SkiaFormat {
    /// Alpha type for images of this format.
    pub fn alpha_type(&self) -> skia::AlphaType {
        if self.opaque {
            skia::AlphaType::Opaque
        } else {
            skia::AlphaType::Premul
        }
    }

    /// Skia image info of a `width` x `height` image in this format.
    pub fn image_info(&self, width: i32, height: i32) -> skia::ImageInfo {
        skia::ImageInfo::new((width, height), self.color_type, self.alpha_type(), None)
    }
}

/// Every fourcc the renderer handles, in the order `mem_formats` reports them.
pub(crate) const FOURCCS: &[Fourcc] = &[
    Fourcc::Argb8888,
    Fourcc::Xrgb8888,
    Fourcc::Abgr8888,
    Fourcc::Xbgr8888,
    Fourcc::Abgr2101010,
    Fourcc::Xbgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Xrgb2101010,
    Fourcc::Abgr16161616f,
    Fourcc::Xbgr16161616f,
];

/// Looks up how `fourcc` is drawn with Skia on Vulkan.
pub(crate) fn skia_format(fourcc: Fourcc) -> Option<SkiaFormat> {
    let (vk, skia_vk, color_type, opaque) = match fourcc {
        Fourcc::Argb8888 => (
            vk::Format::B8G8R8A8_UNORM,
            skvk::Format::B8G8R8A8_UNORM,
            skia::ColorType::BGRA8888,
            false,
        ),
        Fourcc::Xrgb8888 => (
            vk::Format::B8G8R8A8_UNORM,
            skvk::Format::B8G8R8A8_UNORM,
            skia::ColorType::BGRA8888,
            true,
        ),
        Fourcc::Abgr8888 => (
            vk::Format::R8G8B8A8_UNORM,
            skvk::Format::R8G8B8A8_UNORM,
            skia::ColorType::RGBA8888,
            false,
        ),
        Fourcc::Xbgr8888 => (
            vk::Format::R8G8B8A8_UNORM,
            skvk::Format::R8G8B8A8_UNORM,
            skia::ColorType::RGBA8888,
            true,
        ),
        Fourcc::Abgr2101010 => (
            vk::Format::A2B10G10R10_UNORM_PACK32,
            skvk::Format::A2B10G10R10_UNORM_PACK32,
            skia::ColorType::RGBA1010102,
            false,
        ),
        Fourcc::Xbgr2101010 => (
            vk::Format::A2B10G10R10_UNORM_PACK32,
            skvk::Format::A2B10G10R10_UNORM_PACK32,
            skia::ColorType::RGBA1010102,
            true,
        ),
        Fourcc::Argb2101010 => (
            vk::Format::A2R10G10B10_UNORM_PACK32,
            skvk::Format::A2R10G10B10_UNORM_PACK32,
            skia::ColorType::BGRA1010102,
            false,
        ),
        Fourcc::Xrgb2101010 => (
            vk::Format::A2R10G10B10_UNORM_PACK32,
            skvk::Format::A2R10G10B10_UNORM_PACK32,
            skia::ColorType::BGRA1010102,
            true,
        ),
        Fourcc::Abgr16161616f => (
            vk::Format::R16G16B16A16_SFLOAT,
            skvk::Format::R16G16B16A16_SFLOAT,
            skia::ColorType::RGBAF16,
            false,
        ),
        Fourcc::Xbgr16161616f => (
            vk::Format::R16G16B16A16_SFLOAT,
            skvk::Format::R16G16B16A16_SFLOAT,
            skia::ColorType::RGBAF16,
            true,
        ),
        _ => return None,
    };
    Some(SkiaFormat {
        vk,
        skia_vk,
        color_type,
        opaque,
    })
}
