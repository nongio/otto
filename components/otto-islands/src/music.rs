//! Music activity: an MPRIS bridge through playerctl, drawn with the audio
//! visualiser.
//!
//! A background thread polls playerctl. While a track is loaded and its
//! player's window isn't the focused one, [`MusicMonitor`] keeps a live
//! activity on the island; when the track stops it goes away.

use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use otto_kit::typography::TextStyle;
use otto_kit::utils::extract_accent_color;
use otto_kit::AppContext;
use skia_safe::{Canvas, Color, Data, Image, Paint, RRect, Rect};

use crate::audio_viz::{self, BarAnimator, BarStyle, LevelMeter, BAR_COUNT};
use crate::state::SharedState;
use crate::IslandMode;

// ---------------------------------------------------------------------------
// Music player actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MusicAction {
    PlayPause,
    SkipNext,
    SkipPrev,
    /// Seek to a position (0.0–1.0).
    Seek(f32),
    /// Bring the player's window forward.
    FocusPlayer,
}

/// Title reported when nothing is loaded; the island treats it as "no track".
const NO_MEDIA: &str = "No media";

/// `app_id` the music activity is published under.
pub const MUSIC_APP_ID: &str = "org.otto.music";
/// Buffer for the equaliser child subsurface, sized for the largest bar mode.
pub const EQ_BUF_W: i32 = 220;
pub const EQ_BUF_H: i32 = 32;
/// Bars in the compact pill.
const COMPACT_BARS: usize = 4;
/// Seconds a track may be gone before the island lets go of it, so skipping
/// to the next one doesn't close and reopen it.
const GONE_GRACE_SECS: f64 = 3.0;

// ---------------------------------------------------------------------------
// MusicActivityRenderer
// ---------------------------------------------------------------------------

pub struct MusicActivityRenderer {
    pub title: String,
    pub artist: String,
    pub album_art: Option<Image>,
    pub is_playing: bool,
    pub progress: f32,
    pub duration_secs: f32,
    pub accent: Color,
    pub levels: [f32; BAR_COUNT],
    /// Currently pressed control (for visual feedback).
    pub pressed: Option<MusicAction>,
}

impl MusicActivityRenderer {
    /// Island size for each mode.
    pub fn mode_size(mode: IslandMode) -> (f32, f32) {
        match mode {
            IslandMode::Mini => (28.0, 28.0),
            IslandMode::Compact => (220.0, 30.0),
            IslandMode::Expanded => (340.0, 120.0),
        }
    }

    /// Draw everything except the bars, which live on their own subsurface.
    pub fn draw_without_eq(&self, canvas: &Canvas, mode: IslandMode, w: f32, h: f32) {
        match mode {
            IslandMode::Compact => self.draw_compact(canvas, w, h),
            IslandMode::Expanded => self.draw_open(canvas, w, h),
            // The bars are all a mini island shows.
            IslandMode::Mini => {}
        }
    }

    /// Size and offset of the bars, relative to the island's top-left:
    /// (w, h, x, y).
    pub fn eq_layout(mode: IslandMode, w: f32, h: f32) -> (f32, f32, f32, f32) {
        match mode {
            IslandMode::Mini => (w, h, 0.0, 0.0),
            IslandMode::Compact => {
                let v_pad = 7.0;
                let h_pad = 8.0;
                let eq_w = compact_bars_width();
                (eq_w, h - v_pad * 2.0, w - h_pad - eq_w, v_pad)
            }
            IslandMode::Expanded => {
                let pad = 12.0;
                let rx = pad + (h - pad * 2.0) + pad;
                (w - rx - pad, 22.0, rx, pad + 34.0)
            }
        }
    }

    /// Draw only the bars. The canvas origin is the top-left of the bar area.
    pub fn draw_eq_only(&self, canvas: &Canvas, mode: IslandMode, w: f32, h: f32) {
        let style = match mode {
            IslandMode::Mini => BarStyle::Mini,
            IslandMode::Compact => BarStyle::Compact(COMPACT_BARS),
            IslandMode::Expanded => BarStyle::Large,
        };
        audio_viz::draw_bars(
            canvas,
            Rect::from_xywh(0.0, 0.0, w, h),
            &self.levels,
            self.accent,
            style,
        );
    }

    /// Hit test within the expanded player. `lx`, `ly` are local coords
    /// relative to the top-left of the expanded island content.
    pub fn hit_test_expanded(lx: f32, ly: f32, w: f32, h: f32) -> Option<MusicAction> {
        let pad = 12.0;
        let art_size = h - pad * 2.0;
        let rx = pad + art_size + pad;
        let rw = w - rx - pad;

        if (pad..=pad + art_size).contains(&lx) && (pad..=pad + art_size).contains(&ly) {
            return Some(MusicAction::FocusPlayer);
        }

        // Progress bar — generous vertical hit zone (±8px around the bar).
        let prog_y = h - pad - 26.0;
        if lx >= rx && lx <= rx + rw && (ly - prog_y).abs() <= 8.0 {
            let frac = ((lx - rx) / rw).clamp(0.0, 1.0);
            return Some(MusicAction::Seek(frac));
        }

        let ctrl_y = h - pad - 6.0;
        let ctrl_cx = rx + rw / 2.0;
        let icon_gap = 36.0;
        let hit_r = 18.0;
        let within = |cx: f32| {
            let (dx, dy) = (lx - cx, ly - ctrl_y);
            dx * dx + dy * dy <= hit_r * hit_r
        };
        if within(ctrl_cx - icon_gap) {
            Some(MusicAction::SkipPrev)
        } else if within(ctrl_cx) {
            Some(MusicAction::PlayPause)
        } else if within(ctrl_cx + icon_gap) {
            Some(MusicAction::SkipNext)
        } else {
            None
        }
    }

    fn draw_compact(&self, canvas: &Canvas, w: f32, h: f32) {
        let v_pad = 7.0;
        let h_pad = 10.0;
        let art_size = h - v_pad * 2.0;
        let art_x = h_pad;
        let art_y = v_pad;

        self.draw_art(canvas, art_x, art_y, art_size);

        let eq_x = w - h_pad - compact_bars_width();
        let text_x = art_x + art_size + h_pad;
        let text_max_w = eq_x - text_x - h_pad;

        let mid = h / 2.0;
        Self::draw_text(
            canvas,
            &self.title,
            text_x,
            mid - 1.0,
            11.0,
            220,
            text_max_w,
        );

        let af = font(9.5);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(140, 170, 170, 170));
        let artist = trim_to_width(&self.artist, &af, text_max_w.max(20.0));
        canvas.draw_str(artist, (text_x, mid + 10.0), &af, &ap);
    }

    fn draw_open(&self, canvas: &Canvas, w: f32, h: f32) {
        let pad = 12.0;
        let art_size = h - pad * 2.0;

        self.draw_art(canvas, pad, pad, art_size);

        let rx = pad + art_size + pad;
        let rw = w - rx - pad;

        Self::draw_text(canvas, &self.title, rx, pad + 13.0, 13.0, 255, rw);
        let af = font(10.0);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(150, 180, 180, 180));
        let artist = trim_to_width(&self.artist, &af, rw.max(20.0));
        canvas.draw_str(artist, (rx, pad + 27.0), &af, &ap);

        let prog_y = h - pad - 26.0;
        self.draw_progress_large(canvas, rx, prog_y, rw);

        let ctrl_y = h - pad - 6.0;
        let ctrl_cx = rx + rw / 2.0;
        let icon_gap = 36.0;
        let icon_size = 12.0;
        let alpha = |action: MusicAction, idle: u8| {
            if self.pressed == Some(action) {
                120
            } else {
                idle
            }
        };

        self.draw_skip_prev_a(
            canvas,
            ctrl_cx - icon_gap,
            ctrl_y,
            icon_size,
            alpha(MusicAction::SkipPrev, 200),
        );
        self.draw_play_pause_a(
            canvas,
            ctrl_cx,
            ctrl_y,
            icon_size + 2.0,
            alpha(MusicAction::PlayPause, 255),
        );
        self.draw_skip_next_a(
            canvas,
            ctrl_cx + icon_gap,
            ctrl_y,
            icon_size,
            alpha(MusicAction::SkipNext, 200),
        );
    }

    fn draw_progress_large(&self, canvas: &Canvas, x: f32, y: f32, w: f32) {
        let h = 4.0f32;
        let r = h / 2.0;

        // Track background
        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(Color::from_argb(50, 255, 255, 255));
        canvas.draw_rrect(RRect::new_rect_xy(Rect::from_xywh(x, y, w, h), r, r), &bg);

        // Fill
        let fill_w = (w * self.progress).max(h);
        let mut fg = Paint::default();
        fg.set_anti_alias(true);
        fg.set_color(self.accent);
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_xywh(x, y, fill_w, h), r, r),
            &fg,
        );

        // Time labels
        let time_y = y + h + 14.0;
        let tf = font(9.5);
        let mut tp = Paint::default();
        tp.set_anti_alias(true);
        tp.set_color(Color::from_argb(140, 200, 200, 200));

        let elapsed = format_time_secs(self.progress * self.duration_secs);
        canvas.draw_str(&elapsed, (x, time_y), &tf, &tp);

        let remaining = format_time_secs((1.0 - self.progress) * self.duration_secs);
        let neg = format!("-{remaining}");
        let (rw, _) = tf.measure_str(&neg, None);
        canvas.draw_str(&neg, (x + w - rw, time_y), &tf, &tp);
    }

    fn draw_art(&self, canvas: &Canvas, x: f32, y: f32, size: f32) {
        let r = size * 0.18;
        let dst = Rect::from_xywh(x, y, size, size);
        if let Some(art) = &self.album_art {
            canvas.save();
            canvas.clip_rrect(
                RRect::new_rect_xy(dst, r, r),
                skia_safe::ClipOp::Intersect,
                true,
            );
            let src = Rect::from_xywh(0.0, 0.0, art.width() as f32, art.height() as f32);
            canvas.draw_image_rect(
                art,
                Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
                dst,
                &Paint::default(),
            );
            canvas.restore();
        } else {
            let mut ph = Paint::default();
            ph.set_anti_alias(true);
            ph.set_color(Color::from_argb(55, 255, 255, 255));
            canvas.draw_rrect(RRect::new_rect_xy(dst, r, r), &ph);
            let nf = font(size * 0.42);
            let mut np = Paint::default();
            np.set_anti_alias(true);
            np.set_color(Color::from_argb(130, 255, 255, 255));
            canvas.draw_str("\u{266A}", (x + size * 0.2, y + size * 0.68), &nf, &np);
        }
    }

    fn draw_text(canvas: &Canvas, text: &str, x: f32, y: f32, size: f32, alpha: u8, max_w: f32) {
        let f = font_bold(size);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(alpha, 255, 255, 255));
        let label = trim_to_width(text, &f, max_w.max(20.0));
        canvas.draw_str(label, (x, y), &f, &paint);
    }

    fn draw_play_pause_a(&self, canvas: &Canvas, cx: f32, cy: f32, size: f32, alpha: u8) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(alpha, 255, 255, 255));

        if self.is_playing {
            let bar_w = size * 0.28;
            let bar_h = size * 0.8;
            let gap = size * 0.22;
            let lx = cx - gap / 2.0 - bar_w;
            let rx = cx + gap / 2.0;
            let ty = cy - bar_h / 2.0;
            canvas.draw_rect(Rect::from_xywh(lx, ty, bar_w, bar_h), &paint);
            canvas.draw_rect(Rect::from_xywh(rx, ty, bar_w, bar_h), &paint);
        } else {
            let mut b = skia_safe::PathBuilder::new();
            let h = size * 0.85;
            let w = h * 0.866;
            b.move_to((cx - w / 2.0, cy - h / 2.0));
            b.line_to((cx + w / 2.0, cy));
            b.line_to((cx - w / 2.0, cy + h / 2.0));
            b.close();
            canvas.draw_path(&b.detach(), &paint);
        }
    }

    fn draw_skip_prev_a(&self, canvas: &Canvas, cx: f32, cy: f32, size: f32, alpha: u8) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(alpha, 255, 255, 255));

        let h = size * 0.75;
        let w = h * 0.866;
        let bar_w = size * 0.18;
        let tx = cx + bar_w / 2.0;
        let mut b = skia_safe::PathBuilder::new();
        b.move_to((tx, cy - h / 2.0));
        b.line_to((tx - w, cy));
        b.line_to((tx, cy + h / 2.0));
        b.close();
        canvas.draw_path(&b.detach(), &paint);
        canvas.draw_rect(
            Rect::from_xywh(cx - w - bar_w / 2.0, cy - h / 2.0, bar_w, h),
            &paint,
        );
    }

    fn draw_skip_next_a(&self, canvas: &Canvas, cx: f32, cy: f32, size: f32, alpha: u8) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(alpha, 255, 255, 255));

        let h = size * 0.75;
        let w = h * 0.866;
        let bar_w = size * 0.18;
        let tx = cx - bar_w / 2.0;
        let mut b = skia_safe::PathBuilder::new();
        b.move_to((tx, cy - h / 2.0));
        b.line_to((tx + w, cy));
        b.line_to((tx, cy + h / 2.0));
        b.close();
        canvas.draw_path(&b.detach(), &paint);
        canvas.draw_rect(
            Rect::from_xywh(cx + w - bar_w / 2.0, cy - h / 2.0, bar_w, h),
            &paint,
        );
    }
}

fn compact_bars_width() -> f32 {
    COMPACT_BARS as f32 * 3.0 + (COMPACT_BARS as f32 - 1.0) * 2.0
}

// ---------------------------------------------------------------------------
// PlaybackInfo — shared between playerctl monitor and island
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PlaybackInfo {
    pub track_title: String,
    pub track_artist: String,
    pub art_url: String,
    pub is_playing: bool,
    pub progress: f32,
    pub duration_secs: f32,
    /// Every player the current track was read from. A browser publishes the
    /// same track twice (engine and integration shim), and either name can be
    /// the one that matches the focused window's app_id. The first is the one
    /// the controls drive.
    pub player_names: Vec<String>,
}

impl PlaybackInfo {
    fn has_track(&self) -> bool {
        !self.track_title.is_empty() && self.track_title != NO_MEDIA
    }
}

pub type SharedPlayback = Arc<Mutex<PlaybackInfo>>;

// ---------------------------------------------------------------------------
// MusicMonitor — keeps the music activity in step with the player
// ---------------------------------------------------------------------------

pub struct MusicMonitor {
    pub playback: SharedPlayback,
    meter: LevelMeter,
    bars: BarAnimator,
    /// Cached album art image + the URL it was loaded from
    album_art: Option<Image>,
    last_art_url: String,
    accent_color: Color,
    /// When the track disappeared, for the grace period before dismissing.
    gone_since: Option<std::time::Instant>,
    /// Island activity ID (None when no music activity exists)
    activity_id: Option<u64>,
    /// What the island last drew the track from: artist, art and play state
    /// change the pill without changing the activity, so they are diffed here.
    last_track: Option<(String, String, bool)>,
}

impl MusicMonitor {
    pub fn new(playback: SharedPlayback, meter: LevelMeter) -> Self {
        Self {
            playback,
            meter,
            bars: BarAnimator::default(),
            album_art: None,
            last_art_url: String::new(),
            accent_color: Color::from_rgb(180, 180, 180),
            activity_id: None,
            gone_since: None,
            last_track: None,
        }
    }

    fn info(&self) -> Option<PlaybackInfo> {
        self.playback.lock().ok().map(|info| info.clone())
    }

    /// Whether a track is playing, which is what keeps the bars moving.
    pub fn is_playing(&self) -> bool {
        self.playback.lock().is_ok_and(|info| info.is_playing)
    }

    /// Listen to the audio only while the bars are on screen and moving.
    pub fn set_meter_active(&mut self, active: bool) {
        self.meter.set_active(active);
    }

    /// Advance the bars one frame.
    pub fn step_bars(&mut self) {
        let Some(info) = self.info() else { return };
        self.bars
            .step(self.meter.level(), info.is_playing, &info.track_title);
    }

    /// Reload the album art when the track's art URL changed.
    fn refresh_art(&mut self, info: &PlaybackInfo) {
        if info.art_url == self.last_art_url {
            return;
        }
        self.last_art_url = info.art_url.clone();
        self.album_art = load_album_art(&info.art_url);
        self.accent_color = match &self.album_art {
            Some(art) => extract_accent_color(art),
            None => Color::from_rgb(180, 180, 180),
        };
    }

    /// Whether the player's own window is the focused one. The island then
    /// stays out of the way: the player is already showing what it plays.
    fn is_music_app_focused(info: &PlaybackInfo) -> bool {
        let focused = otto_kit::utils::focus_watcher::current_focused_app();
        player_owns_window(info, &focused.app_id, &focused.title)
    }

    /// Create, update or dismiss the music activity to match the player.
    pub fn sync_to_island(&mut self, state: &SharedState) {
        let Some(info) = self.info() else { return };
        self.refresh_art(&info);
        let has_track = info.has_track();
        let focused = has_track && Self::is_music_app_focused(&info);

        let mut island = state.lock().unwrap();
        match (has_track, self.activity_id) {
            (true, None) => {
                let id = island.create_activity(
                    MUSIC_APP_ID.to_string(),
                    info.track_title.clone(),
                    "audio-headphones".to_string(),
                    None,
                    0, // persistent
                    crate::activity::Priority::Normal,
                    true, // live
                    focused,
                );
                if let Some(a) = island.activities.iter_mut().find(|a| a.id == id) {
                    a.source = crate::activity::ActivitySource::Internal;
                }
                self.activity_id = Some(id);
                self.gone_since = None;
            }
            (true, Some(id)) => {
                self.gone_since = None;
                island.set_activity_quiet(id, focused);
                island.update_activity(id, &info.track_title, -1.0);
                let track = (
                    info.track_artist.clone(),
                    info.art_url.clone(),
                    info.is_playing,
                );
                if self.last_track.as_ref() != Some(&track) {
                    self.last_track = Some(track);
                    island.dirty = true;
                }
            }
            // The player has quit: there is no next track to wait for.
            (false, Some(id)) if info.player_names.is_empty() => {
                island.dismiss_activity(id);
                self.activity_id = None;
                self.gone_since = None;
            }
            (false, Some(id)) => {
                let now = std::time::Instant::now();
                match self.gone_since {
                    None => self.gone_since = Some(now),
                    Some(since) if now.duration_since(since).as_secs_f64() >= GONE_GRACE_SECS => {
                        island.dismiss_activity(id);
                        self.activity_id = None;
                        self.gone_since = None;
                    }
                    Some(_) => {}
                }
            }
            _ => self.gone_since = None,
        }
    }

    /// Whether the island is waiting out the grace period of a stopped track.
    pub fn grace_deadline(&self) -> Option<std::time::Instant> {
        self.gone_since
            .map(|since| since + Duration::from_secs_f64(GONE_GRACE_SECS))
    }

    /// Bring the player forward. The window already showing the track comes
    /// first, since a browser may have several; then a window named after the
    /// player; then the player itself over MPRIS, which lets a browser switch
    /// to the tab that is playing.
    pub fn focus_player(&self) {
        let Some(info) = self.info() else { return };
        use otto_kit::utils::focus_watcher::activate_window;
        if activate_window(|app_id, title| {
            !app_id.is_empty() && window_titled_after_track(&info, title)
        }) || activate_window(|app_id, _| window_named_after_player(&info, app_id))
        {
            return;
        }
        std::thread::spawn(move || match raise_over_mpris(&info.player_names) {
            Ok(true) => {}
            Ok(false) => tracing::info!(players = ?info.player_names, "no window to focus for the player"),
            Err(err) => tracing::warn!("raising the player over MPRIS failed: {err}"),
        });
    }

    /// The player the controls drive.
    pub fn player(&self) -> Option<String> {
        self.info()?.player_names.first().cloned()
    }

    /// Get a renderer for the current state (if a track is loaded).
    pub fn renderer(&self) -> Option<MusicActivityRenderer> {
        let info = self.info()?;
        if !info.has_track() {
            return None;
        }
        Some(MusicActivityRenderer {
            title: info.track_title,
            artist: info.track_artist,
            album_art: self.album_art.clone(),
            is_playing: info.is_playing,
            progress: info.progress,
            duration_secs: info.duration_secs,
            accent: self.accent_color,
            levels: self.bars.levels(),
            pressed: None,
        })
    }
}

/// Whether the window with `app_id` and `title` is the one playing `info`.
///
/// A player named like the window's app_id owns it ("spotify"). Browsers name
/// their player after the engine ("chromium") whatever the browser is called,
/// but a browser window is titled after its active tab, and a media tab is
/// titled after what it plays, so a window whose title holds the track title
/// is showing the track.
fn player_owns_window(info: &PlaybackInfo, app_id: &str, title: &str) -> bool {
    !app_id.is_empty()
        && (window_named_after_player(info, app_id) || window_titled_after_track(info, title))
}

fn window_named_after_player(info: &PlaybackInfo, app_id: &str) -> bool {
    info.player_names
        .iter()
        .any(|name| !name.is_empty() && app_id.eq_ignore_ascii_case(name))
}

fn window_titled_after_track(info: &PlaybackInfo, title: &str) -> bool {
    let track = info.track_title.trim();
    track.chars().count() >= 3 && title.contains(track)
}

/// Ask the player over MPRIS to show itself. playerctl names a player without
/// its instance suffix ("chromium" for `org.mpris.MediaPlayer2.chromium.instance42`),
/// so the bus name is looked up by prefix.
fn raise_over_mpris(players: &[String]) -> zbus::Result<bool> {
    let conn = zbus::blocking::Connection::session()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    let names = dbus.list_names()?;
    for player in players.iter().filter(|p| !p.is_empty()) {
        let exact = format!("org.mpris.MediaPlayer2.{player}");
        let prefix = format!("{exact}.");
        let Some(name) = names
            .iter()
            .find(|n| n.as_str() == exact || n.starts_with(&prefix))
        else {
            continue;
        };
        let proxy = zbus::blocking::Proxy::new(
            &conn,
            name.to_string(),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2",
        )?;
        if proxy.get_property::<bool>("CanRaise").unwrap_or(false) {
            proxy.call_method("Raise", &())?;
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Background threads
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Background threads
// ---------------------------------------------------------------------------

/// Field separator for the `playerctl` format string. ASCII unit separator, so
/// it cannot collide with a track title.
const FIELD_SEP: char = '\u{1f}';

/// One MPRIS player as reported by `playerctl --all-players`.
#[derive(Debug, Clone, Default, PartialEq)]
struct PlayerEntry {
    name: String,
    status: String,
    title: String,
    artist: String,
    art_url: String,
    position: f64,
    length: f64,
}

impl PlayerEntry {
    fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split(FIELD_SEP).collect();
        if f.len() < 7 {
            return None;
        }
        Some(Self {
            name: f[0].to_string(),
            status: f[1].to_string(),
            title: f[2].to_string(),
            artist: f[3].to_string(),
            art_url: f[4].to_string(),
            position: f[5].parse().unwrap_or(0.0),
            length: f[6].parse().unwrap_or(0.0),
        })
    }

    fn is_playing(&self) -> bool {
        self.status.eq_ignore_ascii_case("playing")
    }

    fn is_stopped(&self) -> bool {
        self.status.eq_ignore_ascii_case("stopped")
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

/// Pick the player to display, merging duplicate entries for the same track.
///
/// `playerctl` without `--player` picks whichever player sorts first, which for
/// a browser is the metadata-poor engine player — that is why album art went
/// missing. Prefer a playing player, then the one describing the track best,
/// then fill any gaps from the other entries for the same track.
fn select_player(entries: Vec<PlayerEntry>) -> Option<PlayerEntry> {
    let mut candidates: Vec<PlayerEntry> = if entries.iter().any(|e| e.is_playing()) {
        entries.into_iter().filter(|e| e.is_playing()).collect()
    } else {
        entries
    };
    if candidates.is_empty() {
        return None;
    }

    let best_idx = candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, e)| e.richness())
        .map(|(i, _)| i)?;
    let mut best = candidates.swap_remove(best_idx);
    let mut names = vec![best.name.clone()];

    for other in candidates {
        // Same duration means the same track, published twice.
        if other.length != best.length || best.length == 0.0 {
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
        best.position = best.position.max(other.position);
        names.push(other.name);
    }

    best.name = names.join(",");
    Some(best)
}

pub fn start_playerctl_monitor() -> SharedPlayback {
    let shared = Arc::new(Mutex::new(PlaybackInfo {
        track_title: NO_MEDIA.to_string(),
        track_artist: String::new(),
        art_url: String::new(),
        is_playing: false,
        progress: 0.0,
        duration_secs: 0.0,
        player_names: Vec::new(),
    }));
    let shared_for_thread = shared.clone();

    let format = format!(
        "{{{{playerName}}}}{s}{{{{status}}}}{s}{{{{title}}}}{s}{{{{artist}}}}{s}\
         {{{{mpris:artUrl}}}}{s}{{{{position}}}}{s}{{{{mpris:length}}}}",
        s = FIELD_SEP
    );

    thread::spawn(move || loop {
        // Single playerctl call covering every player's metadata + status.
        let output = Command::new("playerctl")
            .args(["--all-players", "metadata", "--format", &format])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string());

        let selected = output.as_deref().and_then(|out| {
            select_player(
                out.lines()
                    .filter_map(PlayerEntry::parse)
                    .collect::<Vec<_>>(),
            )
        });

        // The island's event loop is fully retained — it sleeps until something
        // wakes it. Nothing else knows a track appeared, changed or stopped, so
        // this thread has to. Waking only on change keeps an idle desktop with
        // no player asleep.
        let mut wake = false;

        if let Some(entry) = selected {
            // When status is "Stopped", report "No media" so the island is
            // dismissed after the grace period. Paused tracks keep their title.
            let track_title = if entry.is_stopped() || entry.title.is_empty() {
                NO_MEDIA.to_string()
            } else {
                entry.title.clone()
            };
            let length = if entry.length > 0.0 {
                entry.length
            } else {
                1.0
            };
            let progress = (entry.position / length).clamp(0.0, 1.0) as f32;
            let is_playing = entry.is_playing();

            if let Ok(mut info) = shared_for_thread.lock() {
                // A playing track keeps the island's clock and progress bar
                // moving, so keep waking while one plays, not only on change.
                wake = info.track_title != track_title
                    || info.track_artist != entry.artist
                    || info.art_url != entry.art_url
                    || info.is_playing != is_playing
                    || info.progress != progress
                    || is_playing;
                info.track_title = track_title;
                info.track_artist = entry.artist;
                info.art_url = entry.art_url;
                info.is_playing = is_playing;
                info.progress = progress;
                info.duration_secs = (length / 1_000_000.0) as f32;
                info.player_names = entry.name.split(',').map(str::to_string).collect();
            }
        } else if let Ok(mut info) = shared_for_thread.lock() {
            // playerctl failed — no player running. Clear track info so the
            // island is dismissed after the grace period.
            wake = info.track_title != NO_MEDIA;
            info.is_playing = false;
            info.track_title = NO_MEDIA.to_string();
            info.track_artist.clear();
            info.player_names.clear();
        }

        if wake {
            AppContext::request_wakeup();
        }
        thread::sleep(Duration::from_millis(1500));
    });

    shared
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a transport action on `player` through playerctl. Without `--player`,
/// playerctl picks whichever player sorts first, which is not necessarily the
/// one the island shows.
pub fn execute_action(action: MusicAction, player: String, duration_secs: f32) {
    let args: Vec<String> = match action {
        MusicAction::PlayPause => vec!["play-pause".into()],
        MusicAction::SkipNext => vec!["next".into()],
        MusicAction::SkipPrev => vec!["previous".into()],
        MusicAction::Seek(frac) if duration_secs > 0.0 => {
            vec!["position".into(), format!("{:.1}", frac * duration_secs)]
        }
        MusicAction::Seek(_) | MusicAction::FocusPlayer => return,
    };
    thread::spawn(move || {
        let _ = Command::new("playerctl")
            .arg(format!("--player={player}"))
            .args(&args)
            .status();
    });
}

fn font(size: f32) -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight: 400,
        size,
    }
    .font()
}

fn font_bold(size: f32) -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight: 600,
        size,
    }
    .font()
}

/// Decode `%XX` escapes in a URL path. MPRIS `file://` art URLs are proper
/// URLs, so any path with a space or a non-ASCII character (accented artist
/// names, for instance) arrives percent-encoded and must be decoded before it
/// can be opened.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
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

/// Extract the filesystem path from a `file://` URL, decoding percent escapes.
/// Accepts both `file:///path` and `file://localhost/path`.
fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    Some(percent_decode(path))
}

fn load_album_art(url: &str) -> Option<Image> {
    if url.is_empty() {
        return None;
    }

    let bytes = if url.starts_with("file://") {
        let path = file_url_to_path(url)?;
        match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to read album art");
                return None;
            }
        }
    } else if url.starts_with("http://") || url.starts_with("https://") {
        // Bounded: this runs on the island's update thread, so an unreachable
        // host must not stall the UI.
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .new_agent();
        match agent.get(url).call() {
            Ok(resp) => resp.into_body().read_to_vec().ok()?,
            Err(e) => {
                tracing::warn!(url, error = %e, "failed to fetch album art");
                return None;
            }
        }
    } else {
        tracing::warn!(url, "unsupported album art URL scheme");
        return None;
    };

    let data = Data::new_copy(&bytes);
    Image::from_encoded(data)
}

fn trim_to_width<'a>(text: &'a str, font: &skia_safe::Font, max_width: f32) -> &'a str {
    let (width, _) = font.measure_str(text, None);
    if width <= max_width {
        return text;
    }
    // Find the longest prefix that fits (byte-boundary safe)
    for end in (1..text.len()).rev() {
        if !text.is_char_boundary(end) {
            continue;
        }
        let sub = &text[..end];
        let (w, _) = font.measure_str(sub, None);
        if w <= max_width {
            return sub;
        }
    }
    ""
}

fn format_time_secs(total_secs: f32) -> String {
    let secs = total_secs.max(0.0) as u32;
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {

    fn playing(track: &str, players: &[&str]) -> PlaybackInfo {
        PlaybackInfo {
            track_title: track.into(),
            track_artist: String::new(),
            art_url: String::new(),
            is_playing: true,
            progress: 0.0,
            duration_secs: 0.0,
            player_names: players.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn the_album_art_focuses_the_player() {
        let (w, h) = MusicActivityRenderer::mode_size(IslandMode::Expanded);
        assert_eq!(
            MusicActivityRenderer::hit_test_expanded(h / 2.0, h / 2.0, w, h),
            Some(MusicAction::FocusPlayer)
        );
        assert_eq!(MusicActivityRenderer::hit_test_expanded(4.0, 4.0, w, h), None);
    }

    #[test]
    fn window_named_after_the_player_owns_it() {
        let info = playing("Groove Waltz", &["spotify"]);
        assert!(player_owns_window(&info, "Spotify", "Spotify Premium"));
        assert!(!player_owns_window(&info, "otto-files", "Files"));
    }

    #[test]
    fn browser_window_showing_the_track_owns_it() {
        let info = playing("Lofi beats to study to", &["chromium"]);
        assert!(player_owns_window(
            &info,
            "google-chrome",
            "Lofi beats to study to - YouTube - Google Chrome"
        ));
        // Another tab of the same browser is not the player.
        assert!(!player_owns_window(
            &info,
            "google-chrome",
            "Pull Request #211 - Google Chrome"
        ));
    }

    #[test]
    fn short_track_titles_do_not_match_by_title() {
        let info = playing("Go", &["chromium"]);
        assert!(!player_owns_window(&info, "google-chrome", "Go - Google Chrome"));
        assert!(!player_owns_window(&info, "", "Go"));
    }

    use super::*;

    fn line(name: &str, status: &str, title: &str, artist: &str, art: &str, len: &str) -> String {
        format!("{name}\u{1f}{status}\u{1f}{title}\u{1f}{artist}\u{1f}{art}\u{1f}0\u{1f}{len}")
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(
            file_url_to_path("file:///home/u/art%20dir/Beyonc%C3%A9.png").as_deref(),
            Some("/home/u/art dir/Beyoncé.png")
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/cover.png").as_deref(),
            Some("/tmp/cover.png")
        );
        assert_eq!(file_url_to_path("https://example.com/a.png"), None);
    }

    #[test]
    fn browser_duplicate_players_are_merged() {
        // Chromium publishes the track twice: the engine player has no art and
        // no artist, the integration shim has both.
        let entries = vec![
            PlayerEntry::parse(&line(
                "chromium",
                "Playing",
                "On Hold • The xx",
                "",
                "",
                "224179773",
            ))
            .unwrap(),
            PlayerEntry::parse(&line(
                "plasma-browser-integration",
                "Playing",
                "On Hold",
                "The xx",
                "file:///tmp/art.png",
                "224179773",
            ))
            .unwrap(),
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
            PlayerEntry::parse(&line("vlc", "Paused", "Old", "A", "file:///a.png", "100")).unwrap(),
            PlayerEntry::parse(&line("spotify", "Playing", "New", "B", "", "200")).unwrap(),
        ];
        let picked = select_player(entries).unwrap();
        assert_eq!(picked.title, "New");
        assert_eq!(picked.name, "spotify");
    }

    #[test]
    fn unrelated_tracks_are_not_merged() {
        let entries = vec![
            PlayerEntry::parse(&line("a", "Playing", "One", "", "", "100")).unwrap(),
            PlayerEntry::parse(&line("b", "Playing", "Two", "X", "file:///b.png", "999")).unwrap(),
        ];
        let picked = select_player(entries).unwrap();
        assert_eq!(picked.title, "Two");
        assert_eq!(picked.name, "b");
    }
}
