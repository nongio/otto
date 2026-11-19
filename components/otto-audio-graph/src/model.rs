//! The graph as PipeWire reports it: nodes, ports, links, clients and the
//! engine settings, kept by global id.

// Rust guideline compliant 2026-02-21

use std::collections::BTreeMap;

/// The engine's sample rate, in the `settings` metadata.
pub const SETTING_RATE: &str = "clock.rate";
/// The engine's default quantum, in frames.
pub const SETTING_QUANTUM: &str = "clock.quantum";
/// The smallest quantum a stream may ask for.
pub const SETTING_MIN_QUANTUM: &str = "clock.min-quantum";
/// The largest quantum a stream may ask for.
pub const SETTING_MAX_QUANTUM: &str = "clock.max-quantum";
/// A quantum forced on every node; `0` lets the engine choose.
pub const SETTING_FORCE_QUANTUM: &str = "clock.force-quantum";
/// A rate forced on the engine; `0` lets the engine choose.
pub const SETTING_FORCE_RATE: &str = "clock.force-rate";

/// Which way a port carries data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// Data flows into the node.
    Input,
    /// Data flows out of the node.
    Output,
}

/// What kind of data a port or node carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaType {
    Audio,
    Midi,
    Video,
    /// Anything else, such as a control port.
    Other,
}

impl MediaType {
    /// The media type a `media.class` names, as in `Audio/Sink` or
    /// `Stream/Output/Video`.
    #[must_use]
    pub fn from_media_class(class: &str) -> Self {
        if class.contains("Audio") {
            Self::Audio
        } else if class.contains("Midi") {
            Self::Midi
        } else if class.contains("Video") {
            Self::Video
        } else {
            Self::Other
        }
    }

    /// The media type a port's `format.dsp` names, as in
    /// `32 bit float mono audio` or `8 bit raw midi`.
    #[must_use]
    pub fn from_format_dsp(format: &str) -> Self {
        let format = format.to_ascii_lowercase();
        if format.contains("audio") {
            Self::Audio
        } else if format.contains("midi") || format.contains("ump") {
            Self::Midi
        } else if format.contains("video") {
            Self::Video
        } else {
            Self::Other
        }
    }
}

/// Where a node is in its life, as PipeWire says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeState {
    #[default]
    Creating,
    Suspended,
    Idle,
    Running,
    Error,
}

/// One node: a device, an app's stream, a filter or a virtual device.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    /// The global id, which changes every time the node appears.
    pub id: u32,
    /// Every property PipeWire reports for it.
    pub props: BTreeMap<String, String>,
    pub state: NodeState,
}

impl Node {
    /// A property, if set and not empty.
    #[must_use]
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// `node.name`, the name that stays the same across plugs and boots.
    #[must_use]
    pub fn name(&self) -> &str {
        self.prop("node.name").unwrap_or_default()
    }

    /// `media.class`, such as `Audio/Sink` or `Stream/Output/Audio`.
    #[must_use]
    pub fn media_class(&self) -> &str {
        self.prop("media.class").unwrap_or_default()
    }

    /// The app behind a stream: its name, else its program.
    #[must_use]
    pub fn app(&self) -> Option<&str> {
        self.prop("application.name")
            .or_else(|| self.prop("application.process.binary"))
    }

    /// The name to show: the description, else the nick, else the app, else
    /// `node.name`.
    #[must_use]
    pub fn title(&self) -> &str {
        self.prop("node.description")
            .or_else(|| self.prop("node.nick"))
            .or_else(|| self.app())
            .unwrap_or_else(|| self.name())
    }

    /// Whether this node is an app's stream rather than a device or a virtual
    /// node.
    #[must_use]
    pub fn is_stream(&self) -> bool {
        self.media_class().starts_with("Stream/")
    }

    /// Whether a hardware device is behind this node.
    #[must_use]
    pub fn is_device(&self) -> bool {
        self.prop("device.id").is_some()
    }

    /// The media type its class names.
    #[must_use]
    pub fn media_type(&self) -> MediaType {
        MediaType::from_media_class(self.media_class())
    }
}

/// One port of a node.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Port {
    pub id: u32,
    /// The global id of the node it belongs to.
    pub node: u32,
    pub direction: Option<Direction>,
    /// `port.name`, such as `playback_FL`; stable for a given node.
    pub name: String,
    /// `audio.channel`, such as `FL`, when the port carries one channel.
    pub channel: Option<String>,
    /// The type named by `format.dsp`, when the port says.
    pub format: Option<MediaType>,
    /// `port.monitor`: a sink's monitor of what it plays.
    pub monitor: bool,
    pub props: BTreeMap<String, String>,
}

impl Port {
    /// The name to show: the channel, else the port name.
    #[must_use]
    pub fn label(&self) -> &str {
        self.channel.as_deref().unwrap_or(&self.name)
    }
}

/// One link from an output port to an input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Link {
    pub id: u32,
    pub output_node: u32,
    pub output_port: u32,
    pub input_node: u32,
    pub input_port: u32,
    /// The client that made it.
    pub client: Option<u32>,
}

/// A program connected to PipeWire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Client {
    pub id: u32,
    /// `application.name`.
    pub app: Option<String>,
}

/// One change in the graph, as [`Connection`](crate::Connection) reports it.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Connected to the server; the objects follow.
    Connected,
    /// Every object present when the connection was made has been reported.
    Synced,
    /// The server went away, or could not be reached. Everything is gone.
    Disconnected,
    /// A node appeared or changed.
    Node(Node),
    /// A port appeared.
    Port(Port),
    /// A link appeared.
    Link(Link),
    /// A client appeared.
    Client(Client),
    /// The object with this global id is gone.
    Removed(u32),
    /// An engine setting changed; `None` when it was cleared.
    Setting { key: String, value: Option<String> },
    /// The default output or input is now the node named `name`; `None`
    /// when the session manager names none.
    Default {
        endpoint: crate::mixer::Endpoint,
        name: Option<String>,
    },
    /// A node's volume or mute changed.
    Volume {
        node: u32,
        volume: crate::mixer::Volume,
    },
}

/// The graph as last reported, keyed by global id.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    nodes: BTreeMap<u32, Node>,
    ports: BTreeMap<u32, Port>,
    links: BTreeMap<u32, Link>,
    clients: BTreeMap<u32, Client>,
    settings: BTreeMap<String, String>,
    connected: bool,
    synced: bool,
    pub(crate) mixer: crate::mixer::Mixer,
}

impl Graph {
    /// An empty graph, not connected.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one event.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Connected => {
                self.clear();
                self.connected = true;
            }
            Event::Synced => self.synced = true,
            Event::Disconnected => {
                self.clear();
                self.connected = false;
            }
            Event::Node(node) => {
                self.nodes.insert(node.id, node);
            }
            Event::Port(port) => {
                self.ports.insert(port.id, port);
            }
            Event::Link(link) => {
                self.links.insert(link.id, link);
            }
            Event::Client(client) => {
                self.clients.insert(client.id, client);
            }
            Event::Removed(id) => {
                self.nodes.remove(&id);
                self.ports.remove(&id);
                self.links.remove(&id);
                self.clients.remove(&id);
                self.mixer.remove(id);
            }
            Event::Setting { key, value } => match value {
                Some(value) => {
                    self.settings.insert(key, value);
                }
                None => {
                    self.settings.remove(&key);
                }
            },
            Event::Default { endpoint, name } => self.mixer.set_default(endpoint, name),
            Event::Volume { node, volume } => self.mixer.set_volume(node, volume),
        }
    }

    fn clear(&mut self) {
        self.nodes.clear();
        self.ports.clear();
        self.links.clear();
        self.clients.clear();
        self.settings.clear();
        self.mixer = crate::mixer::Mixer::default();
        self.synced = false;
    }

    /// Whether a server is connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Whether everything present at connection time has been reported.
    #[must_use]
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// Every node, by id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    #[must_use]
    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Every port, by id.
    pub fn ports(&self) -> impl Iterator<Item = &Port> {
        self.ports.values()
    }

    #[must_use]
    pub fn port(&self, id: u32) -> Option<&Port> {
        self.ports.get(&id)
    }

    /// The ports of `node` going `direction`, in id order.
    pub fn ports_of(&self, node: u32, direction: Direction) -> impl Iterator<Item = &Port> {
        self.ports
            .values()
            .filter(move |p| p.node == node && p.direction == Some(direction))
    }

    /// Every link, by id.
    pub fn links(&self) -> impl Iterator<Item = &Link> {
        self.links.values()
    }

    #[must_use]
    pub fn link(&self, id: u32) -> Option<&Link> {
        self.links.get(&id)
    }

    /// The link from `output` to `input`, if there is one.
    #[must_use]
    pub fn link_between(&self, output: u32, input: u32) -> Option<&Link> {
        self.links
            .values()
            .find(|l| l.output_port == output && l.input_port == input)
    }

    /// Every client, by id.
    pub fn clients(&self) -> impl Iterator<Item = &Client> {
        self.clients.values()
    }

    /// Whether a client calling itself `app` is connected.
    #[must_use]
    pub fn has_client(&self, app: &str) -> bool {
        self.clients.values().any(|c| c.app.as_deref() == Some(app))
    }

    /// An engine setting from the `settings` metadata.
    #[must_use]
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(String::as_str)
    }

    /// An engine setting read as a number; `None` when unset, unreadable or 0.
    #[must_use]
    pub fn setting_u32(&self, key: &str) -> Option<u32> {
        self.setting(key)
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v != 0)
    }

    /// The media a port carries: its own format, else its node's class.
    #[must_use]
    pub fn port_media(&self, port: &Port) -> MediaType {
        match port.format {
            Some(media) if media != MediaType::Other => media,
            _ => self
                .nodes
                .get(&port.node)
                .map_or(MediaType::Other, Node::media_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, class: &str) -> Node {
        Node {
            id,
            props: [("media.class".to_string(), class.to_string())].into(),
            state: NodeState::Idle,
        }
    }

    #[test]
    fn removal_takes_any_kind_of_object() {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(Event::Node(node(1, "Audio/Sink")));
        graph.apply(Event::Port(Port {
            id: 2,
            node: 1,
            ..Port::default()
        }));
        graph.apply(Event::Removed(2));
        assert!(graph.port(2).is_none());
        graph.apply(Event::Removed(1));
        assert!(graph.node(1).is_none());
    }

    #[test]
    fn disconnecting_empties_the_graph() {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(Event::Node(node(1, "Audio/Sink")));
        graph.apply(Event::Disconnected);
        assert!(!graph.is_connected());
        assert_eq!(graph.nodes().count(), 0);
    }

    #[test]
    fn a_port_without_a_format_takes_its_node_s_media() {
        let mut graph = Graph::new();
        graph.apply(Event::Node(node(1, "Video/Source")));
        let port = Port {
            id: 2,
            node: 1,
            ..Port::default()
        };
        assert_eq!(graph.port_media(&port), MediaType::Video);
        let midi = Port {
            format: Some(MediaType::from_format_dsp("8 bit raw midi")),
            ..port
        };
        assert_eq!(graph.port_media(&midi), MediaType::Midi);
    }

    #[test]
    fn a_zero_setting_reads_as_unset() {
        let mut graph = Graph::new();
        graph.apply(Event::Setting {
            key: SETTING_FORCE_QUANTUM.into(),
            value: Some("0".into()),
        });
        assert_eq!(graph.setting_u32(SETTING_FORCE_QUANTUM), None);
        graph.apply(Event::Setting {
            key: SETTING_FORCE_QUANTUM.into(),
            value: Some("256".into()),
        });
        assert_eq!(graph.setting_u32(SETTING_FORCE_QUANTUM), Some(256));
    }
}
