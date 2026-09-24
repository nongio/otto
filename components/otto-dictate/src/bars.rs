//! The equaliser: one bar per speech band, each as tall as that band is loud
//! in the last few milliseconds of microphone audio.

// Rust guideline compliant 2026-02-21

use std::f32::consts::PI;

use crate::capture::SAMPLE_RATE;

/// Samples analysed per frame: 32 ms at 16 kHz, a power of two so bins land
/// on round frequencies (31.25 Hz apart).
pub const WINDOW: usize = 512;

/// One bar per band, low to high, in Hz. Octaves across the range that
/// carries speech: voice pitch at the bottom, sibilants at the top.
const BANDS: [(f32, f32); 5] = [
    (100.0, 250.0),
    (250.0, 500.0),
    (500.0, 1000.0),
    (1000.0, 2000.0),
    (2000.0, 4000.0),
];

/// How far above a band's noise floor, in dB, a sound starts to show and
/// fills the bar.
const ABOVE_FLOOR_DB: f32 = 6.0;
const RANGE_DB: f32 = 40.0;
/// How fast a band's noise floor creeps up towards a steady sound, in dB per
/// frame (about 10 dB a second at 24 fps). It drops at once to anything
/// quieter, so it settles on the background within a few seconds and speech,
/// which comes and goes, stays above it.
const FLOOR_RISE_DB: f32 = 0.4;
/// Lowest a noise floor goes. The first frames of a capture can be digital
/// silence; a floor that followed them down would take the bars to the top
/// for as long as it takes to climb back to the room.
const MIN_FLOOR_DB: f32 = -70.0;
/// Shortest a bar is drawn, as a fraction of the tallest.
const REST: f32 = 0.1;

/// The bars' current heights, 0.0 to 1.0.
#[derive(Debug)]
pub struct Bars {
    levels: [f32; BANDS.len()],
    /// Each band's background level in dB; `None` until the first frame.
    floors: Option<[f32; BANDS.len()]>,
    window: Vec<f32>,
    frames: u32,
}

impl Default for Bars {
    fn default() -> Self {
        // Hann window, so a band doesn't leak into its neighbours.
        let window = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / (WINDOW - 1) as f32).cos())
            .collect();
        Self {
            levels: [REST; BANDS.len()],
            floors: None,
            window,
            frames: 0,
        }
    }
}

impl Bars {
    /// Advance one frame from the latest `samples` (16 kHz mono).
    pub fn step(&mut self, samples: &[f32]) {
        let Some(db) = self.band_db(samples) else {
            return;
        };
        let floors = self.floors.get_or_insert(db.map(|d| d.max(MIN_FLOOR_DB)));
        let mut targets = [REST; BANDS.len()];
        for ((target, floor), db) in targets.iter_mut().zip(floors.iter_mut()).zip(db) {
            *floor = if db < *floor { db } else { *floor + FLOOR_RISE_DB }.max(MIN_FLOOR_DB);
            let t = ((db - *floor - ABOVE_FLOOR_DB) / RANGE_DB).clamp(0.0, 1.0);
            *target = REST + (1.0 - REST) * t;
        }
        self.frames += 1;
        // Twice a second, for tuning the constants against a real room.
        if self.frames.is_multiple_of(12) {
            tracing::debug!(db = ?db.map(|d| d.round()), floor = ?floors.map(|f| f.round()), "bands");
        }
        for (bar, target) in self.levels.iter_mut().zip(targets) {
            // Rise fast, fall slower, like a VU needle.
            *bar += (target - *bar) * if target > *bar { 0.6 } else { 0.2 };
        }
    }

    /// Each band's energy in dB relative to full scale, or `None` before a
    /// full window was captured.
    fn band_db(&self, samples: &[f32]) -> Option<[f32; BANDS.len()]> {
        if samples.len() < WINDOW {
            return None;
        }
        let mut out = [0.0; BANDS.len()];
        let frame: Vec<f32> = samples[samples.len() - WINDOW..]
            .iter()
            .zip(&self.window)
            .map(|(s, w)| s * w)
            .collect();
        let bin_hz = SAMPLE_RATE as f32 / WINDOW as f32;
        for (out, (low, high)) in out.iter_mut().zip(BANDS) {
            let bins = (low / bin_hz).ceil() as usize..=(high / bin_hz).floor() as usize;
            let count = bins.clone().count().max(1) as f32;
            // A direct DFT of the few bins a band needs is cheaper here than a
            // full FFT, and needs no crate.
            let power: f32 = bins.map(|k| bin_power(&frame, k)).sum::<f32>() / count;
            // Normalised so a full-scale sine in a bin reads 0 dB.
            *out = 10.0 * (power / (WINDOW as f32 / 4.0).powi(2) + 1e-12).log10();
        }
        Some(out)
    }

    /// The bars' heights, 0.0 to 1.0, low band first.
    pub fn levels(&self) -> &[f32] {
        &self.levels
    }
}

/// Power of DFT bin `k` of `frame`.
fn bin_power(frame: &[f32], k: usize) -> f32 {
    let step = 2.0 * PI * k as f32 / frame.len() as f32;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (n, sample) in frame.iter().enumerate() {
        let angle = step * n as f32;
        re += sample * angle.cos();
        im -= sample * angle.sin();
    }
    re * re + im * im
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amplitude: f32) -> Vec<f32> {
        (0..WINDOW)
            .map(|n| amplitude * (2.0 * PI * hz * n as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// Deterministic white-ish noise.
    fn noise(amplitude: f32) -> Vec<f32> {
        let mut x: u32 = 12345;
        (0..WINDOW)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                amplitude * ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    #[test]
    fn a_tone_is_loudest_in_its_own_band() {
        let db = Bars::default().band_db(&sine(1500.0, 0.1)).unwrap();
        let loudest = (0..db.len()).max_by(|a, b| db[*a].total_cmp(&db[*b])).unwrap();
        assert_eq!(loudest, 3, "1.5 kHz is in the fourth band: {db:?}");
    }

    #[test]
    fn steady_noise_settles_to_rest() {
        let mut bars = Bars::default();
        for _ in 0..24 * 10 {
            bars.step(&noise(0.05));
        }
        assert!(bars.levels.iter().all(|l| *l < REST + 0.05), "{:?}", bars.levels);
    }

    #[test]
    fn silence_at_the_start_does_not_hold_the_bars_up() {
        let mut bars = Bars::default();
        for _ in 0..3 {
            bars.step(&[0.0; WINDOW]);
        }
        for _ in 0..24 * 4 {
            bars.step(&noise(0.1));
        }
        assert!(bars.levels.iter().all(|l| *l < REST + 0.05), "{:?}", bars.levels);
    }

    #[test]
    fn speech_over_noise_stands_out() {
        let mut bars = Bars::default();
        for _ in 0..24 * 3 {
            bars.step(&noise(0.005));
        }
        let voiced: Vec<f32> = noise(0.005).iter().zip(sine(400.0, 0.2)).map(|(n, s)| n + s).collect();
        for _ in 0..4 {
            bars.step(&voiced);
        }
        assert!(bars.levels[1] > 0.6, "400 Hz band: {:?}", bars.levels);
        assert!(bars.levels[4] < 0.3, "top band stays low: {:?}", bars.levels);
    }
}
