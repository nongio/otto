//! Where the playing track's sound goes: which PipeWire stream carries it, or
//! whether it plays on another device altogether.
//!
//! A thread follows every audio output stream in the PipeWire graph and keeps
//! a snapshot of them. [`route`] matches the MPRIS player against that
//! snapshot: by process (the stream comes from the player or one of its
//! children, as a browser's audio service does) or, when the player's process
//! is hidden behind a sandbox, by name. The meter then captures that stream
//! itself, so the bars follow the player to any speaker.
//!
//! A track that plays while nothing plays locally is on another device: a
//! phone or a speaker driven over the network (Spotify Connect, a cast). The
//! island says so instead of drawing bars that would never move.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use otto_kit::AppContext;

/// One audio output stream in the PipeWire graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamNode {
    /// `object.serial`, which a capture stream targets.
    pub serial: u32,
    /// `application.process.id`, when the stream reports one.
    pub pid: Option<u32>,
    /// Names the stream goes by, lowercased: application name, binary, id.
    pub names: Vec<String>,
    /// Whether the node is running, i.e. actually playing.
    pub running: bool,
}

/// Where the meter should listen for the playing track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The player's own stream, by `object.serial`.
    Stream(u32),
    /// The player's stream is not recognised, but something plays locally:
    /// the default output's monitor is the best guess.
    DefaultOutput,
    /// Nothing plays locally: the track is on another device.
    Elsewhere,
}

/// The processes and names an MPRIS player goes by.
#[derive(Debug, Clone, Copy)]
pub struct Player<'a> {
    pub pids: &'a [u32],
    pub names: &'a [String],
}

/// Decide where `player`'s sound goes among `streams`. `parent_of` gives a
/// process's parent, so a stream from a child of the player counts as its own.
pub fn route(
    player: Player<'_>,
    streams: &[StreamNode],
    parent_of: impl Fn(u32) -> Option<u32>,
) -> Route {
    let wanted = player_names(player.names);
    let belongs = |stream: &StreamNode| {
        stream.pid.is_some_and(|pid| {
            player
                .pids
                .iter()
                .any(|&ancestor| descends_from(pid, ancestor, &parent_of))
        }) || stream.names.iter().any(|name| wanted.contains(name))
    };
    // The newest of the player's running streams: a browser may keep an old
    // one open for another tab.
    if let Some(stream) = streams
        .iter()
        .filter(|s| s.running && belongs(s))
        .max_by_key(|s| s.serial)
    {
        return Route::Stream(stream.serial);
    }
    if streams.iter().any(|s| s.running) {
        Route::DefaultOutput
    } else {
        Route::Elsewhere
    }
}

/// The names to look for in a stream's properties: each player name, and the
/// last part of a reverse-DNS desktop entry ("firefox" for
/// `org.mozilla.firefox`).
fn player_names(names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for name in names {
        let name = name.to_lowercase();
        if let Some((_, last)) = name.rsplit_once('.') {
            out.push(last.to_string());
        }
        out.push(name);
    }
    out
}

/// Whether `pid` is `ancestor` or one of its descendants.
fn descends_from(pid: u32, ancestor: u32, parent_of: impl Fn(u32) -> Option<u32>) -> bool {
    // Deep enough for any real process tree, and a guard against a cycle
    // read from a /proc that changed underneath.
    const MAX_DEPTH: usize = 64;
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if current == ancestor {
            return true;
        }
        match parent_of(current) {
            Some(parent) if parent > 1 && parent != current => current = parent,
            _ => return false,
        }
    }
    false
}

/// A process's parent, read from `/proc/<pid>/stat`.
pub fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesised and may itself contain ") ".
    let (_, rest) = stat.rsplit_once(") ")?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// The audio output streams in the PipeWire graph, kept up to date on a
/// thread of their own.
#[derive(Clone)]
pub struct AudioStreams {
    shared: Arc<Mutex<Snapshot>>,
}

#[derive(Default)]
struct Snapshot {
    streams: Vec<StreamNode>,
    /// Bumped on every change, so a reader can tell a stale answer.
    generation: u64,
}

impl AudioStreams {
    /// Start following the graph. A PipeWire failure is logged and the
    /// snapshot then stays empty.
    pub fn start() -> Self {
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let for_thread = shared.clone();
        thread::spawn(move || {
            if let Err(error) = watch(&for_thread) {
                tracing::error!(%error, "PipeWire stream watcher failed");
            }
        });
        Self { shared }
    }

    /// The streams and the snapshot's generation.
    pub fn snapshot(&self) -> (Vec<StreamNode>, u64) {
        self.shared
            .lock()
            .map(|s| (s.streams.clone(), s.generation))
            .unwrap_or_default()
    }
}

fn stream_node(
    serial: u32,
    props: &pipewire::spa::utils::dict::DictRef,
    running: bool,
) -> StreamNode {
    let names = [
        "application.name",
        "application.process.binary",
        "application.id",
        "pipewire.access.portal.app_id",
    ]
    .iter()
    .filter_map(|key| props.get(key))
    .filter(|name| !name.is_empty())
    .map(str::to_lowercase)
    .collect();
    StreamNode {
        serial,
        pid: props
            .get("application.process.id")
            .and_then(|p| p.parse().ok()),
        names,
        running,
    }
}

fn watch(shared: &Arc<Mutex<Snapshot>>) -> Result<(), pipewire::Error> {
    use pipewire as pw;
    use pw::node::{Node, NodeListener, NodeState};
    use pw::types::ObjectType;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let registry_weak = registry.downgrade();

    // Bound nodes by global id; dropping one unbinds it.
    struct Bound {
        _node: Node,
        _listener: NodeListener,
    }
    let bound: Rc<RefCell<HashMap<u32, Bound>>> = Rc::default();
    let nodes: Rc<RefCell<HashMap<u32, StreamNode>>> = Rc::default();

    let publish = {
        let shared = shared.clone();
        let nodes = nodes.clone();
        move || {
            let streams: Vec<StreamNode> = nodes.borrow().values().cloned().collect();
            if let Ok(mut snapshot) = shared.lock() {
                let mut sorted = streams;
                sorted.sort_by_key(|s| s.serial);
                if snapshot.streams != sorted {
                    snapshot.streams = sorted;
                    snapshot.generation += 1;
                    AppContext::request_wakeup();
                }
            }
        }
    };
    let publish = Rc::new(publish);

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let bound = bound.clone();
            let nodes = nodes.clone();
            let publish = publish.clone();
            move |global| {
                if global.type_ != ObjectType::Node {
                    return;
                }
                let Some(props) = global.props else { return };
                if props.get("media.class") != Some("Stream/Output/Audio") {
                    return;
                }
                let Some(serial) = props.get("object.serial").and_then(|s| s.parse().ok()) else {
                    return;
                };
                let Some(registry) = registry_weak.upgrade() else {
                    return;
                };
                let node: Node = match registry.bind(global) {
                    Ok(node) => node,
                    Err(error) => {
                        tracing::debug!(%error, id = global.id, "binding a stream node failed");
                        return;
                    }
                };
                let id = global.id;
                let listener = node
                    .add_listener_local()
                    .info({
                        let nodes = nodes.clone();
                        let publish = publish.clone();
                        move |info| {
                            let running = matches!(info.state(), NodeState::Running);
                            let stream = match info.props() {
                                Some(props) => stream_node(serial, props, running),
                                None => StreamNode {
                                    running,
                                    ..nodes.borrow().get(&id).cloned().unwrap_or_default()
                                },
                            };
                            nodes.borrow_mut().insert(id, stream);
                            publish();
                        }
                    })
                    .register();
                nodes
                    .borrow_mut()
                    .insert(id, stream_node(serial, props, false));
                bound.borrow_mut().insert(
                    id,
                    Bound {
                        _node: node,
                        _listener: listener,
                    },
                );
                publish();
            }
        })
        .global_remove({
            let bound = bound.clone();
            let nodes = nodes.clone();
            let publish = publish.clone();
            move |id| {
                bound.borrow_mut().remove(&id);
                if nodes.borrow_mut().remove(&id).is_some() {
                    publish();
                }
            }
        })
        .register();

    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(serial: u32, pid: Option<u32>, names: &[&str], running: bool) -> StreamNode {
        StreamNode {
            serial,
            pid,
            names: names.iter().map(|n| n.to_string()).collect(),
            running,
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// A process tree: browser 100, its audio service 120, an unrelated 200.
    fn parent_of(pid: u32) -> Option<u32> {
        match pid {
            120 => Some(100),
            100 | 200 => Some(1),
            _ => None,
        }
    }

    #[test]
    fn the_player_s_own_stream_is_captured() {
        let player = Player {
            pids: &[200],
            names: &names(&["spotify"]),
        };
        let streams = [
            stream(10, Some(100), &["firefox"], true),
            stream(11, Some(200), &["spotify"], true),
        ];
        assert_eq!(route(player, &streams, parent_of), Route::Stream(11));
    }

    #[test]
    fn a_child_process_s_stream_belongs_to_the_player() {
        // Chromium plays from its audio service, a child of the browser.
        let player = Player {
            pids: &[100],
            names: &names(&["chromium"]),
        };
        let streams = [stream(10, Some(120), &["audio service"], true)];
        assert_eq!(route(player, &streams, parent_of), Route::Stream(10));
    }

    #[test]
    fn a_sandboxed_player_is_matched_by_name() {
        // A Flatpak's D-Bus peer is its proxy, not the app.
        let player = Player {
            pids: &[],
            names: &names(&["spotify", "com.spotify.Client"]),
        };
        let streams = [stream(10, Some(4242), &["spotify"], true)];
        assert_eq!(route(player, &streams, parent_of), Route::Stream(10));
    }

    #[test]
    fn a_reverse_dns_desktop_entry_matches_its_last_part() {
        let player = Player {
            pids: &[],
            names: &names(&["org.mozilla.firefox"]),
        };
        let streams = [stream(10, None, &["firefox"], true)];
        assert_eq!(route(player, &streams, parent_of), Route::Stream(10));
    }

    #[test]
    fn a_paused_stream_is_not_the_one_playing() {
        let player = Player {
            pids: &[200],
            names: &names(&["spotify"]),
        };
        let streams = [
            stream(10, Some(200), &["spotify"], false),
            stream(11, Some(100), &["firefox"], true),
        ];
        assert_eq!(route(player, &streams, parent_of), Route::DefaultOutput);
    }

    #[test]
    fn nothing_playing_locally_is_another_device() {
        let player = Player {
            pids: &[200],
            names: &names(&["spotify"]),
        };
        let streams = [stream(10, Some(200), &["spotify"], false)];
        assert_eq!(route(player, &streams, parent_of), Route::Elsewhere);
        assert_eq!(route(player, &[], parent_of), Route::Elsewhere);
    }

    #[test]
    fn the_newest_of_the_player_s_streams_wins() {
        let player = Player {
            pids: &[100],
            names: &names(&["chromium"]),
        };
        let streams = [
            stream(10, Some(120), &["chromium"], true),
            stream(15, Some(120), &["chromium"], true),
        ];
        assert_eq!(route(player, &streams, parent_of), Route::Stream(15));
    }

    #[test]
    fn a_parent_is_not_a_descendant() {
        assert!(!descends_from(100, 120, parent_of));
        assert!(descends_from(120, 100, parent_of));
        assert!(!descends_from(200, 100, parent_of));
    }

    #[test]
    fn this_process_parent_is_read_from_proc() {
        let own = std::process::id();
        assert_eq!(parent_pid(own), Some(std::os::unix::process::parent_id()));
    }
}
