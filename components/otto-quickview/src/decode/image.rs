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

/// The most an animation may come back as. Every frame is uncompressed RGBA
/// and travels down a pipe, so the ceiling is on the whole strip rather than
/// on one frame: a hundred frames of a modest GIF is more bytes than a single
/// very large photograph, and it is the total the host has to hold.
const MAX_ANIMATION_BYTES: u64 = 64 * 1024 * 1024;

/// How small a frame may be shrunk to bring a long animation inside the
/// budget. Past this the animation has stopped being a preview of anything,
/// and the first frame — sharp, at the size that was asked for — says more.
const MIN_ANIMATION_EDGE: i32 = 64;

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

    // A GIF or an animated WEBP is played rather than sampled: the strip of
    // frames comes back in one payload and the host runs the clock. Falling
    // through on `None` is deliberate — an animation too long or too large to
    // carry is still a picture, and its first frame is shown as one.
    if let Some(payload) = animation(&mut codec, intrinsic, target) {
        return payload;
    }

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

    // A codec that cannot sample — PNG has no sample size at all — has just
    // handed back the whole picture whatever was asked for, and a screenshot
    // requested as a 256-pixel thumbnail would otherwise go down the pipe and
    // into the caller's cache at 22 MB. Fit it into the target here, where
    // the resample costs one pass in the worker rather than a resident
    // full-size copy in the browser for as long as the thumbnail lives.
    let fit = fit_within(scaled, target);
    match to_pixels_at(&image, intrinsic, fit) {
        Some(pixels) => PreviewPayload::Pixels {
            pixels,
            pages: 1,
            page: 1,
        },
        None => payload::unavailable(otto_kit::t_owned!("quickview-error-image-readback")),
    }
}

/// Every frame of an animation, at one size, with the delays that pace them.
///
/// `None` for anything that is not an animation, and for one that cannot be
/// carried: the caller then decodes the first frame as an ordinary picture,
/// which is what Quick View did for every GIF before this.
///
/// Frames are decoded at the source's own size because a frame is rarely a
/// whole picture — GIF frames are patches composited onto what came before,
/// and Skia does that compositing in the destination buffer, which is why the
/// same buffer is handed back to it as `prior_frame`. Each finished frame is
/// then resampled down to the size the strip is carried at, so the budget is
/// spent on the animation rather than on the decode.
fn animation(codec: &mut Codec, intrinsic: ISize, target: ISize) -> Option<PreviewPayload> {
    let count = codec.get_frame_count();
    if count <= 1 || count > crate::payload::MAX_FRAMES as usize {
        return None;
    }
    // One full-size frame has to exist at a time whatever the strip is
    // carried at, so the still image's own ceiling applies here too.
    if pixel_count(intrinsic) > MAX_DECODE_PIXELS {
        return None;
    }

    let (size, stride) = fit_budget(fit_within(intrinsic, target), count)?;

    let info = codec
        .info()
        .with_dimensions(intrinsic)
        .with_color_type(skia_safe::ColorType::RGBA8888)
        .with_alpha_type(skia_safe::AlphaType::Premul);
    let row_bytes = intrinsic.width as usize * 4;
    let mut composited = vec![0u8; row_bytes.checked_mul(intrinsic.height as usize)?];

    let mut delays: Vec<u32> = Vec::with_capacity(count / stride + 1);
    let mut data = Vec::with_capacity((pixel_count(size) * 4) as usize * (count / stride + 1));
    let mut held: Option<usize> = None;
    for index in 0..count {
        let frame = codec.get_frame_info(index)?;
        let options = skia_safe::codec::Options {
            frame_index: index,
            // Only when the buffer already holds exactly the frame this one
            // is drawn over; otherwise Skia decodes the chain itself.
            prior_frame: held.filter(|held| frame.required_frame == *held as i32),
            ..Default::default()
        };
        let result =
            codec.get_pixels_with_options(&info, &mut composited, row_bytes, Some(&options));
        match result {
            skia_safe::codec::Result::Success | skia_safe::codec::Result::IncompleteInput => {}
            // A truncated or broken animation still has whatever it managed
            // before the bad frame; one frame is not an animation, so that
            // falls back to the still path.
            _ if index >= 2 => break,
            _ => return None,
        }
        held = Some(index);

        // A dropped frame is still decoded — the next kept one is composited
        // over it — but it is not carried. Its time goes to the frame that is
        // showing instead of it, so a thinned animation still runs at the
        // length the author gave it rather than at half of it.
        let duration = frame.duration.max(0) as u32;
        if index % stride != 0 {
            if let Some(last) = delays.last_mut() {
                *last += duration;
            }
            continue;
        }

        let image = skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(&composited),
            row_bytes,
        )?;
        let frame_pixels = to_pixels_at(&image, intrinsic, size)?;
        data.extend_from_slice(&frame_pixels.data);
        delays.push(duration);
    }

    if delays.len() < 2 {
        return None;
    }

    Some(PreviewPayload::Pixels {
        pixels: Pixels {
            width: size.width.max(0) as u32,
            height: size.height.max(0) as u32,
            intrinsic_width: intrinsic.width.max(0) as u32,
            intrinsic_height: intrinsic.height.max(0) as u32,
            data,
            frame_delays: delays,
        },
        pages: 1,
        page: 1,
    })
}

/// What a `count`-frame animation can be carried as: the size each frame is
/// resampled to, and how many source frames one carried frame stands for.
///
/// A long recording — a screencast GIF runs to hundreds of frames — is past
/// the budget several times over at the size a single picture would be shown
/// at, and both ways of getting it back cost something. Halving the frame
/// rate once is the cheaper of the two: a preview of a recording still reads
/// at half its frames, while a quarter of the width is a picture of a
/// picture. So one thinning step is spent first, and the rest comes off the
/// size.
///
/// `None` when even the smallest frame this will settle for is past the
/// budget, which the caller answers with a still first frame.
fn fit_budget(fit: ISize, count: usize) -> Option<(ISize, usize)> {
    let over = |size: ISize, stride: usize| {
        pixel_count(size) * 4 * count.div_ceil(stride) as u64 > MAX_ANIMATION_BYTES
    };
    let mut size = fit;
    let mut stride = 1;
    if over(size, stride) {
        stride = 2;
    }
    while over(size, stride) {
        if size.width <= MIN_ANIMATION_EDGE || size.height <= MIN_ANIMATION_EDGE {
            return None;
        }
        size = ISize::new((size.width / 2).max(1), (size.height / 2).max(1));
    }
    Some((size, stride))
}

/// `size` shrunk, aspect kept, until it fits in `bounds`. Never grown.
fn fit_within(size: ISize, bounds: ISize) -> ISize {
    if size.width <= bounds.width && size.height <= bounds.height {
        return size;
    }
    let scale = (bounds.width as f32 / size.width as f32)
        .min(bounds.height as f32 / size.height as f32)
        .min(1.0);
    ISize::new(
        ((size.width as f32 * scale).round() as i32).max(1),
        ((size.height as f32 * scale).round() as i32).max(1),
    )
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
        // Stamped by `decode`, which is where the sniffed type is known.
        icon: Vec::new(),
    }
}

/// Copy a decoded image out into a plain premultiplied RGBA buffer, which is
/// all the wire format and the drawing side know about.
pub(crate) fn to_pixels(image: &skia_safe::Image, intrinsic: ISize) -> Option<Pixels> {
    to_pixels_at(image, intrinsic, image.dimensions())
}

/// Read `image` back at `size`, resampling on the way when the two differ.
fn to_pixels_at(image: &skia_safe::Image, intrinsic: ISize, size: ISize) -> Option<Pixels> {
    let width = size.width.max(0) as u32;
    let height = size.height.max(0) as u32;
    let info = ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    let row_bytes = width as usize * 4;
    let mut data = vec![0u8; row_bytes * height as usize];
    let read = if size == image.dimensions() {
        image.read_pixels(
            &info,
            &mut data,
            row_bytes,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        )
    } else {
        let target = skia_safe::Pixmap::new(&info, &mut data, row_bytes)?;
        image.scale_pixels(
            &target,
            skia_safe::SamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::Linear,
            ),
            skia_safe::image::CachingHint::Disallow,
        )
    };
    read.then_some(Pixels {
        width,
        height,
        intrinsic_width: intrinsic.width.max(0) as u32,
        intrinsic_height: intrinsic.height.max(0) as u32,
        data,
        frame_delays: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG cannot be decoded at a sample size, so without a resample a big
    /// one would come back whole however small the request.
    #[test]
    fn a_png_asked_for_small_comes_back_small() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((1200, 900)).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        let png = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .unwrap();
        let path =
            std::env::temp_dir().join(format!("otto-quickview-fit-{}.png", std::process::id()));
        std::fs::write(&path, png.as_bytes()).unwrap();
        let mut file = File::open(&path).unwrap();
        let request = Request {
            width: 128,
            height: 128,
            ..Request::default()
        };
        let payload = raster(&mut file, &request);
        let _ = std::fs::remove_file(&path);
        let PreviewPayload::Pixels { pixels, .. } = payload else {
            panic!("pixels expected");
        };
        assert_eq!((pixels.width, pixels.height), (128, 96));
        assert_eq!(
            (pixels.intrinsic_width, pixels.intrinsic_height),
            (1200, 900)
        );
        assert_eq!(pixels.data.len(), 128 * 96 * 4);
        // Still red after the resample, and premultiplied opaque.
        assert_eq!(&pixels.data[..4], &[255, 0, 0, 255]);
    }

    /// Three frames of flat colour, 8 × 6, shown for 40, 60 and 80 ms. Written
    /// out by hand rather than encoded here: Skia has no GIF encoder, and a
    /// fixture is the only way to exercise the animated path at all.
    const THREE_FRAME_GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x08, 0x00, 0x06, 0x00, 0x81, 0x00, 0x00, 0xFF, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0xFF, 0x0B, 0x4E, 0x45,
        0x54, 0x53, 0x43, 0x41, 0x50, 0x45, 0x32, 0x2E, 0x30, 0x03, 0x01, 0x00, 0x00, 0x00, 0x21,
        0xF9, 0x04, 0x00, 0x04, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x06,
        0x00, 0x00, 0x08, 0x0E, 0x00, 0x01, 0x08, 0x1C, 0x48, 0xB0, 0xA0, 0xC1, 0x83, 0x08, 0x13,
        0x0E, 0x0C, 0x08, 0x00, 0x21, 0xF9, 0x04, 0x01, 0x06, 0x00, 0x01, 0x00, 0x2C, 0x00, 0x00,
        0x00, 0x00, 0x08, 0x00, 0x06, 0x00, 0x81, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x08, 0x0E, 0x00, 0x01, 0x08, 0x1C, 0x48, 0xB0, 0xA0, 0xC1, 0x83,
        0x08, 0x13, 0x0E, 0x0C, 0x08, 0x00, 0x21, 0xF9, 0x04, 0x01, 0x08, 0x00, 0x01, 0x00, 0x2C,
        0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x06, 0x00, 0x81, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x0E, 0x00, 0x01, 0x08, 0x1C, 0x48, 0xB0, 0xA0,
        0xC1, 0x83, 0x08, 0x13, 0x0E, 0x0C, 0x08, 0x00, 0x3B,
    ];

    #[test]
    fn an_animated_gif_comes_back_as_every_frame() {
        let path =
            std::env::temp_dir().join(format!("otto-quickview-anim-{}.gif", std::process::id()));
        std::fs::write(&path, THREE_FRAME_GIF).unwrap();
        let mut file = File::open(&path).unwrap();
        let payload = raster(&mut file, &Request::default());
        let _ = std::fs::remove_file(&path);

        let PreviewPayload::Pixels { pixels, .. } = payload else {
            panic!("pixels expected");
        };
        assert_eq!(pixels.frames(), 3);
        assert_eq!(pixels.frame_delays, vec![40, 60, 80]);
        assert_eq!((pixels.width, pixels.height), (8, 6));
        // Every frame is in the buffer, one after another.
        assert_eq!(pixels.data.len(), 8 * 6 * 4 * 3);
        // And they are the frames they should be: red, then green, then blue.
        let frame = |index: usize| {
            let stride = 8 * 6 * 4;
            pixels.data[index * stride..index * stride + 4].to_vec()
        };
        assert_eq!(frame(0), vec![255, 0, 0, 255]);
        assert_eq!(frame(1), vec![0, 255, 0, 255]);
        assert_eq!(frame(2), vec![0, 0, 255, 255]);
    }

    #[test]
    fn a_long_animation_gives_up_frames_before_it_gives_up_size() {
        // Short enough to carry whole: nothing is given up.
        assert_eq!(
            fit_budget(ISize::new(400, 300), 20),
            Some((ISize::new(400, 300), 1))
        );
        // A screencast's worth of frames at panel size is past the budget
        // several times over. Half the frames go first, and only what is
        // still over comes off the size.
        let (size, stride) = fit_budget(ISize::new(900, 563), 143).expect("carried");
        assert_eq!(stride, 2);
        assert!(size.width < 900 && size.width >= 400, "kept {size:?}");
        assert!(pixel_count(size) * 4 * 72 <= MAX_ANIMATION_BYTES);
    }

    #[test]
    fn an_animation_too_large_to_carry_falls_back_to_its_first_frame() {
        // The budget is what decides, so a target that would need more than
        // it allows must come back as a still rather than as nothing.
        let mut codec = Codec::from_data(Data::new_copy(THREE_FRAME_GIF)).expect("codec");
        let huge = ISize::new(100_000, 100_000);
        assert!(
            animation(&mut codec, huge, ISize::new(8, 6)).is_none(),
            "an animation past the decode ceiling is not carried"
        );
    }

    #[test]
    fn fitting_keeps_the_aspect_and_never_grows() {
        assert_eq!(
            fit_within(ISize::new(2880, 1920), ISize::new(256, 256)),
            ISize::new(256, 171)
        );
        assert_eq!(
            fit_within(ISize::new(100, 80), ISize::new(256, 256)),
            ISize::new(100, 80)
        );
        assert_eq!(
            fit_within(ISize::new(1000, 4000), ISize::new(512, 512)),
            ISize::new(128, 512)
        );
    }

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
