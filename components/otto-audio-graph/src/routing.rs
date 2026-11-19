//! The saved routing: the links to keep, by identity.
//!
//! Kept in `$XDG_CONFIG_HOME/otto/audio/routing.toml`. The otto-audio window
//! writes it and `otto-audio-service` reads it again whenever it changes.
//!
//! ```toml
//! version = 1
//!
//! [[link]]
//! output = { kind = "stream", app = "mpv", direction = "playback", port = "output_FL" }
//! input = { kind = "node", name = "alsa_output.usb-headset", port = "playback_FL" }
//!
//! [engine]
//! force_quantum = 256
//! ```

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::identity::{identity, NodeIdentity, PortRef};
use crate::model::{Direction, Graph, Link};

/// The format version this crate writes.
const VERSION: u32 = 1;

/// One saved link, from an output port to an input port.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SavedLink {
    pub output: PortRef,
    pub input: PortRef,
}

/// The engine settings to put back whenever the sound server starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSettings {
    /// `clock.force-rate`, in Hz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_rate: Option<u32>,
    /// `clock.force-quantum`, in frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_quantum: Option<u32>,
}

impl EngineSettings {
    /// Whether nothing is forced.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.force_rate.is_none() && self.force_quantum.is_none()
    }
}

/// Every saved link, and the engine settings to keep.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routing {
    #[serde(default)]
    version: u32,
    #[serde(default, skip_serializing_if = "EngineSettings::is_empty")]
    pub engine: EngineSettings,
    #[serde(default, rename = "link")]
    links: Vec<SavedLink>,
    /// The virtual devices the service makes; see `virtual_devices.rs`.
    #[serde(default, rename = "virtual", skip_serializing_if = "Vec::is_empty")]
    pub(crate) virtual_devices: Vec<crate::virtual_devices::VirtualDevice>,
}

/// Why the routing could not be read or written.
#[derive(Debug)]
pub struct RoutingError {
    path: PathBuf,
    kind: RoutingErrorKind,
}

#[derive(Debug)]
enum RoutingErrorKind {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    NoConfigDir,
}

impl RoutingError {
    /// The file it was about.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        match &self.kind {
            RoutingErrorKind::Io(e) => write!(f, "{path}: {e}"),
            RoutingErrorKind::Parse(e) => write!(f, "{path}: {e}"),
            RoutingErrorKind::Serialize(e) => write!(f, "{path}: {e}"),
            RoutingErrorKind::NoConfigDir => write!(f, "no configuration directory"),
        }
    }
}

impl std::error::Error for RoutingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            RoutingErrorKind::Io(e) => Some(e),
            RoutingErrorKind::Parse(e) => Some(e),
            RoutingErrorKind::Serialize(e) => Some(e),
            RoutingErrorKind::NoConfigDir => None,
        }
    }
}

impl Routing {
    /// Where the routing is kept: `$XDG_CONFIG_HOME/otto/audio/routing.toml`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        otto_foundations::xdg::config_home().map(|dir| dir.join("otto/audio/routing.toml"))
    }

    /// Read the saved routing from [`Routing::path`]. A missing file is an
    /// empty routing.
    ///
    /// # Errors
    ///
    /// When the file exists but cannot be read or parsed.
    pub fn load() -> Result<Self, RoutingError> {
        let path = Self::path().ok_or_else(|| RoutingError {
            path: PathBuf::new(),
            kind: RoutingErrorKind::NoConfigDir,
        })?;
        Self::load_from(&path)
    }

    /// Read the routing kept at `path`. A missing file is an empty routing.
    ///
    /// # Errors
    ///
    /// When the file exists but cannot be read or parsed.
    pub fn load_from(path: &Path) -> Result<Self, RoutingError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(RoutingError {
                    path: path.to_path_buf(),
                    kind: RoutingErrorKind::Io(e),
                })
            }
        };
        toml::from_str(&text).map_err(|e| RoutingError {
            path: path.to_path_buf(),
            kind: RoutingErrorKind::Parse(e),
        })
    }

    /// Write the routing to [`Routing::path`], replacing it in one step.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save(&self) -> Result<(), RoutingError> {
        let path = Self::path().ok_or_else(|| RoutingError {
            path: PathBuf::new(),
            kind: RoutingErrorKind::NoConfigDir,
        })?;
        self.save_to(&path)
    }

    /// Write the routing to `path`, replacing it in one step.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save_to(&self, path: &Path) -> Result<(), RoutingError> {
        let io = |e| RoutingError {
            path: path.to_path_buf(),
            kind: RoutingErrorKind::Io(e),
        };
        let mut out = self.clone();
        out.version = VERSION;
        let text = toml::to_string(&out).map_err(|e| RoutingError {
            path: path.to_path_buf(),
            kind: RoutingErrorKind::Serialize(e),
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        otto_foundations::fs::write_atomically(path, text.as_bytes()).map_err(io)
    }

    /// The saved links, in the order they were saved.
    #[must_use]
    pub fn links(&self) -> &[SavedLink] {
        &self.links
    }

    /// Whether `link` is saved.
    #[must_use]
    pub fn contains(&self, link: &SavedLink) -> bool {
        self.links.contains(link)
    }

    /// Save `link`; `false` when it was saved already.
    pub fn add(&mut self, link: SavedLink) -> bool {
        if self.contains(&link) {
            return false;
        }
        self.links.push(link);
        true
    }

    /// Forget `link`; `false` when it was not saved.
    pub fn remove(&mut self, link: &SavedLink) -> bool {
        let before = self.links.len();
        self.links.retain(|l| l != link);
        self.links.len() != before
    }

    /// The live port pairs every saved link names, as `(output, input)` port
    /// ids. An app with several streams of one identity has each matched.
    #[must_use]
    pub fn resolve(&self, graph: &Graph) -> Vec<(u32, u32)> {
        let mut by_identity: HashMap<NodeIdentity, Vec<u32>> = HashMap::new();
        for node in graph.nodes() {
            if let Some(id) = identity(node) {
                by_identity.entry(id).or_default().push(node.id);
            }
        }
        let ports_named = |r: &PortRef, direction: Direction| -> Vec<u32> {
            by_identity
                .get(&r.node)
                .into_iter()
                .flatten()
                .flat_map(|node| graph.ports_of(*node, direction))
                .filter(|p| p.name == r.port)
                .map(|p| p.id)
                .collect()
        };
        let mut pairs = Vec::new();
        for link in &self.links {
            let outputs = ports_named(&link.output, Direction::Output);
            let inputs = ports_named(&link.input, Direction::Input);
            for o in &outputs {
                for i in &inputs {
                    if !pairs.contains(&(*o, *i)) {
                        pairs.push((*o, *i));
                    }
                }
            }
        }
        pairs
    }
}

impl SavedLink {
    /// The saved form of a live link, when both its ends have an identity.
    #[must_use]
    pub fn of(graph: &Graph, link: &Link) -> Option<Self> {
        let end = |port_id: u32| -> Option<PortRef> {
            let port = graph.port(port_id)?;
            let node = graph.node(port.node)?;
            Some(PortRef {
                node: identity(node)?,
                port: port.name.clone(),
            })
        };
        Some(Self {
            output: end(link.output_port)?,
            input: end(link.input_port)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::StreamDirection;
    use crate::model::{Event, Node, NodeState, Port};

    fn saved() -> SavedLink {
        SavedLink {
            output: PortRef {
                node: NodeIdentity::Stream {
                    app: "mpv".into(),
                    direction: StreamDirection::Playback,
                    role: None,
                },
                port: "output_FL".into(),
            },
            input: PortRef {
                node: NodeIdentity::Node {
                    name: "headset".into(),
                },
                port: "playback_FL".into(),
            },
        }
    }

    #[test]
    fn the_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio/routing.toml");
        let mut routing = Routing::default();
        assert!(routing.add(saved()));
        assert!(!routing.add(saved()));
        routing.engine.force_quantum = Some(256);
        routing.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[link]]"), "{text}");
        let back = Routing::load_from(&path).unwrap();
        assert_eq!(back.links(), routing.links());
        assert_eq!(back.engine.force_quantum, Some(256));
        assert_eq!(back.engine.force_rate, None);
    }

    #[test]
    fn a_missing_file_is_an_empty_routing() {
        let dir = tempfile::tempdir().unwrap();
        let routing = Routing::load_from(&dir.path().join("none.toml")).unwrap();
        assert!(routing.links().is_empty());
    }

    #[test]
    fn a_broken_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing.toml");
        std::fs::write(&path, "[[link]]\noutput = 3\n").unwrap();
        let error = Routing::load_from(&path).unwrap_err();
        assert_eq!(error.path(), path);
    }

    #[test]
    fn the_hand_written_form_reads() {
        let text = r#"
            version = 1
            [[link]]
            output = { kind = "stream", app = "mpv", direction = "playback", port = "output_FL" }
            input = { kind = "node", name = "headset", port = "playback_FL" }
        "#;
        let routing: Routing = toml::from_str(text).unwrap();
        assert_eq!(routing.links(), [saved()]);
    }

    #[test]
    fn forgetting_removes_only_that_link() {
        let mut routing = Routing::default();
        routing.add(saved());
        assert!(routing.remove(&saved()));
        assert!(!routing.remove(&saved()));
        assert!(routing.links().is_empty());
    }

    fn graph() -> Graph {
        let mut graph = Graph::new();
        let node = |id: u32, props: &[(&str, &str)]| Node {
            id,
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state: NodeState::Running,
        };
        let port = |id: u32, node: u32, direction: Direction, name: &str| Port {
            id,
            node,
            direction: Some(direction),
            name: name.into(),
            ..Port::default()
        };
        graph.apply(Event::Connected);
        let mpv = [
            ("media.class", "Stream/Output/Audio"),
            ("application.name", "mpv"),
        ];
        graph.apply(Event::Node(node(1, &mpv)));
        graph.apply(Event::Node(node(2, &mpv)));
        graph.apply(Event::Node(node(
            3,
            &[("media.class", "Audio/Sink"), ("node.name", "headset")],
        )));
        graph.apply(Event::Port(port(11, 1, Direction::Output, "output_FL")));
        graph.apply(Event::Port(port(21, 2, Direction::Output, "output_FL")));
        graph.apply(Event::Port(port(31, 3, Direction::Input, "playback_FL")));
        graph.apply(Event::Port(port(32, 3, Direction::Output, "monitor_FL")));
        graph
    }

    #[test]
    fn a_saved_link_resolves_for_every_matching_stream() {
        let mut routing = Routing::default();
        routing.add(saved());
        assert_eq!(routing.resolve(&graph()), vec![(11, 31), (21, 31)]);
    }

    #[test]
    fn a_saved_link_with_a_missing_end_waits() {
        let mut routing = Routing::default();
        routing.add(saved());
        let mut graph = graph();
        graph.apply(Event::Removed(3));
        graph.apply(Event::Removed(31));
        assert!(routing.resolve(&graph).is_empty());
    }

    #[test]
    fn a_live_link_saves_by_identity() {
        let graph = graph();
        let link = Link {
            id: 40,
            output_node: 1,
            output_port: 11,
            input_node: 3,
            input_port: 31,
            client: None,
        };
        assert_eq!(SavedLink::of(&graph, &link), Some(saved()));
    }
}
