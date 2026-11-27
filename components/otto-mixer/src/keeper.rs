//! What the service links, given the graph and the saved routing.
//!
//! Every saved link whose two ends exist and are not linked is linked. A link
//! that existed and then vanished while both its ports are still there was
//! removed by someone; [`LoopGuard`] decides whether to put it back. A link
//! whose port went away is simply waited for: unplugging a device is not a
//! reason to forget it.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use otto_audio_graph::{
    EngineSettings, Graph, LoopGuard, Routing, Verdict, SETTING_FORCE_QUANTUM, SETTING_FORCE_RATE,
};

/// How long a requested link may take to show up before it is asked for
/// again. PipeWire answers within milliseconds; a link still missing after a
/// second failed, and asking again on every graph change would flood the
/// server with doomed requests.
const REQUEST_GRACE: Duration = Duration::from_secs(1);

/// One step's decisions.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Port pairs to link, `(output, input)`.
    pub link: Vec<(u32, u32)>,
    /// Engine settings to set, `(key, value)`.
    pub settings: Vec<(&'static str, String)>,
    /// Port pairs the guard gave up on this step, to report once.
    pub given_up: Vec<(u32, u32)>,
}

/// The service's memory between steps.
#[derive(Debug, Default)]
pub struct Keeper {
    routing: Routing,
    guard: LoopGuard<(u32, u32)>,
    /// Saved pairs seen linked, so their disappearance can be told from a
    /// pair that never was.
    present: HashSet<(u32, u32)>,
    /// Pairs asked for, and when.
    requested: HashMap<(u32, u32), Instant>,
    /// The engine settings put in force since the server last connected.
    engine_applied: Option<EngineSettings>,
}

impl Keeper {
    /// A keeper for `routing`.
    #[must_use]
    pub fn new(routing: Routing) -> Self {
        Self {
            routing,
            ..Self::default()
        }
    }

    /// Take a new saved routing. The guard starts over, since what the
    /// person wants has changed.
    pub fn set_routing(&mut self, routing: Routing) {
        if routing.engine != self.routing.engine {
            self.engine_applied = None;
        }
        self.routing = routing;
        self.guard.reset();
    }

    /// The server went away: everything it had is gone, ids included.
    pub fn disconnected(&mut self) {
        self.present.clear();
        self.requested.clear();
        self.engine_applied = None;
    }

    /// Decide what to do about `graph` at `now`.
    pub fn step(&mut self, graph: &Graph, now: Instant) -> Plan {
        let mut plan = Plan::default();
        if !graph.is_connected() {
            return plan;
        }
        let wanted = self.routing.resolve(graph);
        for pair in &wanted {
            if graph.link_between(pair.0, pair.1).is_some() {
                self.present.insert(*pair);
                self.requested.remove(pair);
                continue;
            }
            if self
                .requested
                .get(pair)
                .is_some_and(|at| now.saturating_duration_since(*at) < REQUEST_GRACE)
            {
                continue;
            }
            if self.present.remove(pair) {
                match self.guard.removed(pair, now) {
                    Verdict::Restore => {}
                    Verdict::GiveUp { first } => {
                        if first {
                            plan.given_up.push(*pair);
                        }
                        continue;
                    }
                }
            } else if self.guard.has_given_up(pair) {
                continue;
            }
            self.requested.insert(*pair, now);
            plan.link.push(*pair);
        }
        self.present.retain(|pair| wanted.contains(pair));
        self.requested.retain(|pair, _| wanted.contains(pair));

        let engine = self.routing.engine;
        if graph.is_synced() && self.engine_applied != Some(engine) {
            for (key, value) in [
                (SETTING_FORCE_RATE, engine.force_rate),
                (SETTING_FORCE_QUANTUM, engine.force_quantum),
            ] {
                if let Some(value) = value {
                    if graph.setting_u32(key) != Some(value) {
                        plan.settings.push((key, value.to_string()));
                    }
                }
            }
            self.engine_applied = Some(engine);
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_audio_graph::{
        Direction, Event, Link, Node, NodeIdentity, NodeState, Port, PortRef, SavedLink,
    };

    fn graph() -> Graph {
        let mut graph = Graph::new();
        graph.apply(Event::Connected);
        for (id, name) in [(1, "a"), (2, "b")] {
            graph.apply(Event::Node(Node {
                id,
                props: [
                    ("media.class".to_string(), "Audio/Sink".to_string()),
                    ("node.name".to_string(), name.to_string()),
                ]
                .into(),
                state: NodeState::Idle,
            }));
        }
        graph.apply(Event::Port(Port {
            id: 10,
            node: 1,
            direction: Some(Direction::Output),
            name: "monitor_FL".into(),
            ..Port::default()
        }));
        graph.apply(Event::Port(Port {
            id: 20,
            node: 2,
            direction: Some(Direction::Input),
            name: "playback_FL".into(),
            ..Port::default()
        }));
        graph.apply(Event::Synced);
        graph
    }

    fn routing() -> Routing {
        let mut routing = Routing::default();
        routing.add(SavedLink {
            output: PortRef {
                node: NodeIdentity::Node { name: "a".into() },
                port: "monitor_FL".into(),
            },
            input: PortRef {
                node: NodeIdentity::Node { name: "b".into() },
                port: "playback_FL".into(),
            },
        });
        routing
    }

    fn link(graph: &mut Graph, id: u32) {
        graph.apply(Event::Link(Link {
            id,
            output_node: 1,
            output_port: 10,
            input_node: 2,
            input_port: 20,
            client: None,
        }));
    }

    #[test]
    fn a_saved_link_with_both_ends_is_made() {
        let mut keeper = Keeper::new(routing());
        let plan = keeper.step(&graph(), Instant::now());
        assert_eq!(plan.link, vec![(10, 20)]);
    }

    #[test]
    fn a_request_is_not_repeated_while_it_is_pending() {
        let mut keeper = Keeper::new(routing());
        let graph = graph();
        let now = Instant::now();
        keeper.step(&graph, now);
        assert!(keeper
            .step(&graph, now + Duration::from_millis(100))
            .link
            .is_empty());
        assert_eq!(keeper.step(&graph, now + REQUEST_GRACE * 2).link.len(), 1);
    }

    #[test]
    fn a_link_already_there_is_left_alone() {
        let mut keeper = Keeper::new(routing());
        let mut graph = graph();
        link(&mut graph, 30);
        assert!(keeper.step(&graph, Instant::now()).link.is_empty());
    }

    #[test]
    fn a_link_removed_by_someone_is_put_back_until_the_guard_gives_up() {
        let mut keeper = Keeper::new(routing());
        let mut graph = graph();
        let start = Instant::now();
        let mut restored = 0;
        let mut gave_up = false;
        for i in 0..6 {
            let now = start + Duration::from_millis(200 * i);
            link(&mut graph, 30 + i as u32);
            keeper.step(&graph, now);
            graph.apply(Event::Removed(30 + i as u32));
            let plan = keeper.step(&graph, now + Duration::from_millis(50));
            restored += plan.link.len();
            gave_up |= !plan.given_up.is_empty();
        }
        assert_eq!(restored, 3);
        assert!(gave_up);
    }

    #[test]
    fn an_unplugged_end_is_waited_for_not_fought_over() {
        let mut keeper = Keeper::new(routing());
        let mut graph = graph();
        link(&mut graph, 30);
        let now = Instant::now();
        keeper.step(&graph, now);
        graph.apply(Event::Removed(30));
        graph.apply(Event::Removed(20));
        graph.apply(Event::Removed(2));
        assert!(keeper.step(&graph, now).link.is_empty());
        // The device comes back with new ids.
        graph.apply(Event::Node(Node {
            id: 3,
            props: [
                ("media.class".to_string(), "Audio/Sink".to_string()),
                ("node.name".to_string(), "b".to_string()),
            ]
            .into(),
            state: NodeState::Idle,
        }));
        graph.apply(Event::Port(Port {
            id: 40,
            node: 3,
            direction: Some(Direction::Input),
            name: "playback_FL".into(),
            ..Port::default()
        }));
        assert_eq!(keeper.step(&graph, now).link, vec![(10, 40)]);
    }

    #[test]
    fn forced_engine_settings_are_put_back_once() {
        let mut routing = routing();
        routing.engine.force_quantum = Some(256);
        let mut keeper = Keeper::new(routing);
        let graph = graph();
        let now = Instant::now();
        let plan = keeper.step(&graph, now);
        assert_eq!(
            plan.settings,
            vec![(SETTING_FORCE_QUANTUM, "256".to_string())]
        );
        assert!(keeper.step(&graph, now).settings.is_empty());
        keeper.disconnected();
        assert_eq!(keeper.step(&graph, now).settings.len(), 1);
    }
}
