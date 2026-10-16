//! High-quality image resampling on the CPU, in linear light.
//!
//! For images that are resampled once and then drawn many times, like a
//! wallpaper, where quality matters more than the few hundred milliseconds it
//! takes. Skia's own `draw_image_rect` with a cubic filter reads a fixed 4x4
//! block of source pixels per output pixel whatever the scale, so shrinking a
//! photo by more than 2x skips source pixels: fine detail aliases into moiré
//! and jaggies. This resampler widens its kernel with the scale instead, as
//! ImageMagick, Pillow and libvips do, so every source pixel contributes.
//!
//! - **Kernels.** Lanczos-3 when an axis shrinks (sharp, with negligible
//!   ringing once widened), Mitchell-Netravali when it grows (no visible
//!   ringing or blockiness). Chosen per axis, so a non-uniform stretch gets
//!   the right one on each.
//! - **Linear light.** Filtering averages *light*, so it runs on linear,
//!   premultiplied values: averaging sRGB-encoded values darkens fine
//!   high-contrast detail (1 px black and white stripes come out at sRGB 128
//!   instead of the 188 that is actually half the light). Skia converts the
//!   source to linear on the way in, honouring an embedded colour profile.
//! - **Memory.** The source is read in strips and the horizontal pass kept for
//!   only the rows the vertical kernel is using, so an 8K photo needs no
//!   full-size intermediate buffer. Output bands run on separate threads.

use std::sync::OnceLock;

use skia_safe as skia;

/// Rows of source read from Skia at a time.
const STRIP_ROWS: i32 = 32;
/// The most threads one resample uses.
const MAX_THREADS: usize = 8;
/// The fewest output rows worth a thread of their own.
const MIN_ROWS_PER_THREAD: i32 = 64;

/// Resample the `src` region of `image` (in image pixels, fractional bounds
/// allowed) into a new raster image of exactly `size` pixels.
///
/// Source pixels outside `src` but inside the image still feed the kernel
/// near the region's edges, so a crop matches the same region of a resample
/// of the whole image; at the image's own edges the kernel is renormalised
/// over the pixels that exist. The result is 8-bit sRGB, premultiplied.
///
/// Returns `None` for an empty `size` or `src`, or if Skia cannot read the
/// image.
pub fn resample(image: &skia::Image, src: skia::Rect, size: (i32, i32)) -> Option<skia::Image> {
    let (out_w, out_h) = size;
    let (img_w, img_h) = (image.width(), image.height());
    if out_w <= 0 || out_h <= 0 || img_w <= 0 || img_h <= 0 {
        return None;
    }
    if !(src.width() > 0.0 && src.height() > 0.0) {
        return None;
    }

    // A lazily decoded image (`Image::from_encoded`) would be decoded again
    // for every strip read below, or held only as long as Skia's cache keeps
    // it; decode it once up front.
    let raster;
    let image = if image.is_lazy_generated() {
        raster = image.make_raster_image(None, skia::image::CachingHint::Disallow)?;
        &raster
    } else {
        image
    };

    let cols = Axis::new(src.left, src.width(), out_w, img_w);
    let rows = Axis::new(src.top, src.height(), out_h, img_h);
    // Read only the columns the horizontal kernel touches.
    let (x0, x1) = cols.source_span();
    let src_cols = (x1 - x0) as usize;

    let mut out = vec![0u8; out_w as usize * out_h as usize * 4];
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(MAX_THREADS)
        .min(((out_h / MIN_ROWS_PER_THREAD) as usize).max(1));
    let band_rows = (out_h as usize).div_ceil(threads);

    let ok = std::thread::scope(|scope| {
        let handles: Vec<_> = out
            .chunks_mut(band_rows * out_w as usize * 4)
            .enumerate()
            .map(|(band, chunk)| {
                let (cols, rows) = (&cols, &rows);
                let first_row = band * band_rows;
                scope.spawn(move || {
                    resample_band(image, cols, rows, x0, src_cols, out_w, first_row, chunk)
                })
            })
            .collect();
        handles
            .into_iter()
            .all(|handle| handle.join().unwrap_or(false))
    });
    if !ok {
        return None;
    }

    let info = skia::ImageInfo::new(
        (out_w, out_h),
        skia::ColorType::RGBA8888,
        skia::AlphaType::Premul,
        Some(skia::ColorSpace::new_srgb()),
    );
    skia::images::raster_from_data(&info, skia::Data::new_copy(&out), out_w as usize * 4)
}

/// [`resample`] of the whole image.
pub fn resample_to(image: &skia::Image, size: (i32, i32)) -> Option<skia::Image> {
    resample(
        image,
        skia::Rect::from_iwh(image.width(), image.height()),
        size,
    )
}

/// One output band: rows `first_row..` of the output, as many as `out` holds.
#[allow(clippy::too_many_arguments)]
fn resample_band(
    image: &skia::Image,
    cols: &Axis,
    rows: &Axis,
    x0: i32,
    src_cols: usize,
    out_w: i32,
    first_row: usize,
    out: &mut [u8],
) -> bool {
    let out_w = out_w as usize;
    let band_len = out.len() / (out_w * 4);
    if band_len == 0 {
        return true;
    }
    let taps = &rows.taps[first_row..first_row + band_len];
    let lo = taps.first().map(|t| t.first).unwrap_or(0);
    let hi = taps
        .iter()
        .map(|t| t.first + t.weights.len() as i32)
        .max()
        .unwrap_or(lo);

    let mut reader = StripReader::new(image, x0, src_cols, lo, hi);
    // Horizontally resampled source rows, oldest first: `window[i]` is source
    // row `window_first + i`.
    let mut window: std::collections::VecDeque<Vec<[f32; 4]>> = Default::default();
    let mut window_first = lo;
    let mut column = vec![[0f32; 4]; out_w];

    for (y, tap) in taps.iter().enumerate() {
        let need_hi = tap.first + tap.weights.len() as i32;
        // Rows below this output row's kernel are done with.
        while window_first < tap.first && !window.is_empty() {
            window.pop_front();
            window_first += 1;
        }
        if window.is_empty() {
            window_first = tap.first;
        }
        while window_first + (window.len() as i32) < need_hi {
            let row_index = window_first + window.len() as i32;
            let Some(source_row) = reader.row(row_index) else {
                return false;
            };
            window.push_back(horizontal(source_row, cols, x0));
        }

        for px in column.iter_mut() {
            *px = [0.0; 4];
        }
        for (k, weight) in tap.weights.iter().enumerate() {
            let row = &window[(tap.first - window_first) as usize + k];
            for (acc, px) in column.iter_mut().zip(row.iter()) {
                for c in 0..4 {
                    acc[c] += px[c] * weight;
                }
            }
        }
        let line = &mut out[y * out_w * 4..(y + 1) * out_w * 4];
        for (dst, px) in line.chunks_exact_mut(4).zip(column.iter()) {
            dst.copy_from_slice(&encode(*px));
        }
    }
    true
}

/// Resample one source row (columns from `x0`) horizontally.
fn horizontal(row: &[[f32; 4]], cols: &Axis, x0: i32) -> Vec<[f32; 4]> {
    cols.taps
        .iter()
        .map(|tap| {
            let mut acc = [0f32; 4];
            let start = (tap.first - x0) as usize;
            for (px, weight) in row[start..start + tap.weights.len()]
                .iter()
                .zip(tap.weights.iter())
            {
                for c in 0..4 {
                    acc[c] += px[c] * weight;
                }
            }
            acc
        })
        .collect()
}

/// Linear premultiplied light back to 8-bit sRGB, premultiplied in the
/// encoded space as Skia's N32 images are.
fn encode(px: [f32; 4]) -> [u8; 4] {
    // A widened Lanczos overshoots a little at hard edges; clamp to what a
    // pixel can hold.
    let alpha = px[3].clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return [0; 4];
    }
    let lut = encode_lut();
    let channel = |value: f32| {
        let linear = (value / alpha).clamp(0.0, 1.0);
        let index = (linear * (LUT_SIZE - 1) as f32 + 0.5) as usize;
        let encoded = lut[index] as f32;
        (encoded * alpha + 0.5) as u8
    };
    [
        channel(px[0]),
        channel(px[1]),
        channel(px[2]),
        (alpha * 255.0 + 0.5) as u8,
    ]
}

/// Entries in the linear-to-sRGB table. Fine enough that the darkest sRGB
/// codes, where the curve is steepest, still round correctly.
const LUT_SIZE: usize = 1 << 16;

fn encode_lut() -> &'static [u8] {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    LUT.get_or_init(|| {
        (0..LUT_SIZE)
            .map(|i| {
                let linear = i as f64 / (LUT_SIZE - 1) as f64;
                let encoded = if linear <= 0.003_130_8 {
                    12.92 * linear
                } else {
                    1.055 * linear.powf(1.0 / 2.4) - 0.055
                };
                (encoded * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect()
    })
}

/// Reads source rows as linear, premultiplied `f32` RGBA, a strip at a time.
struct StripReader<'a> {
    image: &'a skia::Image,
    x0: i32,
    cols: usize,
    /// One past the last row anyone will ask for.
    hi: i32,
    strip_first: i32,
    strip_rows: i32,
    strip: Vec<[f32; 4]>,
}

impl<'a> StripReader<'a> {
    fn new(image: &'a skia::Image, x0: i32, cols: usize, lo: i32, hi: i32) -> Self {
        Self {
            image,
            x0,
            cols,
            hi,
            strip_first: lo,
            strip_rows: 0,
            strip: Vec::new(),
        }
    }

    fn row(&mut self, y: i32) -> Option<&[[f32; 4]]> {
        if y < self.strip_first || y >= self.strip_first + self.strip_rows {
            let rows = STRIP_ROWS.min(self.hi - y).max(1);
            self.strip.resize(self.cols * rows as usize, [0.0; 4]);
            let info = skia::ImageInfo::new(
                (self.cols as i32, rows),
                skia::ColorType::RGBAF32,
                skia::AlphaType::Premul,
                Some(skia::ColorSpace::new_srgb_linear()),
            );
            let read = self.image.read_pixels(
                &info,
                self.strip.as_mut_slice(),
                self.cols * 16,
                (self.x0, y),
                skia::image::CachingHint::Allow,
            );
            if !read {
                return None;
            }
            self.strip_first = y;
            self.strip_rows = rows;
        }
        let start = (y - self.strip_first) as usize * self.cols;
        Some(&self.strip[start..start + self.cols])
    }
}

/// The kernel weights of one output pixel along one axis: source pixels
/// `first..first + weights.len()`.
struct Tap {
    first: i32,
    weights: Vec<f32>,
}

/// Every output pixel's taps along one axis.
struct Axis {
    taps: Vec<Tap>,
}

impl Axis {
    /// `out` output pixels covering source `start..start + extent`, out of
    /// `len` source pixels.
    fn new(start: f32, extent: f32, out: i32, len: i32) -> Self {
        let ratio = out as f64 / extent as f64;
        let (kernel, support): (fn(f64) -> f64, f64) = if ratio < 1.0 {
            (lanczos3, 3.0)
        } else {
            (mitchell, 2.0)
        };
        // Shrinking widens the kernel so each output pixel averages every
        // source pixel it covers; growing keeps it at its natural width.
        let filter_scale = (1.0 / ratio).max(1.0);
        let reach = support * filter_scale;
        let step = extent as f64 / out as f64;

        let taps = (0..out)
            .map(|o| {
                let centre = start as f64 + (o as f64 + 0.5) * step;
                let first = ((centre - reach - 0.5).floor() as i32).clamp(0, len - 1);
                let last = ((centre + reach - 0.5).ceil() as i32).clamp(0, len - 1);
                let mut weights: Vec<f64> = (first..=last)
                    .map(|j| kernel((j as f64 + 0.5 - centre) / filter_scale))
                    .collect();
                let sum: f64 = weights.iter().sum();
                if sum.abs() > 1e-12 {
                    for w in weights.iter_mut() {
                        *w /= sum;
                    }
                } else {
                    // A kernel that lands entirely between pixels: take the
                    // nearest one.
                    weights = vec![0.0; weights.len()];
                    let nearest = ((centre - 0.5).round() as i32).clamp(first, last);
                    weights[(nearest - first) as usize] = 1.0;
                }
                Tap {
                    first,
                    weights: weights.into_iter().map(|w| w as f32).collect(),
                }
            })
            .collect();
        Self { taps }
    }

    /// The source pixels any output pixel reads: `lo..hi`.
    fn source_span(&self) -> (i32, i32) {
        let lo = self.taps.iter().map(|t| t.first).min().unwrap_or(0);
        let hi = self
            .taps
            .iter()
            .map(|t| t.first + t.weights.len() as i32)
            .max()
            .unwrap_or(lo + 1);
        (lo, hi)
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

fn lanczos3(x: f64) -> f64 {
    if x.abs() < 3.0 {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// Mitchell-Netravali with B = C = 1/3.
fn mitchell(x: f64) -> f64 {
    const B: f64 = 1.0 / 3.0;
    const C: f64 = 1.0 / 3.0;
    let x = x.abs();
    if x < 1.0 {
        ((12.0 - 9.0 * B - 6.0 * C) * x * x * x
            + (-18.0 + 12.0 * B + 6.0 * C) * x * x
            + (6.0 - 2.0 * B))
            / 6.0
    } else if x < 2.0 {
        ((-B - 6.0 * C) * x * x * x
            + (6.0 * B + 30.0 * C) * x * x
            + (-12.0 * B - 48.0 * C) * x
            + (8.0 * B + 24.0 * C))
            / 6.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque image from a per-pixel function returning sRGB bytes.
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
            Some(skia::ColorSpace::new_srgb()),
        );
        skia::images::raster_from_data(&info, skia::Data::new_copy(&data), (w * 4) as usize)
            .unwrap()
    }

    /// The RGBA bytes of `image`, unpremultiplied.
    fn pixels(image: &skia::Image) -> Vec<[u8; 4]> {
        let (w, h) = (image.width(), image.height());
        let info = skia::ImageInfo::new(
            (w, h),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Unpremul,
            Some(skia::ColorSpace::new_srgb()),
        );
        let mut data = vec![[0u8; 4]; (w * h) as usize];
        assert!(image.read_pixels(
            &info,
            data.as_mut_slice(),
            (w * 4) as usize,
            (0, 0),
            skia::image::CachingHint::Disallow,
        ));
        data
    }

    fn stripes(w: i32, h: i32) -> skia::Image {
        image(w, h, |x, _| if x % 2 == 0 { [0; 3] } else { [255; 3] })
    }

    /// The light of 1 px black and white stripes is half white: sRGB 188.
    const HALF_LIGHT_SRGB: i32 = 188;

    /// What Skia's single-pass draw does, for the comparisons below.
    fn skia_mitchell(image: &skia::Image, size: (i32, i32)) -> skia::Image {
        let mut surface = skia::surfaces::raster_n32_premul(size).unwrap();
        surface.canvas().draw_image_rect_with_sampling_options(
            image,
            None,
            skia::Rect::from_iwh(size.0, size.1),
            skia::SamplingOptions::from(skia::CubicResampler::mitchell()),
            &skia::Paint::default(),
        );
        surface.image_snapshot()
    }

    /// The red channel of every pixel at least `margin` pixels from the
    /// left and right edges, where the kernel is cut short and renormalised.
    fn interior(image: &skia::Image, margin: i32) -> Vec<i32> {
        let w = image.width();
        pixels(image)
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let x = *i as i32 % w;
                x >= margin && x < w - margin
            })
            .map(|(_, p)| p[0] as i32)
            .collect()
    }

    fn spread(image: &skia::Image) -> (i32, i32) {
        let px = interior(image, 4);
        (*px.iter().min().unwrap(), *px.iter().max().unwrap())
    }

    /// Shrinking detail finer than the output can hold must average it, not
    /// sample it: 1 px stripes at 0.45x come out as an even grey. Skia's
    /// fixed-width cubic skips source pixels at this ratio and leaves a beat
    /// pattern instead — the aliasing this resampler exists to avoid.
    #[test]
    fn fine_detail_shrinks_to_an_even_tone_without_aliasing() {
        let source = stripes(1000, 40);
        let ours = resample_to(&source, (450, 18)).unwrap();
        let (min, max) = spread(&ours);
        assert!(max - min <= 6, "stripes alias: {min}..{max}");

        let (skia_min, skia_max) = spread(&skia_mitchell(&source, (450, 18)));
        assert!(
            skia_max - skia_min > 40,
            "the single-pass reference was expected to alias: {skia_min}..{skia_max}"
        );
    }

    /// The even tone is the right one: half the light, which is sRGB 188, not
    /// the 128 averaging the encoded values gives.
    #[test]
    fn filtering_averages_light_not_encoded_values() {
        let ours = resample_to(&stripes(1000, 40), (500, 20)).unwrap();
        for value in interior(&ours, 4) {
            assert!(
                (value - HALF_LIGHT_SRGB).abs() <= 2,
                "got {value}, half the light is {HALF_LIGHT_SRGB}"
            );
        }
    }

    /// A flat colour stays exactly that colour at any ratio, both ways.
    #[test]
    fn a_flat_colour_is_reproduced_exactly() {
        let source = image(300, 200, |_, _| [200, 30, 90]);
        for size in [(299, 199), (137, 91), (31, 17), (301, 201), (777, 512)] {
            for px in pixels(&resample_to(&source, size).unwrap()) {
                assert_eq!(px, [200, 30, 90, 255], "at {size:?}");
            }
        }
    }

    /// Averaging must not cost sharpness: a hard edge shrunk by half is still
    /// a transition a couple of pixels wide.
    #[test]
    fn a_hard_edge_stays_sharp() {
        let source = image(400, 4, |x, _| if x < 200 { [0; 3] } else { [255; 3] });
        let row: Vec<i32> = pixels(&resample_to(&source, (200, 2)).unwrap())[..200]
            .iter()
            .map(|p| p[0] as i32)
            .collect();
        let soft = row.iter().filter(|&&v| v > 10 && v < 245).count();
        assert!(soft <= 3, "edge spread over {soft} pixels: {row:?}");
        assert!(row[..95].iter().all(|&v| v <= 2) && row[105..].iter().all(|&v| v >= 253));
    }

    /// A smooth gradient keeps its shape both ways: it rises monotonically
    /// and stays within a shade or two of where it should be.
    #[test]
    fn a_gradient_stays_smooth_shrinking_and_growing() {
        let source = image(256, 4, |x, _| [x as u8; 3]);
        for out in [100, 211, 500, 1023] {
            let row: Vec<i32> = pixels(&resample_to(&source, (out, 2)).unwrap())[..out as usize]
                .iter()
                .map(|p| p[0] as i32)
                .collect();
            // Away from the ends, where the kernel is renormalised.
            let margin = (out / 32).max(3) as usize;
            for pair in row[margin..row.len() - margin].windows(2) {
                assert!(pair[1] >= pair[0] - 1, "not monotonic at {out}: {pair:?}");
                assert!(pair[1] - pair[0] <= 4, "a step at {out}: {pair:?}");
            }
        }
    }

    /// Growing an image must not leave the blocks nearest sampling does: a
    /// 2x2 checker enlarged 1.79x has no runs of identical columns.
    #[test]
    fn enlarging_is_smooth_not_blocky() {
        let source = image(64, 64, |x, y| [(x * 4) as u8, (y * 4) as u8, 128]);
        let out = resample_to(&source, (115, 115)).unwrap();
        let px = pixels(&out);
        let row: Vec<u8> = px[57 * 115..58 * 115].iter().map(|p| p[0]).collect();
        let repeats = row.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(repeats < 115 / 4, "{repeats} repeated columns: {row:?}");
    }

    /// A region resampled on its own matches the same region of a resample
    /// of the whole image: the kernel reads past the crop, not up to it.
    #[test]
    fn a_crop_matches_the_same_region_of_the_whole() {
        let source = image(400, 300, |x, y| {
            [
                ((x * 7) % 256) as u8,
                ((y * 5) % 256) as u8,
                ((x + y) % 256) as u8,
            ]
        });
        let whole = pixels(&resample_to(&source, (200, 150)).unwrap());
        let crop = resample(
            &source,
            skia::Rect::from_xywh(100.0, 60.0, 200.0, 160.0),
            (100, 80),
        )
        .unwrap();
        let crop = pixels(&crop);
        for y in 0..80 {
            for x in 0..100 {
                let a = whole[((y + 30) * 200 + x + 50) as usize];
                let b = crop[(y * 100 + x) as usize];
                for c in 0..3 {
                    assert!(
                        (a[c] as i32 - b[c] as i32).abs() <= 1,
                        "({x},{y}): {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    /// Transparent pixels contribute no colour: white on transparent, shrunk,
    /// stays white where it is partly covered, with no dark fringe.
    #[test]
    fn transparency_leaves_no_dark_fringe() {
        let (w, h) = (100, 10);
        let mut data = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                data.extend_from_slice(if x < 50 {
                    &[0, 0, 0, 0]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        let info = skia::ImageInfo::new(
            (w, h),
            skia::ColorType::RGBA8888,
            skia::AlphaType::Premul,
            Some(skia::ColorSpace::new_srgb()),
        );
        let source =
            skia::images::raster_from_data(&info, skia::Data::new_copy(&data), (w * 4) as usize)
                .unwrap();
        for px in pixels(&resample_to(&source, (33, 3)).unwrap()) {
            if px[3] >= 16 {
                assert!(px[0] >= 250, "fringe: {px:?}");
            }
        }
    }

    /// An image decoded lazily from a file resamples the same as a raster
    /// one: it is decoded once up front, not once per strip.
    #[test]
    fn a_lazily_decoded_image_resamples_like_a_raster_one() {
        let source = image(300, 200, |x, y| [(x % 256) as u8, (y % 256) as u8, 77]);
        let png = source
            .encode(None, skia::EncodedImageFormat::PNG, 100)
            .unwrap();
        let lazy = skia::Image::from_encoded(png).unwrap();
        assert!(lazy.is_lazy_generated());
        let a = pixels(&resample_to(&source, (123, 77)).unwrap());
        let b = pixels(&resample_to(&lazy, (123, 77)).unwrap());
        assert_eq!(a, b);
    }

    /// An empty request is refused rather than producing a zero-sized image.
    #[test]
    fn empty_requests_are_refused() {
        let source = image(10, 10, |_, _| [0; 3]);
        assert!(resample_to(&source, (0, 10)).is_none());
        assert!(resample(&source, skia::Rect::new_empty(), (5, 5)).is_none());
    }
}
