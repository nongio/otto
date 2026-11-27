//! An EasyEffects output preset, as far as a speaker correction can take
//! it: the convolver's impulse response and output gain, a high-pass filter
//! before it, and a limiter after it. Laptop makers ship their speaker
//! tuning this way, with the impulse response (`<kernel-name>.irs`) beside
//! the preset or in an `irs/` folder:
//!
//! ```json
//! { "output": {
//!     "plugins_order": ["filter#0", "convolver#0", "limiter#0"],
//!     "filter#0": { "bypass": false, "type": "High-pass", "frequency": 60.0 },
//!     "convolver#0": { "bypass": false, "kernel-name": "IR_22ms", "output-gain": 6.0 },
//!     "limiter#0": { "bypass": false, "threshold": 0.0, "release": 8.0 }
//! } }
//! ```
//!
//! Plugins switched off (`"bypass": true`) are ignored; the others the
//! effect cannot do are listed in [`EasyEffects::skipped`].

// Rust guideline compliant 2026-02-21

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// What a speaker correction takes from an EasyEffects preset.
#[derive(Debug, Clone, PartialEq)]
pub struct EasyEffects {
    /// The convolver's `kernel-name` (no extension), or the path an older
    /// preset's `kernel-path` gives.
    pub kernel: String,
    /// The convolver's input and output gain together, in dB.
    pub gain: f32,
    /// The frequency of an active high-pass filter before the convolver.
    pub highpass: Option<f32>,
    /// An active limiter's values, in the Limiter effect's units.
    pub limiter: Option<EasyEffectsLimiter>,
    /// The active plugins left out, by EasyEffects' name (`stereo_tools`),
    /// in the preset's order.
    pub skipped: Vec<String>,
}

/// An EasyEffects limiter, as the Limiter effect's parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EasyEffectsLimiter {
    /// `input-gain`, in dB.
    pub input_gain: f32,
    /// `threshold`, the ceiling, in dB.
    pub limit: f32,
    /// `release`, in seconds (EasyEffects gives milliseconds).
    pub release: f32,
}

/// Why a file is not a preset a speaker correction can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EasyEffectsError {
    /// The file is not JSON.
    NotJson,
    /// It has no `output` section: not an output preset.
    NotOutput,
    /// It has no active convolver.
    NoConvolver,
}

impl fmt::Display for EasyEffectsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => write!(f, "not JSON"),
            Self::NotOutput => write!(f, "not an EasyEffects output preset"),
            Self::NoConvolver => write!(f, "the preset has no convolver"),
        }
    }
}

impl std::error::Error for EasyEffectsError {}

/// Read an EasyEffects output preset.
///
/// # Errors
///
/// When the text is not JSON, not an output preset, or has no active
/// convolver.
pub fn parse_easyeffects(text: &str) -> Result<EasyEffects, EasyEffectsError> {
    let root: Value = serde_json::from_str(text).map_err(|_| EasyEffectsError::NotJson)?;
    let output = root
        .get("output")
        .and_then(Value::as_object)
        .ok_or(EasyEffectsError::NotOutput)?;
    let order: Vec<&str> = match output.get("plugins_order").and_then(Value::as_array) {
        Some(order) => order.iter().filter_map(Value::as_str).collect(),
        None => output
            .keys()
            .map(String::as_str)
            .filter(|k| k.contains('#'))
            .collect(),
    };
    let number = |plugin: &Value, key: &str| {
        plugin
            .get(key)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .filter(|v| v.is_finite())
    };
    let mut kernel = None;
    let mut gain = 0.0;
    let mut highpass = None;
    let mut limiter = None;
    let mut skipped = Vec::new();
    for name in order {
        let Some(plugin) = output.get(name) else {
            continue;
        };
        if plugin.get("bypass").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let base = name.split('#').next().unwrap_or(name);
        match base {
            "convolver" if kernel.is_none() => {
                let named = plugin
                    .get("kernel-name")
                    .or_else(|| plugin.get("kernel-path"))
                    .and_then(Value::as_str)
                    .filter(|k| !k.is_empty());
                if let Some(named) = named {
                    kernel = Some(named.to_string());
                    gain = number(plugin, "input-gain").unwrap_or(0.0)
                        + number(plugin, "output-gain").unwrap_or(0.0);
                } else {
                    skipped.push(base.to_string());
                }
            }
            // A high-pass before the convolver; one after it, or any other
            // filter, is not something the effect has.
            "filter"
                if kernel.is_none()
                    && highpass.is_none()
                    && plugin.get("type").and_then(Value::as_str) == Some("High-pass") =>
            {
                highpass = number(plugin, "frequency");
                if highpass.is_none() {
                    skipped.push(base.to_string());
                }
            }
            "limiter" if kernel.is_some() && limiter.is_none() => {
                limiter = Some(EasyEffectsLimiter {
                    input_gain: number(plugin, "input-gain").unwrap_or(0.0),
                    limit: number(plugin, "threshold").unwrap_or(0.0),
                    release: number(plugin, "release").unwrap_or(5.0) / 1000.0,
                });
            }
            _ => skipped.push(base.to_string()),
        }
    }
    Ok(EasyEffects {
        kernel: kernel.ok_or(EasyEffectsError::NoConvolver)?,
        gain,
        highpass,
        limiter,
        skipped,
    })
}

/// Find the impulse response `kernel` names for the preset at `preset`:
/// `<kernel>.irs` (or the name as given) beside it, in an `irs/` folder
/// beside it or one level up (EasyEffects' own layout), or in EasyEffects'
/// own `irs/` folders. An absolute `kernel` is taken as it is.
#[must_use]
pub fn find_kernel(preset: &Path, kernel: &str) -> Option<PathBuf> {
    let given = Path::new(kernel);
    if given.is_absolute() {
        return given.is_file().then(|| given.to_path_buf());
    }
    let mut dirs = Vec::new();
    if let Some(dir) = preset.parent() {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("irs"));
        if let Some(up) = dir.parent() {
            dirs.push(up.join("irs"));
        }
    }
    for base in [
        otto_foundations::xdg::config_home(),
        otto_foundations::xdg::data_home(),
    ]
    .into_iter()
    .flatten()
    {
        dirs.push(base.join("easyeffects/irs"));
    }
    let names = [format!("{kernel}.irs"), kernel.to_string()];
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structure of a laptop maker's speaker preset, trimmed.
    const PRESET: &str = r#"{
        "output": {
            "bass_enhancer#0": { "amount": 4.0, "bypass": true, "scope": 200.0 },
            "blocklist": [],
            "convolver#0": {
                "autogain": true, "bypass": false, "input-gain": 0.0, "ir-width": 100,
                "kernel-name": "IR_22ms_27dB_5t_15s_0c", "output-gain": 6.0
            },
            "filter#0": {
                "bypass": false, "frequency": 60.0, "mode": "RLC (BT)",
                "slope": "x16", "type": "High-pass"
            },
            "limiter#0": {
                "attack": 2.0, "bypass": false, "input-gain": 0.0, "lookahead": 4.0,
                "release": 8.0, "threshold": 0.0
            },
            "multiband_compressor#0": { "band0": { "ratio": 5.0 }, "bypass": false },
            "plugins_order": [
                "filter#0", "bass_enhancer#0", "convolver#0",
                "multiband_compressor#0", "stereo_tools#0", "limiter#0"
            ],
            "stereo_tools#0": { "bypass": false, "stereo-base": 0.3 }
        }
    }"#;

    #[test]
    fn a_speaker_preset_gives_its_kernel_high_pass_gain_and_limiter() {
        let preset = parse_easyeffects(PRESET).unwrap();
        assert_eq!(preset.kernel, "IR_22ms_27dB_5t_15s_0c");
        assert_eq!(preset.gain, 6.0);
        assert_eq!(preset.highpass, Some(60.0));
        assert_eq!(
            preset.limiter,
            Some(EasyEffectsLimiter {
                input_gain: 0.0,
                limit: 0.0,
                release: 0.008,
            })
        );
        // The bass enhancer is switched off, so nothing of it is lost.
        assert_eq!(preset.skipped, ["multiband_compressor", "stereo_tools"]);
    }

    #[test]
    fn other_files_are_told_apart() {
        assert_eq!(
            parse_easyeffects("Preamp: -6 dB"),
            Err(EasyEffectsError::NotJson)
        );
        assert_eq!(
            parse_easyeffects(r#"{ "input": { "plugins_order": [] } }"#),
            Err(EasyEffectsError::NotOutput)
        );
        assert_eq!(
            parse_easyeffects(
                r##"{ "output": { "plugins_order": ["convolver#0"],
                    "convolver#0": { "bypass": true, "kernel-name": "x" } } }"##
            ),
            Err(EasyEffectsError::NoConvolver)
        );
    }

    #[test]
    fn a_high_pass_after_the_convolver_and_a_limiter_before_it_are_skipped() {
        let preset = parse_easyeffects(
            r##"{ "output": {
                "plugins_order": ["limiter#0", "convolver#0", "filter#0"],
                "limiter#0": { "threshold": -3.0 },
                "convolver#0": { "kernel-path": "/irs/room.irs" },
                "filter#0": { "type": "High-pass", "frequency": 80.0 }
            } }"##,
        )
        .unwrap();
        assert_eq!(preset.kernel, "/irs/room.irs");
        assert_eq!(preset.gain, 0.0);
        assert_eq!(preset.highpass, None);
        assert_eq!(preset.limiter, None);
        assert_eq!(preset.skipped, ["limiter", "filter"]);
    }

    #[test]
    fn the_kernel_is_found_beside_the_preset_or_in_an_irs_folder() {
        let dir = tempfile::tempdir().unwrap();
        let preset = dir.path().join("speakers.json");
        assert_eq!(find_kernel(&preset, "nowhere-to-be-found-kernel"), None);

        std::fs::create_dir(dir.path().join("irs")).unwrap();
        let in_irs = dir.path().join("irs/room.irs");
        std::fs::write(&in_irs, b"RIFF").unwrap();
        assert_eq!(find_kernel(&preset, "room"), Some(in_irs.clone()));

        let beside = dir.path().join("room.irs");
        std::fs::write(&beside, b"RIFF").unwrap();
        assert_eq!(find_kernel(&preset, "room"), Some(beside));

        // EasyEffects' layout: output/<preset>.json and irs/<kernel>.irs.
        std::fs::create_dir(dir.path().join("output")).unwrap();
        let nested = dir.path().join("output/speakers.json");
        assert_eq!(find_kernel(&nested, "room"), Some(in_irs.clone()));
        assert_eq!(
            find_kernel(&nested, &in_irs.to_string_lossy()),
            Some(in_irs)
        );
    }
}
