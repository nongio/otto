//! Print the defaults, every output and input with its volume, and the apps
//! playing sound, once the graph has synced.
//!
//! ```sh
//! cargo run -p otto-audio-graph --example mixer
//! ```
//!
//! Read-only: it never changes a volume, a mute or a default.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio_graph::{Connection, Endpoint, Event, Graph};

fn main() {
    let (tx, rx) = mpsc::channel();
    let _connection = Connection::start("otto-audio-mixer", move |event| {
        let _ = tx.send(event);
    });
    let mut graph = Graph::new();
    // Params arrive a moment after the sync; give them half a second.
    let mut until: Option<Instant> = None;
    loop {
        let wait = until.map_or(Duration::from_secs(5), |u| {
            u.saturating_duration_since(Instant::now())
        });
        match rx.recv_timeout(wait) {
            Ok(event) => {
                if event == Event::Synced {
                    until = Some(Instant::now() + Duration::from_millis(500));
                }
                graph.apply(event);
            }
            Err(_) => break,
        }
    }
    for endpoint in [Endpoint::Output, Endpoint::Input] {
        println!(
            "{endpoint:?}, default {:?}",
            graph.default_name(endpoint).unwrap_or("none")
        );
        for node in graph.endpoints(endpoint) {
            let mark = if graph.default_node(endpoint).map(|n| n.id) == Some(node.id) {
                '*'
            } else {
                ' '
            };
            let volume = graph
                .volume(node.id)
                .map(|v| format!("{} %{}", v.percent(), if v.mute { " muted" } else { "" }))
                .unwrap_or_default();
            println!(" {mark} {:>4} {:<40} {volume}", node.id, node.title());
        }
    }
    println!("Apps");
    for app in graph.app_streams() {
        println!(
            "   {:<20} {:?} {} %{}",
            app.app,
            app.nodes,
            app.volume.percent(),
            if app.volume.mute { " muted" } else { "" }
        );
    }
}
