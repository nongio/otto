//! Images — the primary content type, and the one held to the highest standard.
//!
//! Two rules carry this file. **Never materialise a full-resolution decode of a
//! very large image**: Skia's `Codec` can decode at a sample size, and that is
//! the difference between previewing a 200 MP photograph and refusing to.
//! **Never upscale a fit-sized decode when the user zooms in**: past roughly
//! 1:1 the worker is asked again at a finer scale, so looking closely shows
//! detail rather than blur.

use std::fs::File;

use skia_safe::{Codec, Data, ISize, ImageInfo};

use crate::payload;
use crate::payload::{Pixels, PreviewPayload};

use super::{read_capped, Request};

/// The most pixels a single decode may produce. A 100 MP decode at RGBA is
/// 400 MB, which is already past what a preview should ever hold; the sample
/// size is chosen to stay under this, and a codec that cannot scale is refused
/// rather than allowed to blow the address-space limit.
const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;

pub fn raster(file: &mut File, request: &Request) -> PreviewPayload {
    let bytes = match read_capped(file, request.budget.max_read) {
        Ok(bytes) => bytes,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "quickview-error-read-image",
                error = err.to_string()
            ))
        }
    };
    let data = Data::new_copy(&bytes);
    let Some(mut codec) = Codec::from_data(data) else {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-image-unsupported"));
    };

    let intrinsic = codec.dimensions();
    if intrinsic.width <= 0 || intrinsic.height <= 0 {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-image-no-size"));
    }

    let target = target_size(intrinsic, request);
    // The codec picks the nearest sample size it can actually deliver, which is
    // rarely exactly what was asked for.
    let scale = (target.width as f32 / intrinsic.width as f32).clamp(0.0, 1.0);
    let scaled = if scale >= 1.0 {
        intrinsic
    } else {
        codec.get_scaled_dimensions(scale)
    };

    if pixel_count(scaled) > MAX_DECODE_PIXELS {
        return too_large(intrinsic, request);
    }

    let info = codec
        .info()
        .with_dimensions(scaled)
        .with_color_type(skia_safe::ColorType::RGBA8888)
        .with_alpha_type(skia_safe::AlphaType::Premul);

    let image = match codec.get_image(info, None) {
        Ok(image) => image,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "quickview-error-image-decode",
                error = format!("{err:?}")
            ))
        }
    };

    match to_pixels(&image, intrinsic) {
        Some(pixels) => PreviewPayload::Pixels {
            pixels,
            pages: 1,
            page: 1,
        },
        None => payload::unavailable(otto_kit::t_owned!("quickview-error-image-readback")),
    }
}

/// SVG, rendered at the size it will be shown rather than at some nominal one,
/// so it stays sharp at every zoom level. Skia's own SVG module does this —
/// Quick View deliberately does not become a new consumer of `resvg`.
pub fn svg(file: &mut File, request: &Request) -> PreviewPayload {
    let bytes = match read_capped(file, request.budget.max_read.min(64 * 1024 * 1024)) {
        Ok(bytes) => bytes,
        Err(err) => {
            return payload::unavailable(otto_kit::t_owned!(
                "quickview-error-read-drawing",
                error = err.to_string()
            ))
        }
    };

    // Skia's own `LocalResourceProvider` would happily open whatever an
    // `xlink:href` points at. That is precisely the hole the sandbox exists to
    // close, so the drawing gets a provider that refuses everything external
    // and offers fonts only. The network namespace already forbids the remote
    // case; this forbids the local one at the same time.
    let Ok(mut dom) = skia_safe::svg::Dom::from_bytes(&bytes, SealedResources) else {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-drawing-parse"));
    };

    let width = request.width.clamp(1, 8192) as i32;
    let height = request.height.clamp(1, 8192) as i32;
    let Some(mut surface) = skia_safe::surfaces::raster_n32_premul((width, height)) else {
        return payload::unavailable(otto_kit::t_owned!("quickview-error-drawing-surface"));
    };
    dom.set_container_size(skia_safe::Size::new(width as f32, height as f32));
    dom.render(surface.canvas());

    let image = surface.image_snapshot();
    match to_pixels(&image, ISize::new(width, height)) {
        Some(pixels) => PreviewPayload::Pixels {
            pixels,
            pages: 1,
            page: 1,
        },
        None => payload::unavailable(otto_kit::t_owned!("quickview-error-drawing-readback")),
    }
}

/// A resource provider that provides no resources.
///
/// An SVG may reference images and fonts by URL. Honouring those would let a
/// file the user merely *looked at* pull in another file — the sandbox blocks
/// the syscall, but refusing here means the drawing renders promptly with the
/// reference missing rather than after the kernel says no.
///
/// Fonts are the exception: text in an SVG should still be laid out, and the
/// system font manager reads only what is already on the font path.
#[derive(Debug)]
struct SealedResources;

impl skia_safe::resources::ResourceProvider for SealedResources {
    fn load(&self, _resource_path: &str, _resource_name: &str) -> Option<skia_safe::Data> {
        None
    }

    fn load_typeface(&self, _name: &str, _url: &str) -> Option<skia_safe::Typeface> {
        None
    }

    fn font_mgr(&self) -> skia_safe::FontMgr {
        skia_safe::FontMgr::default()
    }
}

/// What size to decode at.
///
/// The size the host asked for — which is already twice the panel, so the
/// image survives being scaled down for display and has something in hand
/// when the user starts zooming — but never more than the source actually
/// has, since upsampling in the worker would only move the blur earlier in
/// the pipeline.
///
/// The doubling belongs to the host and happens once. Doubling again here
/// meant every preview was decoded at four times the panel, and the payload
/// is uncompressed pixels: the cost lands twice, in the decode and in the
/// bytes that then go down the pipe.
fn target_size(intrinsic: ISize, request: &Request) -> ISize {
    let zoom = request.zoom.max(1.0);
    let wanted_w = (request.width as f32 * zoom).ceil() as i32;
    let wanted_h = (request.height as f32 * zoom).ceil() as i32;
    ISize::new(
        wanted_w.clamp(1, intrinsic.width),
        wanted_h.clamp(1, intrinsic.height),
    )
}

fn pixel_count(size: ISize) -> u64 {
    (size.width.max(0) as u64) * (size.height.max(0) as u64)
}

/// An image too large to decode is described rather than shown. Refusing with
/// its real dimensions is more use than a blank rectangle.
fn too_large(intrinsic: ISize, request: &Request) -> PreviewPayload {
    PreviewPayload::Card {
        title: request.name.clone(),
        subtitle: otto_kit::t_owned!("quickview-image-too-large"),
        facts: vec![
            crate::payload::Fact {
                key: otto_kit::t_owned!("quickview-fact-dimensions"),
                value: format!("{} × {}", intrinsic.width, intrinsic.height),
            },
            crate::payload::Fact {
                key: otto_kit::t_owned!("quickview-fact-pixels"),
                value: otto_kit::t_owned!(
                    "quickview-megapixels",
                    count = (pixel_count(intrinsic) / 1_000_000) as f64
                ),
            },
        ],
        hero: None,
    }
}

/// Copy a decoded image out into a plain premultiplied RGBA buffer, which is
/// all the wire format and the drawing side know about.
pub(crate) fn to_pixels(image: &skia_safe::Image, intrinsic: ISize) -> Option<Pixels> {
    let width = image.width().max(0) as u32;
    let height = image.height().max(0) as u32;
    let info = ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    let row_bytes = width as usize * 4;
    let mut data = vec![0u8; row_bytes * height as usize];
    image
        .read_pixels(
            &info,
            &mut data,
            row_bytes,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        )
        .then_some(Pixels {
            width,
            height,
            intrinsic_width: intrinsic.width.max(0) as u32,
            intrinsic_height: intrinsic.height.max(0) as u32,
            data,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_never_exceeds_the_source() {
        let intrinsic = ISize::new(100, 80);
        let request = Request {
            width: 1600,
            height: 1200,
            ..Request::default()
        };
        let target = target_size(intrinsic, &request);
        assert_eq!(target.width, 100);
        assert_eq!(target.height, 80);
    }

    #[test]
    fn zooming_asks_for_more_detail() {
        let intrinsic = ISize::new(10_000, 8_000);
        let fit = target_size(
            intrinsic,
            &Request {
                width: 800,
                height: 600,
                zoom: 1.0,
                ..Request::default()
            },
        );
        let zoomed = target_size(
            intrinsic,
            &Request {
                width: 800,
                height: 600,
                zoom: 4.0,
                ..Request::default()
            },
        );
        assert!(
            zoomed.width > fit.width,
            "zooming in must request a finer decode, got {} then {}",
            fit.width,
            zoomed.width
        );
    }
}
