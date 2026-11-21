//! An equaliser against the running PipeWire server, sitting on a null sink
//! the test makes and removes itself. The person's own devices and streams
//! are never touched.
//!
//! Ignored by default because it needs a server, `pactl`, `pw-dump` and
//! `pw-play`:
//!
//! ```sh
//! cargo test -p otto-audio-graph --test live_effects -- --ignored
//! ```

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio_graph::{Connection, EffectKind, EffectSide, Event, Graph, Routing};

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

fn node_id(graph: &Graph, name: &str) -> Option<u32> {
    graph.nodes().find(|n| n.name() == name).map(|n| n.id)
}

/// Node `id`'s params as `pw-dump` prints them, with the white space taken
/// out: `"mix_0:Gain2",1.000000`.
fn dumped(id: u32) -> String {
    let out = Command::new("pw-dump")
        .arg(id.to_string())
        .output()
        .expect("run pw-dump");
    String::from_utf8_lossy(&out.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Wait until `pw-dump` of node `id` shows `needle`.
fn shows(id: u32, needle: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if dumped(id).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Play a fifth of a second of silence into the node called `target`.
fn play_silence(target: &str) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("silence.wav");
    let rate: u32 = 48_000;
    let bytes: u32 = rate / 5 * 4;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    wav.resize(wav.len() + bytes as usize, 0);
    std::fs::write(&path, wav).expect("write the silence");
    let status = Command::new("pw-play")
        .args(["--target", target, "-P"])
        .arg("{ state.restore-target = false state.restore-props = false }")
        .arg(&path)
        .status()
        .expect("run pw-play");
    assert!(status.success(), "pw-play failed");
}

#[test]
#[ignore = "needs a running PipeWire server, pactl, pw-dump and pw-play"]
fn an_equaliser_is_made_changed_live_and_removed() {
    let device = "otto-effects-test";
    let _sink = NullSink::new(device);
    let mut routing = Routing::default();
    let id = routing.add_effect(
        "Live test EQ",
        EffectKind::Equaliser,
        EffectSide::Output,
        device,
    );
    let effect = routing.effect(&id).expect("the effect").clone();

    let (tx, rx) = mpsc::channel();
    let host = Connection::start("otto-effects-test", move |event| {
        let _ = tx.send(event);
    });
    host.keep_effects(vec![effect.clone()]);
    let mut graph = Graph::new();
    let (sink, stream) = (effect.node_name(), effect.stream_name());
    assert!(
        wait(&rx, &mut graph, |g| {
            let (Some(stream), Some(device)) = (node_id(g, &stream), node_id(g, device)) else {
                return false;
            };
            node_id(g, &sink).is_some()
                && g.links()
                    .any(|l| l.output_node == stream && l.input_node == device)
        }),
        "the effect appears and plays on its device: {:?}",
        graph
            .nodes()
            .map(|n| n.name().to_string())
            .filter(|n| n.starts_with("otto"))
            .collect::<Vec<_>>()
    );
    let node = node_id(&graph, &sink).expect("the effect's sink");
    assert!(
        shows(node, r#""mix_0:Gain2",0.000000"#),
        "on: the dry path is shut"
    );

    // Another connection, as the window is, turns it off through its
    // controls: the node stays, the dry path opens.
    let (window_tx, window_rx) = mpsc::channel();
    let window = Connection::start("otto-effects-test-window", move |event| {
        let _ = window_tx.send(event);
    });
    let mut window_graph = Graph::new();
    assert!(wait(&window_rx, &mut window_graph, |g| node_id(g, &sink)
        .is_some()));
    let mut off = effect.clone();
    off.enabled = false;
    window.set_effect_controls(&off);
    // A filter chain reports the controls it was given once it runs.
    play_silence(&sink);
    assert!(
        shows(node, r#""mix_0:Gain2",1.000000"#),
        "off: the dry path is open"
    );
    assert_eq!(node_id(&graph, &sink), Some(node), "not made again");

    host.keep_effects(Vec::new());
    assert!(
        wait(&rx, &mut graph, |g| node_id(g, &sink).is_none()
            && node_id(g, &stream).is_none()),
        "the effect goes"
    );
}
