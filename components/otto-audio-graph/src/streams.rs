//! App streams one by one, and what kind of device an output or input is.
//!
//! [`Graph::app_streams`](crate::Graph::app_streams) folds an app's streams
//! into one for a menu; a mixer that moves streams needs each of them, with
//! where it plays or records now and where it was asked to go
//! (`target.object` in the `default` metadata). The otto-mixer window and the
//! top bar's sound menu both read these, so a device is called the same
//! thing in both.

// Rust guideline compliant 2026-02-21

use crate::effects::EFFECT_PREFIX;
use crate::mixer::{Endpoint, Volume};
use crate::model::{Graph, Node, NodeState};

/// What every virtual device's node names start with.
const VIRTUAL_PREFIX: &str = "otto-virtual.";

/// What kind of device an output or input is, as people tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Bluetooth,
    Usb,
    Hdmi,
    /// An AirPlay receiver on the network, which plays about 2 s late.
    AirPlay,
    /// Made in software: a null sink, a loopback, an effect.
    Virtual,
    /// A card's own speakers, jack or microphone.
    BuiltIn,
}

impl Node {
    /// What kind of device this output or input is, from what the session
    /// manager says of it.
    #[must_use]
    pub fn device_kind(&self) -> DeviceKind {
        if self.is_airplay() {
            return DeviceKind::AirPlay;
        }
        if !self.is_device() {
            return DeviceKind::Virtual;
        }
        let api = self.prop("device.api").unwrap_or_default();
        let bus = self.prop("device.bus").unwrap_or_default();
        let name = self.name();
        if api == "bluez5" || bus == "bluetooth" {
            DeviceKind::Bluetooth
        } else if bus == "usb" {
            DeviceKind::Usb
        } else if name.contains("hdmi") || name.contains("iec958") {
            DeviceKind::Hdmi
        } else {
            DeviceKind::BuiltIn
        }
    }
}

/// One app stream: a playback stream, or a recording one.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    /// Its node id, which commands name.
    pub id: u32,
    /// The app's name: `application.name`, else its program, else the
    /// stream's own name.
    pub app: String,
    /// `application.process.binary`, which a rule may name instead.
    pub binary: Option<String>,
    /// What it plays or records, from `media.name`, when that says more
    /// than the app's name.
    pub media: Option<String>,
    /// `application.icon-name`, for an icon from the person's theme.
    pub icon: Option<String>,
    /// Whether it plays ([`Endpoint::Output`]) or records
    /// ([`Endpoint::Input`]).
    pub direction: Endpoint,
    /// Its volume, once its `Props` have been read.
    pub volume: Option<Volume>,
    /// The nodes it plays to or records from now, by id, from its links.
    pub linked: Vec<u32>,
    /// Where it was asked to go: `target.object`, a node's name or serial;
    /// `None` while it follows the default.
    pub target: Option<String>,
    /// Whether sound passes through it now.
    pub running: bool,
}

impl Graph {
    /// The app streams that play (`Endpoint::Output`) or record
    /// (`Endpoint::Input`) sound, in id order.
    ///
    /// Filters' and loopbacks' own streams, level monitors, recordings of an
    /// output's monitor and the streams of Otto's own effects and virtual
    /// devices are left out: they are not an app.
    #[must_use]
    pub fn streams(&self, endpoint: Endpoint) -> Vec<Stream> {
        self.nodes()
            .filter(|node| is_app_stream(node, endpoint))
            .map(|node| {
                let mut linked: Vec<u32> = self
                    .links()
                    .filter_map(|link| match endpoint {
                        Endpoint::Output if link.output_node == node.id => Some(link.input_node),
                        Endpoint::Input if link.input_node == node.id => Some(link.output_node),
                        _ => None,
                    })
                    .collect();
                linked.sort_unstable();
                linked.dedup();
                let app = node
                    .app()
                    .or_else(|| node.prop("node.description"))
                    .unwrap_or_else(|| node.name())
                    .to_string();
                let media = node
                    .prop("media.name")
                    .filter(|m| !m.eq_ignore_ascii_case(&app))
                    .map(str::to_string);
                Stream {
                    id: node.id,
                    binary: node.prop("application.process.binary").map(str::to_string),
                    media,
                    icon: node.prop("application.icon-name").map(str::to_string),
                    direction: endpoint,
                    volume: self.volume(node.id).cloned(),
                    linked,
                    target: self.stream_target(node.id).map(str::to_string),
                    running: node.state == NodeState::Running,
                    app,
                }
            })
            .collect()
    }

    /// Where stream `id` was asked to go, as its `target.object` in the
    /// `default` metadata says; `None` while it follows the default.
    #[must_use]
    pub fn stream_target(&self, id: u32) -> Option<&str> {
        self.mixer.target(id)
    }

    /// The node a `target.object` value names: by `node.name`, else by
    /// `object.serial`, else by id, as the session manager reads it.
    #[must_use]
    pub fn node_targeted(&self, target: &str) -> Option<&Node> {
        self.nodes()
            .find(|n| n.name() == target)
            .or_else(|| {
                self.nodes()
                    .find(|n| n.prop("object.serial") == Some(target))
            })
            .or_else(|| {
                let id: u32 = target.parse().ok()?;
                self.node(id)
            })
    }

    /// The app streams recording from a microphone now: running recording
    /// streams linked from a source (a device's, a virtual one or an input
    /// effect), not from an output's monitor.
    #[must_use]
    pub fn recording_streams(&self) -> Vec<Stream> {
        self.streams(Endpoint::Input)
            .into_iter()
            .filter(|s| s.running)
            .filter(|s| {
                s.linked.iter().any(|id| {
                    self.node(*id)
                        .is_some_and(|n| n.media_class().starts_with("Audio/Source"))
                })
            })
            .collect()
    }
}

/// Whether `node` is an app's own audio stream going `endpoint`'s way.
fn is_app_stream(node: &Node, endpoint: Endpoint) -> bool {
    let class = match endpoint {
        Endpoint::Output => "Stream/Output/Audio",
        Endpoint::Input => "Stream/Input/Audio",
    };
    let name = node.name();
    node.media_class() == class
        && node.prop("node.link-group").is_none()
        && node.prop("stream.monitor") != Some("true")
        && node.prop("stream.capture.sink") != Some("true")
        && !name.starts_with(EFFECT_PREFIX)
        && !name.starts_with(VIRTUAL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Event, Link};

    fn node(id: u32, props: &[(&str, &str)]) -> Event {
        Event::Node(Node {
            id,
            props: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            state: NodeState::Running,
        })
    }

    fn link(id: u32, output_node: u32, input_node: u32) -> Event {
        Event::Link(Link {
            id,
            output_node,
            output_port: id * 10,
            input_node,
            input_port: id * 10 + 1,
            client: None,
        })
    }

    /// Speakers (1) and a microphone (2); mpv playing (10), a call app
    /// recording the microphone (11) and a recorder of the speakers'
    /// monitor (12), and a filter's own stream (13).
    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(node(
            1,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "speakers"),
                ("device.id", "40"),
            ],
        ));
        graph.apply(node(
            2,
            &[
                ("media.class", "Audio/Source"),
                ("node.name", "mic"),
                ("device.id", "40"),
            ],
        ));
        graph.apply(node(
            10,
            &[
                ("media.class", "Stream/Output/Audio"),
                ("application.name", "mpv"),
                ("application.process.binary", "mpv"),
                ("media.name", "song.flac"),
            ],
        ));
        graph.apply(node(
            11,
            &[
                ("media.class", "Stream/Input/Audio"),
                ("application.name", "Firefox"),
            ],
        ));
        graph.apply(node(
            12,
            &[
                ("media.class", "Stream/Input/Audio"),
                ("application.name", "OBS"),
            ],
        ));
        graph.apply(node(
            13,
            &[
                ("media.class", "Stream/Output/Audio"),
                ("node.name", "otto-effect.eq.play"),
                ("node.link-group", "filter-chain-1"),
            ],
        ));
        graph.apply(link(100, 10, 1));
        graph.apply(link(101, 2, 11));
        graph.apply(link(102, 1, 12));
        graph.apply(link(103, 13, 1));
        graph
    }

    #[test]
    fn each_stream_says_where_it_plays_and_filters_are_left_out() {
        let graph = graph();
        let playing = graph.streams(Endpoint::Output);
        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].app, "mpv");
        assert_eq!(playing[0].media.as_deref(), Some("song.flac"));
        assert_eq!(playing[0].linked, vec![1]);
        assert_eq!(playing[0].target, None);
        let recording: Vec<u32> = graph
            .streams(Endpoint::Input)
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(recording, vec![11, 12]);
    }

    #[test]
    fn only_a_microphone_counts_as_recording() {
        let graph = graph();
        let ids: Vec<u32> = graph.recording_streams().iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![11]);
    }

    #[test]
    fn a_moved_stream_keeps_its_target_until_it_goes() {
        let mut graph = graph();
        graph.apply(Event::Target {
            stream: 10,
            target: Some("speakers".into()),
        });
        assert_eq!(graph.stream_target(10), Some("speakers"));
        assert_eq!(graph.node_targeted("speakers").map(|n| n.id), Some(1));
        assert_eq!(graph.node_targeted("2").map(|n| n.id), Some(2));
        graph.apply(Event::Removed(10));
        assert_eq!(graph.stream_target(10), None);
    }

    #[test]
    fn a_device_says_what_kind_it_is() {
        let mut graph = graph();
        graph.apply(node(
            3,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "bluez_output.AA"),
                ("device.id", "41"),
                ("device.api", "bluez5"),
            ],
        ));
        graph.apply(node(
            4,
            &[("media.class", "Audio/Sink"), ("node.name", "null")],
        ));
        let kind = |id| graph.node(id).map(Node::device_kind);
        assert_eq!(kind(1), Some(DeviceKind::BuiltIn));
        assert_eq!(kind(3), Some(DeviceKind::Bluetooth));
        assert_eq!(kind(4), Some(DeviceKind::Virtual));
    }
}
