//! The mixer: the default output and input, each node's volume and mute,
//! and the streams playing sound, by app.
//!
//! The defaults come from the session manager's `default` metadata: it
//! writes the device in use to `default.audio.sink` and `.source`, and a
//! person's choice is written to `default.configured.audio.sink` and
//! `.source`, as `wpctl set-default` does.
//!
//! Volume and mute are read from each audio node's `Props` param. They are
//! changed where the session manager keeps them, as `wpctl` and the
//! PulseAudio server do: through the card's `Route` when the node belongs to
//! a card, so the card's mixer moves and the setting is saved with the
//! route, and through the node's own `Props` otherwise. Volumes are cubic on
//! the wire; [`Volume::linear`] is the fraction a slider shows.

// Rust guideline compliant 2026-02-21

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;

use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pw::spa::utils::SpaTypes;

use crate::connection::Connection;
use crate::model::{Event, Graph, Node};

/// The metadata key holding the output device in use.
const DEFAULT_SINK: &str = "default.audio.sink";
/// The metadata key holding the input device in use.
const DEFAULT_SOURCE: &str = "default.audio.source";
/// The metadata key a person's choice of output is written to.
const CONFIGURED_SINK: &str = "default.configured.audio.sink";
/// The metadata key a person's choice of input is written to.
const CONFIGURED_SOURCE: &str = "default.configured.audio.source";
/// The metadata key, on a stream, naming the node the session manager should
/// link it to, as `pactl move-sink-input` and `pw-metadata` set it.
const TARGET_OBJECT: &str = "target.object";

/// The loudest a slider goes: 100 %, the device's or the stream's own full
/// volume. Louder would amplify in software and clip.
const MAX_LINEAR: f32 = 1.0;

/// Output or input: which of the two defaults, or which kind of device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// Where sound plays: a sink.
    Output,
    /// Where sound is recorded from: a source.
    Input,
}

impl Endpoint {
    /// Whether `node` is a device or virtual device of this kind, as its
    /// `media.class` says (`Audio/Sink`, `Audio/Source`,
    /// `Audio/Source/Virtual`…).
    #[must_use]
    pub fn matches(self, node: &Node) -> bool {
        let class = node.media_class();
        match self {
            Self::Output => class.starts_with("Audio/Sink"),
            Self::Input => class.starts_with("Audio/Source"),
        }
    }

    fn configured_key(self) -> &'static str {
        match self {
            Self::Output => CONFIGURED_SINK,
            Self::Input => CONFIGURED_SOURCE,
        }
    }
}

/// A node's volume as the session manager keeps it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Volume {
    /// Each channel's volume, cubic, as on the wire: 1.0 is full volume.
    pub channels: Vec<f32>,
    pub mute: bool,
}

impl Volume {
    /// The volume a slider shows, 0.0 to 1.0: the cube root of the mean of
    /// the channels, as `wpctl` reports it.
    #[must_use]
    pub fn linear(&self) -> f32 {
        if self.channels.is_empty() {
            return 0.0;
        }
        let mean = self.channels.iter().sum::<f32>() / self.channels.len() as f32;
        to_linear(mean)
    }

    /// [`Self::linear`] as a whole percentage.
    #[must_use]
    pub fn percent(&self) -> u32 {
        (self.linear() * 100.0).round() as u32
    }

    /// The channels for a new linear volume, keeping their balance: each is
    /// scaled by the same factor, so a left-heavy stereo stays left-heavy.
    #[must_use]
    pub fn channels_at(&self, linear: f32) -> Vec<f32> {
        let target = to_cubic(linear.clamp(0.0, MAX_LINEAR));
        let count = self.channels.len().max(1);
        let mean = self.channels.iter().sum::<f32>() / count as f32;
        if mean <= f32::EPSILON || self.channels.is_empty() {
            return vec![target; count];
        }
        let factor = target / mean;
        self.channels.iter().map(|c| c * factor).collect()
    }
}

/// A cubic volume, as on the wire, as the linear fraction a slider shows.
#[must_use]
pub fn to_linear(cubic: f32) -> f32 {
    cubic.max(0.0).cbrt()
}

/// A linear fraction, as a slider shows it, as the cubic volume on the wire.
#[must_use]
pub fn to_cubic(linear: f32) -> f32 {
    let linear = linear.max(0.0);
    linear * linear * linear
}

/// The streams of one app that play sound, shown as one.
#[derive(Debug, Clone, PartialEq)]
pub struct AppStreams {
    /// The app's name, from `application.name`.
    pub app: String,
    /// Its playback streams' node ids, in id order.
    pub nodes: Vec<u32>,
    /// The first stream's volume; a change sets every stream to it.
    pub volume: Volume,
}

/// What the graph knows of the mixer: the defaults and the volumes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Mixer {
    default_output: Option<String>,
    default_input: Option<String>,
    volumes: HashMap<u32, Volume>,
}

impl Mixer {
    pub(crate) fn set_default(&mut self, endpoint: Endpoint, name: Option<String>) {
        match endpoint {
            Endpoint::Output => self.default_output = name,
            Endpoint::Input => self.default_input = name,
        }
    }

    pub(crate) fn set_volume(&mut self, node: u32, volume: Volume) {
        self.volumes.insert(node, volume);
    }

    pub(crate) fn remove(&mut self, id: u32) {
        self.volumes.remove(&id);
    }
}

impl Graph {
    /// The `node.name` of the default output or input, as the session
    /// manager last said.
    #[must_use]
    pub fn default_name(&self, endpoint: Endpoint) -> Option<&str> {
        match endpoint {
            Endpoint::Output => self.mixer.default_output.as_deref(),
            Endpoint::Input => self.mixer.default_input.as_deref(),
        }
    }

    /// The default output or input, while it is in the graph.
    #[must_use]
    pub fn default_node(&self, endpoint: Endpoint) -> Option<&Node> {
        let name = self.default_name(endpoint)?;
        self.nodes()
            .find(|n| endpoint.matches(n) && n.name() == name)
    }

    /// Every output or input to choose from, devices and virtual devices,
    /// by the name they show.
    #[must_use]
    pub fn endpoints(&self, endpoint: Endpoint) -> Vec<&Node> {
        let mut nodes: Vec<&Node> = self.nodes().filter(|n| endpoint.matches(n)).collect();
        nodes.sort_by(|a, b| a.title().cmp(b.title()).then(a.id.cmp(&b.id)));
        nodes
    }

    /// The volume of node `id`, once its `Props` have been read.
    #[must_use]
    pub fn volume(&self, id: u32) -> Option<&Volume> {
        self.mixer.volumes.get(&id)
    }

    /// The apps playing sound, each with its playback streams, by name.
    ///
    /// A stream counts when it is an audio playback stream with a volume
    /// read. Filters' own streams (the ones an effect or an echo canceller
    /// plays into a device with) and level monitors are left out: they are
    /// not an app.
    #[must_use]
    pub fn app_streams(&self) -> Vec<AppStreams> {
        let mut apps: Vec<AppStreams> = Vec::new();
        for node in self.nodes() {
            if !is_app_playback(node) {
                continue;
            }
            let Some(volume) = self.volume(node.id) else {
                continue;
            };
            let app = node.app().unwrap_or_else(|| node.title()).to_string();
            match apps.iter_mut().find(|a| a.app == app) {
                Some(entry) => entry.nodes.push(node.id),
                None => apps.push(AppStreams {
                    app,
                    nodes: vec![node.id],
                    volume: volume.clone(),
                }),
            }
        }
        apps.sort_by_key(|a| a.app.to_lowercase());
        apps
    }
}

/// Whether `node` is an app's own audio playback stream.
fn is_app_playback(node: &Node) -> bool {
    node.media_class() == "Stream/Output/Audio"
        && node.prop("node.link-group").is_none()
        && node.prop("stream.monitor") != Some("true")
}

/// `{ "name": "<name>" }`, the JSON the `default` metadata holds.
fn default_json(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            c if u32::from(c) < 0x20 => escaped.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => escaped.push(c),
        }
    }
    format!("{{ \"name\": \"{escaped}\" }}")
}

/// The `name` in a `default` metadata value, `{ "name": "<name>" }`.
///
/// Read by hand: the value is one flat object written by the session
/// manager, and a JSON parser would be a dependency for one field.
fn name_in_json(json: &str) -> Option<String> {
    let after_key = &json[json.find("\"name\"")? + "\"name\"".len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let mut chars = after_colon.strip_prefix('"')?.chars();
    let mut name = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(name),
            '\\' => match chars.next()? {
                'n' => name.push('\n'),
                't' => name.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    name.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => name.push(other),
            },
            c => name.push(c),
        }
    }
    None
}

impl Connection {
    /// Make `name` (a `node.name`) the default output or input, as a person's
    /// choice. The session manager moves the streams that follow the
    /// default onto it.
    pub fn set_default(&self, endpoint: Endpoint, name: &str) {
        self.send_mixer(Command::Default {
            endpoint,
            name: name.to_string(),
        });
    }

    /// Set node `node`'s volume to `linear`, 0.0 to 1.0, as a slider shows
    /// it. The channels keep their balance.
    pub fn set_volume(&self, node: u32, linear: f32) {
        self.send_mixer(Command::Volume { node, linear });
    }

    /// Mute or unmute node `node`.
    pub fn set_mute(&self, node: u32, mute: bool) {
        self.send_mixer(Command::Mute { node, mute });
    }

    /// Ask the session manager to move stream `stream` onto the node called
    /// `target` (a `node.name`), or to its own choice with `None`, by setting
    /// the stream's `target.object` in the `default` metadata.
    pub fn set_target(&self, stream: u32, target: Option<&str>) {
        self.send_mixer(Command::Target {
            stream,
            target: target.map(str::to_string),
        });
    }
}

/// What the mixer can be asked to do, on the connection's thread.
#[derive(Debug)]
pub(crate) enum Command {
    Default { endpoint: Endpoint, name: String },
    Volume { node: u32, linear: f32 },
    Mute { node: u32, mute: bool },
    Target { stream: u32, target: Option<String> },
}

/// A node as `connection.rs` keeps it bound: its info listener, the
/// mixer's listener when it is an audio node, and the proxy.
pub(crate) type BoundNode = (
    pw::node::NodeListener,
    Option<pw::node::NodeListener>,
    pw::node::Node,
);

/// The proxies and listeners the connection keeps, by global id.
pub(crate) type Bound = RefCell<HashMap<u32, Box<dyn Any>>>;

/// Where a card node's volume lives: its card and the card's device index
/// (`card.profile.device`) the routes are kept by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CardDevice {
    card: u32,
    device: i32,
}

/// What the mixer keeps on the connection's thread.
#[derive(Default)]
pub(crate) struct Thread {
    /// The `default` metadata, by its global id.
    default: RefCell<Option<(u32, pw::metadata::Metadata)>>,
    /// Each card device's active route index, from the cards' `Route`
    /// params.
    routes: RefCell<HashMap<CardDevice, i32>>,
    /// Each audio node's card device, for nodes that belong to a card.
    cards: RefCell<HashMap<u32, CardDevice>>,
    /// Each audio node's last volume, which a change scales from.
    volumes: RefCell<HashMap<u32, Volume>>,
}

impl Thread {
    /// Card `card`'s active routes, as `devices.rs` read them: the route
    /// index each of its sink or source devices plays or records through.
    pub(crate) fn set_routes(&self, card: u32, routes: impl IntoIterator<Item = (u32, u32)>) {
        let mut map = self.routes.borrow_mut();
        map.retain(|k, _| k.card != card);
        for (device, index) in routes {
            if let (Ok(device), Ok(index)) = (i32::try_from(device), i32::try_from(index)) {
                map.insert(CardDevice { card, device }, index);
            }
        }
    }

    pub(crate) fn clear(&self) {
        *self.default.borrow_mut() = None;
        self.routes.borrow_mut().clear();
        self.cards.borrow_mut().clear();
        self.volumes.borrow_mut().clear();
    }

    /// Global `id` is gone.
    pub(crate) fn forget(&self, id: u32, emit: &dyn Fn(Event)) {
        self.cards.borrow_mut().remove(&id);
        self.volumes.borrow_mut().remove(&id);
        self.routes.borrow_mut().retain(|k, _| k.card != id);
        let was_default = self
            .default
            .borrow()
            .as_ref()
            .is_some_and(|(d, _)| *d == id);
        if was_default {
            *self.default.borrow_mut() = None;
            for endpoint in [Endpoint::Output, Endpoint::Input] {
                emit(Event::Default {
                    endpoint,
                    name: None,
                });
            }
        }
    }

    /// Note which card device `node` belongs to, from its properties.
    fn note_card(&self, node: u32, props: &pw::spa::utils::dict::DictRef) {
        let card = props.get("device.id").and_then(|v| v.parse().ok());
        let device = props
            .get("card.profile.device")
            .and_then(|v| v.parse().ok());
        match (card, device) {
            (Some(card), Some(device)) => {
                self.cards
                    .borrow_mut()
                    .insert(node, CardDevice { card, device });
            }
            _ => {
                self.cards.borrow_mut().remove(&node);
            }
        }
    }
}

/// Follow an audio node's volume: its card device from its properties, and
/// its `Props` param. `None` for a node that is not audio.
pub(crate) fn watch_node(
    proxy: &pw::node::Node,
    id: u32,
    props: Option<&pw::spa::utils::dict::DictRef>,
    mixer: &Rc<Thread>,
    emit: &Rc<dyn Fn(Event)>,
) -> Option<pw::node::NodeListener> {
    let class = props.and_then(|p| p.get("media.class")).unwrap_or_default();
    if !class.contains("Audio") {
        return None;
    }
    if let Some(props) = props {
        mixer.note_card(id, props);
    }
    let listener = proxy
        .add_listener_local()
        .info({
            let mixer = Rc::downgrade(mixer);
            move |info| {
                let Some(mixer) = mixer.upgrade() else {
                    return;
                };
                if info.change_mask().contains(pw::node::NodeChangeMask::PROPS) {
                    if let Some(props) = info.props() {
                        mixer.note_card(id, props);
                    }
                }
            }
        })
        .param({
            let mixer = Rc::downgrade(mixer);
            let emit = Rc::downgrade(emit);
            move |_seq, kind, _index, _next, pod| {
                let (Some(mixer), Some(emit), Some(pod)) = (mixer.upgrade(), emit.upgrade(), pod)
                else {
                    return;
                };
                if kind != ParamType::Props {
                    return;
                }
                let mut volumes = mixer.volumes.borrow_mut();
                let previous = volumes.get(&id).cloned().unwrap_or_default();
                let Some(volume) = read_props(pod, &previous) else {
                    return;
                };
                if volumes.get(&id) == Some(&volume) {
                    return;
                }
                volumes.insert(id, volume.clone());
                drop(volumes);
                emit(Event::Volume { node: id, volume });
            }
        })
        .register();
    proxy.subscribe_params(&[ParamType::Props]);
    Some(listener)
}

/// Bind the `default` metadata, to follow the defaults and to set them.
pub(crate) fn bind_default(
    registry: &pw::registry::RegistryRc,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    bound: &Bound,
    mixer: &Rc<Thread>,
    emit: &Rc<dyn Fn(Event)>,
) {
    let id = global.id;
    let proxy: pw::metadata::Metadata = match registry.bind(global) {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::debug!(%error, id, "binding the default metadata failed");
            return;
        }
    };
    let listener = proxy
        .add_listener_local()
        .property({
            let emit = Rc::downgrade(emit);
            move |subject, key, _type, value| {
                let Some(emit) = emit.upgrade() else {
                    return 0;
                };
                if subject != 0 {
                    return 0;
                }
                let name = || value.and_then(name_in_json);
                match key {
                    Some(DEFAULT_SINK) => emit(Event::Default {
                        endpoint: Endpoint::Output,
                        name: name(),
                    }),
                    Some(DEFAULT_SOURCE) => emit(Event::Default {
                        endpoint: Endpoint::Input,
                        name: name(),
                    }),
                    // Everything was cleared.
                    None => {
                        for endpoint in [Endpoint::Output, Endpoint::Input] {
                            emit(Event::Default {
                                endpoint,
                                name: None,
                            });
                        }
                    }
                    _ => {}
                }
                0
            }
        })
        .register();
    *mixer.default.borrow_mut() = Some((id, proxy));
    bound.borrow_mut().insert(id, Box::new(listener));
}

/// Carry out `command` on the connection's thread.
pub(crate) fn run(bound: &Bound, mixer: &Thread, command: Command) {
    match command {
        Command::Default { endpoint, name } => match mixer.default.borrow().as_ref() {
            Some((_, metadata)) => metadata.set_property(
                0,
                endpoint.configured_key(),
                Some("Spa:String:JSON"),
                Some(&default_json(&name)),
            ),
            None => tracing::warn!(?endpoint, "no default metadata to set"),
        },
        Command::Volume { node, linear } => {
            let current = mixer.volumes.borrow().get(&node).cloned();
            let Some(current) = current else {
                tracing::debug!(node, "setting the volume of a node with none read");
                return;
            };
            let channels = Value::ValueArray(ValueArray::Float(current.channels_at(linear)));
            set_props(
                bound,
                mixer,
                node,
                Property::new(pw::spa::sys::SPA_PROP_channelVolumes, channels),
            );
        }
        Command::Target { stream, target } => match mixer.default.borrow().as_ref() {
            Some((_, metadata)) => {
                metadata.set_property(stream, TARGET_OBJECT, None, target.as_deref());
            }
            None => tracing::warn!(stream, "no default metadata to move a stream with"),
        },
        Command::Mute { node, mute } => set_props(
            bound,
            mixer,
            node,
            Property::new(pw::spa::sys::SPA_PROP_mute, Value::Bool(mute)),
        ),
    }
}

/// Set one property of node `node`'s volume: on its card's route when it
/// belongs to a card, on the node otherwise.
fn set_props(bound: &Bound, mixer: &Thread, node: u32, property: Property) {
    let props = Value::Object(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Route.as_raw(),
        properties: vec![property.clone()],
    });
    let card = mixer.cards.borrow().get(&node).copied();
    let route = card.and_then(|c| mixer.routes.borrow().get(&c).copied());
    let bound = bound.borrow();
    if let (Some(card), Some(index)) = (card, route) {
        let Some(device) = bound
            .get(&card.card)
            .and_then(|b| crate::devices::proxy_in(b.as_ref()))
        else {
            tracing::debug!(node, card = card.card, "the card is not bound");
            return;
        };
        let route = Value::Object(Object {
            type_: SpaTypes::ObjectParamRoute.as_raw(),
            id: ParamType::Route.as_raw(),
            properties: vec![
                Property::new(pw::spa::sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
                Property::new(
                    pw::spa::sys::SPA_PARAM_ROUTE_device,
                    Value::Int(card.device),
                ),
                Property::new(pw::spa::sys::SPA_PARAM_ROUTE_props, props),
                Property::new(pw::spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
            ],
        });
        if let Some(bytes) = serialize(&route) {
            if let Some(pod) = Pod::from_bytes(&bytes) {
                device.set_param(ParamType::Route, 0, pod);
            }
        }
        return;
    }
    let Some((_, _, proxy)) = bound.get(&node).and_then(|b| b.downcast_ref::<BoundNode>()) else {
        tracing::debug!(node, "setting the volume of a node that is not bound");
        return;
    };
    let props = Value::Object(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Props.as_raw(),
        properties: vec![property],
    });
    if let Some(bytes) = serialize(&props) {
        if let Some(pod) = Pod::from_bytes(&bytes) {
            proxy.set_param(ParamType::Props, 0, pod);
        }
    }
}

fn serialize(value: &Value) -> Option<Vec<u8>> {
    match PodSerializer::serialize(Cursor::new(Vec::new()), value) {
        Ok((cursor, _)) => Some(cursor.into_inner()),
        Err(error) => {
            tracing::warn!(?error, "building a param failed");
            None
        }
    }
}

/// The volume in a `Props` param, on top of `previous`: a node reports more
/// than one `Props` object, and only one carries the volume.
fn read_props(pod: &Pod, previous: &Volume) -> Option<Volume> {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return None;
    };
    volume_from(&object.properties, previous)
}

/// The volume in a list of `Props` properties, on top of `previous`; `None`
/// when they carry neither volumes nor a mute.
fn volume_from(properties: &[Property], previous: &Volume) -> Option<Volume> {
    let mut volume = previous.clone();
    let mut found = false;
    for property in properties {
        match (property.key, &property.value) {
            (pw::spa::sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(v))) => {
                volume.channels.clone_from(v);
                found = true;
            }
            (pw::spa::sys::SPA_PROP_mute, Value::Bool(mute)) => {
                volume.mute = *mute;
                found = true;
            }
            _ => {}
        }
    }
    found.then_some(volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeState;

    fn node(id: u32, props: &[(&str, &str)]) -> Node {
        Node {
            id,
            props: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            state: NodeState::Running,
        }
    }

    fn volume(linear: f32) -> Volume {
        Volume {
            channels: vec![to_cubic(linear); 2],
            mute: false,
        }
    }

    #[test]
    fn a_cubic_volume_reads_as_wpctl_shows_it() {
        let volume = Volume {
            channels: vec![0.091_125, 0.091_125],
            mute: false,
        };
        assert_eq!(volume.percent(), 45);
        assert!((to_cubic(0.45) - 0.091_125).abs() < 1e-6);
    }

    #[test]
    fn a_new_volume_keeps_the_balance() {
        let volume = Volume {
            channels: vec![0.2, 0.1],
            mute: false,
        };
        let channels = volume.channels_at(0.5);
        assert!((channels[0] / channels[1] - 2.0).abs() < 1e-4);
        let mean = (channels[0] + channels[1]) / 2.0;
        assert!((to_linear(mean) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_silent_node_comes_up_level_and_volumes_stop_at_full() {
        let silent = Volume {
            channels: vec![0.0, 0.0],
            mute: false,
        };
        assert_eq!(silent.channels_at(0.5), vec![0.125, 0.125]);
        assert_eq!(volume(0.5).channels_at(1.7), vec![1.0, 1.0]);
    }

    #[test]
    fn the_default_metadata_round_trips_a_name() {
        let json = default_json("alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(
            json,
            r#"{ "name": "alsa_output.pci-0000_00_1f.3.analog-stereo" }"#
        );
        assert_eq!(
            name_in_json(&json).as_deref(),
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo")
        );
        let odd = "a \"quoted\" \\ name";
        assert_eq!(name_in_json(&default_json(odd)).as_deref(), Some(odd));
        assert_eq!(name_in_json(r#"{"name":"x"}"#).as_deref(), Some("x"));
        assert_eq!(name_in_json("{}"), None);
    }

    #[test]
    fn props_without_a_volume_leave_it_alone() {
        let previous = volume(0.5);
        let unrelated = [Property::new(
            pw::spa::sys::SPA_PROP_volume,
            Value::Float(1.0),
        )];
        assert_eq!(volume_from(&unrelated, &previous), None);
        let muted = [Property::new(
            pw::spa::sys::SPA_PROP_mute,
            Value::Bool(true),
        )];
        let read = volume_from(&muted, &previous).expect("a mute");
        assert!(read.mute);
        assert_eq!(read.channels, previous.channels);
    }

    #[test]
    fn a_props_pod_reads_back() {
        let pod = Value::Object(Object {
            type_: SpaTypes::ObjectParamProps.as_raw(),
            id: ParamType::Props.as_raw(),
            properties: vec![
                Property::new(
                    pw::spa::sys::SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(vec![0.125, 0.125])),
                ),
                Property::new(pw::spa::sys::SPA_PROP_mute, Value::Bool(true)),
            ],
        });
        let bytes = serialize(&pod).expect("serialized");
        let read = read_props(Pod::from_bytes(&bytes).expect("a pod"), &Volume::default())
            .expect("a volume");
        assert_eq!(read.percent(), 50);
        assert!(read.mute);
    }

    #[test]
    fn the_defaults_name_nodes_of_their_own_kind() {
        let mut graph = Graph::new();
        graph.apply(Event::Node(node(
            1,
            &[("media.class", "Audio/Sink"), ("node.name", "speakers")],
        )));
        graph.apply(Event::Node(node(
            2,
            &[
                ("media.class", "Audio/Source/Virtual"),
                ("node.name", "mic-cleanup"),
            ],
        )));
        graph.apply(Event::Default {
            endpoint: Endpoint::Output,
            name: Some("speakers".into()),
        });
        graph.apply(Event::Default {
            endpoint: Endpoint::Input,
            name: Some("speakers".into()),
        });
        assert_eq!(graph.default_node(Endpoint::Output).map(|n| n.id), Some(1));
        assert_eq!(graph.default_node(Endpoint::Input), None);
        let inputs: Vec<u32> = graph
            .endpoints(Endpoint::Input)
            .iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(inputs, vec![2]);
    }

    #[test]
    fn streams_group_by_app_and_filters_are_left_out() {
        let mut graph = Graph::new();
        let streams = [
            (10, "Firefox", None),
            (11, "Firefox", None),
            (12, "Spotify", None),
            (13, "echo-cancel", Some("echo-cancel-1")),
        ];
        for (id, app, group) in streams {
            let mut props = vec![
                ("media.class", "Stream/Output/Audio"),
                ("application.name", app),
            ];
            if let Some(group) = group {
                props.push(("node.link-group", group));
            }
            graph.apply(Event::Node(node(id, &props)));
            graph.apply(Event::Volume {
                node: id,
                volume: volume(0.5),
            });
        }
        let apps = graph.app_streams();
        let names: Vec<&str> = apps.iter().map(|a| a.app.as_str()).collect();
        assert_eq!(names, vec!["Firefox", "Spotify"]);
        assert_eq!(apps[0].nodes, vec![10, 11]);

        graph.apply(Event::Removed(12));
        assert_eq!(graph.app_streams().len(), 1);
        assert_eq!(graph.volume(12), None);
    }
}
