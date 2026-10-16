//! Reading a wallpaper file, and rendering it into a screen-sized frame.

use std::sync::Arc;

use skia_safe as skia;

use super::cover;
use crate::utils::resample::resample;

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

/// Render `source` into a frame of exactly `target` pixels, scaled to cover
/// it and centred, so the frame is drawn 1:1 with nothing filtered per frame.
///
/// The visible region is resampled straight into the frame: the centring
/// keeps its sub-pixel precision, and only the part of the image that shows
/// is ever held at screen resolution.
pub fn render_frame(source: &Source, target: (i32, i32)) -> Option<skia::Image> {
    let (tw, th) = target;
    if tw <= 0 || th <= 0 {
        return None;
    }
    let image = match source {
        Source::Raster(image) => image.clone(),
        // Vector art is drawn straight at the frame's size: there is nothing
        // to resample.
        Source::Svg(bytes) => return render_svg_frame(bytes, target),
    };
    let (iw, ih) = (image.width(), image.height());
    if iw <= 0 || ih <= 0 {
        return None;
    }
    // The cover scale, unrounded: the frame is resampled, not drawn, so the
    // crop can sit on fractional source pixels.
    let scale = cover((iw, ih), target).scale_x as f64;
    let (src_w, src_h) = (tw as f64 / scale, th as f64 / scale);
    let src = skia::Rect::from_xywh(
        ((iw as f64 - src_w) / 2.0) as f32,
        ((ih as f64 - src_h) / 2.0) as f32,
        src_w as f32,
        src_h as f32,
    );
    resample(&image, src, target)
}

/// Render an SVG document scaled to cover `target` and centred, at its own
/// aspect ratio.
fn render_svg_frame(bytes: &[u8], target: (i32, i32)) -> Option<skia::Image> {
    let (tw, th) = target;
    let options = usvg::Options {
        languages: vec!["en".to_string()],
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let (sw, sh) = (tree.size().width(), tree.size().height());
    if !(sw > 0.0 && sh > 0.0) {
        return None;
    }
    let scale = (tw as f32 / sw).max(th as f32 / sh);
    let transform = resvg::tiny_skia::Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        (tw as f32 - sw * scale) / 2.0,
        (th as f32 - sh * scale) / 2.0,
    );
    let mut pixmap = resvg::tiny_skia::Pixmap::new(tw as u32, th as u32)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let info = skia::ImageInfo::new(
        (tw, th),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        Some(skia::ColorSpace::new_srgb()),
    );
    skia::images::raster_from_data(&info, skia::Data::new_copy(pixmap.data()), tw as usize * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let frame = render_frame(&source, target).unwrap();
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
        let frame = render_frame(&open(&path).unwrap(), (344, 144)).unwrap();
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
            let frame = render_frame(&source, target).unwrap();
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
