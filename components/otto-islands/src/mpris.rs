//! MPRIS players over D-Bus: what they play, and the transport controls.
//!
//! One thread listens for players appearing, going away and changing
//! (`NameOwnerChanged`, `PropertiesChanged`, `Seeked`); another reads every
//! player when told to, merges them into one [`PlaybackInfo`] and wakes the
//! island only when that changed. `Position` is not signalled, so while a track
//! plays it is read again once a second for the progress bar.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use otto_kit::AppContext;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// Title reported when nothing is loaded; the island treats it as "no track".
pub const NO_MEDIA: &str = "No media";

/// How often a playing track's position is read again.
const POSITION_POLL: Duration = Duration::from_secs(1);
/// Signals tend to come in bursts (a track change is several property
/// changes); they are gathered for this long and read once.
const SIGNAL_SETTLE: Duration = Duration::from_millis(40);

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackInfo {
    pub track_title: String,
    pub track_artist: String,
    pub art_url: String,
    pub is_playing: bool,
    pub progress: f32,
    pub duration_secs: f32,
    /// The bus name of the player the controls drive.
    pub bus_name: String,
    /// The track's `mpris:trackid`, which `SetPosition` needs.
    pub track_id: String,
    /// Every name the playing app goes by: each player the track was read
    /// from (a browser publishes it twice, engine and integration shim) and
    /// their desktop entries. Any of them can match the app's window.
    pub player_names: Vec<String>,
    /// The processes behind those players, where D-Bus reports the app itself
    /// rather than a sandbox's proxy.
    pub player_pids: Vec<u32>,
}

impl PlaybackInfo {
    fn none() -> Self {
        Self {
            track_title: NO_MEDIA.to_string(),
            track_artist: String::new(),
            art_url: String::new(),
            is_playing: false,
            progress: 0.0,
            duration_secs: 0.0,
            bus_name: String::new(),
            track_id: String::new(),
            player_names: Vec::new(),
            player_pids: Vec::new(),
        }
    }

    pub fn has_track(&self) -> bool {
        !self.track_title.is_empty() && self.track_title != NO_MEDIA
    }
}

pub type SharedPlayback = Arc<Mutex<PlaybackInfo>>;

/// The reader's connection, shared with the controls.
static CONNECTION: OnceLock<Connection> = OnceLock::new();

/// Start following the session's MPRIS players.
pub fn start_monitor() -> SharedPlayback {
    let shared = Arc::new(Mutex::new(PlaybackInfo::none()));
    let (changed_tx, changed_rx) = mpsc::channel::<()>();

    thread::spawn(move || {
        if let Err(error) = listen(&changed_tx) {
            tracing::warn!(%error, "MPRIS signal listener stopped");
        }
    });

    let for_reader = shared.clone();
    thread::spawn(move || match Connection::session() {
        Ok(conn) => {
            let conn = CONNECTION.get_or_init(|| conn);
            read_loop(conn, &for_reader, &changed_rx);
        }
        Err(error) => tracing::warn!(%error, "no session bus for MPRIS"),
    });

    shared
}

/// Forward every player signal to the reader.
fn listen(changed: &mpsc::Sender<()>) -> zbus::Result<()> {
    use zbus::message::Type;
    use zbus::MatchRule;

    let conn = Connection::session()?;
    let rules = [
        MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg0ns("org.mpris.MediaPlayer2")?
            .build(),
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .path(MPRIS_PATH)?
            .build(),
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(PLAYER_IFACE)?
            .member("Seeked")?
            .path(MPRIS_PATH)?
            .build(),
    ];
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    for rule in rules {
        dbus.add_match_rule(rule)?;
    }
    for message in zbus::blocking::MessageIterator::from(&conn) {
        if message?.message_type() == Type::Signal && changed.send(()).is_err() {
            break;
        }
    }
    Ok(())
}

fn read_loop(conn: &Connection, shared: &SharedPlayback, changed: &mpsc::Receiver<()>) {
    loop {
        let info = match read_players(conn) {
            Ok(entries) => playback_from(select_player(entries)),
            Err(error) => {
                tracing::warn!(%error, "reading MPRIS players failed");
                PlaybackInfo::none()
            }
        };
        let playing = info.is_playing;
        if let Ok(mut current) = shared.lock() {
            if *current != info {
                *current = info;
                AppContext::request_wakeup();
            }
        }

        let signalled = if playing {
            match changed.recv_timeout(POSITION_POLL) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else if changed.recv().is_ok() {
            true
        } else {
            return;
        };
        if signalled {
            thread::sleep(SIGNAL_SETTLE);
            while changed.try_recv().is_ok() {}
        }
    }
}

/// One MPRIS player and what it is playing.
#[derive(Debug, Clone, Default, PartialEq)]
struct PlayerEntry {
    bus_name: String,
    /// The bus name without the prefix and instance suffix: "chromium" for
    /// `org.mpris.MediaPlayer2.chromium.instance42`.
    name: String,
    desktop_entry: String,
    /// The owning process, unless it is a sandbox's D-Bus proxy.
    pid: Option<u32>,
    status: String,
    title: String,
    artist: String,
    art_url: String,
    track_id: String,
    position_us: i64,
    length_us: i64,
    /// Processes of the other players merged into this one.
    merged_pids: Vec<u32>,
}

impl PlayerEntry {
    fn is_playing(&self) -> bool {
        self.status == "Playing"
    }

    fn is_stopped(&self) -> bool {
        self.status == "Stopped"
    }

    /// How much of the track this player actually describes. Browsers publish
    /// the same track from two players: the engine (title only, everything
    /// mashed into one string) and an integration shim that carries the art and
    /// a separate artist. The richer one is the one worth showing.
    fn richness(&self) -> u8 {
        u8::from(!self.art_url.is_empty()) * 2
            + u8::from(!self.artist.is_empty())
            + u8::from(!self.title.is_empty())
    }
}

fn short_name(bus_name: &str) -> String {
    let name = bus_name.strip_prefix(MPRIS_PREFIX).unwrap_or(bus_name);
    match name.split_once(".instance") {
        Some((name, _)) => name.to_string(),
        None => name.to_string(),
    }
}

fn read_players(conn: &Connection) -> zbus::Result<Vec<PlayerEntry>> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(conn)?;
    let mut entries = Vec::new();
    for bus_name in dbus.list_names()? {
        // playerctld republishes whichever player was last active: reading
        // it would show that player twice.
        if !bus_name.starts_with(MPRIS_PREFIX) || short_name(&bus_name) == "playerctld" {
            continue;
        }
        // A player that doesn't answer is left out; the others still show.
        match read_player(conn, bus_name.as_str()) {
            Ok(mut entry) => {
                entry.pid = dbus
                    .get_connection_unix_process_id(bus_name.as_ref())
                    .ok()
                    .filter(|&pid| !is_dbus_proxy(pid));
                entries.push(entry);
            }
            Err(error) => tracing::debug!(%bus_name, %error, "skipping MPRIS player"),
        }
    }
    Ok(entries)
}

/// Whether `pid` is a sandbox's D-Bus proxy (Flatpak's `xdg-dbus-proxy`),
/// which answers for the app without being it.
fn is_dbus_proxy(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .is_ok_and(|comm| comm.trim() == "xdg-dbus-proxy")
}

fn get_all(
    conn: &Connection,
    bus_name: &str,
    iface: &str,
) -> zbus::Result<HashMap<String, OwnedValue>> {
    let reply = conn.call_method(
        Some(bus_name),
        MPRIS_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &(iface,),
    )?;
    reply.body().deserialize()
}

fn read_player(conn: &Connection, bus_name: &str) -> zbus::Result<PlayerEntry> {
    let player = get_all(conn, bus_name, PLAYER_IFACE)?;
    let root = get_all(conn, bus_name, ROOT_IFACE).unwrap_or_default();

    let mut entry = PlayerEntry {
        bus_name: bus_name.to_string(),
        name: short_name(bus_name),
        desktop_entry: root
            .get("DesktopEntry")
            .and_then(string)
            .unwrap_or_default(),
        status: player
            .get("PlaybackStatus")
            .and_then(string)
            .unwrap_or_default(),
        position_us: player.get("Position").and_then(integer).unwrap_or(0),
        ..PlayerEntry::default()
    };
    if let Some(Value::Dict(metadata)) = player.get("Metadata").map(|v| &**v) {
        let metadata: HashMap<String, OwnedValue> = metadata.try_clone()?.try_into()?;
        entry.title = metadata
            .get("xesam:title")
            .and_then(string)
            .unwrap_or_default();
        entry.artist = metadata
            .get("xesam:artist")
            .and_then(strings)
            .map(|artists| artists.join(", "))
            .unwrap_or_default();
        entry.art_url = metadata
            .get("mpris:artUrl")
            .and_then(string)
            .unwrap_or_default();
        entry.track_id = metadata
            .get("mpris:trackid")
            .and_then(string)
            .unwrap_or_default();
        entry.length_us = metadata.get("mpris:length").and_then(integer).unwrap_or(0);
    }
    Ok(entry)
}

fn string(value: &OwnedValue) -> Option<String> {
    match &**value {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

fn strings(value: &OwnedValue) -> Option<Vec<String>> {
    match &**value {
        Value::Str(s) => Some(vec![s.to_string()]),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| match item {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

/// `mpris:length` and `Position` are `x`, but players send `t`, `i` or `u` too.
fn integer(value: &OwnedValue) -> Option<i64> {
    match &**value {
        Value::I64(v) => Some(*v),
        Value::U64(v) => i64::try_from(*v).ok(),
        Value::I32(v) => Some(i64::from(*v)),
        Value::U32(v) => Some(i64::from(*v)),
        Value::F64(v) => Some(*v as i64),
        _ => None,
    }
}

/// Pick the player to display, merging duplicate entries for the same track.
///
/// Prefer a playing player, then a paused one over a stopped one, then the one
/// describing the track best, then fill any gaps from the other entries for
/// the same track.
fn select_player(entries: Vec<PlayerEntry>) -> Option<PlayerEntry> {
    let rank = |e: &PlayerEntry| {
        if e.is_playing() {
            2
        } else if e.is_stopped() {
            0
        } else {
            1
        }
    };
    let top = entries.iter().map(rank).max()?;
    let mut candidates: Vec<PlayerEntry> = entries.into_iter().filter(|e| rank(e) == top).collect();

    let best_idx = candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, e)| e.richness())
        .map(|(i, _)| i)?;
    let mut best = candidates.swap_remove(best_idx);

    for other in candidates {
        // Same duration means the same track, published twice.
        if other.length_us != best.length_us || best.length_us == 0 {
            continue;
        }
        if best.title.is_empty() {
            best.title = other.title.clone();
        }
        if best.artist.is_empty() {
            best.artist = other.artist.clone();
        }
        if best.art_url.is_empty() {
            best.art_url = other.art_url.clone();
        }
        // The shim's position often does not advance; trust whichever is ahead.
        best.position_us = best.position_us.max(other.position_us);
        // Kept as another name the app goes by.
        best.desktop_entry = if best.desktop_entry.is_empty() {
            other.desktop_entry
        } else {
            best.desktop_entry
        };
        best.name = format!("{},{}", best.name, other.name);
        best.merged_pids.extend(other.pid);
    }
    Some(best)
}

fn playback_from(entry: Option<PlayerEntry>) -> PlaybackInfo {
    let Some(entry) = entry else {
        return PlaybackInfo::none();
    };
    let mut player_names: Vec<String> = entry.name.split(',').map(str::to_string).collect();
    if !entry.desktop_entry.is_empty() {
        player_names.push(entry.desktop_entry.clone());
    }
    // A stopped player has no track worth showing; a paused one keeps its title.
    let track_title = if entry.is_stopped() || entry.title.is_empty() {
        NO_MEDIA.to_string()
    } else {
        entry.title.clone()
    };
    let mut player_pids: Vec<u32> = entry
        .pid
        .into_iter()
        .chain(entry.merged_pids.iter().copied())
        .collect();
    player_pids.sort_unstable();
    player_pids.dedup();
    let (progress, duration_secs) = if entry.length_us > 0 {
        let progress = (entry.position_us as f64 / entry.length_us as f64).clamp(0.0, 1.0);
        (
            progress as f32,
            (entry.length_us as f64 / 1_000_000.0) as f32,
        )
    } else {
        (0.0, 0.0)
    };
    PlaybackInfo {
        track_title,
        is_playing: entry.is_playing(),
        track_artist: entry.artist,
        art_url: entry.art_url,
        progress,
        duration_secs,
        bus_name: entry.bus_name,
        track_id: entry.track_id,
        player_names,
        player_pids,
    }
}

/// A transport control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
    /// Seek to a fraction of the track, 0.0 to 1.0.
    SeekTo(f32),
}

/// Send `control` to the player `info` was read from, off the calling thread.
pub fn send(control: Control, info: &PlaybackInfo) {
    let info = info.clone();
    thread::spawn(move || {
        let Some(conn) = CONNECTION.get() else { return };
        let bus_name = info.bus_name.as_str();
        let call = |method: &str| -> zbus::Result<()> {
            conn.call_method(Some(bus_name), MPRIS_PATH, Some(PLAYER_IFACE), method, &())?;
            Ok(())
        };
        let result = match control {
            Control::PlayPause => call("PlayPause"),
            Control::Next => call("Next"),
            Control::Previous => call("Previous"),
            Control::SeekTo(fraction) if info.duration_secs > 0.0 => {
                let position_us = (f64::from(fraction.clamp(0.0, 1.0))
                    * f64::from(info.duration_secs)
                    * 1_000_000.0) as i64;
                seek_to(conn, &info, position_us)
            }
            Control::SeekTo(_) => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!(%error, ?control, bus_name, "MPRIS control failed");
        }
    });
}

fn seek_to(conn: &Connection, info: &PlaybackInfo, position_us: i64) -> zbus::Result<()> {
    let bus_name = info.bus_name.as_str();
    if let Ok(track) = ObjectPath::try_from(info.track_id.as_str()) {
        conn.call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some(PLAYER_IFACE),
            "SetPosition",
            &(track, position_us),
        )?;
        return Ok(());
    }
    // Without a track id only a relative seek is possible.
    let current_us =
        (f64::from(info.progress) * f64::from(info.duration_secs) * 1_000_000.0) as i64;
    conn.call_method(
        Some(bus_name),
        MPRIS_PATH,
        Some(PLAYER_IFACE),
        "Seek",
        &(position_us - current_us),
    )?;
    Ok(())
}

/// Ask the player to show itself. Returns whether it said it could.
pub fn raise(info: &PlaybackInfo) -> zbus::Result<bool> {
    let Some(conn) = CONNECTION.get() else {
        return Ok(false);
    };
    let root = get_all(conn, &info.bus_name, ROOT_IFACE)?;
    let can_raise = matches!(root.get("CanRaise").map(|v| &**v), Some(Value::Bool(true)));
    if can_raise {
        conn.call_method(
            Some(info.bus_name.as_str()),
            MPRIS_PATH,
            Some(ROOT_IFACE),
            "Raise",
            &(),
        )?;
    }
    Ok(can_raise)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        name: &str,
        status: &str,
        title: &str,
        artist: &str,
        art: &str,
        len: i64,
    ) -> PlayerEntry {
        PlayerEntry {
            bus_name: format!("{MPRIS_PREFIX}{name}"),
            name: name.to_string(),
            status: status.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            art_url: art.to_string(),
            length_us: len,
            ..PlayerEntry::default()
        }
    }

    #[test]
    fn instance_suffix_is_dropped_from_the_name() {
        assert_eq!(
            short_name("org.mpris.MediaPlayer2.chromium.instance1119969"),
            "chromium"
        );
        assert_eq!(short_name("org.mpris.MediaPlayer2.spotify"), "spotify");
    }

    #[test]
    fn browser_duplicate_players_are_merged() {
        // Chromium publishes the track twice: the engine player has no art and
        // no artist, the integration shim has both.
        let entries = vec![
            entry(
                "chromium",
                "Playing",
                "On Hold • The xx",
                "",
                "",
                224_179_773,
            ),
            entry(
                "plasma-browser-integration",
                "Playing",
                "On Hold",
                "The xx",
                "file:///tmp/art.png",
                224_179_773,
            ),
        ];
        let picked = select_player(entries).unwrap();
        assert_eq!(picked.art_url, "file:///tmp/art.png");
        assert_eq!(picked.artist, "The xx");
        assert_eq!(picked.title, "On Hold");
        assert_eq!(picked.name, "plasma-browser-integration,chromium");
    }

    #[test]
    fn playing_player_wins_over_paused() {
        let entries = vec![
            entry("vlc", "Paused", "Old", "A", "file:///a.png", 100),
            entry("spotify", "Playing", "New", "B", "", 200),
        ];
        let picked = select_player(entries).unwrap();
        assert_eq!(picked.title, "New");
        assert_eq!(picked.name, "spotify");
    }

    #[test]
    fn paused_player_wins_over_stopped() {
        let entries = vec![
            entry("vlc", "Stopped", "Old", "A", "file:///a.png", 100),
            entry("spotify", "Paused", "Kept", "", "", 200),
        ];
        let info = playback_from(select_player(entries));
        assert_eq!(info.track_title, "Kept");
    }

    #[test]
    fn unrelated_tracks_are_not_merged() {
        let entries = vec![
            entry("a", "Playing", "One", "", "", 100),
            entry("b", "Playing", "Two", "X", "file:///b.png", 999),
        ];
        let picked = select_player(entries).unwrap();
        assert_eq!(picked.title, "Two");
        assert_eq!(picked.name, "b");
    }

    #[test]
    fn desktop_entry_is_a_name_the_app_goes_by() {
        let mut spotify = entry("spotify", "Playing", "Song", "", "", 100);
        spotify.desktop_entry = "com.spotify.Client".into();
        let info = playback_from(Some(spotify));
        assert_eq!(info.player_names, ["spotify", "com.spotify.Client"]);
    }

    #[test]
    fn no_player_is_no_media() {
        let info = playback_from(None);
        assert!(!info.has_track());
        assert!(info.player_names.is_empty());
    }
}
