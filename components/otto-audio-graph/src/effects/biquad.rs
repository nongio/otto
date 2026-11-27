//! The equaliser's filters: biquad coefficients from the Audio EQ Cookbook
//! (Robert Bristow-Johnson), and the response they make.
//!
//! The chain runs these very coefficients (PipeWire's `bq_raw`), so the
//! curve the window draws is what is heard.

// Rust guideline compliant 2026-02-21

use std::f64::consts::PI;

use super::{Band, BandType};

/// A biquad's coefficients, normalised so `a0` is 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coefficients {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefficients {
    /// A filter that passes sound unchanged.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// The coefficients of `band` at sample rate `rate`.
    #[must_use]
    pub fn of(band: &Band, rate: f64) -> Self {
        let band = band.clamped();
        let nyquist_guard = rate * 0.49;
        let w0 = 2.0 * PI * f64::from(band.freq).min(nyquist_guard) / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * f64::from(band.q));
        let a = 10f64.powf(f64::from(band.gain) / 40.0);
        let root = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            BandType::Peaking => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            BandType::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cos + root),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - root),
                (a + 1.0) + (a - 1.0) * cos + root,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - root,
            ),
            BandType::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cos + root),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - root),
                (a + 1.0) - (a - 1.0) * cos + root,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - root,
            ),
            BandType::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandType::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// The gain at `freq` Hz, in dB, at sample rate `rate`.
    #[must_use]
    pub fn gain_db(&self, freq: f64, rate: f64) -> f64 {
        let w = 2.0 * PI * freq / rate;
        // H(e^jw) = (b0 + b1 e^-jw + b2 e^-2jw) / (1 + a1 e^-jw + a2 e^-2jw)
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        let num = num_re * num_re + num_im * num_im;
        let den = (den_re * den_re + den_im * den_im).max(f64::MIN_POSITIVE);
        10.0 * (num / den).max(1e-12).log10()
    }
}

/// The equaliser's whole response at `freq` Hz, in dB: the preamp plus every
/// band, at the rate effects run at.
#[must_use]
pub fn response_db(preamp: f32, bands: &[Band], freq: f32) -> f32 {
    let rate = f64::from(super::EFFECT_RATE);
    let sum: f64 = bands
        .iter()
        .map(|b| Coefficients::of(b, rate).gain_db(f64::from(freq), rate))
        .sum();
    (f64::from(preamp) + sum) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn band(kind: BandType, freq: f32, gain: f32, q: f32) -> Band {
        Band {
            kind,
            freq,
            gain,
            q,
        }
    }

    #[test]
    fn a_peak_has_its_gain_at_its_centre_and_none_far_away() {
        let c = Coefficients::of(&band(BandType::Peaking, 1000.0, 6.0, 1.0), RATE);
        assert!((c.gain_db(1000.0, RATE) - 6.0).abs() < 0.01);
        assert!(c.gain_db(20.0, RATE).abs() < 0.1);
        assert!(c.gain_db(18_000.0, RATE).abs() < 0.1);
    }

    #[test]
    fn shelves_reach_their_gain_on_their_side() {
        let low = Coefficients::of(&band(BandType::LowShelf, 200.0, -5.0, 0.7), RATE);
        assert!((low.gain_db(20.0, RATE) + 5.0).abs() < 0.1);
        assert!(low.gain_db(10_000.0, RATE).abs() < 0.1);
        let high = Coefficients::of(&band(BandType::HighShelf, 4000.0, 3.0, 0.7), RATE);
        assert!((high.gain_db(20_000.0, RATE) - 3.0).abs() < 0.2);
        assert!(high.gain_db(50.0, RATE).abs() < 0.1);
    }

    #[test]
    fn pass_filters_cut_on_their_far_side() {
        let lp = Coefficients::of(&band(BandType::LowPass, 1000.0, 0.0, 0.707), RATE);
        assert!(lp.gain_db(100.0, RATE).abs() < 0.1);
        assert!((lp.gain_db(1000.0, RATE) + 3.0).abs() < 0.1);
        assert!(lp.gain_db(10_000.0, RATE) < -30.0);
        let hp = Coefficients::of(&band(BandType::HighPass, 1000.0, 0.0, 0.707), RATE);
        assert!(hp.gain_db(100.0, RATE) < -30.0);
    }

    #[test]
    fn the_response_adds_the_preamp_and_the_bands() {
        let bands = [
            band(BandType::Peaking, 100.0, 3.0, 1.0),
            band(BandType::Peaking, 5000.0, -2.0, 1.0),
        ];
        assert!((response_db(-1.0, &bands, 100.0) - 2.0).abs() < 0.05);
        assert!((response_db(-1.0, &bands, 5000.0) + 3.0).abs() < 0.05);
        assert!((response_db(0.0, &[], 1000.0)).abs() < 1e-6);
        assert!(Coefficients::IDENTITY.gain_db(1234.0, RATE).abs() < 1e-9);
    }
}
