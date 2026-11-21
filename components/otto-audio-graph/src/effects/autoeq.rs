//! AutoEQ's `ParametricEQ.txt`, the Equalizer APO format its headphone
//! profiles come in:
//!
//! ```text
//! Preamp: -6.2 dB
//! Filter 1: ON LSC Fc 105 Hz Gain 6.4 dB Q 0.70
//! Filter 2: ON PK Fc 2210 Hz Gain 4.1 dB Q 1.61
//! Filter 3: ON HSC Fc 10000 Hz Gain -2.0 dB Q 0.70
//! ```

// Rust guideline compliant 2026-02-21

use std::fmt;

use super::{Band, BandType, MAX_BANDS};

/// The Q a filter without one gets: a Butterworth response.
const DEFAULT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// What a `ParametricEQ.txt` holds.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoEq {
    /// In dB; 0 when the file has no preamp line.
    pub preamp: f32,
    /// At most [`MAX_BANDS`], in the file's order.
    pub bands: Vec<Band>,
    /// Filters left out: switched off, of a type the equaliser lacks, or
    /// past [`MAX_BANDS`].
    pub skipped: usize,
}

/// Why a file is not an AutoEQ profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoEqError {
    /// No filter line could be read.
    NoFilters,
    /// A filter line that names a frequency the equaliser can use was
    /// unreadable; the line number, from 1.
    BadLine(usize),
}

impl fmt::Display for AutoEqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFilters => write!(f, "no filters in the file"),
            Self::BadLine(line) => write!(f, "line {line} could not be read"),
        }
    }
}

impl std::error::Error for AutoEqError {}

/// Read an AutoEQ `ParametricEQ.txt`.
///
/// # Errors
///
/// When it holds no usable filter, or a filter line is malformed.
pub fn parse_autoeq(text: &str) -> Result<AutoEq, AutoEqError> {
    let mut preamp = 0.0;
    let mut bands = Vec::new();
    let mut skipped = 0;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        let head = head.trim().to_ascii_lowercase();
        if head == "preamp" {
            preamp = number_before(rest, "db").ok_or(AutoEqError::BadLine(index + 1))?;
        } else if head.starts_with("filter") {
            match filter(rest) {
                Some(Some(band)) if bands.len() < MAX_BANDS => bands.push(band),
                Some(_) => skipped += 1,
                None => return Err(AutoEqError::BadLine(index + 1)),
            }
        }
    }
    if bands.is_empty() {
        return Err(AutoEqError::NoFilters);
    }
    Ok(AutoEq {
        preamp,
        bands,
        skipped,
    })
}

/// One filter line after its `Filter N:`. `Some(None)` for a filter that is
/// off or of a type the equaliser lacks; `None` when it cannot be read.
fn filter(rest: &str) -> Option<Option<Band>> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let mut at = 0;
    match words.first().map(|w| w.to_ascii_uppercase()) {
        Some(w) if w == "ON" => at = 1,
        Some(w) if w == "OFF" => return Some(None),
        _ => {}
    }
    let kind = match words.get(at)?.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" | "PEAK" => BandType::Peaking,
        "LSC" | "LS" | "LSQ" => BandType::LowShelf,
        "HSC" | "HS" | "HSQ" => BandType::HighShelf,
        "LP" | "LPQ" => BandType::LowPass,
        "HP" | "HPQ" => BandType::HighPass,
        _ => return Some(None),
    };
    let value = |key: &str| -> Option<f32> {
        let at = words.iter().position(|w| w.eq_ignore_ascii_case(key))?;
        words.get(at + 1)?.parse().ok()
    };
    let freq = value("Fc")?;
    let gain = if kind.has_gain() { value("Gain")? } else { 0.0 };
    let q = match words.iter().position(|w| w.eq_ignore_ascii_case("Q")) {
        Some(_) => value("Q")?,
        None => DEFAULT_Q,
    };
    Some(Some(
        Band {
            kind,
            freq,
            gain,
            q,
        }
        .clamped(),
    ))
}

/// The number in `text` before `unit`, as in ` -6.2 dB`.
fn number_before(text: &str, unit: &str) -> Option<f32> {
    let lower = text.to_ascii_lowercase();
    let text = lower.trim().trim_end_matches(unit).trim();
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Preamp: -6.2 dB
Filter 1: ON LSC Fc 105 Hz Gain 6.4 dB Q 0.70
Filter 2: ON PK Fc 2210 Hz Gain 4.1 dB Q 1.61
Filter 3: OFF PK Fc 3000 Hz Gain 1.0 dB Q 1.00
Filter 4: ON HSC Fc 10000 Hz Gain -2.0 dB Q 0.70
Filter 5: ON LP Fc 18000 Hz
";

    #[test]
    fn a_profile_reads_as_its_preamp_and_bands() {
        let eq = parse_autoeq(SAMPLE).unwrap();
        assert_eq!(eq.preamp, -6.2);
        assert_eq!(eq.skipped, 1);
        assert_eq!(eq.bands.len(), 4);
        assert_eq!(
            eq.bands[0],
            Band {
                kind: BandType::LowShelf,
                freq: 105.0,
                gain: 6.4,
                q: 0.7
            }
        );
        assert_eq!(eq.bands[1].kind, BandType::Peaking);
        assert_eq!(eq.bands[2].gain, -2.0);
        assert_eq!(eq.bands[3].kind, BandType::LowPass);
        assert_eq!(eq.bands[3].q, DEFAULT_Q);
    }

    #[test]
    fn bands_past_the_limit_and_unknown_types_are_skipped() {
        let mut text = String::new();
        for n in 1..=12 {
            text.push_str(&format!(
                "Filter {n}: ON PK Fc {} Hz Gain 1 dB Q 1\n",
                n * 100
            ));
        }
        text.push_str("Filter 13: ON NO Fc 100 Hz\n");
        let eq = parse_autoeq(&text).unwrap();
        assert_eq!(eq.preamp, 0.0);
        assert_eq!(eq.bands.len(), MAX_BANDS);
        assert_eq!(eq.skipped, 3);
    }

    #[test]
    fn values_out_of_range_are_brought_in() {
        let eq = parse_autoeq("Filter 1: ON PK Fc 5 Hz Gain 40 dB Q 50").unwrap();
        assert_eq!(eq.bands[0].freq, super::super::MIN_FREQ);
        assert_eq!(eq.bands[0].gain, super::super::MAX_GAIN);
        assert_eq!(eq.bands[0].q, super::super::MAX_Q);
    }

    #[test]
    fn files_that_are_not_profiles_are_refused() {
        assert_eq!(parse_autoeq(""), Err(AutoEqError::NoFilters));
        assert_eq!(parse_autoeq("hello: world\n"), Err(AutoEqError::NoFilters));
        assert_eq!(
            parse_autoeq("Preamp: loud\nFilter 1: ON PK Fc 100 Hz Gain 1 dB Q 1"),
            Err(AutoEqError::BadLine(1))
        );
        assert_eq!(
            parse_autoeq("Filter 1: ON PK Fc x Hz Gain 1 dB Q 1"),
            Err(AutoEqError::BadLine(1))
        );
    }
}
