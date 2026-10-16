//! Reading a wallpaper file, and rendering it into a screen-sized frame.

use std::sync::Arc;

use skia_safe as skia;

use super::layout::{arrange, Layout};
use crate::utils::resample::{resample, resample_to};

/// A wallpaper file, opened but not yet decoded.
///
/// Opening reads the file and its header only — cheap enough to do where an
/// unreadable path has to be reported straight away. The pixels are decoded
/// by [`render_frame`], which is the slow part and belongs on a worker
/// thread.
#[derive(Clone)]
pub enum Source {
    /// A raster format Skia decodes: JPEG, PNG, WebP, GIF, BMP, … Lazily
    /// decoded, with the file's colour profile and orientation.
    Raster(skia::Image),
    /// An SVG document, rasterised for each frame at the size it is shown.
    Svg(Arc<Vec<u8>>),
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Raster(image) => write!(f, "Raster({}x{})", image.width(), image.height()),
            Source::Svg(bytes) => write!(f, "Svg({} bytes)", bytes.len()),
        }
    }
}

/// Why a wallpaper file could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The file could not be read.
    Unreadable(String),
    /// The file was read but is not an image Otto can decode.
    NotAnImage,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Unreadable(err) => write!(f, "cannot read the file: {err}"),
            OpenError::NotAnImage => write!(f, "not an image Otto can decode"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Open the wallpaper at `path`.
pub fn open(path: &str) -> Result<Source, OpenError> {
    let bytes = std::fs::read(path).map_err(|err| OpenError::Unreadable(err.to_string()))?;
    let is_svg = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    if is_svg {
        // Parse once here so a broken document is reported now rather than
        // as a blank frame later.
        let options = usvg::Options::default();
        usvg::Tree::from_data(&bytes, &options).map_err(|_| OpenError::NotAnImage)?;
        return Ok(Source::Svg(Arc::new(bytes)));
    }
    skia::Image::from_encoded(skia::Data::new_copy(&bytes))
        .map(Source::Raster)
        .ok_or(OpenError::NotAnImage)
}

/// Everything about a frame besides the image and the screen size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStyle {
    pub layout: Layout,
    /// What shows where the image does not: around a fitted or centred one,
    /// and through a transparent one. Always painted opaque.
    pub fill: skia::Color4f,
    /// Physical pixels per logical point, for the modes that draw the image
    /// at its own size.
    pub output_scale: f32,
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            fill: skia::Color4f::new(0.0, 0.0, 0.0, 1.0),
            output_scale: 1.0,
        }
    }
}

/// Render `source` into an opaque frame of exactly `target` pixels, laid out
/// by `style`, so that it is drawn 1:1 with nothing filtered per frame.
///
/// The visible part of the image is resampled straight into place: its edges
/// are snapped to whole pixels, while the region read from the image keeps
/// its sub-pixel position. Where the image lands 1:1 on whole pixels its
/// pixels are copied rather than resampled — a resampling kernel would
/// soften them for nothing.
pub fn render_frame(
    source: &Source,
    style: &FrameStyle,
    target: (i32, i32),
) -> Option<skia::Image> {
    let (tw, th) = target;
    if tw <= 0 || th <= 0 {
        return None;
    }
    let mut surface = skia::surfaces::raster_n32_premul((tw, th))?;
    let canvas = surface.canvas();
    let mut fill = style.fill;
    fill.a = 1.0;
    canvas.clear(fill);
    match source {
        Source::Raster(image) => draw_raster(canvas, image, style, target)?,
        Source::Svg(bytes) => draw_svg(canvas, bytes, style, target)?,
    }
    Some(surface.image_snapshot())
}

fn draw_raster(
    canvas: &skia::Canvas,
    image: &skia::Image,
    style: &FrameStyle,
    target: (i32, i32),
) -> Option<()> {
    let (iw, ih) = (image.width(), image.height());
    if iw <= 0 || ih <= 0 {
        return None;
    }
    let arrangement = arrange(
        style.layout,
        (iw as f32, ih as f32),
        target,
        style.output_scale,
    );
    let dest = arrangement.dest;

    if arrangement.tiled {
        let tile = (
            dest.width().round().max(1.0) as i32,
            dest.height().round().max(1.0) as i32,
        );
        let tile_image = if tile == (iw, ih) {
            image.clone()
        } else {
            resample_to(image, tile)?
        };
        // Whole-pixel tiles from the origin, sampled 1:1: an exact copy of
        // the tile everywhere.
        let mut paint = skia::Paint::default();
        paint.set_shader(tile_image.to_shader(
            (skia::TileMode::Repeat, skia::TileMode::Repeat),
            skia::SamplingOptions::default(),
            None,
        ));
        canvas.draw_rect(skia::Rect::from_iwh(target.0, target.1), &paint);
        return Some(());
    }

    // The part of the screen the image covers, snapped to whole pixels.
    let left = dest.left.max(0.0).round();
    let top = dest.top.max(0.0).round();
    let right = dest.right.min(target.0 as f32).round();
    let bottom = dest.bottom.min(target.1 as f32).round();
    if right <= left || bottom <= top {
        return Some(());
    }
    let out = ((right - left) as i32, (bottom - top) as i32);
    // The image region that lands there, unrounded.
    let (per_x, per_y) = (iw as f32 / dest.width(), ih as f32 / dest.height());
    let src = skia::Rect::from_xywh(
        (left - dest.left) * per_x,
        (top - dest.top) * per_y,
        out.0 as f32 * per_x,
        out.1 as f32 * per_y,
    );

    if let Some(exact) = whole_pixel_copy(src, out) {
        canvas.draw_image_rect_with_sampling_options(
            image,
            Some((&exact, skia::canvas::SrcRectConstraint::Strict)),
            skia::Rect::from_xywh(left, top, out.0 as f32, out.1 as f32),
            skia::SamplingOptions::default(),
            &skia::Paint::default(),
        );
    } else {
        let piece = resample(image, src, out)?;
        canvas.draw_image(&piece, (left, top), None);
    }
    Some(())
}

/// `src` as a whole-pixel rect, when it is a 1:1 copy of `out` pixels at a
/// whole-pixel origin.
fn whole_pixel_copy(src: skia::Rect, out: (i32, i32)) -> Option<skia::Rect> {
    const EPSILON: f32 = 1e-3;
    let whole = |v: f32| (v - v.round()).abs() < EPSILON;
    let same_size = (src.width() - out.0 as f32).abs() < EPSILON
        && (src.height() - out.1 as f32).abs() < EPSILON;
    (same_size && whole(src.left) && whole(src.top)).then(|| {
        skia::Rect::from_xywh(
            src.left.round(),
            src.top.round(),
            out.0 as f32,
            out.1 as f32,
        )
    })
}

/// Draw an SVG document as laid out by `style`, as vectors: there is nothing
/// to resample.
fn draw_svg(
    canvas: &skia::Canvas,
    bytes: &[u8],
    style: &FrameStyle,
    target: (i32, i32),
) -> Option<()> {
    let options = usvg::Options {
        languages: vec!["en".to_string()],
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let (sw, sh) = (tree.size().width(), tree.size().height());
    if !(sw > 0.0 && sh > 0.0) {
        return None;
    }
    let arrangement = arrange(style.layout, (sw, sh), target, style.output_scale);
    let dest = arrangement.dest;

    if arrangement.tiled {
        let tile = (
            dest.width().round().max(1.0) as i32,
            dest.height().round().max(1.0) as i32,
        );
        let tile_image = rasterise_svg(
            &tree,
            tile,
            (tile.0 as f32 / sw, tile.1 as f32 / sh),
            (0.0, 0.0),
        )?;
        let mut paint = skia::Paint::default();
        paint.set_shader(tile_image.to_shader(
            (skia::TileMode::Repeat, skia::TileMode::Repeat),
            skia::SamplingOptions::default(),
            None,
        ));
        canvas.draw_rect(skia::Rect::from_iwh(target.0, target.1), &paint);
        return Some(());
    }

    let image = rasterise_svg(
        &tree,
        target,
        (dest.width() / sw, dest.height() / sh),
        (dest.left, dest.top),
    )?;
    canvas.draw_image(&image, (0, 0), None);
    Some(())
}

/// Rasterise `tree` into a transparent `size` image, scaled by `scale` and
/// moved by `offset`.
fn rasterise_svg(
    tree: &usvg::Tree,
    size: (i32, i32),
    scale: (f32, f32),
    offset: (f32, f32),
) -> Option<skia::Image> {
    let transform =
        resvg::tiny_skia::Transform::from_row(scale.0, 0.0, 0.0, scale.1, offset.0, offset.1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.0 as u32, size.1 as u32)?;
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let info = skia::ImageInfo::new(
        size,
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        Some(skia::ColorSpace::new_srgb()),
    );
    skia::images::raster_from_data(
        &info,
        skia::Data::new_copy(pixmap.data()),
        size.0 as usize * 4,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::Mode;

    fn write_png(name: &str, w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 3]) -> String {
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let info = skia::ImageInfo::new(
            (w, h),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            None,
        );
        let image =
            skia::images::raster_from_data(&info, skia::Data::new_copy(&data), (w * 4) as usize)
                .unwrap();
        let png = image
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("otto-kit-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, png.as_bytes()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn pixel(image: &skia::Image, x: i32, y: i32) -> [u8; 4] {
        let info = skia::ImageInfo::new(
            (1, 1),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Unpremul,
            None,
        );
        let mut px = [0u8; 4];
        assert!(image.read_pixels(
            &info,
            &mut px,
            4,
            (x, y),
            skia::image::CachingHint::Disallow
        ));
        px
    }

    /// A frame is exactly the screen's size, whatever the image's.
    #[test]
    fn a_frame_is_the_size_of_the_screen() {
        let path = write_png("size.png", 160, 90, |_, _| [10, 20, 30]);
        let source = open(&path).unwrap();
        for target in [(344, 144), (100, 100), (90, 160), (1, 1)] {
            let frame = render_frame(&source, &FrameStyle::default(), target).unwrap();
            assert_eq!((frame.width(), frame.height()), target);
        }
    }

    /// Covering crops the long side evenly: a 16:9 image on a 21:9 screen
    /// loses the same amount from the top as from the bottom.
    #[test]
    fn a_frame_is_centred() {
        // Top half red, bottom half blue, with a 10-row green band at each
        // end that a centred crop to 1440/1935 of the height cuts away.
        let path = write_png("centre.png", 384, 216, |_, y| {
            if !(10..206).contains(&y) {
                [0, 255, 0]
            } else if y < 108 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            }
        });
        let frame =
            render_frame(&open(&path).unwrap(), &FrameStyle::default(), (344, 144)).unwrap();
        assert_eq!(pixel(&frame, 172, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 172, 143), [0, 0, 255, 255]);
        let middle_up = pixel(&frame, 172, 70);
        let middle_down = pixel(&frame, 172, 73);
        assert_eq!(middle_up, [255, 0, 0, 255]);
        assert_eq!(middle_down, [0, 0, 255, 255]);
    }

    /// An SVG keeps its own shape: a circle in a 16:9 document is still
    /// round on a 21:9 screen, and a square one, rather than squashed into
    /// the square an SVG used to be rasterised to first.
    #[test]
    fn an_svg_keeps_its_aspect() {
        let dir = std::env::temp_dir().join(format!("otto-kit-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("circle.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90">
                <rect width="160" height="90" fill="#000"/>
                <circle cx="80" cy="45" r="10" fill="#fff"/>
            </svg>"##,
        )
        .unwrap();
        let source = open(path.to_str().unwrap()).unwrap();
        for target in [(344, 144), (300, 300), (144, 344)] {
            let frame = render_frame(&source, &FrameStyle::default(), target).unwrap();
            assert_eq!((frame.width(), frame.height()), target);
            // The circle's bounding box, from its bright pixels.
            let (mut left, mut right, mut top, mut bottom) =
                (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
            for y in 0..target.1 {
                for x in 0..target.0 {
                    if pixel(&frame, x, y)[0] > 127 {
                        left = left.min(x);
                        right = right.max(x);
                        top = top.min(y);
                        bottom = bottom.max(y);
                    }
                }
            }
            let (w, h) = (right - left + 1, bottom - top + 1);
            assert!((w - h).abs() <= 2, "{target:?}: the circle is {w}x{h}");
            // And centred.
            assert!(((left + right) / 2 - target.0 / 2).abs() <= 1, "{target:?}");
            assert!(((top + bottom) / 2 - target.1 / 2).abs() <= 1, "{target:?}");
        }
    }

    fn style(mode: Mode) -> FrameStyle {
        FrameStyle {
            layout: Layout {
                mode,
                ..Layout::default()
            },
            fill: skia::Color4f::new(0.0, 0.0, 1.0, 1.0),
            output_scale: 1.0,
        }
    }

    fn noise_png(name: &str, w: i32, h: i32) -> String {
        write_png(name, w, h, |x, y| {
            let mut s = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
            s ^= s >> 15;
            s = s.wrapping_mul(0x2C1B_3C6D);
            s ^= s >> 12;
            [
                (s & 0xff) as u8,
                ((s >> 8) & 0xff) as u8,
                ((s >> 16) & 0xff) as u8,
            ]
        })
    }

    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];

    /// Fit shows the whole image with the fill colour around it, its edges
    /// on whole pixels.
    #[test]
    fn fit_letterboxes_with_the_fill_colour() {
        let path = write_png("fit.png", 200, 100, |_, _| [255, 0, 0]);
        let frame = render_frame(&open(&path).unwrap(), &style(Mode::Fit), (300, 300)).unwrap();
        // 1.5x to 300x150, centred: rows 75 to 224.
        assert_eq!(pixel(&frame, 150, 74), BLUE);
        assert_eq!(pixel(&frame, 150, 75), RED);
        assert_eq!(pixel(&frame, 150, 224), RED);
        assert_eq!(pixel(&frame, 150, 225), BLUE);
        assert_eq!(pixel(&frame, 0, 150), RED);
        assert_eq!(pixel(&frame, 299, 150), RED);
    }

    /// The position picks the crop: `(0.5, 0.0)` keeps the image's top edge,
    /// which centring cuts away.
    #[test]
    fn fill_position_keeps_the_chosen_edge() {
        let path = write_png("top.png", 384, 216, |_, y| {
            if y < 10 {
                [0, 255, 0]
            } else {
                [255, 0, 0]
            }
        });
        let source = open(&path).unwrap();
        let centred = render_frame(&source, &style(Mode::Fill), (344, 144)).unwrap();
        assert_eq!(pixel(&centred, 172, 0), RED);
        let mut top = style(Mode::Fill);
        top.layout.position = (0.5, 0.0);
        let top = render_frame(&source, &top, (344, 144)).unwrap();
        assert_eq!(pixel(&top, 172, 0), [0, 255, 0, 255]);
    }

    /// Zoom magnifies around the position: at 2x a centred frame shows only
    /// the middle half of the image, so a border a quarter deep is gone.
    #[test]
    fn zoom_crops_into_the_image() {
        let path = write_png("zoom.png", 200, 200, |x, y| {
            if (50..150).contains(&x) && (50..150).contains(&y) {
                [255, 0, 0]
            } else {
                [0, 255, 0]
            }
        });
        let mut zoomed = style(Mode::Fill);
        zoomed.layout.zoom = 2.0;
        let frame = render_frame(&open(&path).unwrap(), &zoomed, (100, 100)).unwrap();
        for (x, y) in [(1, 1), (98, 1), (1, 98), (98, 98), (50, 50)] {
            assert_eq!(pixel(&frame, x, y), RED, "({x},{y})");
        }
    }

    /// Centre at scale 1 copies the image's pixels exactly, no resampling.
    #[test]
    fn center_copies_pixels_exactly() {
        let path = noise_png("center.png", 50, 40);
        let source = open(&path).unwrap();
        let Source::Raster(image) = &source else {
            unreachable!()
        };
        let frame = render_frame(&source, &style(Mode::Center), (100, 100)).unwrap();
        assert_eq!(pixel(&frame, 24, 50), BLUE);
        for y in 0..40 {
            for x in 0..50 {
                assert_eq!(
                    pixel(&frame, x + 25, y + 30),
                    pixel(image, x, y),
                    "({x},{y})"
                );
            }
        }
        // Larger than the screen, it is cropped, still exactly.
        let frame = render_frame(&source, &style(Mode::Center), (20, 10)).unwrap();
        assert_eq!(pixel(&frame, 0, 0), pixel(image, 15, 15));
    }

    /// Centre follows the output's scale: the image's pixels are points.
    #[test]
    fn center_scales_with_the_output() {
        let path = write_png("center2.png", 20, 10, |_, _| [255, 0, 0]);
        let mut hidpi = style(Mode::Center);
        hidpi.output_scale = 2.0;
        let frame = render_frame(&open(&path).unwrap(), &hidpi, (100, 100)).unwrap();
        assert_eq!(pixel(&frame, 30, 40), RED);
        assert_eq!(pixel(&frame, 69, 59), RED);
        assert_eq!(pixel(&frame, 29, 40), BLUE);
        assert_eq!(pixel(&frame, 70, 59), BLUE);
    }

    /// Tiles repeat exactly, from the top-left corner.
    #[test]
    fn tile_repeats_exactly() {
        let path = noise_png("tile.png", 30, 20);
        let source = open(&path).unwrap();
        let Source::Raster(image) = &source else {
            unreachable!()
        };
        let frame = render_frame(&source, &style(Mode::Tile), (100, 70)).unwrap();
        for y in 0..70 {
            for x in 0..100 {
                assert_eq!(
                    pixel(&frame, x, y),
                    pixel(image, x % 30, y % 20),
                    "({x},{y})"
                );
            }
        }
    }

    /// Stretch maps the image's corners onto the screen's.
    #[test]
    fn stretch_fills_the_screen_exactly() {
        let path = write_png("stretch.png", 40, 40, |x, y| match (x < 20, y < 20) {
            (true, true) => [255, 0, 0],
            (false, true) => [0, 255, 0],
            (true, false) => [0, 0, 255],
            (false, false) => [255, 255, 255],
        });
        let frame = render_frame(&open(&path).unwrap(), &style(Mode::Stretch), (200, 50)).unwrap();
        assert_eq!(pixel(&frame, 2, 2), RED);
        assert_eq!(pixel(&frame, 197, 2), [0, 255, 0, 255]);
        assert_eq!(pixel(&frame, 2, 47), BLUE);
        assert_eq!(pixel(&frame, 197, 47), [255, 255, 255, 255]);
    }

    /// A transparent image shows the fill colour through it, and every frame
    /// is opaque.
    #[test]
    fn transparency_shows_the_fill_colour() {
        let mut surface = skia::surfaces::raster_n32_premul((10, 10)).unwrap();
        surface.canvas().clear(skia::Color::TRANSPARENT);
        let png = surface
            .image_snapshot()
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let dir = std::env::temp_dir().join(format!("otto-kit-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clear.png");
        std::fs::write(&path, png.as_bytes()).unwrap();
        let frame = render_frame(
            &open(path.to_str().unwrap()).unwrap(),
            &style(Mode::Fill),
            (20, 20),
        )
        .unwrap();
        assert_eq!(pixel(&frame, 10, 10), BLUE);
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        assert!(matches!(
            open("/nonexistent/otto/wallpaper.png"),
            Err(OpenError::Unreadable(_))
        ));
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused() {
        let dir = std::env::temp_dir().join(format!("otto-kit-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("not-an-image.png");
        std::fs::write(&path, b"plain text").unwrap();
        assert_eq!(
            open(path.to_str().unwrap()).unwrap_err(),
            OpenError::NotAnImage
        );
    }

    /// The orientation a camera records in EXIF is applied: a JPEG stored
    /// landscape but tagged "rotate 90° clockwise" opens portrait.
    #[test]
    fn exif_orientation_is_respected() {
        let landscape = {
            let mut surface = skia::surfaces::raster_n32_premul((64, 32)).unwrap();
            surface.canvas().clear(skia::Color::RED);
            surface.image_snapshot()
        };
        let jpeg = landscape
            .encode(None, skia::EncodedImageFormat::JPEG, 90)
            .unwrap();
        let tagged = with_exif_orientation(jpeg.as_bytes(), 6);
        let dir = std::env::temp_dir().join(format!("otto-kit-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rotated.jpg");
        std::fs::write(&path, tagged).unwrap();
        let Source::Raster(image) = open(path.to_str().unwrap()).unwrap() else {
            panic!("a JPEG opens as a raster source");
        };
        assert_eq!((image.width(), image.height()), (32, 64));
    }

    /// `jpeg` with an APP1 EXIF segment carrying `orientation` inserted
    /// straight after the SOI marker.
    fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\0\x2a\0\0\0\x08"); // big-endian, IFD at 8
        tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_be_bytes()); // count
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]); // value padding
        tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }
}
