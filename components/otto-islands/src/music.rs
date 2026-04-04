//! Music activity — MPRIS bridge via playerctl + PipeWire audio levels.
//!
//! Spawns background threads that poll playerctl and capture PipeWire audio.
//! When music is playing, creates/updates an island activity. When stopped,
//! dismisses it.

use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use otto_kit::typography::TextStyle;
use otto_kit::utils::extract_accent_color;
use skia_safe::{Canvas, Color, Data, Image, Paint, RRect, Rect};

use crate::activity::{ActivityRenderer, PresentationMode};
use crate::state::SharedState;

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
}

const BAR_COUNT: usize = 8;

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

impl ActivityRenderer for MusicActivityRenderer {
    fn size(&self, mode: PresentationMode) -> (f32, f32) {
        match mode {
            PresentationMode::Compact => (220.0, 30.0),
            PresentationMode::Banner => (298.0, 36.0), // Zoom: slightly larger compact
            PresentationMode::Expanded => (340.0, 120.0), // Extended player
            PresentationMode::Minimal => (28.0, 28.0),
            PresentationMode::Idle => (28.0, 28.0),
        }
    }

    fn draw(&self, canvas: &Canvas, mode: PresentationMode, w: f32, h: f32) {
        // Note: don't clear here — draw_slot_centered already clears the buffer
        match mode {
            PresentationMode::Compact | PresentationMode::Banner => self.draw_compact(canvas, w, h),
            PresentationMode::Expanded => self.draw_open(canvas, w, h),
            PresentationMode::Minimal => self.draw_minimal(canvas, w, h),
            PresentationMode::Idle => self.draw_minimal(canvas, w, h),
        }
    }
}

impl MusicActivityRenderer {
    /// Return the bounding rect of the EQ bars region for partial redraws.
    /// Coordinates are relative to the content origin (0,0).
    pub fn eq_region(&self, mode: PresentationMode, w: f32, h: f32) -> Rect {
        match mode {
            PresentationMode::Minimal | PresentationMode::Idle => {
                // Mini: entire surface is the EQ
                Rect::from_xywh(0.0, 0.0, w, h)
            }
            PresentationMode::Compact | PresentationMode::Banner => {
                // Compact: EQ is on the right side
                let pad = 8.0;
                let icon_size = h - pad * 2.0;
                let eq_x = w - pad - 80.0; // approximate EQ area on right
                Rect::from_xywh(eq_x.max(0.0), 0.0, w - eq_x.max(0.0), h)
            }
            PresentationMode::Expanded => {
                // Expanded: EQ is in the bottom-right area
                let pad = 12.0;
                let art_size = h - pad * 2.0;
                let rx = pad + art_size + pad;
                Rect::from_xywh(rx, h * 0.4, w - rx, h * 0.6)
            }
        }
    }

    /// Hit test within the expanded player. `lx`, `ly` are local coords
    /// relative to the top-left of the expanded island content.
    pub fn hit_test_expanded(&self, lx: f32, ly: f32, w: f32, h: f32) -> Option<MusicAction> {
        let pad = 12.0;
        let art_size = h - pad * 2.0;
        let rx = pad + art_size + pad;
        let rw = w - rx - pad;

        // Progress bar — generous vertical hit zone (±8px around the bar).
        let prog_y = h - pad - 26.0;
        if lx >= rx && lx <= rx + rw && (ly - prog_y).abs() <= 8.0 {
            let frac = ((lx - rx) / rw).clamp(0.0, 1.0);
            return Some(MusicAction::Seek(frac));
        }

        // Controls
        let ctrl_y = h - pad - 6.0;
        let ctrl_cx = rx + rw / 2.0;
        let icon_gap = 36.0;
        let hit_r = 18.0;

        // Skip prev
        let dx = lx - (ctrl_cx - icon_gap);
        let dy = ly - ctrl_y;
        if dx * dx + dy * dy <= hit_r * hit_r {
            return Some(MusicAction::SkipPrev);
        }
        // Play/pause
        let dx = lx - ctrl_cx;
        let dy = ly - ctrl_y;
        if dx * dx + dy * dy <= hit_r * hit_r {
            return Some(MusicAction::PlayPause);
        }
        // Skip next
        let dx = lx - (ctrl_cx + icon_gap);
        let dy = ly - ctrl_y;
        if dx * dx + dy * dy <= hit_r * hit_r {
            return Some(MusicAction::SkipNext);
        }
        None
    }

    fn draw_compact(&self, canvas: &Canvas, w: f32, h: f32) {
        let v_pad = 7.0;
        let h_pad = 10.0;
        let art_size = h - v_pad * 2.0;
        let art_x = h_pad;
        let art_y = v_pad;

        // Album art
        self.draw_art(canvas, art_x, art_y, art_size);

        // Equalizer on the far right (4 bars for compact)
        let compact_bars = 4;
        let eq_area_w = compact_bars as f32 * 3.0 + (compact_bars as f32 - 1.0) * 2.0;
        let eq_x = w - h_pad - eq_area_w;
        let eq_y = v_pad;
        let eq_h = h - v_pad * 2.0;
        self.draw_equalizer_n(canvas, eq_x, eq_y, eq_area_w, eq_h, compact_bars);

        // Title + artist between art and equalizer
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

        // --- Album art (left) ---
        self.draw_art(canvas, pad, pad, art_size);

        // --- Right column: title, artist, EQ, progress, controls ---
        let rx = pad + art_size + pad;
        let rw = w - rx - pad;

        // Title + artist
        Self::draw_text(canvas, &self.title, rx, pad + 13.0, 13.0, 255, rw);
        let af = font(10.0);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(150, 180, 180, 180));
        let artist = trim_to_width(&self.artist, &af, rw.max(20.0));
        canvas.draw_str(artist, (rx, pad + 27.0), &af, &ap);

        // Equalizer (between text and progress)
        let eq_y = pad + 34.0;
        let eq_h = 22.0;
        self.draw_equalizer_large(canvas, rx, eq_y, rw, eq_h);

        // Progress bar
        let prog_y = h - pad - 26.0;
        self.draw_progress_large(canvas, rx, prog_y, rw);

        // Controls
        let ctrl_y = h - pad - 6.0;
        let ctrl_cx = rx + rw / 2.0;
        let icon_gap = 36.0;
        let icon_size = 12.0;

        let prev_alpha = if self.pressed == Some(MusicAction::SkipPrev) {
            100
        } else {
            200
        };
        let pp_alpha = if self.pressed == Some(MusicAction::PlayPause) {
            150
        } else {
            255
        };
        let next_alpha = if self.pressed == Some(MusicAction::SkipNext) {
            100
        } else {
            200
        };

        self.draw_skip_prev_a(canvas, ctrl_cx - icon_gap, ctrl_y, icon_size, prev_alpha);
        self.draw_play_pause_a(canvas, ctrl_cx, ctrl_y, icon_size + 4.0, pp_alpha);
        self.draw_skip_next_a(canvas, ctrl_cx + icon_gap, ctrl_y, icon_size, next_alpha);
    }

    fn draw_minimal(&self, canvas: &Canvas, w: f32, h: f32) {
        // 3-bar mini equalizer centered in the circle
        let bar_count = 3;
        let bar_w = 3.0f32;
        let bar_gap = 2.5f32;
        let bars_total = bar_count as f32 * bar_w + (bar_count - 1) as f32 * bar_gap;
        let start_x = (w - bars_total) / 2.0;
        let center_y = h / 2.0;
        let max_h = h * 0.5;

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        let ac = self.accent;
        paint.set_color(Color::from_argb(220, ac.r(), ac.g(), ac.b()));

        for i in 0..bar_count {
            let level = self.levels[i].clamp(0.1, 1.0);
            let bar_h = level * max_h;
            let bx = start_x + i as f32 * (bar_w + bar_gap);
            let by = center_y - bar_h / 2.0;
            canvas.draw_rrect(
                RRect::new_rect_xy(Rect::from_xywh(bx, by, bar_w, bar_h), 1.0, 1.0),
                &paint,
            );
        }
    }

    fn draw_equalizer_large(&self, canvas: &Canvas, x: f32, y: f32, w: f32, h: f32) {
        let bar_w = 6.0f32;
        let bar_gap = 4.0f32;
        let bars_total = BAR_COUNT as f32 * bar_w + (BAR_COUNT - 1) as f32 * bar_gap;
        let start_x = x + (w - bars_total) / 2.0;
        let bottom = y + h;

        let ac = self.accent;
        for i in 0..BAR_COUNT {
            let level = self.levels[i].clamp(0.08, 1.0);
            let bar_h = level * h;
            let bx = start_x + i as f32 * (bar_w + bar_gap);
            let by = bottom - bar_h;

            // Gradient-like effect: brighter at top
            let alpha_top = (180.0 + level * 75.0) as u8;
            let alpha_bot = (80.0 + level * 40.0) as u8;

            // Bottom portion (dimmer)
            let mut paint_bot = Paint::default();
            paint_bot.set_anti_alias(true);
            paint_bot.set_color(Color::from_argb(alpha_bot, ac.r(), ac.g(), ac.b()));
            let bot_h = bar_h * 0.6;
            canvas.draw_rrect(
                RRect::new_rect_xy(Rect::from_xywh(bx, bottom - bot_h, bar_w, bot_h), 2.0, 2.0),
                &paint_bot,
            );

            // Top portion (brighter)
            let mut paint_top = Paint::default();
            paint_top.set_anti_alias(true);
            paint_top.set_color(Color::from_argb(alpha_top, ac.r(), ac.g(), ac.b()));
            let top_h = bar_h * 0.5;
            canvas.draw_rrect(
                RRect::new_rect_xy(Rect::from_xywh(bx, by, bar_w, top_h), 2.0, 2.0),
                &paint_top,
            );
        }
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

    fn draw_equalizer_n(&self, canvas: &Canvas, x: f32, y: f32, w: f32, h: f32, count: usize) {
        let bar_w = 3.0f32;
        let bar_gap = 2.0f32;
        let bars_total = count as f32 * bar_w + (count as f32 - 1.0) * bar_gap;
        let start_x = x + (w - bars_total) / 2.0;
        let center_y = y + h / 2.0;

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        let ac = self.accent;
        paint.set_color(Color::from_argb(220, ac.r(), ac.g(), ac.b()));

        for i in 0..count {
            let level = self.levels[i % BAR_COUNT].clamp(0.05, 1.0);
            let bar_h = level * h;
            let bx = start_x + i as f32 * (bar_w + bar_gap);
            let by = center_y - bar_h / 2.0;
            let r = RRect::new_rect_xy(Rect::from_xywh(bx, by, bar_w, bar_h), 1.0, 1.0);
            canvas.draw_rrect(r, &paint);
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

    fn draw_progress(&self, canvas: &Canvas, x: f32, y: f32, w: f32) {
        let h = 3.0f32;
        let r = h / 2.0;

        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(Color::from_argb(60, 255, 255, 255));
        canvas.draw_rrect(RRect::new_rect_xy(Rect::from_xywh(x, y, w, h), r, r), &bg);

        let fill_w = (w * self.progress).max(h);
        let mut fg = Paint::default();
        fg.set_anti_alias(true);
        fg.set_color(self.accent);
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_xywh(x, y, fill_w, h), r, r),
            &fg,
        );
    }
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
}

pub type SharedPlayback = Arc<Mutex<PlaybackInfo>>;

// ---------------------------------------------------------------------------
// MusicMonitor — polls playerctl + captures PipeWire audio
// ---------------------------------------------------------------------------

pub struct MusicMonitor {
    pub playback: SharedPlayback,
    pub audio_level: Arc<Mutex<f32>>,
    /// Cached album art image + the URL it was loaded from
    album_art: Option<Image>,
    last_art_url: String,
    accent_color: Color,
    /// Equalizer state
    phase: f32,
    levels: [f32; BAR_COUNT],
    offsets: [f32; BAR_COUNT],
    last_track_name: String,
    /// Grace period: when the track disappears, wait before dismissing.
    gone_since: Option<std::time::Instant>,
    /// Island activity ID (None when no music activity exists)
    activity_id: Option<u64>,
}

impl MusicMonitor {
    pub fn new(playback: SharedPlayback, audio_level: Arc<Mutex<f32>>) -> Self {
        Self {
            playback,
            audio_level,
            album_art: None,
            last_art_url: String::new(),
            accent_color: Color::from_rgb(180, 180, 180),
            phase: 0.0,
            levels: [0.12; BAR_COUNT],
            offsets: [0.0; BAR_COUNT],
            last_track_name: String::new(),
            activity_id: None,
            gone_since: None,
        }
    }

    /// Tick the music monitor. Returns a renderer if music is playing.
    pub fn tick(&mut self) -> Option<MusicActivityRenderer> {
        let info = self.playback.lock().ok()?.clone();

        // Update album art if URL changed
        if info.art_url != self.last_art_url {
            self.last_art_url = info.art_url.clone();
            self.album_art = load_album_art(&info.art_url);
            if let Some(art) = &self.album_art {
                self.accent_color = extract_accent_color(art);
            } else {
                self.accent_color = Color::from_rgb(180, 180, 180);
            }
        }

        // Update equalizer
        self.phase += if info.is_playing { 0.35 } else { 0.08 };
        let base_level = self
            .audio_level
            .lock()
            .map(|v| *v)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let playing_gain = if info.is_playing { 1.4 } else { 0.35 };
        let envelope = (base_level * playing_gain).clamp(0.0, 1.0);

        // Spread frequencies widely so each bar moves independently.
        const FREQ: [f32; BAR_COUNT] = [0.6, 1.1, 0.8, 1.4, 0.5, 1.25, 0.7, 1.0];

        for i in 0..BAR_COUNT {
            let wave = ((self.phase * FREQ[i] + self.offsets[i]).sin() * 0.5) + 0.5;
            // Minimum idle shimmer so bars are never invisible.
            let idle = if info.is_playing {
                0.08 + wave * 0.07
            } else {
                0.03 + wave * 0.03
            };
            // Volume-driven height — scales with actual audio level.
            let driven = envelope * (0.4 + wave * 0.5);
            let target = (idle + driven).clamp(0.0, 1.0);
            let current = self.levels[i];
            // Smooth heavily for 5fps — ease toward target.
            self.levels[i] =
                current + (target - current) * if target > current { 0.45 } else { 0.30 };
        }

        if info.track_title != self.last_track_name {
            self.offsets = track_offsets(&info.track_title);
            self.last_track_name = info.track_title.clone();
        }

        // Keep island visible as long as there's a real track loaded.
        let has_track = !info.track_title.is_empty() && info.track_title != "No media";
        if has_track {
            Some(MusicActivityRenderer {
                title: info.track_title,
                artist: info.track_artist,
                album_art: self.album_art.clone(),
                is_playing: info.is_playing,
                progress: info.progress,
                duration_secs: info.duration_secs,
                accent: self.accent_color,
                levels: self.levels,
                pressed: None,
            })
        } else {
            None
        }
    }

    /// Sync with the island state: create/update/dismiss the music activity.
    /// Uses a 3-second grace period before dismissing to survive track changes.
    pub fn sync_to_island(&mut self, state: &SharedState) {
        let renderer = self.tick();

        let mut island = state.lock().unwrap();
        match (&renderer, self.activity_id) {
            (Some(_renderer), None) => {
                // Music started — create activity
                let id = island.create_activity(
                    "org.otto.music".to_string(),
                    "Now Playing".to_string(),
                    "audio-headphones".to_string(),
                    None,
                    0, // persistent
                    crate::activity::Priority::Normal,
                    true, // live
                );
                // Mark as Internal so the island system recognizes it as music.
                if let Some(a) = island.activities.iter_mut().find(|a| a.id == id) {
                    a.source = crate::activity::ActivitySource::Internal;
                }
                self.activity_id = Some(id);
                self.gone_since = None;
            }
            (Some(_renderer), Some(id)) => {
                // Music still playing — update title
                self.gone_since = None;
                if let Some(info) = self.playback.lock().ok() {
                    island.update_activity(id, &info.track_title, -1.0);
                }
            }
            (None, Some(id)) => {
                // Track gone — start or check grace period.
                let now = std::time::Instant::now();
                match self.gone_since {
                    None => {
                        self.gone_since = Some(now);
                    }
                    Some(since) if now.duration_since(since).as_secs_f64() >= 3.0 => {
                        island.dismiss_activity(id);
                        self.activity_id = None;
                        self.gone_since = None;
                    }
                    _ => {} // still within grace period
                }
            }
            (None, None) => {
                self.gone_since = None;
            }
        }
    }

    /// Get a renderer for the current state (if music is playing).
    pub fn renderer(&self) -> Option<MusicActivityRenderer> {
        let info = self.playback.lock().ok()?.clone();
        let has_track = !info.track_title.is_empty() && info.track_title != "No media";
        if !has_track {
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
            levels: self.levels,
            pressed: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Background threads
// ---------------------------------------------------------------------------

pub fn start_playerctl_monitor() -> SharedPlayback {
    let shared = Arc::new(Mutex::new(PlaybackInfo {
        track_title: "No media".to_string(),
        track_artist: String::new(),
        art_url: String::new(),
        is_playing: false,
        progress: 0.0,
        duration_secs: 0.0,
    }));
    let shared_for_thread = shared.clone();

    thread::spawn(move || loop {
        let is_playing = Command::new("playerctl")
            .arg("status")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .eq_ignore_ascii_case("playing")
            })
            .unwrap_or(false);

        let track_title = Command::new("playerctl")
            .args(["metadata", "--format", "{{title}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "No media".to_string());

        let track_artist = Command::new("playerctl")
            .args(["metadata", "--format", "{{artist}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let art_url = Command::new("playerctl")
            .args(["metadata", "mpris:artUrl"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let position = Command::new("playerctl")
            .args(["metadata", "--format", "{{position}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .unwrap_or(0.0);

        let length = Command::new("playerctl")
            .args(["metadata", "mpris:length"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .unwrap_or(1.0);

        let progress = if length > 0.0 {
            (position / length).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };

        // length is in microseconds from MPRIS
        let duration_secs = (length / 1_000_000.0) as f32;

        if let Ok(mut info) = shared_for_thread.lock() {
            info.track_title = track_title;
            info.track_artist = track_artist;
            info.art_url = art_url;
            info.is_playing = is_playing;
            info.progress = progress;
            info.duration_secs = duration_secs;
        }
        thread::sleep(Duration::from_millis(1500));
    });

    shared
}

pub fn start_pipewire_level_monitor() -> Arc<Mutex<f32>> {
    let shared_level = Arc::new(Mutex::new(0.0f32));
    let level_for_thread = shared_level.clone();

    thread::spawn(move || {
        if let Err(err) = run_pipewire_level_loop(level_for_thread) {
            tracing::error!("PipeWire level monitor failed: {err}");
        }
    });

    shared_level
}

fn run_pipewire_level_loop(shared_level: Arc<Mutex<f32>>) -> Result<(), pipewire::Error> {
    use pipewire as pw;
    use pw::properties::properties;
    use pw::spa;
    use spa::param::format::{MediaSubtype, MediaType};
    use spa::param::format_utils;
    use spa::pod::Pod;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    struct UserData {
        channels: u32,
        level: Arc<Mutex<f32>>,
        skip_count: u32,
    }

    let user_data = UserData {
        channels: 2,
        level: shared_level,
        skip_count: 0,
    };

    let stream = pw::stream::StreamBox::new(
        &core,
        "otto-islands-audio-capture",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::STREAM_CAPTURE_SINK => "true",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else { return };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut audio_info = spa::param::audio::AudioInfoRaw::default();
            if audio_info.parse(param).is_ok() {
                user_data.channels = audio_info.channels().max(1);
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let n_channels = user_data.channels.max(1) as usize;
            let chunk = data.chunk();
            let chunk_offset = chunk.offset() as usize;
            let chunk_size = chunk.size() as usize;
            let Some(samples) = data.data() else { return };
            let start = chunk_offset;
            let end = start.saturating_add(chunk_size).min(samples.len());
            if end <= start {
                return;
            }
            let bytes = &samples[start..end];
            let sample_count = bytes.len() / std::mem::size_of::<f32>();
            if sample_count == 0 {
                return;
            }
            let mut peak = 0.0f32;
            let mut sum_sq = 0.0f32;
            let mut seen = 0usize;
            for n in (0..sample_count).step_by(n_channels) {
                let s = n * std::mem::size_of::<f32>();
                let e = s + std::mem::size_of::<f32>();
                if e > bytes.len() {
                    break;
                }
                let val =
                    f32::from_le_bytes([bytes[s], bytes[s + 1], bytes[s + 2], bytes[s + 3]]).abs();
                peak = peak.max(val);
                sum_sq += val * val;
                seen += 1;
            }
            if seen == 0 {
                return;
            }
            let rms = (sum_sq / seen as f32).sqrt();
            let normalized = (peak * 1.35 + rms * 0.65).clamp(0.0, 1.0);
            // Throttle: only update shared level every ~6 buffers (~15fps).
            user_data.skip_count += 1;
            if user_data.skip_count >= 6 {
                user_data.skip_count = 0;
                if let Ok(mut level) = user_data.level.lock() {
                    *level = normalized;
                }
            }
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).unwrap()];

    let target_object = std::env::var("OTTO_ISLANDS_PW_TARGET")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .or_else(detect_default_sink_node_id);

    stream.connect(
        spa::utils::Direction::Input,
        target_object,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    mainloop.run();
    Ok(())
}

fn detect_default_sink_node_id() -> Option<u32> {
    let output = Command::new("wpctl")
        .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next()?.trim();
    let id_token = first_line
        .strip_prefix("id ")?
        .split(',')
        .next()
        .map(str::trim)?;
    id_token.parse::<u32>().ok()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute a music player action via playerctl.
pub fn execute_action(action: MusicAction) {
    match action {
        MusicAction::Seek(frac) => {
            thread::spawn(move || {
                // Get track length, compute absolute position, seek.
                let length = Command::new("playerctl")
                    .args(["metadata", "mpris:length"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .trim()
                            .parse::<f64>()
                            .ok()
                    })
                    .unwrap_or(0.0);
                if length > 0.0 {
                    // length is in microseconds; playerctl position takes seconds.
                    let secs = frac as f64 * length / 1_000_000.0;
                    let _ = Command::new("playerctl")
                        .args(["position", &format!("{secs:.1}")])
                        .status();
                }
            });
        }
        action => {
            let arg = match action {
                MusicAction::PlayPause => "play-pause",
                MusicAction::SkipNext => "next",
                MusicAction::SkipPrev => "previous",
                MusicAction::Seek(_) => unreachable!(),
            };
            thread::spawn(move || {
                let _ = Command::new("playerctl").arg(arg).status();
            });
        }
    }
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

fn load_album_art(url: &str) -> Option<Image> {
    let path = url.strip_prefix("file://")?;
    let bytes = fs::read(path).ok()?;
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

fn track_offsets(track: &str) -> [f32; BAR_COUNT] {
    let mut hash: u32 = 2166136261;
    for b in track.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let mut out = [0.0f32; BAR_COUNT];
    for item in out.iter_mut() {
        let normalized = (hash & 0xFFFF) as f32 / 65535.0;
        hash = hash.wrapping_mul(1664525).wrapping_add(1013904223);
        *item = normalized * std::f32::consts::TAU;
    }
    out
}
