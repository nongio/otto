//! Effects: equalisers, limiters, compressors and noise suppression the
//! service hosts as PipeWire filter chains.
//!
//! An effect sits on a device. An **output** effect is a sink apps play into,
//! whose processed sound plays on its device; an **input** effect records
//! from its device and is a source apps record from. A device can be another
//! effect of the same side, which chains them.
//!
//! Each is kept in the routing file as an [`Effect`]:
//!
//! ```toml
//! [[effect]]
//! id = "headphone-eq"
//! name = "Headphone EQ"
//! kind = "equaliser"
//! side = "output"
//! device = "bluez_output.AC_80_0A_00_00_00.1"
//! enabled = true
//! preamp = -4.0
//!
//! [[effect.band]]
//! type = "low-shelf"
//! freq = 105.0
//! gain = 4.0
//! q = 0.7
//! ```
//!
//! The service makes each with `libpipewire-module-filter-chain`, loaded into
//! its own context (see `chain.rs` for the graph and `made.rs` for the
//! thread). Every value a person edits, On included, is a control of the
//! chain and is changed live through the filter node's `Props`; only a new
//! kind, side, device or name makes the chain again.

// Rust guideline compliant 2026-02-21

mod autoeq;
mod biquad;
mod chain;
mod made;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Node;
use crate::routing::Routing;

pub use autoeq::{parse_autoeq, AutoEq, AutoEqError};
pub use biquad::{response_db, Coefficients};
pub use chain::{support, Support};
pub(crate) use made::{set_controls, Made};

/// What every effect's node names start with.
pub const EFFECT_PREFIX: &str = "otto-effect.";

/// The most bands an equaliser has. The chain always holds this many
/// filters per channel, unused ones passing sound unchanged, so adding and
/// removing a band never makes the chain again.
pub const MAX_BANDS: usize = 10;

/// The sample rate every effect runs at, in Hz. The equaliser's coefficients
/// are computed for it, and noise suppression needs it; the server resamples
/// around the chain when the engine runs at another rate.
pub const EFFECT_RATE: u32 = 48_000;

/// The lowest and highest frequency a band may have, in Hz.
pub const MIN_FREQ: f32 = 20.0;
pub const MAX_FREQ: f32 = 20_000.0;
/// The gain range of a band and of the preamp, in dB.
pub const MAX_GAIN: f32 = 24.0;
/// The range of a band's Q.
pub const MIN_Q: f32 = 0.1;
pub const MAX_Q: f32 = 10.0;

/// What an effect does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    /// A parametric equaliser of up to [`MAX_BANDS`] bands and a preamp.
    Equaliser,
    /// Keeps peaks under a ceiling.
    Limiter,
    /// Evens out loud and quiet passages.
    Compressor,
    /// Takes background noise out of a voice.
    NoiseSuppression,
}

impl EffectKind {
    /// Every kind, in the order they are offered.
    pub const ALL: [Self; 4] = [
        Self::Equaliser,
        Self::Limiter,
        Self::Compressor,
        Self::NoiseSuppression,
    ];

    /// The parameters this kind has besides On; an equaliser's are its
    /// bands and preamp instead.
    #[must_use]
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            Self::Equaliser => &[],
            Self::Limiter => &chain::LIMITER_PARAMS,
            Self::Compressor => &chain::COMPRESSOR_PARAMS,
            Self::NoiseSuppression => &chain::NOISE_PARAMS,
        }
    }
}

/// Which way sound goes through an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectSide {
    /// Apps play into it, and it plays on its device.
    Output,
    /// It records from its device, and apps record from it.
    Input,
}

/// The unit a parameter is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Decibels,
    Milliseconds,
    Seconds,
    /// A ratio, `1:n`.
    Ratio,
    Percent,
}

/// One parameter of a limiter, compressor or noise suppressor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// What the routing file calls it.
    pub key: &'static str,
    /// The plugin's control port it sets.
    pub(crate) port: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
}

/// The kind of one equaliser band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BandType {
    LowShelf,
    Peaking,
    HighShelf,
    LowPass,
    HighPass,
}

impl BandType {
    /// Every type, in the order they are offered.
    pub const ALL: [Self; 5] = [
        Self::LowShelf,
        Self::Peaking,
        Self::HighShelf,
        Self::LowPass,
        Self::HighPass,
    ];

    /// Whether the band's gain does anything; a pass filter has none.
    #[must_use]
    pub fn has_gain(self) -> bool {
        !matches!(self, Self::LowPass | Self::HighPass)
    }
}

/// One equaliser band.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    #[serde(rename = "type")]
    pub kind: BandType,
    /// The centre or corner frequency, in Hz.
    pub freq: f32,
    /// In dB; ignored by a pass filter.
    #[serde(default)]
    pub gain: f32,
    pub q: f32,
}

impl Band {
    /// This band, with every value inside its range.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            kind: self.kind,
            freq: finite_or(self.freq, 1000.0).clamp(MIN_FREQ, MAX_FREQ),
            gain: finite_or(self.gain, 0.0).clamp(-MAX_GAIN, MAX_GAIN),
            q: finite_or(self.q, 0.707).clamp(MIN_Q, MAX_Q),
        }
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The bands a new equaliser starts with, and what the Flat preset puts
/// back: a shelf at each end and three peaks between, all at 0 dB.
#[must_use]
pub fn flat_bands() -> Vec<Band> {
    let band = |kind, freq, q| Band {
        kind,
        freq,
        gain: 0.0,
        q,
    };
    vec![
        band(BandType::LowShelf, 105.0, 0.7),
        band(BandType::Peaking, 250.0, 1.0),
        band(BandType::Peaking, 1000.0, 1.0),
        band(BandType::Peaking, 3000.0, 1.0),
        band(BandType::HighShelf, 8000.0, 0.7),
    ]
}

fn yes() -> bool {
    true
}

/// One effect, as the routing file keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// What its node names are made from; never changes once made.
    pub id: String,
    /// The name apps and the Sound pane show.
    pub name: String,
    pub kind: EffectKind,
    pub side: EffectSide,
    /// The `node.name` of the device it sits on: a sink for an output
    /// effect, a source for an input one, or another effect's node.
    pub device: String,
    /// Off passes sound through unchanged.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// An equaliser's gain before its bands, in dB.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub preamp: f32,
    /// An equaliser's bands, at most [`MAX_BANDS`].
    #[serde(default, rename = "band", skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<Band>,
    /// The preset last applied, shown until a band is changed by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The values of the kind's [`ParamSpec`]s, by key; a missing one is at
    /// its default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f32>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

impl Effect {
    /// The `node.name` apps see: the sink of an output effect, the source of
    /// an input one.
    #[must_use]
    pub fn node_name(&self) -> String {
        format!("{EFFECT_PREFIX}{}", self.id)
    }

    /// The `node.name` of its other side, the stream to or from its device.
    #[must_use]
    pub fn stream_name(&self) -> String {
        match self.side {
            EffectSide::Output => format!("{EFFECT_PREFIX}{}.play", self.id),
            EffectSide::Input => format!("{EFFECT_PREFIX}{}.rec", self.id),
        }
    }

    /// Whether `node` is one of its two nodes.
    #[must_use]
    pub fn owns(&self, node: &Node) -> bool {
        let name = node.name();
        name == self.node_name() || name == self.stream_name()
    }

    /// How many channels it processes: stereo for an output, mono for an
    /// input, since microphones are mono.
    #[must_use]
    pub fn channels(&self) -> u32 {
        match self.side {
            EffectSide::Output => 2,
            EffectSide::Input => 1,
        }
    }

    /// The value of parameter `spec`, inside its range.
    #[must_use]
    pub fn param(&self, spec: &ParamSpec) -> f32 {
        self.params
            .get(spec.key)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(spec.default)
            .clamp(spec.min, spec.max)
    }

    /// What makes the chain itself; a change to any of it makes the chain
    /// again, while everything else is changed live.
    pub(crate) fn same_chain(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.kind == other.kind
            && self.side == other.side
            && self.device == other.device
    }
}

impl Routing {
    /// The effects, in the order they were added.
    #[must_use]
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The effect `id`.
    #[must_use]
    pub fn effect(&self, id: &str) -> Option<&Effect> {
        self.effects.iter().find(|e| e.id == id)
    }

    /// Change the effect `id` with `change`; the bands and values are kept
    /// in range afterwards. `false` when there is none or nothing changed.
    pub fn update_effect(&mut self, id: &str, change: impl FnOnce(&mut Effect)) -> bool {
        let Some(effect) = self.effects.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        let before = effect.clone();
        change(effect);
        effect.bands.truncate(MAX_BANDS);
        for band in &mut effect.bands {
            *band = band.clamped();
        }
        effect.preamp = finite_or(effect.preamp, 0.0).clamp(-MAX_GAIN, MAX_GAIN);
        *effect != before
    }

    /// Add an effect of `kind` on `device`, called `name`; returns its id. A
    /// new equaliser starts flat.
    pub fn add_effect(
        &mut self,
        name: &str,
        kind: EffectKind,
        side: EffectSide,
        device: &str,
    ) -> String {
        let taken: Vec<&str> = self.effects.iter().map(|e| e.id.as_str()).collect();
        let id = crate::virtual_devices::id_for(name, &taken);
        let bands = if kind == EffectKind::Equaliser {
            flat_bands()
        } else {
            Vec::new()
        };
        self.effects.push(Effect {
            id: id.clone(),
            name: name.trim().to_string(),
            kind,
            side,
            device: device.to_string(),
            enabled: true,
            preamp: 0.0,
            bands,
            preset: None,
            params: BTreeMap::new(),
        });
        id
    }

    /// Remove the effect `id`. An effect that sat on it moves onto the
    /// device it sat on, so a chain stays whole. `false` when there is none.
    pub fn remove_effect(&mut self, id: &str) -> bool {
        let Some(at) = self.effects.iter().position(|e| e.id == id) else {
            return false;
        };
        let removed = self.effects.remove(at);
        let node = removed.node_name();
        for effect in &mut self.effects {
            if effect.device == node {
                effect.device.clone_from(&removed.device);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_equaliser_starts_flat_and_others_have_no_bands() {
        let mut routing = Routing::default();
        let eq = routing.add_effect(
            "Headphone EQ",
            EffectKind::Equaliser,
            EffectSide::Output,
            "hp",
        );
        let lim = routing.add_effect("Limiter", EffectKind::Limiter, EffectSide::Output, "hp");
        assert_eq!(eq, "headphone-eq");
        let eq = routing.effect(&eq).unwrap();
        assert_eq!(eq.bands, flat_bands());
        assert_eq!(eq.node_name(), "otto-effect.headphone-eq");
        assert_eq!(eq.stream_name(), "otto-effect.headphone-eq.play");
        assert!(routing.effect(&lim).unwrap().bands.is_empty());
    }

    #[test]
    fn removing_an_effect_keeps_the_chain_whole() {
        let mut routing = Routing::default();
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        let node = routing.effect(&eq).unwrap().node_name();
        let lim = routing.add_effect("Limiter", EffectKind::Limiter, EffectSide::Output, &node);
        assert!(routing.remove_effect(&eq));
        assert!(!routing.remove_effect(&eq));
        assert_eq!(routing.effect(&lim).unwrap().device, "hp");
    }

    #[test]
    fn updates_stay_in_range() {
        let mut routing = Routing::default();
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        assert!(routing.update_effect(&eq, |e| {
            e.bands[0].freq = 1.0;
            e.bands[0].gain = f32::NAN;
            e.bands[0].q = 99.0;
            e.preamp = -100.0;
            e.bands.extend(std::iter::repeat_n(flat_bands()[1], 20));
        }));
        let effect = routing.effect(&eq).unwrap();
        assert_eq!(effect.bands.len(), MAX_BANDS);
        assert_eq!(effect.bands[0].freq, MIN_FREQ);
        assert_eq!(effect.bands[0].gain, 0.0);
        assert_eq!(effect.bands[0].q, MAX_Q);
        assert_eq!(effect.preamp, -MAX_GAIN);
        assert!(!routing.update_effect(&eq, |_| {}));
        assert!(!routing.update_effect("none", |e| e.enabled = false));
    }

    #[test]
    fn missing_params_read_as_their_defaults() {
        let mut routing = Routing::default();
        let id = routing.add_effect("C", EffectKind::Compressor, EffectSide::Output, "hp");
        let spec = EffectKind::Compressor.params()[0];
        let effect = routing.effect(&id).unwrap();
        assert_eq!(effect.param(&spec), spec.default);
        routing.update_effect(&id, |e| {
            e.params.insert(spec.key.into(), spec.max + 100.0);
        });
        assert_eq!(routing.effect(&id).unwrap().param(&spec), spec.max);
    }

    #[test]
    fn the_file_keeps_effects_and_old_files_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing.toml");
        let mut routing = Routing::default();
        routing.add_virtual("Mix", crate::VirtualKind::Output, 2);
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        routing.update_effect(&eq, |e| e.preamp = -3.5);
        let ns = routing.add_effect(
            "Mic",
            EffectKind::NoiseSuppression,
            EffectSide::Input,
            "mic",
        );
        routing.update_effect(&ns, |e| {
            e.params.insert("threshold".into(), 80.0);
            e.enabled = false;
        });
        routing.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[effect]]"), "{text}");
        assert!(text.contains("[[effect.band]]"), "{text}");
        let back = Routing::load_from(&path).unwrap();
        assert_eq!(back.effects(), routing.effects());

        let old: Routing = toml::from_str(
            "version = 1\n[[virtual]]\nid = \"a\"\nname = \"A\"\nkind = \"output\"\n",
        )
        .unwrap();
        assert!(old.effects().is_empty());
        let bare: Routing = toml::from_str(
            "[[effect]]\nid = \"l\"\nname = \"L\"\nkind = \"limiter\"\nside = \"output\"\ndevice = \"hp\"\n",
        )
        .unwrap();
        assert!(bare.effects()[0].enabled);
    }
}
