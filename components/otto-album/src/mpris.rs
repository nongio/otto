//! The MPRIS side: what is playing, and telling the player to play or pause.
//!
//! A background thread owns the D-Bus connection and keeps a snapshot of the
//! current track in a mutex; the render side reads it every frame. Cover art
//! crosses the thread boundary as encoded bytes, not as a decoded image —
//! Skia images are not `Send`, and decoding belongs on the thread that draws.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{fdo::DBusProxy, proxy::Builder as ProxyBuilder, Connection, Proxy};
use zbus::names::OwnedBusName;
use zbus::zvariant::Value;

const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// What the player says, as of the last poll.
#[derive(Clone, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    /// Track length and the last reported position, in microseconds.
    pub length: u64,
    position: u64,
    /// When that position was reported, so it can be carried forward.
    reported_at: Option<Instant>,
    /// Encoded cover art, and the URL it came from.
    pub art: Option<Arc<Vec<u8>>>,
    pub art_url: String,
    /// Bumped whenever `art` changes, so the renderer knows to re-decode.
    pub art_generation: u64,
    /// False until a player has actually been found.
    pub connected: bool,
}

impl NowPlaying {
    /// Position now: the player reports lazily, so the elapsed wall time since
    /// the last report is added while it is playing.
    pub fn position(&self) -> u64 {
        match (self.playing, self.reported_at) {
            (true, Some(at)) => self.position + at.elapsed().as_micros() as u64,
            _ => self.position,
        }
    }
}

/// A handle the UI keeps: read the snapshot, ask the player to toggle.
#[derive(Clone)]
pub struct Mpris {
    state: Arc<Mutex<NowPlaying>>,
    commands: std::sync::mpsc::Sender<Command>,
}

enum Command {
    PlayPause,
}

impl Mpris {
    /// Start polling in the background. Returns immediately; the snapshot
    /// stays empty until a player shows up.
    pub fn spawn() -> Self {
        let state = Arc::new(Mutex::new(NowPlaying::default()));
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("mpris".into())
            .spawn(move || run(worker_state, rx))
            .expect("spawn mpris thread");
        Self {
            state,
            commands: tx,
        }
    }

    pub fn snapshot(&self) -> NowPlaying {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn play_pause(&self) {
        let _ = self.commands.send(Command::PlayPause);
    }
}

fn run(state: Arc<Mutex<NowPlaying>>, rx: std::sync::mpsc::Receiver<Command>) {
    let Ok(connection) = Connection::session() else {
        tracing::warn!("no session bus; MPRIS is unavailable");
        return;
    };

    loop {
        match find_player(&connection) {
            Some(name) => {
                tracing::info!("following {}", name.as_str());
                follow(&connection, &name, &state, &rx)
            }
            None => {
                if let Ok(mut s) = state.lock() {
                    s.connected = false;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// The first player on the bus, preferring one that is actually playing.
fn find_player(connection: &Connection) -> Option<OwnedBusName> {
    let dbus = DBusProxy::new(connection).ok()?;
    let names = dbus.list_names().ok()?;
    let players: Vec<_> = names
        .into_iter()
        .filter(|n| n.starts_with(PLAYER_PREFIX))
        .collect();

    players
        .iter()
        .find(|name| {
            player_proxy(connection, name)
                .and_then(|p| p.get_property::<String>("PlaybackStatus").ok())
                .is_some_and(|status| status == "Playing")
        })
        .or_else(|| players.first())
        .cloned()
}

/// Property caching is deliberately off. With it on, `get_property` answers
/// from a cache that is only refreshed by a `PropertiesChanged` listener, and
/// this poller never pumps one — so every read returns whatever the track was
/// when the proxy was built, and the widget freezes on the first song.
fn player_proxy<'a>(connection: &'a Connection, name: &OwnedBusName) -> Option<Proxy<'a>> {
    ProxyBuilder::new_bare(connection)
        .destination(name.clone())
        .ok()?
        .path(PLAYER_PATH)
        .ok()?
        .interface(PLAYER_IFACE)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()
}

/// Poll one player until it goes away.
fn follow(
    connection: &Connection,
    name: &OwnedBusName,
    state: &Arc<Mutex<NowPlaying>>,
    rx: &std::sync::mpsc::Receiver<Command>,
) {
    let Some(player) = player_proxy(connection, name) else {
        return;
    };
    let mut generation = 0u64;
    let mut last_art_url = String::new();
    let mut art: Option<Arc<Vec<u8>>> = None;

    loop {
        // Commands first, so a click is not delayed by a poll.
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::PlayPause => {
                    let _ = player.call_method("PlayPause", &());
                }
            }
        }

        let Ok(status) = player.get_property::<String>("PlaybackStatus") else {
            return; // player vanished
        };
        let metadata = player
            .get_property::<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>(
                "Metadata",
            )
            .unwrap_or_default();
        let position = player.get_property::<i64>("Position").unwrap_or(0).max(0) as u64;

        let art_url = string_of(metadata.get("mpris:artUrl")).unwrap_or_default();
        if art_url != last_art_url {
            art = load_art(&art_url);
            last_art_url = art_url.clone();
            generation += 1;
        }

        if let Ok(mut s) = state.lock() {
            s.title = string_of(metadata.get("xesam:title")).unwrap_or_default();
            s.artist = first_string_of(metadata.get("xesam:artist")).unwrap_or_default();
            s.album = string_of(metadata.get("xesam:album")).unwrap_or_default();
            s.length = metadata
                .get("mpris:length")
                .and_then(|v| u64::try_from(v.downcast_ref::<Value>().ok()?).ok())
                .unwrap_or(0);
            s.playing = status == "Playing";
            s.position = position;
            s.reported_at = Some(Instant::now());
            s.art = art.clone();
            s.art_url = last_art_url.clone();
            s.art_generation = generation;
            s.connected = true;
        }

        tracing::debug!(
            title = %string_of(metadata.get("xesam:title")).unwrap_or_default(),
            status = %status,
            keys = metadata.len(),
            "mpris poll"
        );

        std::thread::sleep(Duration::from_millis(700));
    }
}

/// Cover art: a local file is read directly, an http URL is fetched with curl
/// so the app does not carry an HTTP stack for one request per track.
fn load_art(url: &str) -> Option<Arc<Vec<u8>>> {
    if url.is_empty() {
        return None;
    }
    if let Some(path) = url.strip_prefix("file://") {
        let path = percent_decode(path);
        return std::fs::read(path).ok().map(Arc::new);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let out = std::process::Command::new("curl")
            .args(["-sL", "--max-time", "8", url])
            .output()
            .ok()?;
        if out.status.success() && !out.stdout.is_empty() {
            return Some(Arc::new(out.stdout));
        }
    }
    None
}

fn string_of(value: Option<&zbus::zvariant::OwnedValue>) -> Option<String> {
    String::try_from(value?.downcast_ref::<Value>().ok()?).ok()
}

/// `xesam:artist` is a list; the first entry is the one to show.
fn first_string_of(value: Option<&zbus::zvariant::OwnedValue>) -> Option<String> {
    let list = Vec::<String>::try_from(value?.downcast_ref::<Value>().ok()?).ok()?;
    list.into_iter().next()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
