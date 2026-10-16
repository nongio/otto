//! Painting a placed wallpaper image.

use skia_safe as skia;

use super::{arrange, Layout, Placement};

/// How the wallpaper image is sampled: bilinear, with mipmaps.
///
/// On the desktop the image is a frame rendered ahead of time at the
/// screen's size ([`super::render_frame`]) and drawn 1:1, so every pixel
/// centre lands on a texel centre and bilinear reads exactly that texel —
/// the same pixels as nearest sampling, but without nearest's coin-toss when
/// rounding nudges a coordinate. The same recorded picture is also replayed small, as the
/// exposé backdrop and the workspace selector's previews, and there the
/// mipmaps are what keeps a 0.2x copy from aliasing.
fn sampling() -> skia::SamplingOptions {
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

/// Fill `bounds` with `image` laid out by `layout` on `fill`, sampled as it
/// is drawn.
///
/// For a surface that shows the wallpaper without the compositor's frames —
/// the lock screen and the greeter, which are separate clients — so it is
/// laid out the way the desktop is. `bounds` and the image's natural size
/// are in the same units: the image's pixels are taken as `bounds` units for
/// the modes that draw it at its own size, so pass a point-sized `bounds` to
/// keep an image pixel a point. The desktop itself renders a frame ahead of
/// time instead, which is sharper and cheaper to draw.
pub fn paint_layout(
    canvas: &skia::Canvas,
    image: &skia::Image,
    layout: Layout,
    fill: skia::Color4f,
    bounds: skia::Rect,
) {
    let mut fill = fill;
    fill.a = 1.0;
    canvas.draw_rect(bounds, &skia::Paint::new(fill, None));
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    if iw <= 0.0 || ih <= 0.0 || bounds.is_empty() {
        return;
    }
    let target = (
        bounds.width().round() as i32,
        bounds.height().round() as i32,
    );
    let arrangement = arrange(layout, (iw, ih), target, 1.0, None);
    let dest = arrangement.dest;
    let placement = Placement {
        scale_x: dest.width() / iw,
        scale_y: dest.height() / ih,
        offset_x: bounds.left + dest.left,
        offset_y: bounds.top + dest.top,
    };
    // Repeated for a tile; elsewhere nothing past the image's edge, so the
    // fill shows around a fitted or centred one.
    let tiling = if arrangement.tiled {
        skia::TileMode::Repeat
    } else {
        skia::TileMode::Decal
    };
    let mut paint = skia::Paint::default();
    paint.set_shader(image.to_shader((tiling, tiling), sampling(), &placement.matrix()));
    canvas.draw_rect(bounds, &paint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::{cover, render_frame, FrameStyle, Source};

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

    /// A frame rendered for the screen reaches it pixel for pixel.
    #[test]
    fn a_frame_is_painted_pixel_for_pixel() {
        let (screen_w, screen_h) = (344, 144);
        let source = render_frame(
            &Source::Raster(noise(384, 216)),
            &FrameStyle::default(),
            (screen_w, screen_h),
        )
        .unwrap();
        let (iw, ih) = (source.width(), source.height());
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
        let source = render_frame(
            &Source::Raster(noise(999, 701)),
            &FrameStyle::default(),
            (screen_w, screen_h),
        )
        .unwrap();
        let placement = cover((source.width(), source.height()), (screen_w, screen_h));
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
            Placement {
                scale_x: 1.0,
                scale_y: 1.0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
            skia::Rect::from_iwh(1000, 200),
        );
        let px = surface_pixels(&mut surface);
        let reds: Vec<i32> = px.chunks(4).map(|p| p[0] as i32).collect();
        let (min, max) = (*reds.iter().min().unwrap(), *reds.iter().max().unwrap());
        assert!(max - min <= 8, "aliased: {min}..{max}");
    }

    /// Drawn without a frame, the layout still applies: fitted, a wide red
    /// image sits on the fill colour.
    #[test]
    fn paint_layout_lays_the_image_out() {
        let source = image(40, 20, |_, _| [255, 0, 0]);
        let mut surface = skia::surfaces::raster_n32_premul((100, 100)).unwrap();
        let layout = Layout {
            mode: crate::wallpaper::Mode::Fit,
            ..Layout::default()
        };
        paint_layout(
            surface.canvas(),
            &source,
            layout,
            skia::Color4f::new(0.0, 0.0, 1.0, 1.0),
            skia::Rect::from_iwh(100, 100),
        );
        let px = surface_pixels(&mut surface);
        let at = |x: i32, y: i32| {
            let i = ((y * 100 + x) * 4) as usize;
            px[i..i + 4].to_vec()
        };
        // Red and blue are the two non-zero channels; which byte is which
        // depends on N32, so compare against each other.
        assert_ne!(at(50, 50), at(50, 5), "the image is not the bar");
        assert_eq!(at(50, 5), at(50, 95), "both bars are the fill");
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
