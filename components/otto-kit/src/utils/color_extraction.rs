//! Dominant accent color extraction from a Skia image.
//!
//! Algorithm:
//! 1. Sample up to 32×32 pixels via integer strides.
//! 2. Quantize each RGB into 8 levels per channel → 512 buckets.
//! 3. Count per-bucket population.
//! 4. Score each bucket: `population × saturation^1.5 × brightness_weight`.
//!    `brightness_weight` peaks at ~0.55, penalising near-black and near-white.
//! 5. Return the bucket-centre colour with the highest score.

use skia_safe::{AlphaType, Color, ColorType, Image, ImageInfo, Surface};

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
        if brightness < 0.35 || brightness > 0.93 || sat < 0.2 {
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
    let mut surface = match Surface::new_raster(&info, None, None) {
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
