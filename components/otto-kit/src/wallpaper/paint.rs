//! Painting a placed wallpaper image.

use skia_safe as skia;

use super::Placement;

/// How the wallpaper image is sampled: bilinear, with mipmaps.
///
/// The image is resampled ahead of time to draw 1:1 at a whole-pixel offset
/// ([`super::cover_size`]), so on the desktop every pixel centre lands on a
/// texel centre and bilinear reads exactly that texel — the same pixels as
/// nearest sampling, but without nearest's coin-toss when rounding nudges a
/// coordinate. The same recorded picture is also replayed small, as the
/// exposé backdrop and the workspace selector's previews, and there the
/// mipmaps are what keeps a 0.2x copy from aliasing.
pub fn sampling() -> skia::SamplingOptions {
    skia::SamplingOptions::new(skia::FilterMode::Linear, skia::MipmapMode::Linear)
}

/// Fill `bounds` on `canvas` with `image` mapped by `placement`.
///
/// Outside the image the edge pixels are extended rather than repeated: a
/// placement that does not quite cover would otherwise show the opposite
/// edge of the picture, and with bilinear sampling even a covering one
/// blends it into its outermost pixels.
pub fn paint(canvas: &skia::Canvas, image: &skia::Image, placement: Placement, bounds: skia::Rect) {
    let mut paint = skia::Paint::default();
    paint.set_shader(image.to_shader(
        (skia::TileMode::Clamp, skia::TileMode::Clamp),
        sampling(),
        &placement.matrix(),
    ));
    canvas.draw_rect(bounds, &paint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::{cover, cover_size};

    fn image(w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 3]) -> skia::Image {
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
        skia::images::raster_from_data(&info, skia::Data::new_copy(&data), (w * 4) as usize)
            .unwrap()
    }

    /// Pseudo-random texels: every one differs from its neighbours, so
    /// reading the wrong one changes the pixel.
    fn noise(w: i32, h: i32) -> skia::Image {
        image(w, h, |x, y| {
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

    fn surface_pixels(surface: &mut skia::Surface) -> Vec<u8> {
        let info = surface.image_info();
        let mut data = vec![0u8; (info.width() * info.height() * 4) as usize];
        let ok = surface.read_pixels(
            &info,
            data.as_mut_slice(),
            info.width() as usize * 4,
            (0, 0),
        );
        assert!(ok);
        data
    }

    fn image_pixels(image: &skia::Image) -> Vec<u8> {
        let info = skia::ImageInfo::new_n32_premul((image.width(), image.height()), None);
        let mut data = vec![0u8; (image.width() * image.height() * 4) as usize];
        assert!(image.read_pixels(
            &info,
            data.as_mut_slice(),
            image.width() as usize * 4,
            (0, 0),
            skia::image::CachingHint::Disallow,
        ));
        data
    }

    /// Drawn at the cover size, the wallpaper reaches the screen pixel for
    /// pixel: the crop of the image the placement selects, untouched.
    #[test]
    fn a_cover_sized_image_is_painted_pixel_for_pixel() {
        let (screen_w, screen_h) = (344, 144);
        let (iw, ih) = cover_size((384, 216), (screen_w, screen_h));
        let source = noise(iw, ih);
        let placement = cover((iw, ih), (screen_w, screen_h));
        assert!(placement.is_pixel_exact());

        let mut surface = skia::surfaces::raster_n32_premul((screen_w, screen_h)).unwrap();
        paint(
            surface.canvas(),
            &source,
            placement,
            skia::Rect::from_iwh(screen_w, screen_h),
        );
        let painted = surface_pixels(&mut surface);
        let all = image_pixels(&source);
        let (dx, dy) = (-placement.offset_x as i32, -placement.offset_y as i32);
        for y in 0..screen_h {
            for x in 0..screen_w {
                let p = ((y * screen_w + x) * 4) as usize;
                let s = (((y + dy) * iw + x + dx) * 4) as usize;
                assert_eq!(painted[p..p + 4], all[s..s + 4], "({x},{y})");
            }
        }
    }

    /// A repaint cropped to a damage rect gives exactly the pixels a full
    /// paint does there.
    #[test]
    fn a_cropped_repaint_matches_the_full_paint() {
        let (screen_w, screen_h) = (344, 144);
        let size = cover_size((999, 701), (screen_w, screen_h));
        let source = noise(size.0, size.1);
        let placement = cover(size, (screen_w, screen_h));
        let bounds = skia::Rect::from_iwh(screen_w, screen_h);

        let mut full = skia::surfaces::raster_n32_premul((screen_w, screen_h)).unwrap();
        paint(full.canvas(), &source, placement, bounds);
        let mut partial = skia::surfaces::raster_n32_premul((screen_w, screen_h)).unwrap();
        partial.canvas().clear(skia::Color::BLACK);
        partial
            .canvas()
            .clip_irect(skia::IRect::from_xywh(41, 23, 150, 97), None);
        paint(partial.canvas(), &source, placement, bounds);

        let (a, b) = (surface_pixels(&mut full), surface_pixels(&mut partial));
        for y in 23..120 {
            for x in 41..191 {
                let i = ((y * screen_w + x) * 4) as usize;
                assert_eq!(a[i..i + 4], b[i..i + 4], "({x},{y})");
            }
        }
    }

    /// Replayed small, as the exposé backdrop and the workspace previews do,
    /// fine detail averages out instead of aliasing: 1 px stripes at 0.2x
    /// are an even grey.
    #[test]
    fn a_small_replay_does_not_alias() {
        let source = image(1000, 200, |x, _| if x % 2 == 0 { [0; 3] } else { [255; 3] });
        let mut surface = skia::surfaces::raster_n32_premul((200, 40)).unwrap();
        surface.canvas().scale((0.2, 0.2));
        paint(
            surface.canvas(),
            &source,
            Placement::IDENTITY,
            skia::Rect::from_iwh(1000, 200),
        );
        let px = surface_pixels(&mut surface);
        let reds: Vec<i32> = px.chunks(4).map(|p| p[0] as i32).collect();
        let (min, max) = (*reds.iter().min().unwrap(), *reds.iter().max().unwrap());
        assert!(max - min <= 8, "aliased: {min}..{max}");
    }

    /// Past the image's edge the edge pixels extend: the far side of the
    /// picture never shows up at the near side.
    #[test]
    fn the_far_edge_never_wraps_around() {
        let source = image(
            10,
            10,
            |x, _| if x == 9 { [0, 0, 255] } else { [255, 0, 0] },
        );
        let mut surface = skia::surfaces::raster_n32_premul((20, 10)).unwrap();
        let placement = Placement {
            scale_x: 1.7,
            scale_y: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        paint(
            surface.canvas(),
            &source,
            placement,
            skia::Rect::from_iwh(20, 10),
        );
        let px = surface_pixels(&mut surface);
        // N32 is BGRA here or RGBA; blue is whichever is not red at column 0.
        let left = &px[0..4];
        assert!(left[0] == 255 || left[2] == 255, "{left:?}");
        assert!(
            left[0] == 0 || left[2] == 0,
            "blue wrapped to the left edge: {left:?}"
        );
    }
}
