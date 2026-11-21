//! The filter chain behind each effect: which plugin each kind needs and
//! whether it is installed, the module's arguments, and the controls that
//! change it live.
//!
//! Every chain has the same shape, one copy per channel:
//!
//! ```text
//! in ──► [the effect] ──► mix "In 1" (wet, × preamp)
//!  └───────────────────► mix "In 2" (dry)
//! ```
//!
//! On sets the mixer to wet and Off to dry, so turning an effect off or on
//! is a control change with no gap in the sound. An equaliser is
//! [`MAX_BANDS`] PipeWire `bq_raw` filters in a row whose coefficients come
//! from `biquad.rs`; unused ones pass sound unchanged. A limiter and a
//! compressor are the SWH LADSPA plugins, noise suppression the RNNoise one;
//! they are found on the LADSPA path, and an effect whose plugin is missing
//! is not made.

// Rust guideline compliant 2026-02-21

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::biquad::Coefficients;
use super::{Effect, EffectKind, EffectSide, ParamSpec, Unit, EFFECT_RATE, MAX_BANDS};
use crate::module::quoted;

/// Where LADSPA plugins are looked for when `LADSPA_PATH` is not set: the
/// directories distributions install them to, the ones PipeWire searches too.
const LADSPA_DIRS: [&str; 5] = [
    "/usr/lib/ladspa",
    "/usr/lib64/ladspa",
    "/usr/local/lib/ladspa",
    "/usr/local/lib64/ladspa",
    "/usr/lib/x86_64-linux-gnu/ladspa",
];

/// A LADSPA plugin an effect kind is built on.
struct Plugin {
    /// The library's file name, without `.so`.
    file: &'static str,
    label: &'static str,
    /// Its audio ports. A stereo plugin takes both channels of one chain; a
    /// mono one is used once per channel.
    inputs: &'static [&'static str],
    outputs: &'static [&'static str],
    /// The package most distributions ship it in.
    package: &'static str,
}

/// Steve Harris's lookahead limiter.
const LIMITER: Plugin = Plugin {
    file: "fast_lookahead_limiter_1913",
    label: "fastLookaheadLimiter",
    inputs: &["Input 1", "Input 2"],
    outputs: &["Output 1", "Output 2"],
    package: "swh-plugins",
};

/// Steve Harris's SC4 stereo compressor.
const COMPRESSOR: Plugin = Plugin {
    file: "sc4_1882",
    label: "sc4",
    inputs: &["Left input", "Right input"],
    outputs: &["Left output", "Right output"],
    package: "swh-plugins",
};

/// RNNoise, from werman's noise-suppression-for-voice.
const NOISE: Plugin = Plugin {
    file: "librnnoise_ladspa",
    label: "noise_suppressor_mono",
    inputs: &["Input"],
    outputs: &["Output"],
    package: "noise-suppression-for-voice",
};

pub(super) const LIMITER_PARAMS: [ParamSpec; 3] = [
    ParamSpec {
        key: "input-gain",
        port: "Input gain (dB)",
        min: -20.0,
        max: 20.0,
        default: 0.0,
        unit: Unit::Decibels,
    },
    ParamSpec {
        key: "limit",
        port: "Limit (dB)",
        min: -20.0,
        max: 0.0,
        default: -1.0,
        unit: Unit::Decibels,
    },
    ParamSpec {
        key: "release",
        port: "Release time (s)",
        min: 0.01,
        max: 2.0,
        default: 0.08,
        unit: Unit::Seconds,
    },
];

pub(super) const COMPRESSOR_PARAMS: [ParamSpec; 6] = [
    ParamSpec {
        key: "threshold",
        port: "Threshold level (dB)",
        min: -30.0,
        max: 0.0,
        default: -18.0,
        unit: Unit::Decibels,
    },
    ParamSpec {
        key: "ratio",
        port: "Ratio (1:n)",
        min: 1.0,
        max: 20.0,
        default: 4.0,
        unit: Unit::Ratio,
    },
    ParamSpec {
        key: "attack",
        port: "Attack time (ms)",
        min: 1.5,
        max: 400.0,
        default: 10.0,
        unit: Unit::Milliseconds,
    },
    ParamSpec {
        key: "release",
        port: "Release time (ms)",
        min: 2.0,
        max: 800.0,
        default: 100.0,
        unit: Unit::Milliseconds,
    },
    ParamSpec {
        key: "knee",
        port: "Knee radius (dB)",
        min: 1.0,
        max: 10.0,
        default: 3.0,
        unit: Unit::Decibels,
    },
    ParamSpec {
        key: "makeup",
        port: "Makeup gain (dB)",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Decibels,
    },
];

pub(super) const NOISE_PARAMS: [ParamSpec; 2] = [
    ParamSpec {
        key: "threshold",
        port: "VAD Threshold (%)",
        min: 0.0,
        max: 99.0,
        default: 50.0,
        unit: Unit::Percent,
    },
    ParamSpec {
        key: "grace",
        port: "VAD Grace Period (ms)",
        min: 0.0,
        max: 1000.0,
        default: 200.0,
        unit: Unit::Milliseconds,
    },
];

/// Whether an effect kind can be made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// Everything it needs is installed.
    Ready,
    /// Its plugin, `file`, was not found; `package` provides it.
    Missing {
        file: &'static str,
        package: &'static str,
    },
}

fn plugin_of(kind: EffectKind) -> Option<&'static Plugin> {
    match kind {
        EffectKind::Equaliser => None,
        EffectKind::Limiter => Some(&LIMITER),
        EffectKind::Compressor => Some(&COMPRESSOR),
        EffectKind::NoiseSuppression => Some(&NOISE),
    }
}

/// The directories LADSPA plugins are looked for in.
fn ladspa_dirs() -> Vec<PathBuf> {
    match std::env::var_os("LADSPA_PATH") {
        Some(path) if !path.is_empty() => std::env::split_paths(&path).collect(),
        _ => LADSPA_DIRS.iter().map(PathBuf::from).collect(),
    }
}

fn find(plugin: &Plugin, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(format!("{}.so", plugin.file)))
        .find(|path| path.is_file())
}

/// Whether `kind` can be made here: an equaliser always can, the others
/// when their plugin is installed.
#[must_use]
pub fn support(kind: EffectKind) -> Support {
    match plugin_of(kind) {
        None => Support::Ready,
        Some(plugin) => match find(plugin, &ladspa_dirs()) {
            Some(_) => Support::Ready,
            None => Support::Missing {
                file: plugin.file,
                package: plugin.package,
            },
        },
    }
}

/// The node names inside the chain.
fn input_node(channel: u32) -> String {
    format!("in_{channel}")
}
fn mix_node(channel: u32) -> String {
    format!("mix_{channel}")
}
fn band_node(slot: usize, channel: u32) -> String {
    format!("eq{slot}_{channel}")
}
/// A stereo plugin's single node, or a mono plugin's node for `channel`.
fn plugin_node(plugin: &Plugin, channel: u32) -> String {
    if plugin.inputs.len() > 1 {
        "fx".to_string()
    } else {
        format!("fx_{channel}")
    }
}

/// Every control of `effect`'s chain and its value, as `node:port`.
pub(crate) fn controls(effect: &Effect) -> Vec<(String, f32)> {
    let mut out = Vec::new();
    let channels = effect.channels();
    let wet = if effect.kind == EffectKind::Equaliser {
        10f32.powf(effect.preamp / 20.0)
    } else {
        1.0
    };
    for channel in 0..channels {
        let mix = mix_node(channel);
        out.push((
            format!("{mix}:Gain 1"),
            if effect.enabled { wet } else { 0.0 },
        ));
        out.push((
            format!("{mix}:Gain 2"),
            if effect.enabled { 0.0 } else { 1.0 },
        ));
    }
    match plugin_of(effect.kind) {
        None => {
            let rate = f64::from(EFFECT_RATE);
            for slot in 0..MAX_BANDS {
                let c = effect
                    .bands
                    .get(slot)
                    .map_or(Coefficients::IDENTITY, |b| Coefficients::of(b, rate));
                let values = [
                    ("b0", c.b0),
                    ("b1", c.b1),
                    ("b2", c.b2),
                    ("a0", 1.0),
                    ("a1", c.a1),
                    ("a2", c.a2),
                ];
                for channel in 0..channels {
                    let node = band_node(slot, channel);
                    for (port, value) in values {
                        out.push((format!("{node}:{port}"), value as f32));
                    }
                }
            }
        }
        Some(plugin) => {
            let mut nodes: Vec<String> = (0..channels).map(|c| plugin_node(plugin, c)).collect();
            nodes.dedup();
            for node in nodes {
                for spec in effect.kind.params() {
                    out.push((format!("{node}:{}", spec.port), effect.param(spec)));
                }
            }
        }
    }
    out
}

/// The node and port halves of a control's `node:port` name.
fn split_control(name: &str) -> (&str, &str) {
    name.split_once(':').unwrap_or((name, ""))
}

/// The arguments `libpipewire-module-filter-chain` takes for `effect`, or
/// `None` when its plugin is not installed.
pub(crate) fn args(effect: &Effect) -> Option<String> {
    let plugin = match plugin_of(effect.kind) {
        None => None,
        Some(plugin) => Some((plugin, find(plugin, &ladspa_dirs())?)),
    };
    Some(args_with(
        effect,
        plugin.as_ref().map(|(p, path)| (*p, path.as_path())),
    ))
}

fn args_with(effect: &Effect, plugin: Option<(&Plugin, &Path)>) -> String {
    let channels = effect.channels();
    let controls = controls(effect);
    let control_block = |node: &str| -> String {
        let mut block = String::from("control = {");
        for (name, value) in &controls {
            let (n, port) = split_control(name);
            if n == node {
                let _ = write!(block, " {} = {}", quoted(port), number(*value));
            }
        }
        block.push_str(" }");
        block
    };
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let link = |links: &mut Vec<String>, from: String, to: String| {
        links.push(format!(
            "{{ output = {} input = {} }}",
            quoted(&from),
            quoted(&to)
        ));
    };
    for channel in 0..channels {
        nodes.push(format!(
            "{{ type = builtin name = {} label = copy }}",
            input_node(channel)
        ));
        let mix = mix_node(channel);
        nodes.push(format!(
            "{{ type = builtin name = {mix} label = mixer {} }}",
            control_block(&mix)
        ));
        link(
            &mut links,
            format!("{}:Out", input_node(channel)),
            format!("{mix}:In 2"),
        );
    }
    match plugin {
        None => {
            for channel in 0..channels {
                let mut from = format!("{}:Out", input_node(channel));
                for slot in 0..MAX_BANDS {
                    let node = band_node(slot, channel);
                    nodes.push(format!(
                        "{{ type = builtin name = {node} label = bq_raw {} {} }}",
                        raw_config(&controls, &node),
                        control_block(&node)
                    ));
                    link(&mut links, from, format!("{node}:In"));
                    from = format!("{node}:Out");
                }
                link(&mut links, from, format!("{}:In 1", mix_node(channel)));
            }
        }
        Some((plugin, path)) => {
            let mut made: Vec<String> = Vec::new();
            for channel in 0..channels {
                let node = plugin_node(plugin, channel);
                if !made.contains(&node) {
                    nodes.push(format!(
                        "{{ type = ladspa name = {node} plugin = {} label = {} {} }}",
                        quoted(&path.to_string_lossy()),
                        quoted(plugin.label),
                        control_block(&node)
                    ));
                    made.push(node.clone());
                }
            }
            if plugin.inputs.len() > 1 {
                // One stereo instance: each of its inputs takes a channel,
                // the first channel again when there is only one.
                for (index, port) in plugin.inputs.iter().enumerate() {
                    let channel = (index as u32).min(channels - 1);
                    link(
                        &mut links,
                        format!("{}:Out", input_node(channel)),
                        format!("fx:{port}"),
                    );
                }
                for channel in 0..channels {
                    let port = plugin.outputs[(channel as usize).min(plugin.outputs.len() - 1)];
                    link(
                        &mut links,
                        format!("fx:{port}"),
                        format!("{}:In 1", mix_node(channel)),
                    );
                }
            } else {
                for channel in 0..channels {
                    let node = plugin_node(plugin, channel);
                    link(
                        &mut links,
                        format!("{}:Out", input_node(channel)),
                        format!("{node}:{}", plugin.inputs[0]),
                    );
                    link(
                        &mut links,
                        format!("{node}:{}", plugin.outputs[0]),
                        format!("{}:In 1", mix_node(channel)),
                    );
                }
            }
        }
    }
    let inputs: Vec<String> = (0..channels)
        .map(|c| quoted(&format!("{}:In", input_node(c))))
        .collect();
    let outputs: Vec<String> = (0..channels)
        .map(|c| quoted(&format!("{}:Out", mix_node(c))))
        .collect();
    let positions = crate::virtual_devices::positions(channels).join(" ");
    let name = quoted(&effect.name);
    // The device side targets its device. While the device is gone the
    // session manager plays it on the default, as it does any stream, and
    // moves it back when the device returns. It is never told to wait
    // unlinked instead: the session manager fails such a stream, which takes
    // the whole chain down.
    let device_side = format!(
        "{{ node.name = {} target.object = {} node.passive = true \
         state.restore-target = false state.restore-props = false }}",
        quoted(&effect.stream_name()),
        quoted(&effect.device),
    );
    let (class, capture, playback) = match effect.side {
        EffectSide::Output => ("Audio/Sink", None, Some(device_side)),
        EffectSide::Input => ("Audio/Source", Some(device_side), None),
    };
    // The side apps see; never chosen as a default on its own.
    let app_side = format!(
        "{{ node.name = {} media.class = {class} priority.session = 0 }}",
        quoted(&effect.node_name())
    );
    format!(
        "{{ node.description = {name} media.name = {name} audio.rate = {EFFECT_RATE} \
         audio.channels = {channels} audio.position = [ {positions} ] \
         filter.graph = {{ nodes = [ {} ] links = [ {} ] inputs = [ {} ] outputs = [ {} ] }} \
         capture.props = {} playback.props = {} }}",
        nodes.join(" "),
        links.join(" "),
        inputs.join(" "),
        outputs.join(" "),
        capture.unwrap_or_else(|| app_side.clone()),
        playback.unwrap_or(app_side),
    )
}

/// The `config` a `bq_raw` filter cannot start without: its coefficients
/// at the rate effects run at, the same values its controls carry.
fn raw_config(controls: &[(String, f32)], node: &str) -> String {
    let mut config = format!("config = {{ coefficients = [ {{ rate = {EFFECT_RATE}");
    for (name, value) in controls {
        let (n, port) = split_control(name);
        if n == node {
            let _ = write!(config, " {port} = {}", number(*value));
        }
    }
    config.push_str(" } ] }");
    config
}

/// A float as SPA JSON writes it.
fn number(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "0".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::Routing;

    fn effect(kind: EffectKind, side: EffectSide) -> Effect {
        let mut routing = Routing::default();
        let id = routing.add_effect("Test FX", kind, side, "dev");
        routing.effect(&id).unwrap().clone()
    }

    #[test]
    fn an_equaliser_is_a_row_of_raw_biquads_per_channel() {
        let eq = effect(EffectKind::Equaliser, EffectSide::Output);
        let args = args_with(&eq, None);
        assert_eq!(args.matches("label = bq_raw").count(), MAX_BANDS * 2);
        assert!(
            args.contains("config = { coefficients = [ { rate = 48000 b0 = 1 b1 = 0"),
            "{args}"
        );
        assert!(
            args.contains(r#"{ output = "in_0:Out" input = "mix_0:In 2" }"#),
            "{args}"
        );
        assert!(
            args.contains(r#"{ output = "eq9_1:Out" input = "mix_1:In 1" }"#),
            "{args}"
        );
        assert!(
            args.contains(r#"inputs = [ "in_0:In" "in_1:In" ]"#),
            "{args}"
        );
        assert!(args.contains(r#"node.name = "otto-effect.test-fx" media.class = Audio/Sink"#));
        assert!(args.contains(r#"target.object = "dev""#));
        assert!(args.contains("audio.position = [ FL FR ]"));
    }

    #[test]
    fn on_and_off_swap_wet_and_dry() {
        let mut eq = effect(EffectKind::Equaliser, EffectSide::Output);
        eq.preamp = -6.0;
        let value = |eq: &Effect, name: &str| {
            controls(eq)
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v)
                .unwrap()
        };
        assert!((value(&eq, "mix_0:Gain 1") - 0.501).abs() < 0.001);
        assert_eq!(value(&eq, "mix_0:Gain 2"), 0.0);
        eq.enabled = false;
        assert_eq!(value(&eq, "mix_1:Gain 1"), 0.0);
        assert_eq!(value(&eq, "mix_1:Gain 2"), 1.0);
        // Unused slots pass sound unchanged.
        assert_eq!(value(&eq, "eq9_0:b0"), 1.0);
        assert_eq!(value(&eq, "eq9_0:a1"), 0.0);
    }

    #[test]
    fn a_stereo_plugin_takes_both_channels_and_a_mono_one_is_doubled() {
        let lim = effect(EffectKind::Limiter, EffectSide::Output);
        let args = args_with(&lim, Some((&LIMITER, Path::new("/x/limiter.so"))));
        assert_eq!(args.matches("type = ladspa").count(), 1);
        assert!(
            args.contains(r#"{ output = "in_1:Out" input = "fx:Input 2" }"#),
            "{args}"
        );
        assert!(
            args.contains(r#"{ output = "fx:Output 2" input = "mix_1:In 1" }"#),
            "{args}"
        );
        assert!(args.contains(r#""Limit (dB)" = -1"#), "{args}");

        let mut ns = effect(EffectKind::NoiseSuppression, EffectSide::Input);
        ns.side = EffectSide::Input;
        let args = args_with(&ns, Some((&NOISE, Path::new("/x/rnnoise.so"))));
        assert_eq!(args.matches("type = ladspa").count(), 1);
        assert!(args.contains("audio.position = [ MONO ]"), "{args}");
        assert!(args.contains(r#"node.name = "otto-effect.test-fx.rec" target.object = "dev""#));
        assert!(args.contains(r#"node.name = "otto-effect.test-fx" media.class = Audio/Source"#));

        let comp = effect(EffectKind::Compressor, EffectSide::Input);
        let args = args_with(&comp, Some((&COMPRESSOR, Path::new("/x/sc4.so"))));
        // Mono into a stereo plugin: both its inputs take the one channel.
        assert!(
            args.contains(r#"{ output = "in_0:Out" input = "fx:Right input" }"#),
            "{args}"
        );
    }

    #[test]
    fn a_missing_plugin_is_said_with_its_package() {
        let dirs = [PathBuf::from("/nonexistent")];
        assert_eq!(find(&NOISE, &dirs), None);
        assert_eq!(support(EffectKind::Equaliser), Support::Ready);
    }
}
