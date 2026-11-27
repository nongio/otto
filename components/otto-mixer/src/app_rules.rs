//! Which app streams the service moves to the output their rule names.
//!
//! A rule (`[[rule]]` in the routing file) says where an app's sound plays.
//! The service applies it when one of the app's streams appears, and again
//! when the rule's output comes back after being unplugged, by setting the
//! stream's `target.object` as moving it in the Mixer does. Between those
//! moments a stream moved by hand stays where it was put.
//!
//! A rule's output is a device; when the device has a tuning (output effects
//! on it), the stream goes to the top of that chain so it is heard through
//! it, rather than to the device and then onto the effect.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};

use otto_audio_graph::{Endpoint, Graph, Routing};

/// One stream to move: `stream` (a node id) to the node called `target`, or
/// back to following the default with `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMove {
    pub stream: u32,
    pub target: Option<String>,
}

/// The service's memory of which streams and outputs it has seen.
#[derive(Debug, Default)]
pub struct RuleMover {
    /// Streams a rule has been applied to, or that have none, by id.
    seen: HashSet<u32>,
    /// Each rule output present, by name, with the node id it had; a new id
    /// means it came back.
    outputs: HashMap<String, u32>,
}

impl RuleMover {
    /// The server went away: every id is gone.
    pub fn disconnected(&mut self) {
        self.seen.clear();
        self.outputs.clear();
    }

    /// The moves `routing`'s rules ask for in `graph` now.
    pub fn step(&mut self, routing: &Routing, graph: &Graph) -> Vec<RuleMove> {
        // The outputs rules name that appeared since the last step.
        let mut back: HashSet<&str> = HashSet::new();
        for rule in routing.rules() {
            let Some(name) = rule.output.as_deref() else {
                continue;
            };
            let node = graph
                .nodes()
                .find(|n| n.name() == name && Endpoint::Output.matches(n));
            match node {
                Some(node) => {
                    if self.outputs.insert(name.to_string(), node.id) != Some(node.id) {
                        back.insert(name);
                    }
                }
                None => {
                    self.outputs.remove(name);
                }
            }
        }

        let mut moves = Vec::new();
        let streams = graph.streams(Endpoint::Output);
        for stream in &streams {
            let first = self.seen.insert(stream.id);
            let Some(rule) = graph.node(stream.id).and_then(|n| routing.rule_for(n)) else {
                continue;
            };
            match rule.output.as_deref() {
                Some(name) => {
                    if !(first || back.contains(name)) || !self.outputs.contains_key(name) {
                        continue;
                    }
                    let entry = routing.output_entry(name);
                    let there = stream
                        .linked
                        .iter()
                        .any(|id| graph.node(*id).is_some_and(|n| n.name() == entry));
                    if !there || stream.target.as_deref() != Some(entry.as_str()) {
                        moves.push(RuleMove {
                            stream: stream.id,
                            target: Some(entry),
                        });
                    }
                }
                None => {
                    if first && stream.target.is_some() {
                        moves.push(RuleMove {
                            stream: stream.id,
                            target: None,
                        });
                    }
                }
            }
        }
        // Forget streams that are gone, so the set stays small.
        self.seen.retain(|id| streams.iter().any(|s| s.id == *id));
        moves
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{EffectKind, EffectSide, Event, Link, Node, NodeState};

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

    fn sink(id: u32, name: &str) -> Event {
        node(id, &[("media.class", "Audio/Sink"), ("node.name", name)])
    }

    /// Speakers (1) and a USB interface (2); Spotify (10) and mpv (11)
    /// playing on the speakers.
    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        graph.apply(sink(1, "speakers"));
        graph.apply(sink(2, "usb"));
        for (id, app) in [(10, "Spotify"), (11, "mpv")] {
            graph.apply(node(
                id,
                &[
                    ("media.class", "Stream/Output/Audio"),
                    ("application.name", app),
                ],
            ));
            graph.apply(link(100 + id, id, 1));
        }
        graph
    }

    fn rules() -> Routing {
        let mut routing = Routing::default();
        routing.set_rule("spotify", Some("usb".into()));
        routing
    }

    #[test]
    fn a_stream_moves_when_it_appears_and_not_again() {
        let graph = graph();
        let mut mover = RuleMover::default();
        assert_eq!(
            mover.step(&rules(), &graph),
            [RuleMove {
                stream: 10,
                target: Some("usb".into())
            }]
        );
        // Moved back by hand: left there.
        assert!(mover.step(&rules(), &graph).is_empty());
    }

    #[test]
    fn a_stream_waits_for_its_output_and_moves_when_it_comes_back() {
        let mut graph = graph();
        graph.apply(Event::Removed(2));
        let mut mover = RuleMover::default();
        assert!(mover.step(&rules(), &graph).is_empty());
        graph.apply(sink(3, "usb"));
        assert_eq!(mover.step(&rules(), &graph).len(), 1);
        assert!(mover.step(&rules(), &graph).is_empty());
    }

    #[test]
    fn a_tuned_output_is_reached_through_its_tuning() {
        let graph = graph();
        let mut routing = rules();
        routing.add_effect("EQ", EffectKind::Equaliser, EffectSide::Output, "usb");
        let moves = RuleMover::default().step(&routing, &graph);
        assert_eq!(moves[0].target.as_deref(), Some("otto-effect.eq"));
    }

    #[test]
    fn a_follow_the_default_rule_clears_a_remembered_target() {
        let mut graph = graph();
        graph.apply(Event::Target {
            stream: 11,
            target: Some("usb".into()),
        });
        let mut routing = Routing::default();
        routing.set_rule("mpv", None);
        assert_eq!(
            RuleMover::default().step(&routing, &graph),
            [RuleMove {
                stream: 11,
                target: None
            }]
        );
    }
}
