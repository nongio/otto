//! Linking and unlinking against the running PipeWire server, on two null
//! sinks the test makes and removes itself. Nothing else in the graph is
//! touched.
//!
//! Ignored by default because it needs a server and `pactl`:
//!
//! ```sh
//! cargo test -p otto-audio-graph --test live_link -- --ignored
//! ```

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio_graph::{Connection, Direction, Event, Graph, LinkLifetime};

/// A null sink loaded through pipewire-pulse, unloaded when dropped.
struct NullSink(String);

impl NullSink {
    fn new(name: &str) -> Self {
        let out = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={name}"),
            ])
            .output()
            .expect("run pactl");
        assert!(out.status.success(), "pactl load-module failed");
        Self(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

impl Drop for NullSink {
    fn drop(&mut self) {
        let _ = Command::new("pactl")
            .args(["unload-module", &self.0])
            .status();
    }
}

/// Apply events until `done` holds or five seconds pass.
fn wait(rx: &mpsc::Receiver<Event>, graph: &mut Graph, done: impl Fn(&Graph) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while !done(graph) {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(event) => graph.apply(event),
            Err(_) => return false,
        }
    }
    true
}

fn port(graph: &Graph, node: &str, direction: Direction, name: &str) -> Option<u32> {
    let node = graph.nodes().find(|n| n.name() == node)?;
    graph
        .ports_of(node.id, direction)
        .find(|p| p.name == name)
        .map(|p| p.id)
}

#[test]
#[ignore = "needs a running PipeWire server and pactl"]
fn links_and_unlinks_two_throwaway_sinks() {
    let _a = NullSink::new("otto-audio-graph-test-a");
    let _b = NullSink::new("otto-audio-graph-test-b");
    let (tx, rx) = mpsc::channel();
    let connection = Connection::start("otto-audio-graph-test", move |event| {
        let _ = tx.send(event);
    });
    let mut graph = Graph::new();
    let ends = |g: &Graph| {
        Some((
            port(
                g,
                "otto-audio-graph-test-a",
                Direction::Output,
                "monitor_FL",
            )?,
            port(
                g,
                "otto-audio-graph-test-b",
                Direction::Input,
                "playback_FL",
            )?,
        ))
    };
    assert!(
        wait(&rx, &mut graph, |g| ends(g).is_some()),
        "sinks never showed"
    );
    let (output, input) = ends(&graph).unwrap();

    connection.link(output, input, LinkLifetime::Lingering);
    assert!(
        wait(&rx, &mut graph, |g| g.link_between(output, input).is_some()),
        "the link never appeared"
    );
    let link = graph.link_between(output, input).unwrap().id;

    connection.unlink(link);
    assert!(
        wait(&rx, &mut graph, |g| g.link(link).is_none()),
        "the link was never removed"
    );
}
