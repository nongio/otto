//! Effects: equalisers, limiters, compressors, noise suppression and speaker
//! correction the service hosts as PipeWire filter chains.
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
//! kind, side, device, name or impulse response makes the chain again.
//!
//! A speaker correction keeps its impulse response by file name, in
//! `$XDG_DATA_HOME/otto/audio/impulses/` (see `impulse.rs`):
//!
//! ```toml
//! [[effect]]
//! id = "speaker-correction"
//! name = "Speaker correction"
//! kind = "speaker-correction"
//! side = "output"
//! device = "alsa_output.pci-0000_00_1f.3.analog-stereo"
//! impulse = "IR_22ms_27dB_5t_15s_0c.irs"
//! highpass = true
//!
//! [effect.params]
//! gain = 6.0
//! highpass-freq = 60.0
//! ```

// Rust guideline compliant 2026-02-21

mod autoeq;
mod biquad;
mod chain;
mod easyeffects;
mod impulse;
mod made;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Node;
use crate::routing::Routing;

pub use autoeq::{parse_autoeq, AutoEq, AutoEqError};
pub use biquad::{response_db, Coefficients};
pub use chain::{support, Support};
pub use easyeffects::{
    find_kernel, parse_easyeffects, EasyEffects, EasyEffectsError, EasyEffectsLimiter,
};
pub use impulse::{import_impulse, impulses_dir, ImpulseError, MAX_IMPULSE_BYTES};
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
    /// An impulse response convolved with the sound, after an optional
    /// high-pass: the tuning a laptop's maker ships for its speakers, or a
    /// room correction.
    SpeakerCorrection,
}

impl EffectKind {
    /// Every kind, in the order they are offered.
    pub const ALL: [Self; 5] = [
        Self::Equaliser,
        Self::Limiter,
        Self::Compressor,
        Self::NoiseSuppression,
        Self::SpeakerCorrection,
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
            Self::SpeakerCorrection => &chain::SPEAKER_PARAMS,
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
    Hertz,
}

/// One parameter of a limiter, compressor or noise suppressor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    /// What the routing file calls it.
    pub key: &'static str,
    /// The plugin's control port it sets; empty for a value the chain
    /// applies elsewhere (a speaker correction's gain and high-pass).
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
    /// A speaker correction's impulse response: a file name in
    /// [`impulses_dir`], or an absolute path written by hand. None passes
    /// the sound through the convolver unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impulse: Option<String>,
    /// Whether a speaker correction cuts the lows below its `highpass-freq`
    /// before the impulse response.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highpass: bool,
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
            && self.impulse == other.impulse
    }

    /// Where its impulse response is read from, whether it exists or not.
    #[must_use]
    pub fn impulse_path(&self) -> Option<std::path::PathBuf> {
        impulse::resolve(self.impulse.as_deref()?)
    }

    /// Whether it sits on a device. One on none (an empty `device`) is a
    /// tuning kept for later: it stays in the file but is not made.
    #[must_use]
    pub fn is_placed(&self) -> bool {
        !self.device.is_empty()
    }

    /// Whether it is a device's tuning: an output equaliser or speaker
    /// correction, which shape how the device sounds.
    #[must_use]
    pub fn is_tuning(&self) -> bool {
        self.side == EffectSide::Output
            && matches!(
                self.kind,
                EffectKind::Equaliser | EffectKind::SpeakerCorrection
            )
    }

    /// The most an equaliser boosts any frequency, preamp included, in dB:
    /// above 0 dB a loud song clips. `None` for other kinds.
    #[must_use]
    pub fn peak_gain_db(&self) -> Option<f32> {
        if self.kind != EffectKind::Equaliser {
            return None;
        }
        // 20 Hz to 20 kHz, evenly on a log scale: fine enough to find the
        // top of a narrow peak within a fraction of a dB.
        const STEPS: u16 = 240;
        let peak = (0..=STEPS)
            .map(|step| {
                let freq =
                    MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(f32::from(step) / f32::from(STEPS));
                response_db(self.preamp, &self.bands, freq)
            })
            .fold(f32::MIN, f32::max);
        Some(peak)
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
            impulse: None,
            highpass: false,
        });
        id
    }

    /// Add an effect of `kind`, called `name`, after the effect `id` as the
    /// sound flows: it sits on `id`'s device and `id` moves onto it. Returns
    /// its id, or `None` when there is no effect `id`.
    pub fn add_effect_after(&mut self, id: &str, name: &str, kind: EffectKind) -> Option<String> {
        let (side, device) = {
            let effect = self.effect(id)?;
            (effect.side, effect.device.clone())
        };
        let added = self.add_effect(name, kind, side, &device);
        let node = self.effect(&added)?.node_name();
        self.update_effect(id, |e| e.device = node);
        Some(added)
    }

    /// The node an app plays to for its sound to reach `device` through the
    /// output effects on it: the effect at the top of its chain, or the
    /// device itself when none sits on it.
    #[must_use]
    pub fn output_entry(&self, device: &str) -> String {
        let chain = self.output_chain(device);
        chain
            .first()
            .map_or_else(|| device.to_string(), |e| e.node_name())
    }

    /// The output effects sound passes on its way to `device`, from the
    /// apps' end to the device's.
    #[must_use]
    pub fn output_chain(&self, device: &str) -> Vec<&Effect> {
        let mut chain: Vec<&Effect> = Vec::new();
        let mut at = device.to_string();
        // Bounded, so a hand-edited file that loops cannot hang anyone.
        for _ in 0..8 {
            let Some(effect) = self
                .effects
                .iter()
                .find(|e| e.side == EffectSide::Output && e.device == at && !chain.contains(e))
            else {
                break;
            };
            at = effect.node_name();
            chain.push(effect);
        }
        chain.reverse();
        chain
    }

    /// Move the effect `id` onto `device`, right before it: what sat on the
    /// effect moves onto where the effect was, and what sat on `device`
    /// moves onto the effect. An empty `device` takes the effect out of its
    /// chain and keeps it unmade. `false` when there is no effect `id` or it
    /// is there already.
    pub fn move_effect(&mut self, id: &str, device: &str) -> bool {
        let Some(moved) = self.effect(id).cloned() else {
            return false;
        };
        if moved.device == device {
            return false;
        }
        let node = moved.node_name();
        for effect in &mut self.effects {
            if effect.id != id && effect.device == node {
                effect.device.clone_from(&moved.device);
            }
        }
        if !device.is_empty() {
            for effect in &mut self.effects {
                if effect.id != id && effect.side == moved.side && effect.device == device {
                    effect.device.clone_from(&node);
                }
            }
        }
        self.update_effect(id, |e| e.device = device.to_string())
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

    #[test]
    fn a_speaker_correction_keeps_its_impulse_and_high_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing.toml");
        let mut routing = Routing::default();
        let id = routing.add_effect(
            "Speakers",
            EffectKind::SpeakerCorrection,
            EffectSide::Output,
            "spk",
        );
        assert!(routing.effect(&id).unwrap().impulse.is_none());
        routing.update_effect(&id, |e| {
            e.impulse = Some("fw13.irs".into());
            e.highpass = true;
            e.params.insert("gain".into(), 6.0);
            e.params.insert("highpass-freq".into(), 60.0);
        });
        routing.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("kind = \"speaker-correction\""), "{text}");
        assert!(text.contains("impulse = \"fw13.irs\""), "{text}");
        assert!(text.contains("highpass = true"), "{text}");
        assert_eq!(
            Routing::load_from(&path).unwrap().effects(),
            routing.effects()
        );

        // Neither field is written for other kinds, and a file without them
        // reads with no impulse and the high-pass off.
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        let text = toml::to_string(&routing).unwrap();
        assert_eq!(text.matches("impulse").count(), 1, "{text}");
        assert!(!routing.effect(&eq).unwrap().highpass);
        let bare: Routing = toml::from_str(
            "[[effect]]\nid = \"s\"\nname = \"S\"\nkind = \"speaker-correction\"\nside = \"output\"\ndevice = \"spk\"\n",
        )
        .unwrap();
        assert_eq!(bare.effects()[0].impulse, None);
        assert!(!bare.effects()[0].highpass);
    }

    #[test]
    fn an_absolute_impulse_is_read_where_it_is() {
        let mut routing = Routing::default();
        let id = routing.add_effect(
            "S",
            EffectKind::SpeakerCorrection,
            EffectSide::Output,
            "spk",
        );
        routing.update_effect(&id, |e| e.impulse = Some("/srv/ir/room.wav".into()));
        assert_eq!(
            routing.effect(&id).unwrap().impulse_path(),
            Some(std::path::PathBuf::from("/srv/ir/room.wav"))
        );
    }

    #[test]
    fn a_tuning_moves_between_devices_and_its_chains_stay_whole() {
        let mut routing = Routing::default();
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        let lim = routing
            .add_effect_after(&eq, "Limiter", EffectKind::Limiter)
            .unwrap();
        // apps -> EQ -> Limiter -> hp
        let names = |r: &Routing, d: &str| -> Vec<String> {
            r.output_chain(d).iter().map(|e| e.id.clone()).collect()
        };
        assert_eq!(names(&routing, "hp"), vec![eq.clone(), lim.clone()]);
        assert_eq!(routing.output_entry("hp"), "otto-effect.eq");
        assert_eq!(routing.output_entry("spk"), "spk");

        // To the speakers: the limiter stays on the headphones.
        assert!(routing.move_effect(&eq, "spk"));
        assert_eq!(names(&routing, "hp"), vec![lim.clone()]);
        assert_eq!(names(&routing, "spk"), vec![eq.clone()]);
        assert!(!routing.move_effect(&eq, "spk"));

        // Taken off: kept, but on no device.
        assert!(routing.move_effect(&eq, ""));
        assert!(!routing.effect(&eq).unwrap().is_placed());
        assert!(names(&routing, "spk").is_empty());
    }

    #[test]
    fn an_equaliser_s_peak_counts_its_preamp() {
        let mut routing = Routing::default();
        let eq = routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "hp");
        let peak = |r: &Routing| r.effect(&eq).unwrap().peak_gain_db().unwrap();
        assert!(peak(&routing).abs() < 0.1);
        routing.update_effect(&eq, |e| e.bands[2].gain = 6.0);
        assert!((peak(&routing) - 6.0).abs() < 0.3, "{}", peak(&routing));
        routing.update_effect(&eq, |e| e.preamp = -6.0);
        assert!(peak(&routing).abs() < 0.3);
        let lim = routing.add_effect("L", EffectKind::Limiter, EffectSide::Output, "hp");
        assert_eq!(routing.effect(&lim).unwrap().peak_gain_db(), None);
    }

    #[test]
    fn an_effect_added_after_another_takes_its_place_before_the_device() {
        let mut routing = Routing::default();
        let sc = routing.add_effect(
            "S",
            EffectKind::SpeakerCorrection,
            EffectSide::Output,
            "spk",
        );
        let lim = routing
            .add_effect_after(&sc, "Limiter", EffectKind::Limiter)
            .unwrap();
        let lim = routing.effect(&lim).unwrap().clone();
        assert_eq!(lim.device, "spk");
        assert_eq!(routing.effect(&sc).unwrap().device, lim.node_name());
        assert_eq!(
            routing.add_effect_after("none", "L", EffectKind::Limiter),
            None
        );
    }
}
