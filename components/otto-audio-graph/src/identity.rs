//! Naming a node so the name survives it going away and coming back.
//!
//! A device, a virtual device or a filter is named by `node.name`, which stays
//! the same across plugs and boots. An app's stream is named by its app and its
//! role: playback or capture, plus the media role when the app gives one, so
//! every stream of that app in that role matches.

// Rust guideline compliant 2026-02-21

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::Node;

/// Which way an app's stream goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamDirection {
    /// The app plays: `Stream/Output/…`.
    Playback,
    /// The app records: `Stream/Input/…`.
    Capture,
}

/// What a node is, independent of its global id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NodeIdentity {
    /// A device, virtual device or filter, by `node.name`.
    Node { name: String },
    /// An app's stream.
    Stream {
        /// `application.name`, else `application.process.binary`, else
        /// `node.name`.
        app: String,
        direction: StreamDirection,
        /// `media.role`, when the app gives one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
}

impl fmt::Display for NodeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node { name } => write!(f, "node:{name}"),
            Self::Stream {
                app,
                direction,
                role,
            } => {
                let direction = match direction {
                    StreamDirection::Playback => "playback",
                    StreamDirection::Capture => "capture",
                };
                write!(f, "stream:{app}:{direction}")?;
                if let Some(role) = role {
                    write!(f, ":{role}")?;
                }
                Ok(())
            }
        }
    }
}

/// The identity of `node`, or `None` for a node with no name to go by.
#[must_use]
pub fn identity(node: &Node) -> Option<NodeIdentity> {
    let class = node.media_class();
    let direction = if class.starts_with("Stream/Output") {
        Some(StreamDirection::Playback)
    } else if class.starts_with("Stream/Input") {
        Some(StreamDirection::Capture)
    } else {
        None
    };
    match direction {
        Some(direction) => {
            let app = node.app().or_else(|| node.prop("node.name"))?.to_string();
            Some(NodeIdentity::Stream {
                app,
                direction,
                role: node.prop("media.role").map(str::to_string),
            })
        }
        None => Some(NodeIdentity::Node {
            name: node.prop("node.name")?.to_string(),
        }),
    }
}

/// A port, named by its node's identity and `port.name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PortRef {
    #[serde(flatten)]
    pub node: NodeIdentity,
    pub port: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeState;

    fn node(props: &[(&str, &str)]) -> Node {
        Node {
            id: 7,
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state: NodeState::Idle,
        }
    }

    #[test]
    fn a_device_goes_by_its_node_name() {
        let speaker = node(&[
            ("media.class", "Audio/Sink"),
            ("node.name", "alsa_output.pci-0000_00_1f.3.analog-stereo"),
            ("device.id", "51"),
        ]);
        assert_eq!(
            identity(&speaker),
            Some(NodeIdentity::Node {
                name: "alsa_output.pci-0000_00_1f.3.analog-stereo".into()
            })
        );
    }

    #[test]
    fn a_stream_goes_by_its_app_and_role() {
        let stream = node(&[
            ("media.class", "Stream/Output/Audio"),
            ("node.name", "Firefox"),
            ("application.name", "Firefox"),
            ("media.role", "Music"),
        ]);
        assert_eq!(
            identity(&stream),
            Some(NodeIdentity::Stream {
                app: "Firefox".into(),
                direction: StreamDirection::Playback,
                role: Some("Music".into()),
            })
        );
    }

    #[test]
    fn a_stream_without_an_app_name_falls_back_to_its_binary() {
        let stream = node(&[
            ("media.class", "Stream/Input/Audio"),
            ("node.name", "record-1234"),
            ("application.process.binary", "pw-record"),
        ]);
        let Some(NodeIdentity::Stream { app, direction, .. }) = identity(&stream) else {
            panic!("a stream");
        };
        assert_eq!(app, "pw-record");
        assert_eq!(direction, StreamDirection::Capture);
    }

    #[test]
    fn two_streams_of_one_app_share_an_identity() {
        let a = node(&[
            ("media.class", "Stream/Output/Audio"),
            ("application.name", "mpv"),
            ("node.name", "mpv-1"),
        ]);
        let b = node(&[
            ("media.class", "Stream/Output/Audio"),
            ("application.name", "mpv"),
            ("node.name", "mpv-2"),
        ]);
        assert_eq!(identity(&a), identity(&b));
    }

    #[test]
    fn a_node_with_no_name_has_no_identity() {
        assert_eq!(identity(&node(&[("media.class", "Audio/Sink")])), None);
    }

    #[test]
    fn identities_print_as_keys() {
        let stream = NodeIdentity::Stream {
            app: "mpv".into(),
            direction: StreamDirection::Playback,
            role: None,
        };
        assert_eq!(stream.to_string(), "stream:mpv:playback");
        let device = NodeIdentity::Node { name: "x".into() };
        assert_eq!(device.to_string(), "node:x");
    }
}
