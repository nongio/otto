//! Print the live graph once it has synced, then every change as it happens.
//!
//! ```sh
//! cargo run -p otto-audio-graph --example dump
//! ```
//!
//! Read-only: it never links or unlinks anything.

use std::sync::mpsc;

use otto_audio_graph::{identity, Connection, Direction, Event, Graph};

fn main() {
    let (tx, rx) = mpsc::channel();
    let _connection = Connection::start("otto-audio-dump", move |event| {
        let _ = tx.send(event);
    });
    let mut graph = Graph::new();
    for event in rx {
        let synced = event == Event::Synced;
        if graph.is_synced() {
            println!("{event:?}");
        }
        graph.apply(event);
        if synced {
            print(&graph);
        }
    }
}

fn print(graph: &Graph) {
    for node in graph.nodes() {
        let who = identity(node).map(|i| i.to_string()).unwrap_or_default();
        println!(
            "{:>4} {:<28} {:<22} {:?} {who}",
            node.id,
            node.title(),
            node.media_class(),
            node.state
        );
        for direction in [Direction::Input, Direction::Output] {
            for port in graph.ports_of(node.id, direction) {
                println!(
                    "       {direction:?} {:>4} {} {:?}",
                    port.id,
                    port.name,
                    graph.port_media(port)
                );
            }
        }
    }
    for link in graph.links() {
        println!(
            "link {:>4}: {} -> {}",
            link.id, link.output_port, link.input_port
        );
    }
    for (key, value) in [
        "clock.rate",
        "clock.quantum",
        "clock.force-rate",
        "clock.force-quantum",
    ]
    .iter()
    .map(|k| (k, graph.setting(k)))
    {
        println!("{key} = {value:?}");
    }
}
