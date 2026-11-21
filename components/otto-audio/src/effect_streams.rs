//! Which app streams the service moves onto effects, and back off them.
//!
//! An effect sits on a device, so the streams playing to that device (or,
//! for an input effect, recording from it) should go through the effect
//! instead. The service does not link them itself: it sets each stream's
//! `target.object` in the `default` metadata, as moving a stream in the
//! Sound pane or `pactl move-sink-input` does, and the session manager
//! relinks it. A stream is moved once per appearance, so a person who moves
//! it back by hand is not overruled. When an effect is removed, the streams
//! on it are moved back onto its device.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};

use otto_audio_graph::{Effect, EffectSide, Graph, Node, EFFECT_PREFIX};

/// One stream to move: `stream` (a node id) onto the node called `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub stream: u32,
    pub target: String,
}

/// The service's memory of what it moved.
#[derive(Debug, Default)]
pub struct StreamMover {
    effects: Vec<Effect>,
    /// Per effect id, the streams moved onto it since its node appeared.
    moved: HashMap<String, HashSet<u32>>,
    /// Per effect id, the node id it had when last seen; a new id means it
    /// was made again, and its streams are moved again.
    nodes: HashMap<String, u32>,
}

impl StreamMover {
    /// Take the effects from a new routing file. Returns the moves that put
    /// streams on a removed effect, or one now on another device, back onto
    /// the device it sat on.
    pub fn set_effects(&mut self, effects: Vec<Effect>, graph: &Graph) -> Vec<Move> {
        let mut moves = Vec::new();
        for old in &self.effects {
            let kept = effects
                .iter()
                .any(|e| e.id == old.id && e.device == old.device && e.side == old.side);
            if kept {
                continue;
            }
            self.moved.remove(&old.id);
            self.nodes.remove(&old.id);
            let Some(node) = node_named(graph, &old.node_name()) else {
                continue;
            };
            for stream in streams_on(graph, node.id, old.side) {
                moves.push(Move {
                    stream,
                    target: old.device.clone(),
                });
            }
        }
        self.effects = effects;
        moves
    }

    /// The server went away: every id is gone.
    pub fn disconnected(&mut self) {
        self.moved.clear();
        self.nodes.clear();
    }

    /// The streams on an effect's device to move onto the effect now.
    pub fn step(&mut self, graph: &Graph) -> Vec<Move> {
        let mut moves = Vec::new();
        for effect in &self.effects {
            let (Some(node), Some(device)) = (
                node_named(graph, &effect.node_name()),
                node_named(graph, &effect.device),
            ) else {
                continue;
            };
            if self.nodes.insert(effect.id.clone(), node.id) != Some(node.id) {
                self.moved.remove(&effect.id);
            }
            let moved = self.moved.entry(effect.id.clone()).or_default();
            for stream in streams_on(graph, device.id, effect.side) {
                if moved.insert(stream) {
                    moves.push(Move {
                        stream,
                        target: effect.node_name(),
                    });
                }
            }
        }
        // Forget streams that are gone, so the sets stay small.
        for moved in self.moved.values_mut() {
            moved.retain(|id| graph.node(*id).is_some());
        }
        moves
    }
}

fn node_named<'a>(graph: &'a Graph, name: &str) -> Option<&'a Node> {
    graph.nodes().find(|n| n.name() == name)
}

/// The app streams playing to node `device` (an output) or recording from
/// it (an input), that may be moved.
fn streams_on(graph: &Graph, device: u32, side: EffectSide) -> Vec<u32> {
    let mut streams: Vec<u32> = graph
        .links()
        .filter_map(|link| match side {
            EffectSide::Output if link.input_node == device => Some(link.output_node),
            EffectSide::Input if link.output_node == device => Some(link.input_node),
            _ => None,
        })
        .filter(|id| graph.node(*id).is_some_and(|n| movable(n, side)))
        .collect();
    streams.sort_unstable();
    streams.dedup();
    streams
}

/// Whether `node` is an app's audio stream the session manager may move:
/// not a filter's or loopback's own stream, not a level monitor, and not one
/// that asked to stay where it is.
fn movable(node: &Node, side: EffectSide) -> bool {
    let class = match side {
        EffectSide::Output => "Stream/Output/Audio",
        EffectSide::Input => "Stream/Input/Audio",
    };
    node.media_class() == class
        && node.prop("node.link-group").is_none()
        && node.prop("node.dont-move") != Some("true")
        && node.prop("stream.monitor") != Some("true")
        && !node.name().starts_with(EFFECT_PREFIX)
        && !node.name().starts_with("otto-virtual.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{EffectKind, Event, Link, NodeState, Routing};

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

    /// Speakers (1), the effect's sink (2) and stream (3), an app stream on
    /// the speakers (10) and a filter's own stream on them (11).
    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(node(
            1,
            &[("media.class", "Audio/Sink"), ("node.name", "speakers")],
        ));
        graph.apply(node(
            2,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "otto-effect.eq"),
            ],
        ));
        graph.apply(node(
            3,
            &[
                ("media.class", "Stream/Output/Audio"),
                ("node.name", "otto-effect.eq.play"),
                ("node.link-group", "filter-chain-1"),
            ],
        ));
        graph.apply(node(
            10,
            &[("media.class", "Stream/Output/Audio"), ("node.name", "mpv")],
        ));
        graph.apply(node(
            11,
            &[
                ("media.class", "Stream/Output/Audio"),
                ("node.name", "echo"),
                ("node.link-group", "x"),
            ],
        ));
        graph.apply(link(100, 3, 1));
        graph.apply(link(101, 10, 1));
        graph.apply(link(102, 11, 1));
        graph
    }

    fn effects() -> Vec<Effect> {
        let mut routing = Routing::default();
        routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "speakers");
        routing.effects().to_vec()
    }

    #[test]
    fn app_streams_on_the_device_move_once() {
        let mut graph = graph();
        let mut mover = StreamMover::default();
        assert!(mover.set_effects(effects(), &graph).is_empty());
        assert_eq!(
            mover.step(&graph),
            [Move {
                stream: 10,
                target: "otto-effect.eq".into()
            }]
        );
        // Still on the speakers (moved back by hand): left alone.
        assert!(mover.step(&graph).is_empty());

        // The effect is made again: its streams are moved again.
        graph.apply(Event::Removed(2));
        graph.apply(node(
            4,
            &[
                ("media.class", "Audio/Sink"),
                ("node.name", "otto-effect.eq"),
            ],
        ));
        assert_eq!(mover.step(&graph).len(), 1);
    }

    #[test]
    fn nothing_moves_until_the_effect_exists() {
        let mut graph = graph();
        graph.apply(Event::Removed(2));
        let mut mover = StreamMover::default();
        mover.set_effects(effects(), &graph);
        assert!(mover.step(&graph).is_empty());
    }

    #[test]
    fn removing_an_effect_moves_its_streams_back() {
        let mut graph = graph();
        let mut mover = StreamMover::default();
        mover.set_effects(effects(), &graph);
        mover.step(&graph);
        graph.apply(Event::Removed(101));
        graph.apply(link(103, 10, 2));
        assert_eq!(
            mover.set_effects(Vec::new(), &graph),
            [Move {
                stream: 10,
                target: "speakers".into()
            }]
        );
        assert!(mover.step(&graph).is_empty());
    }
}
