//! Otto's live model of the PipeWire graph.
//!
//! [`Connection`] talks to PipeWire on a thread of its own. It reports every
//! node, port, link and client as an [`Event`], and takes commands: make a
//! link, remove one, change an engine setting. A [`Graph`] applies those
//! events and is what a UI or a daemon reads.
//!
//! Nodes come back with new ids every time they appear, so anything saved
//! names them by [`NodeIdentity`] instead. [`Routing`] is the saved routing:
//! the links to keep, in `$XDG_CONFIG_HOME/otto/audio/routing.toml`, which the
//! otto-mixer window writes and its service keeps in force. [`LoopGuard`] is
//! how the service stops fighting another program over a link, and
//! [`pair_channels`] how two folded ports are linked channel by channel.
//!
//! Each [`Device`] comes with its profiles and ports, and
//! [`Connection::set_profile`] and [`Connection::set_route`] switch them.
//! The mixer is part of the graph too: the default output and input
//! ([`Graph::default_node`]), each node's [`Volume`], the [`AppStreams`]
//! playing, and the commands that change them on [`Connection`].
//!
//! [`Graph::streams`] lists every app stream on its own, with where it plays
//! or records and where it was asked to go; [`AppRule`]s in the routing file
//! say where an app's sound should play, and [`Calls`] what happens to other
//! apps while one records from a microphone.
//!
//! [`Effect`]s are filter chains a program keeps made with
//! [`Connection::keep_effects`]; their values change live through
//! [`Connection::set_effect_controls`].
//!
//! [`Connection::keep_airplay`] keeps PipeWire's AirPlay (RAOP) discovery
//! loaded, which makes a sink for every receiver on the network
//! ([`Node::is_airplay`]); [`airplay_missing`] says what it lacks here.
//!
//! The behaviour is specified in `specs/audio-routing.md`.

#![warn(unreachable_pub)]

mod airplay;
mod connection;
mod devices;
mod effects;
mod guard;
mod identity;
mod mixer;
mod model;
mod module;
mod pairing;
mod profiler;
mod routing;
mod rules;
mod streams;
mod virtual_devices;

pub use airplay::{airplay_missing, AirPlay, AirPlayMissing, AIRPLAY_PACKAGE, AIRPLAY_SINK_PREFIX};
pub use connection::{Connection, LinkLifetime};
pub use devices::{codec_named, Availability, Codec, Device, Profile, Route};
pub use effects::{
    find_kernel, flat_bands, import_impulse, impulses_dir, parse_autoeq, parse_easyeffects,
    response_db, support, AutoEq, AutoEqError, Band, BandType, Coefficients, EasyEffects,
    EasyEffectsError, EasyEffectsLimiter, Effect, EffectKind, EffectSide, ImpulseError, ParamSpec,
    Support, Unit, EFFECT_PREFIX, EFFECT_RATE, MAX_BANDS, MAX_FREQ, MAX_GAIN, MAX_IMPULSE_BYTES,
    MAX_Q, MIN_FREQ, MIN_Q,
};
pub use guard::{LoopGuard, Verdict};
pub use identity::{identity, NodeIdentity, PortRef, StreamDirection};
pub use mixer::{to_cubic, to_linear, AppStreams, Endpoint, Volume};
pub use model::{
    Client, Direction, Event, Graph, Link, MediaType, Node, NodeState, Port, SETTING_FORCE_QUANTUM,
    SETTING_FORCE_RATE, SETTING_MAX_QUANTUM, SETTING_MIN_QUANTUM, SETTING_QUANTUM, SETTING_RATE,
};
pub use pairing::pair_channels;
pub use routing::{EngineSettings, Routing, RoutingError, SavedLink};
pub use rules::{AppRule, Calls, DEFAULT_DUCK_LEVEL};
pub use streams::{DeviceKind, Stream};
pub use virtual_devices::{positions, VirtualDevice, VirtualKind, MAX_CHANNELS};
