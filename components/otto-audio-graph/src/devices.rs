//! Devices: the cards behind the nodes, with their profiles and routes.
//!
//! A PipeWire Device is a sound card, a Bluetooth device or a camera. Its
//! `EnumProfile` param lists the profiles it can run (Analog Stereo Duplex,
//! Pro Audio, High Fidelity Playback with one codec or another), `Profile`
//! the one it runs now, `EnumRoute` its ports (speakers, headphone jack,
//! microphones) and `Route` the port each of its sinks and sources uses now.
//! A Bluetooth device lists one profile per codec, so a codec switch is a
//! profile switch.
//!
//! The connection's thread binds each device, enumerates those four params
//! again whenever the device says one changed, and reports the whole device
//! as an [`Event::Device`] once a core sync says every result is in, so a
//! reader never sees half a list.

// Rust guideline compliant 2026-02-21

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::rc::Rc;

use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pw::spa::utils::result::AsyncSeq;
use pw::spa::utils::{Id, SpaTypes};

use crate::connection::{props_map, State};
use crate::model::{Direction, Event};

/// Keys of a `SPA_TYPE_OBJECT_ParamProfile` object (`spa/param/profile.h`).
const PROFILE_INDEX: u32 = pw::spa::sys::SPA_PARAM_PROFILE_index;
const PROFILE_NAME: u32 = pw::spa::sys::SPA_PARAM_PROFILE_name;
const PROFILE_DESCRIPTION: u32 = pw::spa::sys::SPA_PARAM_PROFILE_description;
const PROFILE_PRIORITY: u32 = pw::spa::sys::SPA_PARAM_PROFILE_priority;
const PROFILE_AVAILABLE: u32 = pw::spa::sys::SPA_PARAM_PROFILE_available;
const PROFILE_SAVE: u32 = pw::spa::sys::SPA_PARAM_PROFILE_save;

/// Keys of a `SPA_TYPE_OBJECT_ParamRoute` object (`spa/param/route.h`).
const ROUTE_INDEX: u32 = pw::spa::sys::SPA_PARAM_ROUTE_index;
const ROUTE_DIRECTION: u32 = pw::spa::sys::SPA_PARAM_ROUTE_direction;
const ROUTE_DEVICE: u32 = pw::spa::sys::SPA_PARAM_ROUTE_device;
const ROUTE_NAME: u32 = pw::spa::sys::SPA_PARAM_ROUTE_name;
const ROUTE_DESCRIPTION: u32 = pw::spa::sys::SPA_PARAM_ROUTE_description;
const ROUTE_PRIORITY: u32 = pw::spa::sys::SPA_PARAM_ROUTE_priority;
const ROUTE_AVAILABLE: u32 = pw::spa::sys::SPA_PARAM_ROUTE_available;
const ROUTE_INFO: u32 = pw::spa::sys::SPA_PARAM_ROUTE_info;
const ROUTE_PROFILES: u32 = pw::spa::sys::SPA_PARAM_ROUTE_profiles;
const ROUTE_DEVICES: u32 = pw::spa::sys::SPA_PARAM_ROUTE_devices;
const ROUTE_SAVE: u32 = pw::spa::sys::SPA_PARAM_ROUTE_save;

/// The params followed, in the order they are enumerated.
const FOLLOWED: [ParamType; 4] = [
    ParamType::EnumProfile,
    ParamType::Profile,
    ParamType::EnumRoute,
    ParamType::Route,
];

/// Whether a profile or a port can be used now, as the device says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Availability {
    /// The device cannot tell, as with built-in speakers.
    #[default]
    Unknown,
    /// Not now: a jack with nothing plugged in, an HDMI port with no display.
    No,
    Yes,
}

impl Availability {
    fn from_raw(raw: u32) -> Self {
        match raw {
            pw::spa::sys::SPA_PARAM_AVAILABILITY_no => Self::No,
            pw::spa::sys::SPA_PARAM_AVAILABILITY_yes => Self::Yes,
            _ => Self::Unknown,
        }
    }
}

/// One profile a device can run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    /// What [`Connection::set_profile`](crate::Connection::set_profile) takes.
    pub index: u32,
    /// The stable name, such as `output:analog-stereo` or `a2dp-sink-ldac`.
    pub name: String,
    /// The name to show, translated by the server.
    pub description: String,
    pub available: Availability,
    /// The higher, the more the session manager prefers it.
    pub priority: u32,
}

impl Profile {
    /// Whether this is the profile that turns the device off.
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.name == "off"
    }

    /// The Bluetooth codec this profile names, such as `ldac` in
    /// `a2dp-sink-ldac`, with the rest of the name: the profile's family.
    #[must_use]
    pub fn codec(&self) -> Option<(&str, &'static Codec)> {
        CODECS.iter().find_map(|codec| {
            let family = self.name.strip_suffix(codec.name)?.strip_suffix('-')?;
            Some((family, codec))
        })
    }

    /// The family a Bluetooth profile belongs to: its name without the
    /// codec, or the whole name when it names none.
    #[must_use]
    pub fn family(&self) -> &str {
        self.codec().map_or(&self.name, |(family, _)| family)
    }

    /// Whether this is a Bluetooth headset profile (HSP/HFP): the
    /// microphone works, the sound is call quality.
    #[must_use]
    pub fn is_headset(&self) -> bool {
        let family = self.family();
        family.starts_with("headset-head-unit")
            || family.starts_with("hfp")
            || family.starts_with("hsp")
    }

    /// Whether this is a Bluetooth playback profile (A2DP or LE audio
    /// playback): the best sound, with no microphone.
    #[must_use]
    pub fn is_hifi(&self) -> bool {
        let family = self.family();
        family.starts_with("a2dp") || family.starts_with("bap-sink")
    }
}

impl Device {
    /// The usable profile of the kind `pick` accepts that the device
    /// prefers: the one running when it is of that kind, else the one with
    /// the highest priority.
    #[must_use]
    pub fn preferred_profile(&self, pick: impl Fn(&Profile) -> bool) -> Option<&Profile> {
        let running = self
            .profile
            .as_ref()
            .filter(|p| pick(p))
            .and_then(|a| self.profiles.iter().find(|p| p.index == a.index));
        running.or_else(|| {
            self.profiles
                .iter()
                .filter(|p| pick(p) && p.available != Availability::No)
                .max_by_key(|p| p.priority)
        })
    }
}

/// One port of a device: speakers, a headphone jack, a microphone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Route {
    /// What [`Connection::set_route`](crate::Connection::set_route) takes.
    pub index: u32,
    /// Whether it plays (`Output`) or records (`Input`).
    pub direction: Option<Direction>,
    /// The stable name, such as `analog-output-headphones`.
    pub name: String,
    /// The name to show, translated by the server.
    pub description: String,
    pub priority: u32,
    pub available: Availability,
    /// What the device says of it: `port.type`, `route.hw-volume`…
    pub info: BTreeMap<String, String>,
    /// The profiles it can be used with, by index.
    pub profiles: Vec<u32>,
    /// The device's sinks and sources it can serve, by the device's own
    /// numbering (`card.profile.device`).
    pub devices: Vec<u32>,
    /// For a port in use, the sink or source it serves now.
    pub device: Option<u32>,
}

impl Route {
    /// Whether the volume is set on the hardware's own mixer, as the active
    /// route says; `None` when it does not say.
    #[must_use]
    pub fn hardware_volume(&self) -> Option<bool> {
        self.info.get("route.hw-volume").map(|v| v == "true")
    }
}

/// A sound card, Bluetooth device or camera, as last reported.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Device {
    /// The global id, which changes every time the device appears.
    pub id: u32,
    /// Every property PipeWire reports for it.
    pub props: BTreeMap<String, String>,
    /// Every profile it lists, in its order.
    pub profiles: Vec<Profile>,
    /// The profile it runs now.
    pub profile: Option<Profile>,
    /// Every port it lists, in its order.
    pub routes: Vec<Route>,
    /// The port each of its sinks and sources uses now.
    pub active_routes: Vec<Route>,
}

impl Device {
    /// A property, if set and not empty.
    #[must_use]
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The name to show: the description, else the nick, else `device.name`.
    #[must_use]
    pub fn title(&self) -> &str {
        self.prop("device.description")
            .or_else(|| self.prop("device.nick"))
            .or_else(|| self.prop("device.name"))
            .unwrap_or_default()
    }

    /// `media.class`, such as `Audio/Device` or `Video/Device`.
    #[must_use]
    pub fn media_class(&self) -> &str {
        self.prop("media.class").unwrap_or_default()
    }

    /// Whether it is a sound device rather than a camera.
    #[must_use]
    pub fn is_audio(&self) -> bool {
        self.media_class() == "Audio/Device"
    }

    /// Whether BlueZ is behind it.
    #[must_use]
    pub fn is_bluetooth(&self) -> bool {
        self.prop("device.api") == Some("bluez5")
    }

    /// The ports `active` can switch between: those serving the same sink or
    /// source, going the same way, under the profile running now.
    #[must_use]
    pub fn routes_for(&self, active: &Route) -> Vec<&Route> {
        let (Some(device), Some(profile)) = (active.device, self.profile.as_ref()) else {
            return Vec::new();
        };
        self.routes
            .iter()
            .filter(|r| {
                r.direction == active.direction
                    && r.devices.contains(&device)
                    && r.profiles.contains(&profile.index)
            })
            .collect()
    }
}

/// A Bluetooth codec as BlueZ's media codecs name it.
#[derive(Debug, PartialEq, Eq)]
pub struct Codec {
    /// The name in a profile's name, such as `aptx_hd`.
    pub name: &'static str,
    /// How people know it, such as `aptX HD`.
    pub label: &'static str,
}

/// Every codec PipeWire's BlueZ plugin can name in a profile, longest names
/// first so `sbc_xq` is not read as `sbc` with something before it. The
/// labels are the codecs' own names, which are not translated.
const CODECS: [Codec; 22] = [
    Codec {
        name: "faststream_duplex",
        label: "FastStream duplex",
    },
    Codec {
        name: "aptx_ll_duplex",
        label: "aptX LL duplex",
    },
    Codec {
        name: "opus_05_duplex",
        label: "Opus duplex",
    },
    Codec {
        name: "lc3plus_hr",
        label: "LC3plus HR",
    },
    Codec {
        name: "faststream",
        label: "FastStream",
    },
    Codec {
        name: "opus_05_51",
        label: "Opus 5.1",
    },
    Codec {
        name: "opus_05_71",
        label: "Opus 7.1",
    },
    Codec {
        name: "opus_05_pro",
        label: "Opus Pro",
    },
    Codec {
        name: "aac_eld",
        label: "AAC-ELD",
    },
    Codec {
        name: "aptx_hd",
        label: "aptX HD",
    },
    Codec {
        name: "aptx_ll",
        label: "aptX LL",
    },
    Codec {
        name: "lc3_swb",
        label: "LC3-SWB",
    },
    Codec {
        name: "opus_05",
        label: "Opus",
    },
    Codec {
        name: "opus_g",
        label: "Opus G",
    },
    Codec {
        name: "sbc_xq",
        label: "SBC-XQ",
    },
    Codec {
        name: "aptx",
        label: "aptX",
    },
    Codec {
        name: "cvsd",
        label: "CVSD",
    },
    Codec {
        name: "ldac",
        label: "LDAC",
    },
    Codec {
        name: "msbc",
        label: "mSBC",
    },
    Codec {
        name: "aac",
        label: "AAC",
    },
    Codec {
        name: "lc3",
        label: "LC3",
    },
    Codec {
        name: "sbc",
        label: "SBC",
    },
];

/// The codec BlueZ names `name`, as a node's `api.bluez5.codec` does.
#[must_use]
pub fn codec_named(name: &str) -> Option<&'static Codec> {
    CODECS.iter().find(|c| c.name == name)
}

/// A device param to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceCommand {
    Profile {
        device: u32,
        index: u32,
    },
    Route {
        device: u32,
        index: u32,
        card_device: u32,
    },
}

impl crate::Connection {
    /// Switch device `device` to the profile at `index`. The server keeps
    /// the choice, so it holds after a restart.
    pub fn set_profile(&self, device: u32, index: u32) {
        self.send(crate::connection::Command::Device(DeviceCommand::Profile {
            device,
            index,
        }));
    }

    /// Point the device's sink or source `card_device` at the port at
    /// `index`. The server keeps the choice.
    pub fn set_route(&self, device: u32, index: u32, card_device: u32) {
        self.send(crate::connection::Command::Device(DeviceCommand::Route {
            device,
            index,
            card_device,
        }));
    }
}

/// What the thread keeps for one bound device; dropping it unbinds it.
pub(crate) struct Bound {
    proxy: Rc<pw::device::Device>,
    _listener: pw::device::DeviceListener,
    _core_listener: pw::core::Listener,
}

/// One enumeration of a param in flight.
struct Pending {
    /// The sequence number the results carry. The protocol numbers each
    /// request itself, so this is learnt from the first result; a result
    /// with another number belongs to a later enumeration of the same param,
    /// whose results always arrive after the earlier one's.
    seq: Option<i32>,
    /// The core sync after which every result is in.
    sync: Option<AsyncSeq>,
    values: Vec<Object>,
}

/// A device as it is being followed.
#[derive(Default)]
struct Tracker {
    device: Device,
    /// The flags each followed param was last enumerated at; the server
    /// flips one of them when the param changes.
    flags: HashMap<u32, u32>,
    pending: HashMap<u32, Pending>,
    /// Whether the device has been reported once.
    reported: bool,
}

/// The device proxy kept in `bound` for a device, for setting its params.
pub(crate) fn proxy_in(bound: &dyn std::any::Any) -> Option<&pw::device::Device> {
    bound
        .downcast_ref::<(Bound, Rc<RefCell<Tracker>>)>()
        .map(|(bound, _)| bound.proxy.as_ref())
}

/// Bind the device `global` and follow its profiles and routes.
pub(crate) fn bind(
    core: &pw::core::CoreRc,
    registry: &pw::registry::RegistryRc,
    state: &Rc<State>,
    emit: &Rc<dyn Fn(Event)>,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) {
    let id = global.id;
    let proxy: pw::device::Device = match registry.bind(global) {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::debug!(%error, id, "binding a device failed");
            return;
        }
    };
    let proxy = Rc::new(proxy);
    let tracker = Rc::new(RefCell::new(Tracker {
        device: Device {
            id,
            props: props_map(global.props),
            ..Device::default()
        },
        ..Tracker::default()
    }));

    let listener = proxy
        .add_listener_local()
        .info({
            let proxy = Rc::downgrade(&proxy);
            let core = core.downgrade();
            let tracker = Rc::downgrade(&tracker);
            let emit = Rc::downgrade(emit);
            move |info| {
                let (Some(proxy), Some(core), Some(tracker), Some(emit)) = (
                    proxy.upgrade(),
                    core.upgrade(),
                    tracker.upgrade(),
                    emit.upgrade(),
                ) else {
                    return;
                };
                on_info(&proxy, &core, &tracker, &emit, info);
            }
        })
        .param({
            let tracker = Rc::downgrade(&tracker);
            move |seq, id, _index, _next, param| {
                if let (Some(tracker), Some(param)) = (tracker.upgrade(), param) {
                    on_param(&tracker, seq, id, param);
                }
            }
        })
        .register();

    let core_listener = core
        .add_listener_local()
        .done({
            let tracker = Rc::downgrade(&tracker);
            let emit = Rc::downgrade(emit);
            let mixer = Rc::downgrade(&state.mixer);
            move |id, seq| {
                if id != pw::core::PW_ID_CORE {
                    return;
                }
                if let (Some(tracker), Some(emit)) = (tracker.upgrade(), emit.upgrade()) {
                    on_done(&tracker, &emit, seq);
                    // The mixer sets a card's volume through its active
                    // route, so it follows the routes read here.
                    if let Some(mixer) = mixer.upgrade() {
                        let t = tracker.borrow();
                        let routes = t
                            .device
                            .active_routes
                            .iter()
                            .filter_map(|r| r.device.map(|device| (device, r.index)));
                        mixer.set_routes(t.device.id, routes);
                    }
                }
            }
        })
        .register();

    // The listeners hold the tracker only weakly; it lives with them.
    let bound = Bound {
        proxy,
        _listener: listener,
        _core_listener: core_listener,
    };
    state
        .bound
        .borrow_mut()
        .insert(id, Box::new((bound, tracker)));
}

fn on_info(
    proxy: &pw::device::Device,
    core: &pw::core::CoreRc,
    tracker: &Rc<RefCell<Tracker>>,
    emit: &Rc<dyn Fn(Event)>,
    info: &pw::device::DeviceInfoRef,
) {
    let mut t = tracker.borrow_mut();
    let mask = info.change_mask();
    if mask.contains(pw::device::DeviceChangeMask::PROPS) {
        if let Some(props) = info.props() {
            t.device.props = props_map(Some(props));
        }
    }
    let mut started = Vec::new();
    if mask.contains(pw::device::DeviceChangeMask::PARAMS) {
        for param in info.params() {
            let id = param.id();
            if !FOLLOWED.contains(&id)
                || !param.flags().contains(pw::spa::param::ParamInfoFlags::READ)
            {
                continue;
            }
            let raw = id.as_raw();
            let flags = param.flags().bits();
            if t.flags.get(&raw) == Some(&flags) {
                continue;
            }
            t.flags.insert(raw, flags);
            proxy.enum_params(0, Some(id), 0, u32::MAX);
            t.pending.insert(
                raw,
                Pending {
                    seq: None,
                    sync: None,
                    values: Vec::new(),
                },
            );
            started.push(raw);
        }
    }
    if started.is_empty() {
        // Nothing to wait for: a camera, or only the properties changed.
        let device = t.device.clone();
        t.reported = true;
        drop(t);
        emit(Event::Device(device));
        return;
    }
    match core.sync(0) {
        Ok(sync) => {
            for raw in started {
                if let Some(pending) = t.pending.get_mut(&raw) {
                    pending.sync = Some(sync);
                }
            }
        }
        Err(error) => tracing::debug!(%error, "syncing after a device enumeration failed"),
    }
}

fn on_param(tracker: &RefCell<Tracker>, seq: i32, id: ParamType, param: &Pod) {
    let mut t = tracker.borrow_mut();
    let Some(pending) = t.pending.get_mut(&id.as_raw()) else {
        return;
    };
    if pending.seq != Some(seq) {
        pending.seq = Some(seq);
        pending.values.clear();
    }
    match PodDeserializer::deserialize_any_from(param.as_bytes()) {
        Ok((_, Value::Object(object))) => pending.values.push(object),
        Ok(_) => {}
        Err(_) => tracing::debug!(?id, "a device param could not be read"),
    }
}

fn on_done(tracker: &RefCell<Tracker>, emit: &Rc<dyn Fn(Event)>, seq: AsyncSeq) {
    let mut t = tracker.borrow_mut();
    let done: Vec<u32> = t
        .pending
        .iter()
        .filter(|(_, p)| p.sync == Some(seq))
        .map(|(id, _)| *id)
        .collect();
    if done.is_empty() {
        return;
    }
    for raw in done {
        let Some(pending) = t.pending.remove(&raw) else {
            continue;
        };
        let device = &mut t.device;
        match ParamType::from_raw(raw) {
            ParamType::EnumProfile => {
                device.profiles = pending.values.iter().filter_map(profile_from).collect();
            }
            ParamType::Profile => {
                device.profile = pending.values.iter().find_map(profile_from);
            }
            ParamType::EnumRoute => {
                device.routes = pending.values.iter().filter_map(route_from).collect();
            }
            ParamType::Route => {
                device.active_routes = pending.values.iter().filter_map(route_from).collect();
            }
            _ => {}
        }
    }
    // Wait until every enumeration started is in, so a reader never sees
    // the profile list without the active profile.
    if !t.pending.is_empty() && !t.reported {
        return;
    }
    t.reported = true;
    let device = t.device.clone();
    drop(t);
    emit(Event::Device(device));
}

/// Set a param on the device a command names, if it is still bound.
pub(crate) fn run(state: &State, command: DeviceCommand) {
    let (device, param, pod) = match command {
        DeviceCommand::Profile { device, index } => {
            (device, ParamType::Profile, profile_pod(index))
        }
        DeviceCommand::Route {
            device,
            index,
            card_device,
        } => (device, ParamType::Route, route_pod(index, card_device)),
    };
    let bound = state.bound.borrow();
    let Some((bound, _)) = bound
        .get(&device)
        .and_then(|b| b.downcast_ref::<(Bound, Rc<RefCell<Tracker>>)>())
    else {
        tracing::debug!(device, "setting a param on a device that is gone");
        return;
    };
    match pod {
        Some(bytes) => match Pod::from_bytes(&bytes) {
            Some(pod) => bound.proxy.set_param(param, 0, pod),
            None => tracing::warn!(device, "a device param did not serialise"),
        },
        None => tracing::warn!(device, "a device param did not serialise"),
    }
}

/// The `Profile` param that switches to profile `index`, kept by the server.
fn profile_pod(index: u32) -> Option<Vec<u8>> {
    serialize(Object {
        type_: SpaTypes::ObjectParamProfile.as_raw(),
        id: ParamType::Profile.as_raw(),
        properties: vec![
            Property::new(PROFILE_INDEX, Value::Int(as_int(index))),
            Property::new(PROFILE_SAVE, Value::Bool(true)),
        ],
    })
}

/// The `Route` param that points sink or source `card_device` at port
/// `index`, kept by the server.
fn route_pod(index: u32, card_device: u32) -> Option<Vec<u8>> {
    serialize(Object {
        type_: SpaTypes::ObjectParamRoute.as_raw(),
        id: ParamType::Route.as_raw(),
        properties: vec![
            Property::new(ROUTE_INDEX, Value::Int(as_int(index))),
            Property::new(ROUTE_DEVICE, Value::Int(as_int(card_device))),
            Property::new(ROUTE_SAVE, Value::Bool(true)),
        ],
    })
}

fn as_int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn serialize(object: Object) -> Option<Vec<u8>> {
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .ok()
        .map(|(cursor, _)| cursor.into_inner())
}

fn int(value: &Value) -> Option<u32> {
    match value {
        Value::Int(v) => u32::try_from(*v).ok(),
        Value::Id(Id(v)) => Some(*v),
        _ => None,
    }
}

fn string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn ints(value: &Value) -> Vec<u32> {
    match value {
        Value::ValueArray(ValueArray::Int(values)) => values
            .iter()
            .filter_map(|v| u32::try_from(*v).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// A route's `info`: a struct of a count, then that many key and value
/// strings.
fn info_dict(value: &Value) -> BTreeMap<String, String> {
    let Value::Struct(fields) = value else {
        return BTreeMap::new();
    };
    let strings: Vec<String> = fields.iter().skip(1).filter_map(string).collect();
    strings
        .chunks_exact(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

fn profile_from(object: &Object) -> Option<Profile> {
    let mut profile = Profile::default();
    let mut has_index = false;
    for property in &object.properties {
        let value = &property.value;
        match property.key {
            PROFILE_INDEX => {
                profile.index = int(value)?;
                has_index = true;
            }
            PROFILE_NAME => profile.name = string(value).unwrap_or_default(),
            PROFILE_DESCRIPTION => profile.description = string(value).unwrap_or_default(),
            PROFILE_PRIORITY => profile.priority = int(value).unwrap_or_default(),
            PROFILE_AVAILABLE => {
                profile.available = int(value).map(Availability::from_raw).unwrap_or_default();
            }
            _ => {}
        }
    }
    has_index.then_some(profile)
}

fn route_from(object: &Object) -> Option<Route> {
    let mut route = Route::default();
    let mut has_index = false;
    for property in &object.properties {
        let value = &property.value;
        match property.key {
            ROUTE_INDEX => {
                route.index = int(value)?;
                has_index = true;
            }
            ROUTE_DIRECTION => {
                route.direction = match int(value) {
                    Some(pw::spa::sys::SPA_DIRECTION_INPUT) => Some(Direction::Input),
                    Some(pw::spa::sys::SPA_DIRECTION_OUTPUT) => Some(Direction::Output),
                    _ => None,
                };
            }
            ROUTE_DEVICE => route.device = int(value),
            ROUTE_NAME => route.name = string(value).unwrap_or_default(),
            ROUTE_DESCRIPTION => route.description = string(value).unwrap_or_default(),
            ROUTE_PRIORITY => route.priority = int(value).unwrap_or_default(),
            ROUTE_AVAILABLE => {
                route.available = int(value).map(Availability::from_raw).unwrap_or_default();
            }
            ROUTE_INFO => route.info = info_dict(value),
            ROUTE_PROFILES => route.profiles = ints(value),
            ROUTE_DEVICES => route.devices = ints(value),
            _ => {}
        }
    }
    has_index.then_some(route)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> Object {
        let pod = Pod::from_bytes(bytes).expect("a whole pod");
        match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
            Ok((_, Value::Object(object))) => object,
            other => panic!("not an object: {other:?}"),
        }
    }

    fn value(object: &Object, key: u32) -> Option<&Value> {
        object
            .properties
            .iter()
            .find(|p| p.key == key)
            .map(|p| &p.value)
    }

    #[test]
    fn a_profile_switch_names_the_index_and_is_kept() {
        let object = read(&profile_pod(3).expect("serialises"));
        assert_eq!(object.type_, SpaTypes::ObjectParamProfile.as_raw());
        assert_eq!(object.id, ParamType::Profile.as_raw());
        assert_eq!(value(&object, PROFILE_INDEX), Some(&Value::Int(3)));
        assert_eq!(value(&object, PROFILE_SAVE), Some(&Value::Bool(true)));
    }

    #[test]
    fn a_route_switch_names_the_port_and_the_sink_it_serves() {
        let object = read(&route_pod(3, 7).expect("serialises"));
        assert_eq!(object.type_, SpaTypes::ObjectParamRoute.as_raw());
        assert_eq!(object.id, ParamType::Route.as_raw());
        assert_eq!(value(&object, ROUTE_INDEX), Some(&Value::Int(3)));
        assert_eq!(value(&object, ROUTE_DEVICE), Some(&Value::Int(7)));
        assert_eq!(value(&object, ROUTE_SAVE), Some(&Value::Bool(true)));
    }

    #[test]
    fn a_route_reads_back_with_its_info_and_lists() {
        let object = Object {
            type_: SpaTypes::ObjectParamRoute.as_raw(),
            id: ParamType::EnumRoute.as_raw(),
            properties: vec![
                Property::new(ROUTE_INDEX, Value::Int(3)),
                Property::new(
                    ROUTE_DIRECTION,
                    Value::Id(Id(pw::spa::sys::SPA_DIRECTION_OUTPUT)),
                ),
                Property::new(ROUTE_NAME, Value::String("analog-output-headphones".into())),
                Property::new(ROUTE_DESCRIPTION, Value::String("Headphones".into())),
                Property::new(
                    ROUTE_AVAILABLE,
                    Value::Id(Id(pw::spa::sys::SPA_PARAM_AVAILABILITY_no)),
                ),
                Property::new(
                    ROUTE_INFO,
                    Value::Struct(vec![
                        Value::Int(2),
                        Value::String("port.type".into()),
                        Value::String("headphones".into()),
                        Value::String("route.hw-volume".into()),
                        Value::String("true".into()),
                    ]),
                ),
                Property::new(
                    ROUTE_PROFILES,
                    Value::ValueArray(ValueArray::Int(vec![2, 1])),
                ),
                Property::new(ROUTE_DEVICES, Value::ValueArray(ValueArray::Int(vec![7]))),
            ],
        };
        let bytes = serialize(object).expect("serialises");
        let route = route_from(&read(&bytes)).expect("a route");
        assert_eq!(route.index, 3);
        assert_eq!(route.direction, Some(Direction::Output));
        assert_eq!(route.description, "Headphones");
        assert_eq!(route.available, Availability::No);
        assert_eq!(
            route.info.get("port.type").map(String::as_str),
            Some("headphones")
        );
        assert_eq!(route.hardware_volume(), Some(true));
        assert_eq!(route.profiles, vec![2, 1]);
        assert_eq!(route.devices, vec![7]);
        assert_eq!(route.device, None);
    }

    #[test]
    fn a_bluetooth_profile_names_its_codec_and_family() {
        let profile = |name: &str| Profile {
            name: name.into(),
            ..Profile::default()
        };
        let xq = profile("a2dp-sink-sbc_xq");
        assert_eq!(
            xq.codec().map(|(f, c)| (f, c.label)),
            Some(("a2dp-sink", "SBC-XQ"))
        );
        let hd = profile("a2dp-sink-aptx_hd");
        assert_eq!(hd.codec().map(|(_, c)| c.label), Some("aptX HD"));
        let msbc = profile("headset-head-unit-msbc");
        assert_eq!(msbc.family(), "headset-head-unit");
        let analog = profile("output:analog-stereo");
        assert_eq!(analog.codec(), None);
        assert_eq!(analog.family(), "output:analog-stereo");
        assert!(profile("off").is_off());
    }

    #[test]
    fn the_ports_offered_serve_the_same_sink_under_the_running_profile() {
        let route = |index, direction, profiles: Vec<u32>, devices: Vec<u32>| Route {
            index,
            direction: Some(direction),
            profiles,
            devices,
            ..Route::default()
        };
        let speakers = route(2, Direction::Output, vec![1, 2], vec![7]);
        let headphones = route(3, Direction::Output, vec![1, 2], vec![7]);
        let hdmi = route(4, Direction::Output, vec![4], vec![8]);
        let mic = route(0, Direction::Input, vec![1], vec![0]);
        let device = Device {
            profile: Some(Profile {
                index: 1,
                ..Profile::default()
            }),
            routes: vec![mic.clone(), speakers.clone(), headphones.clone(), hdmi],
            ..Device::default()
        };
        let active = Route {
            device: Some(7),
            ..speakers.clone()
        };
        let offered: Vec<u32> = device.routes_for(&active).iter().map(|r| r.index).collect();
        assert_eq!(offered, vec![2, 3]);
    }
}
