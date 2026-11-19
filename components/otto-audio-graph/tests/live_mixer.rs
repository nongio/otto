//! Volume and mute against the running PipeWire server, on a null sink the
//! test makes and removes itself. The person's own devices and defaults are
//! never touched.
//!
//! Ignored by default because it needs a server and `pactl`:
//!
//! ```sh
//! cargo test -p otto-audio-graph --test live_mixer -- --ignored
//! ```

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio_graph::{Connection, Endpoint, Event, Graph};

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

fn sink_id(graph: &Graph, name: &str) -> Option<u32> {
    graph.nodes().find(|n| n.name() == name).map(|n| n.id)
}

#[test]
#[ignore = "needs a running PipeWire server and pactl"]
fn sets_volume_and_mute_on_a_throwaway_sink() {
    let name = "otto-mixer-test";
    let _sink = NullSink::new(name);

    let (tx, rx) = mpsc::channel();
    let connection = Connection::start("otto-mixer-test", move |event| {
        let _ = tx.send(event);
    });
    let mut graph = Graph::new();
    assert!(
        wait(&rx, &mut graph, |g| sink_id(g, name)
            .is_some_and(|id| g.volume(id).is_some())),
        "the sink and its volume appear"
    );
    let id = sink_id(&graph, name).expect("the sink");
    assert!(graph.endpoints(Endpoint::Output).iter().any(|n| n.id == id));
    // The default is read, whatever it is; the test never changes it.
    assert!(wait(&rx, &mut graph, |g| g
        .default_name(Endpoint::Output)
        .is_some()));

    connection.set_volume(id, 0.3);
    assert!(
        wait(&rx, &mut graph, |g| g
            .volume(id)
            .is_some_and(|v| v.percent() == 30)),
        "the volume reads back as 30 %"
    );

    connection.set_mute(id, true);
    assert!(
        wait(&rx, &mut graph, |g| g.volume(id).is_some_and(|v| v.mute)),
        "the sink reads back muted"
    );
    connection.set_mute(id, false);
    assert!(wait(&rx, &mut graph, |g| g
        .volume(id)
        .is_some_and(|v| !v.mute)));
}
