//! Sampling choices for drawing images at a size other than their own.
//!
//! Skia's default `SamplingOptions` is nearest-neighbour, which turns any
//! rescale into visible stair-steps. Icons in particular almost never arrive
//! at exactly the size we draw them: an SNI tray pixmap comes in whatever
//! sizes the app happens to publish, and a themed icon is loaded at the
//! nearest available size. These helpers pick a filter that suits the ratio.

use skia_safe::sampling_options::{CubicResampler, SamplingOptions};
use skia_safe::{FilterMode, MipmapMode};

/// Sampling for drawing an icon of `src` pixels into a `dst`-pixel box.
///
/// Both sizes are in *physical* pixels — multiply the logical box by the
/// surface scale before calling.
///
/// - A 1:1 draw needs no filtering at all.
/// - Shrinking by more than 2x aliases under any single-pass filter, so ask
///   for mipmaps and let Skia pick the level.
/// - Everything else (mild minification and any magnification) gets Mitchell,
///   which keeps edges smooth without the ringing of a sharper cubic.
pub fn icon_sampling(src: (i32, i32), dst: (f32, f32)) -> SamplingOptions {
    if src.0 <= 0 || src.1 <= 0 || dst.0 <= 0.0 || dst.1 <= 0.0 {
        return SamplingOptions::default();
    }

    let ratio = (dst.0 / src.0 as f32).min(dst.1 / src.1 as f32);

    if (ratio - 1.0).abs() < 0.01 {
        SamplingOptions::default()
    } else if ratio < 0.5 {
        SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
    } else {
        SamplingOptions::from(CubicResampler::mitchell())
    }
}
