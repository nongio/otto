//! Music activity: the MPRIS player the island shows, drawn with the audio
//! visualiser.
//!
//! [`mpris`](crate::mpris) follows the players. While a track is loaded,
//! [`MusicMonitor`] keeps a live activity on the island, quiet while the
//! player's own window is focused; when the track stops it goes away.
//!
//! [`audio_route`](crate::audio_route) finds the stream the track plays on,
//! which is what the bars listen to. A track playing on another device gets a
//! glyph saying so in place of the bars.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use otto_kit::typography::{ellipsize, TextStyle};
use otto_kit::utils::extract_accent_color;
use otto_kit::utils::focus_watcher::{self, FocusedApp};
use otto_kit::AppContext;
use skia_safe::{Canvas, Color, Data, Image, Paint, RRect, Rect};

use crate::audio_route::{self, AudioStreams, Player, Route};
use crate::audio_viz::{self, BarAnimator, BarStyle, LevelMeter, BAR_COUNT};
use crate::mpris::{self, Control, PlaybackInfo, SharedPlayback};
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
/// Album art bigger than this, encoded, is not loaded.
const MAX_ART_BYTES: u64 = 8 * 1024 * 1024;
/// Album art wider or taller than this is not decoded.
const MAX_ART_SIDE: i32 = 4096;
/// Album art is kept at this size: it is never drawn bigger.
const ART_PX: i32 = 256;
const NEUTRAL_ACCENT: Color = Color::from_rgb(180, 180, 180);
/// How long a playing track must go without a local stream before the island
/// says it plays on another device. A player starting up reports Playing a
/// moment before its stream runs.
const ELSEWHERE_SETTLE: Duration = Duration::from_secs(1);

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
    /// The track plays on another device: a glyph replaces the bars.
    pub elsewhere: bool,
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

    /// Draw only the bars, or what replaces them. The canvas origin is the
    /// top-left of the bar area.
    pub fn draw_eq_only(&self, canvas: &Canvas, mode: IslandMode, w: f32, h: f32) {
        if self.elsewhere {
            return self.draw_elsewhere(canvas, mode, w, h);
        }
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

    /// The glyph for a track on another device; the open island names it too.
    fn draw_elsewhere(&self, canvas: &Canvas, mode: IslandMode, w: f32, h: f32) {
        match mode {
            IslandMode::Mini => {
                let side = h * 0.45;
                let rect = Rect::from_xywh((w - side) / 2.0, (h - side) / 2.0, side, side);
                audio_viz::draw_elsewhere_glyph(canvas, rect, self.accent);
            }
            IslandMode::Compact => {
                audio_viz::draw_elsewhere_glyph(
                    canvas,
                    Rect::from_xywh(0.0, 0.0, w, h),
                    self.accent,
                );
            }
            IslandMode::Expanded => {
                let side = 14.0f32.min(h);
                let top = (h - side) / 2.0;
                audio_viz::draw_elsewhere_glyph(
                    canvas,
                    Rect::from_xywh(0.0, top, side, side),
                    self.accent,
                );
                let text_x = side + 6.0;
                let label_font = font(10.0);
                let mut paint = Paint::default();
                paint.set_anti_alias(true);
                paint.set_color(Color::from_argb(170, 180, 180, 180));
                let label = ellipsize(
                    &label_font,
                    otto_kit::t!("islands-music-elsewhere"),
                    (w - text_x).max(20.0),
                );
                canvas.draw_str(&label, (text_x, h / 2.0 + 3.5), &label_font, &paint);
            }
        }
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
        let artist = ellipsize(&af, &self.artist, text_max_w.max(20.0));
        canvas.draw_str(&artist, (text_x, mid + 10.0), &af, &ap);
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
        let artist = ellipsize(&af, &self.artist, rw.max(20.0));
        canvas.draw_str(&artist, (rx, pad + 27.0), &af, &ap);

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
        let label = ellipsize(&f, text, max_w.max(20.0));
        canvas.draw_str(&label, (x, y), &f, &paint);
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
// MusicMonitor — keeps the music activity in step with the player
// ---------------------------------------------------------------------------

/// Album art, loaded off the island's thread.
struct Art {
    /// The URL the island wants; a load for any other is discarded.
    wanted: String,
    image: Option<Image>,
    accent: Color,
    /// A load finished since the island last looked.
    fresh: bool,
}

pub struct MusicMonitor {
    playback: SharedPlayback,
    meter: LevelMeter,
    streams: AudioStreams,
    /// Where the playing track's sound goes, with the stream snapshot and
    /// player it was worked out for.
    route: Option<(u64, Vec<u32>, Vec<String>, Route)>,
    /// Since when the playing track has had no local stream.
    elsewhere_since: Option<Instant>,
    /// Whether the island shows the track as playing on another device.
    elsewhere: bool,
    bars: BarAnimator,
    art: Arc<Mutex<Art>>,
    /// When the track disappeared, for the grace period before dismissing.
    gone_since: Option<std::time::Instant>,
    /// Island activity ID (None when no music activity exists)
    activity_id: Option<u64>,
    /// The last track the island showed. It is still drawn while the grace
    /// period of a stopped track runs out.
    shown: Option<PlaybackInfo>,
}

impl MusicMonitor {
    pub fn new(playback: SharedPlayback, meter: LevelMeter, streams: AudioStreams) -> Self {
        Self {
            playback,
            meter,
            streams,
            route: None,
            elsewhere_since: None,
            elsewhere: false,
            bars: BarAnimator::default(),
            art: Arc::new(Mutex::new(Art {
                wanted: String::new(),
                image: None,
                accent: NEUTRAL_ACCENT,
                fresh: false,
            })),
            activity_id: None,
            gone_since: None,
            shown: None,
        }
    }

    fn info(&self) -> Option<PlaybackInfo> {
        self.playback.lock().ok().map(|info| info.clone())
    }

    /// Whether a track is playing, which is what keeps the bars moving.
    pub fn is_playing(&self) -> bool {
        self.playback.lock().is_ok_and(|info| info.is_playing)
    }

    /// Where the playing track's sound goes. Worked out again only when the
    /// streams or the player changed.
    fn current_route(&mut self, info: &PlaybackInfo) -> Route {
        let (streams, clients, generation) = self.streams.snapshot();
        if let Some((cached_generation, pids, names, route)) = &self.route {
            if *cached_generation == generation
                && *pids == info.player_pids
                && *names == info.player_names
            {
                return *route;
            }
        }
        let route = audio_route::route(
            Player {
                pids: &info.player_pids,
                names: &info.player_names,
            },
            &streams,
            &clients,
            audio_route::parent_pid,
        );
        tracing::debug!(?route, players = ?info.player_names, "music route");
        self.route = Some((
            generation,
            info.player_pids.clone(),
            info.player_names.clone(),
            route,
        ));
        route
    }

    /// Follow where the playing track's sound goes. Returns whether the island
    /// switched between bars and the other-device glyph. A paused track keeps
    /// what it showed: nothing plays locally then either way.
    pub fn update_route(&mut self, now: Instant) -> bool {
        let Some(info) = self.info().filter(|info| info.is_playing) else {
            self.elsewhere_since = None;
            return false;
        };
        let route = self.current_route(&info);
        let elsewhere = if route == Route::Elsewhere {
            let since = *self.elsewhere_since.get_or_insert(now);
            now.duration_since(since) >= ELSEWHERE_SETTLE
        } else {
            self.elsewhere_since = None;
            false
        };
        std::mem::replace(&mut self.elsewhere, elsewhere) != elsewhere
    }

    /// When a track with no local stream is due to show as playing elsewhere.
    pub fn route_deadline(&self) -> Option<Instant> {
        self.elsewhere_since
            .filter(|_| !self.elsewhere)
            .map(|since| since + ELSEWHERE_SETTLE)
    }

    /// Whether the track shows as playing on another device.
    pub fn plays_elsewhere(&self) -> bool {
        self.elsewhere
    }

    /// Listen to the track's stream while the bars are on screen and moving,
    /// and to nothing otherwise.
    pub fn set_meter_active(&mut self, active: bool) {
        let serial = match self.route.as_ref().map(|(.., route)| *route) {
            Some(Route::Stream(serial)) if active => Some(serial),
            _ => None,
        };
        self.meter.listen(serial);
    }

    /// Advance the bars one frame.
    pub fn step_bars(&mut self) {
        let Some(info) = self.info() else { return };
        self.bars.step(self.meter.level(), &info.track_title);
    }

    /// Start loading the album art when the track's art URL changed. Returns
    /// whether art finished loading since the last call, so the island redraws.
    fn refresh_art(&mut self, url: &str) -> bool {
        let Ok(mut art) = self.art.lock() else {
            return false;
        };
        if art.wanted != url {
            art.wanted = url.to_string();
            art.image = None;
            art.accent = NEUTRAL_ACCENT;
            art.fresh = false;
            if !url.is_empty() {
                let url = url.to_string();
                let slot = self.art.clone();
                thread::spawn(move || {
                    let image = load_album_art(&url);
                    let accent = image.as_ref().map_or(NEUTRAL_ACCENT, extract_accent_color);
                    if let Ok(mut art) = slot.lock() {
                        if art.wanted == url {
                            art.image = image;
                            art.accent = accent;
                            art.fresh = true;
                            AppContext::request_wakeup();
                        }
                    }
                });
            }
            return true;
        }
        std::mem::take(&mut art.fresh)
    }

    /// Create, update or dismiss the music activity to match the player.
    pub fn sync_to_island(&mut self, state: &SharedState) {
        let Some(info) = self.info() else { return };
        let has_track = info.has_track();
        let art_changed = has_track && self.refresh_art(&info.art_url);
        let focused = has_track && player_is_focused(&info);

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
                self.shown = Some(info);
            }
            (true, Some(id)) => {
                self.gone_since = None;
                island.set_activity_quiet(id, focused);
                island.update_activity(id, &info.track_title, -1.0);
                // Artist, art, play state and a seek while paused change the
                // pill without changing the activity. While playing, the
                // progress bar is redrawn on its own clock.
                if art_changed
                    || self
                        .shown
                        .as_ref()
                        .is_none_or(|shown| looks_different(shown, &info))
                {
                    island.dirty = true;
                }
                self.shown = Some(info);
            }
            // The player has quit: there is no next track to wait for.
            (false, Some(id)) if info.player_names.is_empty() => {
                island.dismiss_activity(id);
                self.activity_id = None;
                self.gone_since = None;
                self.shown = None;
            }
            (false, Some(id)) => {
                let now = std::time::Instant::now();
                match self.gone_since {
                    None => self.gone_since = Some(now),
                    Some(since) if now.duration_since(since).as_secs_f64() >= GONE_GRACE_SECS => {
                        island.dismiss_activity(id);
                        self.activity_id = None;
                        self.gone_since = None;
                        self.shown = None;
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

    /// Bring the player forward: its own window, the one showing the track
    /// if it has several; failing that, the window showing the track; failing
    /// that, the player itself over MPRIS, which lets a browser switch to the
    /// tab that is playing.
    pub fn focus_player(&self) {
        let Some(info) = self.shown.clone() else {
            return;
        };
        let windows = focus_watcher::windows();
        let target = windows
            .iter()
            .filter(|w| window_named_after_player(&info, &w.app_id))
            .max_by_key(|w| window_titled_after_track(&info, &w.title))
            .or_else(|| {
                windows
                    .iter()
                    .find(|w| window_titled_after_track(&info, &w.title))
            });
        if let Some(window) = target {
            if focus_watcher::activate_window(|app_id, title| {
                app_id == window.app_id && title == window.title
            }) {
                return;
            }
        }
        thread::spawn(move || match mpris::raise(&info) {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(bus_name = %info.bus_name, "no window to focus for the player")
            }
            Err(error) => tracing::warn!(%error, "raising the player over MPRIS failed"),
        });
    }

    /// Run a transport control on the player the island shows.
    pub fn control(&self, action: MusicAction) {
        let control = match action {
            MusicAction::PlayPause => Control::PlayPause,
            MusicAction::SkipNext => Control::Next,
            MusicAction::SkipPrev => Control::Previous,
            MusicAction::Seek(fraction) => Control::SeekTo(fraction),
            MusicAction::FocusPlayer => return self.focus_player(),
        };
        if let Some(info) = &self.shown {
            mpris::send(control, info);
        }
    }

    /// A renderer for the track the island shows, if any.
    pub fn renderer(&self) -> Option<MusicActivityRenderer> {
        let info = self.shown.as_ref()?;
        let (album_art, accent) = match self.art.lock() {
            Ok(art) if art.wanted == info.art_url => (art.image.clone(), art.accent),
            _ => (None, NEUTRAL_ACCENT),
        };
        Some(MusicActivityRenderer {
            title: info.track_title.clone(),
            artist: info.track_artist.clone(),
            album_art,
            is_playing: info.is_playing,
            progress: info.progress,
            duration_secs: info.duration_secs,
            accent,
            levels: self.bars.levels(),
            elsewhere: self.elsewhere,
            pressed: None,
        })
    }
}

/// Whether the pill drawn from `shown` is out of date for `info`.
fn looks_different(shown: &PlaybackInfo, info: &PlaybackInfo) -> bool {
    shown.track_artist != info.track_artist
        || shown.art_url != info.art_url
        || shown.is_playing != info.is_playing
        || (!info.is_playing && shown.progress != info.progress)
}

/// Whether the player's own window is the focused one. The island then stays
/// out of the way: the player is already showing what it plays.
fn player_is_focused(info: &PlaybackInfo) -> bool {
    player_owns_window(
        info,
        &focus_watcher::current_focused_app(),
        &focus_watcher::windows(),
    )
}

/// Whether `window` is the one playing `info`, among the open `windows`.
///
/// An app whose window is named after the player ("spotify") owns only those
/// windows. Browsers name their player after the engine ("chromium") whatever
/// the browser is called, so when no window is named after the player, the
/// window whose title holds the track title is the one showing it: a browser
/// window is titled after its active tab, and a media tab after what it plays.
fn player_owns_window(info: &PlaybackInfo, window: &FocusedApp, windows: &[FocusedApp]) -> bool {
    if window.app_id.is_empty() {
        return false;
    }
    if windows
        .iter()
        .any(|w| window_named_after_player(info, &w.app_id))
    {
        window_named_after_player(info, &window.app_id)
    } else {
        window_titled_after_track(info, &window.title)
    }
}

fn window_named_after_player(info: &PlaybackInfo, app_id: &str) -> bool {
    !app_id.is_empty()
        && info
            .player_names
            .iter()
            .any(|name| !name.is_empty() && app_id.eq_ignore_ascii_case(name))
}

fn window_titled_after_track(info: &PlaybackInfo, title: &str) -> bool {
    let track = info.track_title.trim();
    track.chars().count() >= 3 && title.contains(track)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Load album art from a `file://` or `https://` URL, scaled down to
/// [`ART_PX`]. The URL comes from whatever the player publishes, which for a
/// browser is the web page, so the size is capped before anything is decoded
/// and only public hosts are fetched.
fn load_album_art(url: &str) -> Option<Image> {
    let bytes = if url.starts_with("file://") {
        read_art_file(url)
    } else if url.starts_with("https://") {
        fetch_art(url)
    } else {
        tracing::debug!(url, "unsupported album art URL");
        None
    }?;
    decode_art(bytes)
}

fn read_art_file(url: &str) -> Option<Vec<u8>> {
    let path = otto_kit::clipboard::uri_to_path(url)?;
    // Only a regular file: a FIFO or a device would block or never end.
    let file = std::fs::File::open(&path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_ART_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_ART_BYTES).then_some(bytes)
}

fn fetch_art(url: &str) -> Option<Vec<u8>> {
    if !is_public_host(url) {
        tracing::debug!(url, "album art host is not public");
        return None;
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    match agent.get(url).call() {
        Ok(response) => response
            .into_body()
            .with_config()
            .limit(MAX_ART_BYTES)
            .read_to_vec()
            .ok(),
        Err(error) => {
            tracing::debug!(url, %error, "failed to fetch album art");
            None
        }
    }
}

/// Whether the host of `url` is not this machine or the local network, as far
/// as the URL itself says.
fn is_public_host(url: &str) -> bool {
    use std::net::IpAddr;
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Ok(IpAddr::V6(ip)) => {
            let unique_local = (ip.segments()[0] & 0xfe00) == 0xfc00;
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
        }
        Err(_) => true,
    }
}

fn decode_art(bytes: Vec<u8>) -> Option<Image> {
    let data = Data::new_copy(&bytes);
    let size = skia_safe::Codec::from_data(data.clone())?.dimensions();
    if size.width <= 0
        || size.height <= 0
        || size.width > MAX_ART_SIDE
        || size.height > MAX_ART_SIDE
    {
        tracing::debug!(?size, "album art too large");
        return None;
    }
    let image = Image::from_encoded(data)?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((ART_PX, ART_PX))?;
    let sampling = skia_safe::SamplingOptions::new(
        skia_safe::FilterMode::Linear,
        skia_safe::MipmapMode::Linear,
    );
    surface.canvas().draw_image_rect_with_sampling_options(
        &image,
        None,
        Rect::from_wh(ART_PX as f32, ART_PX as f32),
        sampling,
        &Paint::default(),
    );
    Some(surface.image_snapshot())
}

fn format_time_secs(total_secs: f32) -> String {
    let secs = total_secs.max(0.0) as u32;
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(track: &str, players: &[&str]) -> PlaybackInfo {
        PlaybackInfo {
            track_title: track.into(),
            track_artist: String::new(),
            art_url: String::new(),
            is_playing: true,
            progress: 0.0,
            duration_secs: 0.0,
            bus_name: String::new(),
            track_id: String::new(),
            player_names: players.iter().map(|p| p.to_string()).collect(),
            player_pids: Vec::new(),
        }
    }

    fn window(app_id: &str, title: &str) -> FocusedApp {
        FocusedApp {
            app_id: app_id.into(),
            title: title.into(),
        }
    }

    #[test]
    fn the_album_art_focuses_the_player() {
        let (w, h) = MusicActivityRenderer::mode_size(IslandMode::Expanded);
        assert_eq!(
            MusicActivityRenderer::hit_test_expanded(h / 2.0, h / 2.0, w, h),
            Some(MusicAction::FocusPlayer)
        );
        assert_eq!(
            MusicActivityRenderer::hit_test_expanded(4.0, 4.0, w, h),
            None
        );
    }

    #[test]
    fn window_named_after_the_player_owns_it() {
        let info = playing("Groove Waltz", &["spotify"]);
        let spotify = window("Spotify", "Spotify Premium");
        let files = window("otto-files", "Groove Waltz");
        let open = [spotify.clone(), files.clone()];
        assert!(player_owns_window(&info, &spotify, &open));
        // A folder named after the track is not the player when the player
        // has a window of its own.
        assert!(!player_owns_window(&info, &files, &open));
    }

    #[test]
    fn browser_window_showing_the_track_owns_it() {
        let info = playing("Lofi beats to study to", &["chromium"]);
        let playing_tab = window(
            "google-chrome",
            "Lofi beats to study to - YouTube - Google Chrome",
        );
        let other_tab = window("google-chrome", "Pull Request #211 - Google Chrome");
        let open = [playing_tab.clone(), other_tab.clone()];
        assert!(player_owns_window(&info, &playing_tab, &open));
        assert!(!player_owns_window(&info, &other_tab, &open));
    }

    #[test]
    fn short_track_titles_do_not_match_by_title() {
        let info = playing("Go", &["chromium"]);
        let chrome = window("google-chrome", "Go - Google Chrome");
        assert!(!player_owns_window(
            &info,
            &chrome,
            std::slice::from_ref(&chrome)
        ));
        assert!(!player_owns_window(&info, &window("", "Go"), &[]));
    }

    #[test]
    fn a_seek_while_paused_redraws() {
        let mut shown = playing("Song", &["spotify"]);
        shown.is_playing = false;
        let mut sought = shown.clone();
        sought.progress = 0.5;
        assert!(looks_different(&shown, &sought));
        // While playing the progress bar keeps its own clock.
        shown.is_playing = true;
        sought.is_playing = true;
        assert!(!looks_different(&shown, &sought));
    }

    #[test]
    fn only_public_hosts_are_fetched() {
        assert!(is_public_host("https://i.scdn.co/image/ab67616d0000b273"));
        for url in [
            "https://localhost/a.png",
            "https://127.0.0.1:8080/a.png",
            "https://user@10.0.0.2/a.png",
            "https://192.168.1.1/a.png",
            "https://[::1]/a.png",
            "https://printer.local/a.png",
        ] {
            assert!(!is_public_host(url), "{url}");
        }
    }

    #[test]
    fn oversized_art_is_not_decoded() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((MAX_ART_SIDE + 1, 1)).unwrap();
        let png = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap();
        assert!(decode_art(png.as_bytes().to_vec()).is_none());
    }

    #[test]
    fn art_is_kept_small() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((1000, 1000)).unwrap();
        let png = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap();
        let art = decode_art(png.as_bytes().to_vec()).unwrap();
        assert_eq!((art.width(), art.height()), (ART_PX, ART_PX));
    }
}
