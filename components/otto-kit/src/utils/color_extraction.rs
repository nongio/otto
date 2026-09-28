//! Dominant accent color extraction from a Skia image.
//!
//! Algorithm:
//! 1. Sample up to 32×32 pixels via integer strides.
//! 2. Quantize each RGB into 8 levels per channel → 512 buckets.
//! 3. Count per-bucket population.
//! 4. Score each bucket: `population × saturation^1.5 × brightness_weight`.
//!    `brightness_weight` peaks at ~0.55, penalising near-black and near-white.
//! 5. Return the bucket-centre colour with the highest score.

use skia_safe::{AlphaType, Color, ColorType, Image, ImageInfo};

const GRID: usize = 32;
const LEVELS: usize = 8;
const BUCKETS: usize = LEVELS * LEVELS * LEVELS;

/// Extract the most visually interesting accent colour from `image`.
///
/// Samples the image at up to 32×32 points, quantizes into coarse RGB buckets,
/// scores each bucket by `population × saturation^1.5 × brightness_weight`,
/// then boosts the winner to be legible on a black background (S≥0.55, V≥0.75).
pub fn extract_accent_color(image: &Image) -> Color {
    let pixels = sample_pixels(image);
    if pixels.is_empty() {
        return Color::from_rgb(120, 120, 120);
    }

    let mut counts = [0u32; BUCKETS];
    for (r, g, b) in &pixels {
        let ri = ((*r as usize) * LEVELS / 256).min(LEVELS - 1);
        let gi = ((*g as usize) * LEVELS / 256).min(LEVELS - 1);
        let bi = ((*b as usize) * LEVELS / 256).min(LEVELS - 1);
        counts[ri * LEVELS * LEVELS + gi * LEVELS + bi] += 1;
    }

    let total = pixels.len() as f32;
    let mut best_score = -1.0f32;
    let mut best_bucket = 0usize;

    for (idx, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let ri = idx / (LEVELS * LEVELS);
        let gi = (idx / LEVELS) % LEVELS;
        let bi = idx % LEVELS;

        let rf = (ri as f32 + 0.5) / LEVELS as f32;
        let gf = (gi as f32 + 0.5) / LEVELS as f32;
        let bf = (bi as f32 + 0.5) / LEVELS as f32;

        let (sat, brightness) = rgb_to_sb(rf, gf, bf);

        // Hard-reject near-black (invisible on dark bg), near-white, and near-grey
        if !(0.35..=0.93).contains(&brightness) || sat < 0.2 {
            continue;
        }

        // Prefer vivid mid-bright colours; peak around V=0.70
        let brightness_weight = 1.0 - ((brightness - 0.70) * 2.0).abs().min(1.0) * 0.5;
        let pop_weight = (count as f32 / total).sqrt();
        let score = pop_weight * sat.powf(1.5) * brightness_weight;

        if score > best_score {
            best_score = score;
            best_bucket = idx;
        }
    }

    let ri = best_bucket / (LEVELS * LEVELS);
    let gi = (best_bucket / LEVELS) % LEVELS;
    let bi = best_bucket % LEVELS;

    let rf = (ri as f32 + 0.5) / LEVELS as f32;
    let gf = (gi as f32 + 0.5) / LEVELS as f32;
    let bf = (bi as f32 + 0.5) / LEVELS as f32;

    let boosted = ensure_visible(rf, gf, bf);
    Color::from_rgb(boosted.0, boosted.1, boosted.2)
}

/// The `n` colours `image` is mostly made of, most common first.
///
/// Samples the image the way [`extract_accent_color`] does, sorts the coarse
/// RGB buckets by how many samples fell in each, and walks down that list
/// keeping a bucket only when it is visibly different from every colour
/// already kept. Each colour is the average of the samples in its bucket, not
/// the bucket's centre, so a flat red comes back as that red.
///
/// Unlike the accent, nothing is boosted or rejected: a palette of a grey
/// photograph is greys. Fewer than `n` colours come back when the picture
/// does not have that many distinct ones, and none for an empty image.
pub fn extract_palette(image: &Image, n: usize) -> Vec<Color> {
    palette_of(&sample_pixels(image), n)
}

/// The palette over already-sampled pixels. Split out so the choice of
/// colours can be tested without a raster surface.
fn palette_of(pixels: &[(u8, u8, u8)], n: usize) -> Vec<Color> {
    /// How far apart two kept colours must be, as a Euclidean distance in
    /// 0..255 RGB. Neighbouring buckets of one gradient are closer than this,
    /// so a sky does not fill the whole palette with blues.
    const MIN_DISTANCE: f32 = 48.0;

    if pixels.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut sums = vec![(0u32, 0u32, 0u32, 0u32); BUCKETS];
    for &(r, g, b) in pixels {
        let ri = (r as usize * LEVELS / 256).min(LEVELS - 1);
        let gi = (g as usize * LEVELS / 256).min(LEVELS - 1);
        let bi = (b as usize * LEVELS / 256).min(LEVELS - 1);
        let bucket = &mut sums[ri * LEVELS * LEVELS + gi * LEVELS + bi];
        bucket.0 += r as u32;
        bucket.1 += g as u32;
        bucket.2 += b as u32;
        bucket.3 += 1;
    }
    let mut buckets: Vec<(u32, (u8, u8, u8))> = sums
        .iter()
        .filter(|s| s.3 > 0)
        .map(|&(r, g, b, count)| {
            (
                count,
                ((r / count) as u8, (g / count) as u8, (b / count) as u8),
            )
        })
        .collect();
    // Most common first; ties broken by the colour so the answer is stable.
    buckets.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut kept: Vec<(u8, u8, u8)> = Vec::with_capacity(n);
    for (_, colour) in buckets {
        if kept.len() == n {
            break;
        }
        let distinct = kept.iter().all(|k| distance(*k, colour) >= MIN_DISTANCE);
        if distinct {
            kept.push(colour);
        }
    }
    kept.into_iter()
        .map(|(r, g, b)| Color::from_rgb(r, g, b))
        .collect()
}

fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let d = |x: u8, y: u8| x as f32 - y as f32;
    (d(a.0, b.0).powi(2) + d(a.1, b.1).powi(2) + d(a.2, b.2).powi(2)).sqrt()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_pixels(image: &Image) -> Vec<(u8, u8, u8)> {
    let iw = image.width();
    let ih = image.height();
    if iw == 0 || ih == 0 {
        return Vec::new();
    }

    let sample_w = (GRID as i32).min(iw);
    let sample_h = (GRID as i32).min(ih);

    let info = ImageInfo::new(
        (sample_w, sample_h),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let mut surface = match skia_safe::surfaces::raster(&info, None, None) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let src = skia_safe::Rect::from_iwh(iw, ih);
    let dst = skia_safe::Rect::from_iwh(sample_w, sample_h);
    surface.canvas().draw_image_rect(
        image,
        Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
        dst,
        &skia_safe::Paint::default(),
    );

    let row_bytes = (sample_w as usize) * 4;
    let mut buf = vec![0u8; row_bytes * sample_h as usize];
    if !surface.read_pixels(&info, &mut buf, row_bytes, (0, 0)) {
        return Vec::new();
    }

    buf.chunks_exact(4).map(|c| (c[0], c[1], c[2])).collect()
}

/// RGB → (saturation, value) in HSV space. Inputs and outputs in 0..1.
fn rgb_to_sb(r: f32, g: f32, b: f32) -> (f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sat = if max < 1e-6 { 0.0 } else { (max - min) / max };
    (sat, max)
}

/// Force a colour to be vivid enough for a black background:
/// clamp HSV saturation ≥ 0.55 and value ≥ 0.75, then convert back to RGB u8.
fn ensure_visible(r: f32, g: f32, b: f32) -> (u8, u8, u8) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let hue = if delta < 1e-6 {
        0.0f32
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = ((hue % 360.0) + 360.0) % 360.0;

    let sat = (if max < 1e-6 { 0.0 } else { delta / max }).max(0.55);
    let val = max.max(0.75);

    let c = val * sat;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = val - c;
    let (r1, g1, b1) = match hue as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeat(colour: (u8, u8, u8), times: usize) -> Vec<(u8, u8, u8)> {
        vec![colour; times]
    }

    #[test]
    fn the_palette_is_the_colours_the_picture_is_made_of_most_common_first() {
        let mut pixels = repeat((242, 132, 92), 50);
        pixels.extend(repeat((20, 40, 200), 30));
        pixels.extend(repeat((250, 250, 250), 10));
        let palette = palette_of(&pixels, 5);
        assert_eq!(
            palette,
            vec![
                Color::from_rgb(242, 132, 92),
                Color::from_rgb(20, 40, 200),
                Color::from_rgb(250, 250, 250),
            ]
        );
    }

    #[test]
    fn near_neighbours_do_not_crowd_the_palette() {
        // Two shades of one blue, a bucket apart, and one red.
        let mut pixels = repeat((30, 60, 200), 40);
        pixels.extend(repeat((30, 60, 232), 30));
        pixels.extend(repeat((220, 30, 30), 5));
        let palette = palette_of(&pixels, 2);
        assert_eq!(
            palette,
            vec![Color::from_rgb(30, 60, 200), Color::from_rgb(220, 30, 30)]
        );
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert!(palette_of(&[], 5).is_empty());
        assert!(palette_of(&repeat((1, 2, 3), 4), 0).is_empty());
    }

    #[test]
    fn an_image_is_sampled() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((40, 20)).unwrap();
        let canvas = surface.canvas();
        canvas.clear(Color::from_rgb(0, 160, 80));
        let mut paint = skia_safe::Paint::default();
        paint.set_color(Color::from_rgb(200, 20, 40));
        canvas.draw_rect(skia_safe::Rect::from_xywh(0.0, 0.0, 10.0, 20.0), &paint);
        let palette = extract_palette(&surface.image_snapshot(), 5);
        assert_eq!(palette.len(), 2, "{palette:?}");
        assert_eq!(palette[0], Color::from_rgb(0, 160, 80));
        assert_eq!(palette[1], Color::from_rgb(200, 20, 40));
    }
}
