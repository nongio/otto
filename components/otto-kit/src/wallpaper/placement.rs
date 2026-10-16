//! Mapping a wallpaper image onto a target, in physical pixels.
//!
//! The one rule everything here keeps: **every offset is a whole pixel.** A
//! wallpaper drawn 1:1 at a half-pixel offset puts each pixel centre exactly
//! on the boundary between two image pixels, so which one nearest sampling
//! reads hangs on the last bit of the interpolated texture coordinate. That
//! bit differs between a whole-screen paint and the cropped rect a partial
//! repaint draws, and between the two triangles a rect is rasterised as — the
//! wallpaper then shimmers around a dragged window, along a diagonal from the
//! damage rect's corner (nongio/otto#361).

use skia_safe as skia;

/// Where an image lands on a target: `target_px = image_px * scale + offset`.
///
/// All values are in physical pixels. Offsets are whole numbers by
/// construction; the scale may be anything.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub scale_x: f32,
    pub scale_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Placement {
    /// The image drawn at its own size at the target's origin.
    pub const IDENTITY: Placement = Placement {
        scale_x: 1.0,
        scale_y: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };

    /// Whether one image pixel lands on exactly one target pixel.
    pub fn is_identity_scale(&self) -> bool {
        self.scale_x == 1.0 && self.scale_y == 1.0
    }

    /// Whether every target pixel centre maps onto an image pixel centre:
    /// the one mapping nearest sampling reproduces exactly, whatever the GPU
    /// and however the draw is cropped.
    pub fn is_pixel_exact(&self) -> bool {
        self.is_identity_scale() && self.offset_x.fract() == 0.0 && self.offset_y.fract() == 0.0
    }

    /// The image-space to target-space matrix, for an image shader.
    pub fn matrix(&self) -> skia::Matrix {
        let mut matrix = skia::Matrix::new_identity();
        matrix.set_scale_translate((self.scale_x, self.scale_y), (self.offset_x, self.offset_y));
        matrix
    }

    /// The image's bounds on the target.
    pub fn dest_rect(&self, image: (i32, i32)) -> skia::Rect {
        skia::Rect::from_xywh(
            self.offset_x,
            self.offset_y,
            image.0 as f32 * self.scale_x,
            image.1 as f32 * self.scale_y,
        )
    }
}

/// Scale `image` uniformly to cover `target`, centred, cropping the excess.
///
/// The centring offset is rounded to a whole pixel, and kept inside the range
/// in which the image still covers the target on both sides.
pub fn cover(image: (i32, i32), target: (i32, i32)) -> Placement {
    let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
    let (tw, th) = (target.0.max(0) as f32, target.1.max(0) as f32);
    let scale = (tw / iw).max(th / ih);
    Placement {
        scale_x: scale,
        scale_y: scale,
        offset_x: centred_offset(tw, iw * scale),
        offset_y: centred_offset(th, ih * scale),
    }
}

/// The size to resample `image` to so that [`cover`] then draws it 1:1.
///
/// One edge matches the target exactly, the other keeps the image's aspect
/// and is at least the target's, so [`cover`] of an image this size has a
/// scale of exactly 1.0. Derived from the target rather than by multiplying
/// the image by a float scale, which can land a pixel over and leave the
/// draw scale a hair under one.
pub fn cover_size(image: (i32, i32), target: (i32, i32)) -> (i32, i32) {
    let (iw, ih) = (image.0.max(1) as f64, image.1.max(1) as f64);
    let (tw, th) = (target.0.max(1), target.1.max(1));
    if (tw as f64 / iw) >= (th as f64 / ih) {
        // Width-bound: the width is the target's, the height follows.
        (tw, ((tw as f64 * ih / iw).round() as i32).max(th))
    } else {
        (((th as f64 * iw / ih).round() as i32).max(tw), th)
    }
}

/// A whole-pixel offset that centres an `extent`-long span on a
/// `target`-long one, kept where the span still covers the target when it is
/// the larger of the two.
fn centred_offset(target: f32, extent: f32) -> f32 {
    let centred = ((target - extent) / 2.0).round();
    if extent >= target {
        // Covering: the offset has to stay in [target - extent, 0]. The
        // ceiling keeps a span that is a fraction of a pixel longer than the
        // target from being pushed off its far edge.
        centred.clamp((target - extent).ceil().min(0.0), 0.0)
    } else {
        centred
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Image and screen sizes the desktop meets: phone-camera and DSLR
    /// photos, 4K/5K/8K renders, ultrawide and portrait screens, and a few
    /// odd sizes that divide nothing.
    const IMAGES: &[(i32, i32)] = &[
        (1920, 1080),
        (2560, 1440),
        (3840, 2160),
        (5120, 2880),
        (7680, 4320),
        (6000, 4000),
        (4032, 3024),
        (3024, 4032),
        (5160, 2160),
        (2048, 2048),
        (999, 701),
        (1366, 768),
        (640, 480),
    ];
    const SCREENS: &[(i32, i32)] = &[
        (1920, 1080),
        (2560, 1440),
        (2560, 1600),
        (2880, 1920),
        (3440, 1440),
        (3840, 2160),
        (5120, 1440),
        (1440, 2560),
        (1366, 768),
        (1280, 800),
    ];

    /// The cover placement of any image on any screen keeps both offsets on
    /// whole pixels and covers the screen edge to edge.
    #[test]
    fn cover_offsets_are_whole_pixels_and_the_screen_is_covered() {
        for &image in IMAGES {
            for &screen in SCREENS {
                let p = cover(image, screen);
                assert_eq!(p.offset_x.fract(), 0.0, "{image:?} on {screen:?}: {p:?}");
                assert_eq!(p.offset_y.fract(), 0.0, "{image:?} on {screen:?}: {p:?}");
                let rect = p.dest_rect(image);
                assert!(
                    rect.left <= 0.0 && rect.top <= 0.0,
                    "{image:?} on {screen:?}"
                );
                assert!(
                    rect.right >= screen.0 as f32 - 1e-3 && rect.bottom >= screen.1 as f32 - 1e-3,
                    "{image:?} on {screen:?}: {rect:?}"
                );
                // Centred to within the half pixel the rounding costs.
                let slack_x = (rect.left + rect.right - screen.0 as f32).abs() / 2.0;
                let slack_y = (rect.top + rect.bottom - screen.1 as f32).abs() / 2.0;
                assert!(slack_x <= 0.5 + 1e-3 && slack_y <= 0.5 + 1e-3);
            }
        }
    }

    /// An image resampled to [`cover_size`] is then drawn exactly 1:1 — the
    /// mapping nearest sampling is exact for. This is the case #361 hit: a
    /// 16:9 photo on a 21:9 screen came out at 3440x1935, centred at a
    /// half-pixel offset of -247.5.
    #[test]
    fn a_wallpaper_resampled_to_the_cover_size_is_drawn_pixel_exact() {
        for &image in IMAGES {
            for &screen in SCREENS {
                let size = cover_size(image, screen);
                assert!(size.0 >= screen.0 && size.1 >= screen.1);
                assert!(size.0 == screen.0 || size.1 == screen.1);
                let p = cover(size, screen);
                assert!(
                    p.is_pixel_exact(),
                    "{image:?} on {screen:?}: {size:?} → {p:?}"
                );
            }
        }
        let p = cover(cover_size((3840, 2160), (3440, 1440)), (3440, 1440));
        assert_eq!((p.offset_x, p.offset_y), (0.0, -248.0));
    }

    /// The property [`Placement::is_pixel_exact`] stands for, checked pixel
    /// by pixel: every target pixel centre maps onto an image pixel centre,
    /// as far as possible from the boundary nearest sampling would have to
    /// decide on.
    #[test]
    fn a_pixel_exact_placement_samples_texel_centres() {
        for &image in IMAGES {
            for &screen in SCREENS {
                let p = cover(cover_size(image, screen), screen);
                for x in 0..screen.0 {
                    let u = (x as f32 + 0.5 - p.offset_x) / p.scale_x;
                    assert_eq!(u.fract(), 0.5, "{image:?} on {screen:?}, column {x}");
                }
                for y in 0..screen.1 {
                    let v = (y as f32 + 0.5 - p.offset_y) / p.scale_y;
                    assert_eq!(v.fract(), 0.5, "{image:?} on {screen:?}, row {y}");
                }
            }
        }
    }

    /// An image a fraction of a pixel longer than the screen is not pushed off
    /// its far edge by rounding the centring offset.
    #[test]
    fn a_nearly_matching_image_still_covers() {
        assert_eq!(centred_offset(1440.0, 1440.4), 0.0);
        assert_eq!(centred_offset(1440.0, 1441.0), -1.0);
        assert_eq!(centred_offset(1440.0, 1442.0), -1.0);
        assert_eq!(centred_offset(1440.0, 1935.0), -248.0);
    }
}
