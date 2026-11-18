//! `otto-audio-service`: keeps the saved routing in force.
//!
//! It follows the PipeWire graph and links both ends of every saved link
//! whenever they exist and are not linked. The links it makes are its own, so
//! they last while it runs. It reads the routing file again whenever the file
//! changes, which is how the otto-audio window tells it about a new save.
//!
//! `--routing PATH` keeps another routing file instead of the person's, for
//! trying the service out on throwaway nodes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use otto_audio::keeper::Keeper;
use otto_audio::SERVICE_APP;
use otto_audio_graph::{Connection, Event, Graph, LinkLifetime, Routing};

/// How long a burst of graph changes is gathered before the service acts on
/// it. A node arrives as a node, then its ports one by one; acting on the
/// first event would see half a node. Short enough that a saved link is back
/// well within the 100 ms the spec allows.
const SETTLE: Duration = Duration::from_millis(40);

enum Message {
    Graph(Event),
    Reload,
}

fn main() {
    otto_kit::logging::init("info");
    let mut args = std::env::args().skip(1);
    let given = match (args.next().as_deref(), args.next()) {
        (Some("--routing"), Some(path)) => Some(PathBuf::from(path)),
        (Some("--version"), None) => {
            println!("otto-audio-service {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        (None, _) => None,
        _ => {
            eprintln!("usage: otto-audio-service [--routing PATH]");
            std::process::exit(2);
        }
    };
    let Some(path) = given.or_else(Routing::path) else {
        tracing::error!("no configuration directory, nothing to keep");
        std::process::exit(1);
    };

    let (tx, rx) = mpsc::channel();
    let connection = Connection::start(SERVICE_APP, {
        let tx = tx.clone();
        move |event| {
            let _ = tx.send(Message::Graph(event));
        }
    });
    watch_routing(&path, tx);

    let mut keeper = Keeper::new(load(&path));
    let mut graph = Graph::new();
    let mut due: Option<Instant> = None;
    loop {
        let message = match due {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(message) => Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            },
        };
        match message {
            Some(Message::Graph(event)) => {
                match event {
                    Event::Disconnected => {
                        tracing::info!("PipeWire went away, waiting for it");
                        keeper.disconnected();
                    }
                    Event::Connected => tracing::info!("connected to PipeWire"),
                    _ => {}
                }
                graph.apply(event);
                due.get_or_insert_with(|| Instant::now() + SETTLE);
            }
            Some(Message::Reload) => {
                tracing::info!(path = %path.display(), "routing changed, reading it again");
                keeper.set_routing(load(&path));
                due = Some(Instant::now());
            }
            None => {
                due = None;
                let plan = keeper.step(&graph, Instant::now());
                for (output, input) in plan.link {
                    tracing::info!(output, input, "restoring a saved link");
                    connection.link(output, input, LinkLifetime::Owned);
                }
                for (key, value) in plan.settings {
                    tracing::info!(key, value, "putting an engine setting back");
                    connection.set_setting(key, Some(&value));
                }
                for (output, input) in plan.given_up {
                    tracing::warn!(
                        output,
                        input,
                        "another program keeps removing a saved link, leaving it removed"
                    );
                }
            }
        }
    }
}

/// The saved routing, or an empty one when it cannot be read.
fn load(path: &Path) -> Routing {
    Routing::load_from(path).unwrap_or_else(|error| {
        tracing::warn!(%error, "the saved routing could not be read");
        Routing::default()
    })
}

/// Send [`Message::Reload`] whenever the routing file is written, replaced or
/// removed. The directory is watched rather than the file, since the window
/// replaces the file by renaming a new one over it.
fn watch_routing(path: &Path, tx: mpsc::Sender<Message>) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!(%error, "cannot create the routing directory, changes will not be followed");
        return;
    }
    let dir: PathBuf = dir.to_path_buf();
    let name = name.to_os_string();
    std::thread::spawn(move || {
        if let Err(error) = follow(&dir, &name, &tx) {
            tracing::warn!(%error, "following the routing file failed");
        }
    });
}

fn follow(dir: &Path, name: &std::ffi::OsStr, tx: &mpsc::Sender<Message>) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    // SAFETY: inotify_init1 takes no pointers; a negative result is checked.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let dir =
        CString::new(dir.as_os_str().to_os_string().into_vec()).map_err(std::io::Error::other)?;
    let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_DELETE | libc::IN_CREATE;
    // SAFETY: `dir` is a valid NUL-terminated string for the whole call.
    if unsafe { libc::inotify_add_watch(fd, dir.as_ptr(), mask) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Room for many events with names; inotify never splits one across reads.
    let mut buf = vec![0u8; 16 * 1024];
    let header = std::mem::size_of::<libc::inotify_event>();
    loop {
        // SAFETY: `buf` is valid for writes of its whole length.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        let n = n as usize;
        let mut offset = 0;
        let mut changed = false;
        while offset + header <= n {
            // SAFETY: the kernel wrote a whole event header at `offset`; the
            // read is unaligned because the buffer is plain bytes.
            let event: libc::inotify_event =
                unsafe { std::ptr::read_unaligned(buf.as_ptr().add(offset).cast()) };
            let start = offset + header;
            let end = (start + event.len as usize).min(n);
            let raw = &buf[start..end];
            let event_name = raw.split(|b| *b == 0).next().unwrap_or_default();
            if event_name == name.as_bytes() {
                changed = true;
            }
            offset = end;
        }
        if changed && tx.send(Message::Reload).is_err() {
            return Ok(());
        }
    }
}
