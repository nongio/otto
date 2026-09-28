//! Pixel formats the Vulkan renderer can draw with.
//!
//! Skia names Vulkan formats with its own enum, and pairs each with a colour
//! type. Only the formats in this table can be imported, rendered into or
//! read back; anything else is reported as unsupported and never advertised.
//! YUV formats are sample-only: Skia reads them through a sampler YCbCr
//! conversion into RGB.

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
    /// The fourcc has no alpha channel (an `X` format, or YUV).
    pub opaque: bool,
    /// A YUV format, sampled through a YCbCr conversion.
    pub ycbcr: bool,
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
        Fourcc::Nv12 => {
            return Some(SkiaFormat {
                vk: vk::Format::G8_B8R8_2PLANE_420_UNORM,
                skia_vk: skvk::Format::G8_B8R8_2PLANE_420_UNORM,
                color_type: skia::ColorType::RGB888x,
                opaque: true,
                ycbcr: true,
            })
        }
        _ => return None,
    };
    Some(SkiaFormat {
        vk,
        skia_vk,
        color_type,
        opaque,
        ycbcr: false,
    })
}

/// The YCbCr conversion Skia samples a YUV image of `fmt` with.
///
/// Clients cannot say how their YUV is encoded yet, so this assumes what
/// video decoders produce by default: BT.601 in limited range with chroma
/// sited between the luma samples. `features` are the format features of
/// the image's modifier; chroma is filtered linearly where they allow it.
pub(crate) fn ycbcr_conversion(
    fmt: SkiaFormat,
    features: vk::FormatFeatureFlags,
) -> Option<skvk::YcbcrConversionInfo> {
    if !fmt.ycbcr {
        return None;
    }
    let linear =
        features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER);
    let identity = skvk::ComponentSwizzle::VK_COMPONENT_SWIZZLE_IDENTITY;
    Some(skvk::YcbcrConversionInfo::new_with_format(
        fmt.skia_vk,
        skvk::SamplerYcbcrModelConversion::YCBCR_601,
        skvk::SamplerYcbcrRange::ITU_NARROW,
        skvk::ChromaLocation::MIDPOINT,
        skvk::ChromaLocation::MIDPOINT,
        if linear {
            skvk::Filter::LINEAR
        } else {
            skvk::Filter::NEAREST
        },
        0,
        skvk::ComponentMapping {
            r: identity,
            g: identity,
            b: identity,
            a: identity,
        },
        features.as_raw(),
    ))
}
