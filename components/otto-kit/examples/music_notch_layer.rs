//! Dynamic Island — feature plugin system
//!
//! Each feature (Music, Notification) implements `IslandFeature`:
//!   - 3 sizes: reduced, hover, open
//!   - 3 draw methods: draw_reduced, draw_hover, draw_open
//!
//! The `Island` struct owns the active feature stack and current ViewMode.

use otto_kit::{
    app_runner::AppRunner,
    input::keycodes,
    protocols::otto_surface_style_v1::ClipMode,
    surfaces::{LayerShellSurface, SubsurfaceSurface},
    typography::TextStyle,
    utils::extract_accent_color,
    App, AppContext,
};
use pipewire as pw;
use skia_safe::{Color, Data, Font, Image, Paint, RRect, Rect};
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use std::cell::RefCell;
use std::fs;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use wayland_client::protocol::wl_keyboard;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer, zwlr_layer_surface_v1::Anchor,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BAR_COUNT: usize = 8;

fn font(size: f32) -> Font {
    TextStyle { family: "Inter", weight: 400, size }.font()
}

fn font_bold(size: f32) -> Font {
    TextStyle { family: "Inter", weight: 600, size }.font()
}

/// Layer shell anchor — max possible island size, transparent container
const LAYER_W: u32 = 480;
const LAYER_H: u32 = 110;

// ---------------------------------------------------------------------------
// IslandFeature trait
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewMode {
    Reduced,
    Hover,
    Open,
}

trait IslandFeature {
    fn reduced_size(&self) -> (f32, f32);
    fn hover_size(&self) -> (f32, f32);
    fn open_size(&self) -> (f32, f32);

    fn draw_reduced(&self, canvas: &skia_safe::Canvas, w: f32, h: f32);
    fn draw_hover(&self, canvas: &skia_safe::Canvas, w: f32, h: f32);
    fn draw_open(&self, canvas: &skia_safe::Canvas, w: f32, h: f32);

    fn size_for_mode(&self, mode: ViewMode) -> (f32, f32) {
        match mode {
            ViewMode::Reduced => self.reduced_size(),
            ViewMode::Hover => self.hover_size(),
            ViewMode::Open => self.open_size(),
        }
    }

    fn draw_for_mode(&self, canvas: &skia_safe::Canvas, mode: ViewMode, w: f32, h: f32) {
        match mode {
            ViewMode::Reduced => self.draw_reduced(canvas, w, h),
            ViewMode::Hover => self.draw_hover(canvas, w, h),
            ViewMode::Open => self.draw_open(canvas, w, h),
        }
    }
}

// ---------------------------------------------------------------------------
// Island — owns active features and current mode
// ---------------------------------------------------------------------------

struct Island {
    mode: ViewMode,
    primary: Option<Box<dyn IslandFeature>>,
    notification: Option<Box<dyn IslandFeature>>,
}

impl Island {
    fn new() -> Self {
        Self { mode: ViewMode::Reduced, primary: None, notification: None }
    }

    /// Total island size: primary + notification stacked vertically
    fn total_size(&self) -> (f32, f32) {
        let (pw, ph) = self
            .primary
            .as_ref()
            .map(|f| f.size_for_mode(self.mode))
            .unwrap_or((28.0, 28.0)); // idle circle when no feature

        let notif_h = self
            .notification
            .as_ref()
            .map(|f| f.size_for_mode(self.mode).1)
            .unwrap_or(0.0);

        let total_h = ph + if notif_h > 0.0 { notif_h + 1.0 } else { 0.0 };
        (pw, total_h)
    }

    /// Draw the island centered in a canvas of size `canvas_w × canvas_h`,
    /// with the pill clipped to `pill_w × pill_h` at vertical offset `pill_y`.
    fn draw(&self, canvas: &skia_safe::Canvas, canvas_w: f32, canvas_h: f32, pill_w: f32, pill_h: f32, pill_y: f32) {
        canvas.clear(Color::TRANSPARENT);

        let ox = (canvas_w - pill_w) / 2.0;
        let oy = pill_y;

        // Draw black pill background centered in canvas
        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(Color::from_rgb(8, 8, 8));
        // Open mode: fixed radius looks like a card; reduced/hover: full pill
        let r = match self.mode {
            ViewMode::Open => 16.0f32,
            _ => pill_h / 2.0,
        };
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_xywh(ox, oy, pill_w, pill_h), r, r),
            &bg,
        );

        // Gray border drawn on top of background, inside the pill bounds
        let mut border = Paint::default();
        border.set_anti_alias(true);
        border.set_color(Color::from_argb(60, 180, 180, 180));
        border.set_style(skia_safe::paint::Style::Stroke);
        border.set_stroke_width(1.0);
        canvas.draw_rrect(
            RRect::new_rect_xy(Rect::from_xywh(ox + 0.5, oy + 0.5, pill_w - 1.0, pill_h - 1.0), r, r),
            &border,
        );

        // Clip further drawing to the pill bounds
        canvas.save();
        canvas.clip_rrect(
            RRect::new_rect_xy(Rect::from_xywh(ox, oy, pill_w, pill_h), r, r),
            skia_safe::ClipOp::Intersect,
            true,
        );
        canvas.translate((ox, oy));

        if let Some(primary) = &self.primary {
            let (_, ph) = primary.size_for_mode(self.mode);
            primary.draw_for_mode(canvas, self.mode, pill_w, ph);

            if let Some(notif) = &self.notification {
                let (_, nh) = notif.size_for_mode(self.mode);
                // divider
                let mut div = Paint::default();
                div.set_anti_alias(true);
                div.set_color(Color::from_argb(40, 255, 255, 255));
                canvas.draw_rect(Rect::from_xywh(12.0, ph, pill_w - 24.0, 1.0), &div);

                canvas.save();
                canvas.translate((0.0, ph + 1.0));
                notif.draw_for_mode(canvas, self.mode, pill_w, nh);
                canvas.restore();
            }
        } else if let Some(notif) = &self.notification {
            notif.draw_for_mode(canvas, self.mode, pill_w, pill_h);
        } else {
            // Idle: white dot inside the black circle
            let mut p = Paint::default();
            p.set_anti_alias(true);
            p.set_color(Color::WHITE);
            canvas.draw_circle((pill_w / 2.0, pill_h / 2.0), pill_h * 0.2, &p);
        }

        canvas.restore();
    }
}

// ---------------------------------------------------------------------------
// MusicFeature
// ---------------------------------------------------------------------------

struct MusicFeature {
    levels: [f32; BAR_COUNT],
    title: String,
    artist: String,
    album_art: Option<Image>,
    is_playing: bool,
    progress: f32, // 0.0 - 1.0
    accent: Color,
}

impl MusicFeature {
    /// Draw the album art (or a placeholder) as a rounded square at (x, y) with given size.
    fn draw_art(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, size: f32) {
        let r = size * 0.18;
        let dst = Rect::from_xywh(x, y, size, size);
        if let Some(art) = &self.album_art {
            canvas.save();
            canvas.clip_rrect(RRect::new_rect_xy(dst, r, r), skia_safe::ClipOp::Intersect, true);
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
            // small music note
            let nf = font(size * 0.42);
            let mut np = Paint::default();
            np.set_anti_alias(true);
            np.set_color(Color::from_argb(130, 255, 255, 255));
            canvas.draw_str("♪", (x + size * 0.2, y + size * 0.68), &nf, &np);
        }
    }

    fn draw_equalizer(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, w: f32, h: f32) {
        let bar_w = 2.0f32;
        let bar_gap = 3.0f32;
        let bars_total = BAR_COUNT as f32 * bar_w + (BAR_COUNT - 1) as f32 * bar_gap;
        let start_x = x + (w - bars_total) / 2.0;
        let center_y = y + h / 2.0;

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        // Use accent color for bars, fallback to white
        let ac = self.accent;
        paint.set_color(Color::from_argb(220, ac.r(), ac.g(), ac.b()));

        for i in 0..BAR_COUNT {
            let level = self.levels[i].clamp(0.05, 1.0);
            let bar_h = level * h;
            let bx = start_x + i as f32 * (bar_w + bar_gap);
            let by = center_y - bar_h / 2.0;
            let r = RRect::new_rect_xy(Rect::from_xywh(bx, by, bar_w, bar_h), 1.0, 1.0);
            canvas.draw_rrect(r, &paint);
        }
    }

    fn draw_text(
        canvas: &skia_safe::Canvas,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        alpha: u8,
        max_w: f32,
    ) {
        let f = font_bold(size);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(alpha, 255, 255, 255));
        let label = trim_to_width(text, &f, max_w.max(20.0));
        canvas.draw_str(label, (x, y), &f, &paint);
    }

    fn draw_play_pause(&self, canvas: &skia_safe::Canvas, cx: f32, cy: f32, size: f32) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::WHITE);

        if self.is_playing {
            // Two vertical bars
            let bar_w = size * 0.28;
            let bar_h = size * 0.8;
            let gap = size * 0.22;
            let lx = cx - gap / 2.0 - bar_w;
            let rx = cx + gap / 2.0;
            let ty = cy - bar_h / 2.0;
            canvas.draw_rect(Rect::from_xywh(lx, ty, bar_w, bar_h), &paint);
            canvas.draw_rect(Rect::from_xywh(rx, ty, bar_w, bar_h), &paint);
        } else {
            // Triangle pointing right
            let path = {
                let mut p = skia_safe::Path::new();
                let h = size * 0.85;
                let w = h * 0.866; // equilateral
                p.move_to((cx - w / 2.0, cy - h / 2.0));
                p.line_to((cx + w / 2.0, cy));
                p.line_to((cx - w / 2.0, cy + h / 2.0));
                p.close();
                p
            };
            canvas.draw_path(&path, &paint);
        }
    }

    fn draw_skip_prev(&self, canvas: &skia_safe::Canvas, cx: f32, cy: f32, size: f32) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(200, 255, 255, 255));

        // Left-pointing triangle + vertical bar
        let h = size * 0.75;
        let w = h * 0.866;
        let bar_w = size * 0.18;
        let tx = cx + bar_w / 2.0;
        let mut path = skia_safe::Path::new();
        path.move_to((tx, cy - h / 2.0));
        path.line_to((tx - w, cy));
        path.line_to((tx, cy + h / 2.0));
        path.close();
        canvas.draw_path(&path, &paint);
        canvas.draw_rect(
            Rect::from_xywh(cx - w - bar_w / 2.0, cy - h / 2.0, bar_w, h),
            &paint,
        );
    }

    fn draw_skip_next(&self, canvas: &skia_safe::Canvas, cx: f32, cy: f32, size: f32) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::from_argb(200, 255, 255, 255));

        // Right-pointing triangle + vertical bar
        let h = size * 0.75;
        let w = h * 0.866;
        let bar_w = size * 0.18;
        let tx = cx - bar_w / 2.0;
        let mut path = skia_safe::Path::new();
        path.move_to((tx, cy - h / 2.0));
        path.line_to((tx + w, cy));
        path.line_to((tx, cy + h / 2.0));
        path.close();
        canvas.draw_path(&path, &paint);
        canvas.draw_rect(
            Rect::from_xywh(cx + w - bar_w / 2.0, cy - h / 2.0, bar_w, h),
            &paint,
        );
    }

    fn draw_progress(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, w: f32) {
        let h = 3.0f32;
        let r = h / 2.0;

        // Track
        let mut bg = Paint::default();
        bg.set_anti_alias(true);
        bg.set_color(Color::from_argb(60, 255, 255, 255));
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
    }
}

impl IslandFeature for MusicFeature {
    fn reduced_size(&self) -> (f32, f32) { (280.0, 32.0) }
    fn hover_size(&self) -> (f32, f32) { (360.0, 56.0) }
    fn open_size(&self) -> (f32, f32) { (460.0, 100.0) }

    fn draw_reduced(&self, canvas: &skia_safe::Canvas, w: f32, h: f32) {
        let v_pad = 7.0;  // more air around the art
        let h_pad = 10.0;
        let art_size = h - v_pad * 2.0;
        let art_x = h_pad;
        let art_y = v_pad;

        // Album art — larger, uses most of the height
        self.draw_art(canvas, art_x, art_y, art_size);

        // Equalizer on the far right
        let eq_area_w = BAR_COUNT as f32 * 4.0 + (BAR_COUNT as f32 - 1.0) * 2.0;
        let eq_x = w - h_pad - eq_area_w;
        let eq_y = v_pad;
        let eq_h = h - v_pad * 2.0;
        self.draw_equalizer(canvas, eq_x, eq_y, eq_area_w, eq_h);

        // Title + artist between art and equalizer
        let text_x = art_x + art_size + h_pad;
        let text_max_w = eq_x - text_x - h_pad;

        let mid = h / 2.0;
        Self::draw_text(canvas, &self.title, text_x, mid - 1.0, 11.0, 220, text_max_w);

        let af = font(9.5);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(140, 170, 170, 170));
        let artist = trim_to_width(&self.artist, &af, text_max_w.max(20.0));
        canvas.draw_str(artist, (text_x, mid + 10.0), &af, &ap);
    }

    fn draw_hover(&self, canvas: &skia_safe::Canvas, w: f32, h: f32) {
        let h_pad = 12.0;  // +2px right
        let v_pad = 7.0;
        let prog_h = 3.0;
        let art_size = h - v_pad * 2.0 - prog_h;
        let art_x = h_pad;
        let art_y = v_pad;

        // Album art
        self.draw_art(canvas, art_x, art_y, art_size);

        // Equalizer on the far right
        let eq_area_w = BAR_COUNT as f32 * 2.0 + (BAR_COUNT as f32 - 1.0) * 3.0;
        let eq_x = w - h_pad - eq_area_w;
        let content_h = h - prog_h;
        let eq_y = v_pad + (art_size - art_size * 0.6) / 2.0;
        let eq_h = art_size * 0.6;
        self.draw_equalizer(canvas, eq_x, eq_y, eq_area_w, eq_h);

        // Title + artist
        let text_x = art_x + art_size + h_pad;
        let text_max_w = eq_x - text_x - h_pad;

        let mid = content_h / 2.0;
        Self::draw_text(canvas, &self.title, text_x, mid - 4.0, 14.0, 255, text_max_w);

        let af = font(11.5);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(170, 190, 190, 190));
        let artist = trim_to_width(&self.artist, &af, text_max_w.max(20.0));
        canvas.draw_str(artist, (text_x, mid + 13.0), &af, &ap);

        // Progress bar flush at the bottom border
        self.draw_progress(canvas, 0.0, h - prog_h, w);
    }

    fn draw_open(&self, canvas: &skia_safe::Canvas, w: f32, h: f32) {
        let pad = 14.0;
        let art_size = 72.0;
        let art_x = pad;
        let art_y = (h - art_size) / 2.0;
        let ctrl_h = 32.0;
        let prog_h = 8.0;
        let text_top = pad + 4.0;

        // Album art using shared helper
        self.draw_art(canvas, art_x, art_y, art_size);

        // Title + artist
        let text_x = art_x + art_size + pad;
        let text_max_w = w - text_x - pad;
        Self::draw_text(canvas, &self.title, text_x, text_top + 14.0, 14.0, 255, text_max_w);

        let af = font(11.0);
        let mut ap = Paint::default();
        ap.set_anti_alias(true);
        ap.set_color(Color::from_argb(160, 180, 180, 180));
        let artist = trim_to_width(&self.artist, &af, text_max_w.max(20.0));
        canvas.draw_str(artist, (text_x, text_top + 30.0), &af, &ap);

        // Controls row (prev / play-pause / next) centered
        let ctrl_y = h - ctrl_h - prog_h - 8.0;
        let ctrl_cx = w / 2.0;
        let icon_gap = 40.0;
        let icon_size = 16.0;

        self.draw_skip_prev(canvas, ctrl_cx - icon_gap, ctrl_y + ctrl_h / 2.0, icon_size);
        self.draw_play_pause(canvas, ctrl_cx, ctrl_y + ctrl_h / 2.0, icon_size + 4.0);
        self.draw_skip_next(canvas, ctrl_cx + icon_gap, ctrl_y + ctrl_h / 2.0, icon_size);

        // Progress bar
        let prog_y = h - prog_h - 4.0;
        self.draw_progress(canvas, pad + art_size + pad, prog_y, w - (art_size + pad * 3.0));
    }
}

// ---------------------------------------------------------------------------
// NotificationFeature
// ---------------------------------------------------------------------------

struct NotificationFeature {
    title: String,
    body: String,
    app_icon: Option<Image>,
}

impl IslandFeature for NotificationFeature {
    fn reduced_size(&self) -> (f32, f32) { (280.0, 46.0) }
    fn hover_size(&self) -> (f32, f32) { (360.0, 60.0) }
    fn open_size(&self) -> (f32, f32) { (400.0, 90.0) }

    fn draw_reduced(&self, canvas: &skia_safe::Canvas, w: f32, h: f32) {
        let pad = 14.0;
        // Bell dot on left
        draw_bell(canvas, pad + 8.0, h / 2.0, 10.0);
        let text_x = pad + 24.0;
        let max_w = w - text_x - pad;
        draw_white_text(canvas, &self.title, text_x, h / 2.0 + 5.0, 12.0, 220, max_w);
    }

    fn draw_hover(&self, canvas: &skia_safe::Canvas, w: f32, h: f32) {
        let pad = 14.0;
        draw_bell(canvas, pad + 8.0, h * 0.38, 11.0);
        let text_x = pad + 26.0;
        let max_w = w - text_x - pad;
        draw_white_text(canvas, &self.title, text_x, h / 2.0 - 2.0, 13.0, 255, max_w);
        draw_white_text(canvas, &self.body, text_x, h / 2.0 + 14.0, 11.0, 150, max_w);
    }

    fn draw_open(&self, canvas: &skia_safe::Canvas, w: f32, _h: f32) {
        let pad = 14.0;
        draw_bell(canvas, pad + 10.0, 22.0, 14.0);
        let text_x = pad + 32.0;
        let max_w = w - text_x - pad;
        draw_white_text(canvas, &self.title, text_x, 24.0, 14.0, 255, max_w);

        let bf = font(11.5);
        let mut bp = Paint::default();
        bp.set_anti_alias(true);
        bp.set_color(Color::from_argb(180, 200, 200, 200));
        // Wrap body into two lines approx
        let body_lines = wrap_text(&self.body, &bf, w - pad * 2.0);
        for (i, line) in body_lines.iter().take(2).enumerate() {
            canvas.draw_str(line, (pad, 44.0 + i as f32 * 16.0), &bf, &bp);
        }
    }
}

fn draw_bell(canvas: &skia_safe::Canvas, cx: f32, cy: f32, size: f32) {
    let mut p = Paint::default();
    p.set_anti_alias(true);
    p.set_color(Color::from_argb(200, 255, 255, 255));
    p.set_stroke_width(1.5);
    p.set_style(skia_safe::paint::Style::Stroke);

    // Bell body (rounded rect)
    let bw = size;
    let bh = size * 0.85;
    let bx = cx - bw / 2.0;
    let by = cy - bh * 0.6;
    canvas.draw_rrect(
        RRect::new_rect_xy(Rect::from_xywh(bx, by, bw, bh), bw * 0.3, bh * 0.3),
        &p,
    );
    // Clapper
    let mut filled = p.clone();
    filled.set_style(skia_safe::paint::Style::Fill);
    canvas.draw_circle((cx, cy + bh * 0.45), size * 0.15, &filled);
}

fn draw_white_text(
    canvas: &skia_safe::Canvas,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    alpha: u8,
    max_w: f32,
) {
    let f = font(size);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::from_argb(alpha, 255, 255, 255));
    let label = trim_to_width(text, &f, max_w.max(20.0));
    canvas.draw_str(label, (x, y), &f, &paint);
}

fn trim_to_width(text: &str, font: &Font, width: f32) -> String {
    if font.measure_str(text, None).0 <= width {
        return text.to_string();
    }
    let ellipsis = "…";
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        let candidate = format!("{}{}", out, ellipsis);
        if font.measure_str(&candidate, None).0 > width {
            out.pop();
            break;
        }
    }
    format!("{}{}", out, ellipsis)
}

fn wrap_text(text: &str, font: &Font, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{} {}", current, word)
        };
        if font.measure_str(&candidate, None).0 > max_w && !current.is_empty() {
            lines.push(current.clone());
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Spring constant for manual animation (applied each frame)
const SPRING: f32 = 0.18;

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct MusicNotchApp {
    layer_surface: Option<LayerShellSurface>,
    island_surface: Option<Rc<RefCell<SubsurfaceSurface>>>,
    island: Rc<RefCell<Island>>,
    visualizer: Rc<RefCell<VisualizerState>>,
    /// Current animated pill size (lerped each frame toward target)
    anim_w: Rc<RefCell<f32>>,
    anim_h: Rc<RefCell<f32>>,
    anim_y: Rc<RefCell<f32>>,
    /// Target pill size and position (set on mode change)
    target_w: Rc<RefCell<f32>>,
    target_h: Rc<RefCell<f32>>,
    target_y: Rc<RefCell<f32>>,
    hovered: Rc<RefCell<bool>>,
    open: Rc<RefCell<bool>>,
}

fn sync_island_features(viz: &VisualizerState, island: &mut Island) {
    if viz.is_playing {
        island.primary = Some(Box::new(MusicFeature {
            levels: viz.levels,
            title: viz.track_title.clone(),
            artist: viz.track_artist.clone(),
            album_art: viz.album_art.clone(),
            is_playing: viz.is_playing,
            progress: viz.progress,
            accent: viz.accent_color,
        }));
    } else {
        island.primary = None;
    }

    if let Some(notif) = viz.notifications.first() {
        island.notification = Some(Box::new(NotificationFeature {
            title: notif.title.clone(),
            body: notif.body.clone(),
            app_icon: None,
        }));
    } else {
        island.notification = None;
    }
}

impl MusicNotchApp {
    fn sync_island_features(&mut self) {
        let viz = self.visualizer.borrow();
        let mut island = self.island.borrow_mut();
        sync_island_features(&viz, &mut island);
    }

    fn update_mode(&mut self) {
        let mode = if *self.open.borrow() {
            ViewMode::Open
        } else if *self.hovered.borrow() {
            ViewMode::Hover
        } else {
            ViewMode::Reduced
        };
        self.island.borrow_mut().mode = mode;
    }

    fn set_target_size(&self) {
        let island = self.island.borrow();
        let (w, h) = island.total_size();
        *self.target_w.borrow_mut() = w;
        *self.target_h.borrow_mut() = h;
        // Y position: reduced stays near top; hover drops down a bit
        let y = match island.mode {
            ViewMode::Reduced => 2.0,
            ViewMode::Hover   => 5.0,
            ViewMode::Open    => 4.0,
        };
        *self.target_y.borrow_mut() = y;
    }

    fn redraw(&self) {
        if let Some(sub) = &self.island_surface {
            let island = self.island.borrow();
            let aw = *self.anim_w.borrow();
            let ah = *self.anim_h.borrow();
            let ay = *self.anim_y.borrow();
            sub.borrow().draw(|canvas| {
                island.draw(canvas, LAYER_W as f32, LAYER_H as f32, aw, ah, ay);
            });
        }
    }

    fn update(&mut self) {
        self.sync_island_features();
        self.update_mode();
        self.set_target_size();
        self.redraw();
    }
}

// ---------------------------------------------------------------------------
// VisualizerState
// ---------------------------------------------------------------------------

struct VisualizerState {
    phase: f32,
    levels: [f32; BAR_COUNT],
    offsets: [f32; BAR_COUNT],
    track_title: String,
    track_artist: String,
    is_playing: bool,
    album_art: Option<Image>,
    last_art_url: String,
    accent_color: Color,
    progress: f32,
    audio_level: Arc<Mutex<f32>>,
    playback_info: Arc<Mutex<PlaybackInfo>>,
    notifications: Vec<NotificationInfo>,
}

struct NotificationInfo {
    title: String,
    body: String,
    expires_at: Instant,
}

impl VisualizerState {
    fn new(audio_level: Arc<Mutex<f32>>, playback_info: Arc<Mutex<PlaybackInfo>>) -> Self {
        Self {
            phase: 0.0,
            levels: [0.12; BAR_COUNT],
            offsets: [0.0; BAR_COUNT],
            track_title: "No media".to_string(),
            track_artist: String::new(),
            is_playing: false,
            album_art: None,
            last_art_url: String::new(),
            accent_color: Color::from_rgb(180, 180, 180),
            progress: 0.0,
            audio_level,
            playback_info,
            notifications: Vec::new(),
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.notifications.retain(|n| n.expires_at > now);

        if let Ok(info) = self.playback_info.lock() {
            if info.track_title != self.track_title || info.track_artist != self.track_artist {
                self.track_title = info.track_title.clone();
                self.track_artist = info.track_artist.clone();
                self.offsets = track_offsets(&self.track_title);
            }
            self.is_playing = info.is_playing;
            self.progress = info.progress;

            if info.art_url != self.last_art_url {
                self.last_art_url = info.art_url.clone();
                self.album_art = load_album_art(&info.art_url);
                if let Some(art) = &self.album_art {
                    self.accent_color = extract_accent_color(art);
                } else {
                    self.accent_color = Color::from_rgb(180, 180, 180);
                }
            }
        }

        self.phase += if self.is_playing { 0.72 } else { 0.18 };
        let base_level = self.audio_level.lock().map(|v| *v).unwrap_or(0.0).clamp(0.0, 1.0);
        let playing_gain = if self.is_playing { 1.4 } else { 0.35 };
        let envelope = (base_level * playing_gain).clamp(0.0, 1.0);

        for i in 0..BAR_COUNT {
            let wave = ((self.phase * (1.0 + i as f32 * 0.17) + self.offsets[i]).sin() * 0.5) + 0.5;
            let idle_amplitude = if self.is_playing { 0.10 } else { 0.04 };
            let idle = idle_amplitude * wave;
            let audio_animated = (envelope * (0.75 + wave * 0.45)).clamp(0.0, 1.0);
            let target = (idle + audio_animated * 0.85).clamp(0.0, 1.0);
            let current = self.levels[i];
            self.levels[i] = current + (target - current) * if target > current { 0.25 } else { 0.08 };
        }
    }
}

fn track_offsets(track: &str) -> [f32; BAR_COUNT] {
    let mut hash: u32 = 2166136261;
    for b in track.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let mut out = [0.0f32; BAR_COUNT];
    for item in out.iter_mut().take(BAR_COUNT) {
        let normalized = (hash & 0xFFFF) as f32 / 65535.0;
        hash = hash.wrapping_mul(1664525).wrapping_add(1013904223);
        *item = normalized * std::f32::consts::TAU;
    }
    out
}

fn load_album_art(url: &str) -> Option<Image> {
    let path = url.strip_prefix("file://")?;
    let bytes = fs::read(path).ok()?;
    let data = Data::new_copy(&bytes);
    Image::from_encoded(data)
}

// ---------------------------------------------------------------------------
// PipeWire & playerctl
// ---------------------------------------------------------------------------

struct PipeWireUserData {
    channels: u32,
    level: Arc<Mutex<f32>>,
}

#[derive(Clone)]
struct PlaybackInfo {
    track_title: String,
    track_artist: String,
    art_url: String,
    is_playing: bool,
    progress: f32,
}

fn start_playerctl_monitor() -> Arc<Mutex<PlaybackInfo>> {
    let shared = Arc::new(Mutex::new(PlaybackInfo {
        track_title: "No media".to_string(),
        track_artist: String::new(),
        art_url: String::new(),
        is_playing: false,
        progress: 0.0,
    }));
    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        loop {
            let is_playing = Command::new("playerctl")
                .arg("status")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("playing"))
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

            // Progress: position / length (0.0 - 1.0)
            let position = Command::new("playerctl")
                .args(["metadata", "--format", "{{position}}"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
                .unwrap_or(0.0);

            let length = Command::new("playerctl")
                .args(["metadata", "mpris:length"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
                .unwrap_or(1.0);

            let progress = if length > 0.0 { (position / length).clamp(0.0, 1.0) as f32 } else { 0.0 };

            if let Ok(mut info) = shared_for_thread.lock() {
                info.track_title = track_title;
                info.track_artist = track_artist;
                info.art_url = art_url;
                info.is_playing = is_playing;
                info.progress = progress;
            }
            thread::sleep(Duration::from_millis(1500));
        }
    });

    shared
}

fn start_pipewire_level_monitor() -> Arc<Mutex<f32>> {
    let shared_level = Arc::new(Mutex::new(0.0f32));
    let level_for_thread = shared_level.clone();

    thread::spawn(move || {
        if let Err(err) = run_pipewire_level_loop(level_for_thread) {
            eprintln!("PipeWire level monitor failed: {}", err);
        }
    });

    shared_level
}

fn run_pipewire_level_loop(shared_level: Arc<Mutex<f32>>) -> Result<(), pw::Error> {
    use pw::properties::properties;
    use pw::spa;
    use spa::param::format::{MediaSubtype, MediaType};
    use spa::param::format_utils;
    use spa::pod::Pod;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let user_data = PipeWireUserData { channels: 2, level: shared_level };

    let stream = pw::stream::StreamBox::new(
        &core,
        "music-notch-audio-capture",
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
            if id != pw::spa::param::ParamType::Format.as_raw() { return; }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else { return; };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw { return; }
            let mut audio_info = spa::param::audio::AudioInfoRaw::default();
            if audio_info.parse(param).is_ok() {
                user_data.channels = audio_info.channels().max(1);
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() { return }
            let data = &mut datas[0];
            let n_channels = user_data.channels.max(1) as usize;
            let chunk = data.chunk();
            let chunk_offset = chunk.offset() as usize;
            let chunk_size = chunk.size() as usize;
            let Some(samples) = data.data() else { return };
            let start = chunk_offset;
            let end = start.saturating_add(chunk_size).min(samples.len());
            if end <= start { return }
            let bytes = &samples[start..end];
            let sample_count = bytes.len() / std::mem::size_of::<f32>();
            if sample_count == 0 { return }
            let mut peak = 0.0f32;
            let mut sum_sq = 0.0f32;
            let mut seen = 0usize;
            for n in (0..sample_count).step_by(n_channels) {
                let s = n * std::mem::size_of::<f32>();
                let e = s + std::mem::size_of::<f32>();
                if e > bytes.len() { break }
                let val = f32::from_le_bytes([bytes[s], bytes[s+1], bytes[s+2], bytes[s+3]]).abs();
                peak = peak.max(val);
                sum_sq += val * val;
                seen += 1;
            }
            if seen == 0 { return }
            let rms = (sum_sq / seen as f32).sqrt();
            let normalized = (peak * 1.35 + rms * 0.65).clamp(0.0, 1.0);
            if let Ok(mut level) = user_data.level.lock() {
                *level = normalized;
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

    let target_object = std::env::var("MUSIC_NOTCH_PW_TARGET")
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
    if !output.status.success() { return None; }
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next()?.trim();
    let id_token = first_line.strip_prefix("id ")?.split(',').next().map(str::trim)?;
    id_token.parse::<u32>().ok()
}

// ---------------------------------------------------------------------------
// App impl
// ---------------------------------------------------------------------------

impl App for MusicNotchApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        let layer_surface = LayerShellSurface::new(Layer::Overlay, "music-notch", LAYER_W, LAYER_H)?;
        layer_surface.set_anchor(Anchor::Top);
        layer_surface.set_margin(2, 0, 0, 0);
        layer_surface.set_exclusive_zone(1);
        layer_surface.set_keyboard_interactivity(
            wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        );
        if let Some(style) = layer_surface.base_surface().surface_style() {
            style.set_masks_to_bounds(ClipMode::Enabled);
        }
        self.layer_surface = Some(layer_surface);
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, _width: i32, _height: i32, _serial: u32) {
        if self.island_surface.is_some() {
            self.update();
            return;
        }

        let Some(layer) = &self.layer_surface else { return };

        // Transparent buffer for the parent so compositor has rendering data
        layer.draw(|canvas| {
            canvas.clear(skia_safe::Color::TRANSPARENT);
        });
        layer.base_surface().on_frame(|| {});

        // Clip subsurface children to this layer's bounds
        if let Some(ss) = layer.base_surface().surface_style() {
            ss.set_masks_to_bounds(ClipMode::Enabled);
        }

        let wl = layer.base_surface().wl_surface().clone();

        // Subsurface at (0,0) — full canvas; pill is drawn centered by Island::draw
        let sub = match SubsurfaceSurface::new(&wl, 0, 0, LAYER_W as i32, LAYER_H as i32) {
            Ok(s) => s,
            Err(e) => { eprintln!("Failed to create subsurface: {e}"); return; }
        };

        // Initialise animated size to the current target size
        let (iw, ih) = self.island.borrow().total_size();
        *self.anim_w.borrow_mut() = iw;
        *self.anim_h.borrow_mut() = ih;
        *self.anim_y.borrow_mut() = 2.0;
        *self.target_w.borrow_mut() = iw;
        *self.target_h.borrow_mut() = ih;
        *self.target_y.borrow_mut() = 2.0;

        let sub = Rc::new(RefCell::new(sub));

        // Frame callback: spring-lerp animated size, sync features, redraw
        let sub_for_frame = sub.clone();
        let mut layer_for_frame = layer.clone();
        let viz_for_frame = self.visualizer.clone();
        let island_for_frame = self.island.clone();
        let anim_w = self.anim_w.clone();
        let anim_h = self.anim_h.clone();
        let anim_y = self.anim_y.clone();
        let target_w = self.target_w.clone();
        let target_h = self.target_h.clone();
        let target_y = self.target_y.clone();
        let hovered_for_frame = self.hovered.clone();
        let open_for_frame = self.open.clone();
        sub.borrow().base_surface().on_frame(move || {
            // Tick visualizer and sync features into island
            viz_for_frame.borrow_mut().tick();
            {
                let viz = viz_for_frame.borrow();
                let mut island = island_for_frame.borrow_mut();
                sync_island_features(&viz, &mut island);

                // Recompute mode every frame based on hover/open/content
                let mode = if *open_for_frame.borrow() {
                    ViewMode::Open
                } else if *hovered_for_frame.borrow() {
                    ViewMode::Hover
                } else {
                    ViewMode::Reduced
                };
                island.mode = mode;

                // Update target size and Y position from current island mode+content
                let (tw, th) = island.total_size();
                *target_w.borrow_mut() = tw;
                *target_h.borrow_mut() = th;
                let ty = match mode {
                    ViewMode::Reduced => 2.0,
                    ViewMode::Hover   => 5.0,
                    ViewMode::Open    => 4.0,
                };
                *target_y.borrow_mut() = ty;
            }

            // Spring lerp toward targets
            {
                let tw = *target_w.borrow();
                let th = *target_h.borrow();
                let ty = *target_y.borrow();
                let mut aw = anim_w.borrow_mut();
                let mut ah = anim_h.borrow_mut();
                let mut ay = anim_y.borrow_mut();
                *aw += (tw - *aw) * SPRING;
                *ah += (th - *ah) * SPRING;
                *ay += (ty - *ay) * SPRING;
            }
            let aw = *anim_w.borrow();
            let ah = *anim_h.borrow();
            let ay = *anim_y.borrow();

            // Resize layer surface and subsurface to match pill height
            let layer_h = (ay + ah).ceil() as u32 + 4;
            layer_for_frame.base_surface_mut().resize(LAYER_W as i32, layer_h as i32);
            layer_for_frame.draw(|canvas| { canvas.clear(skia_safe::Color::TRANSPARENT); });
            layer_for_frame.set_size(LAYER_W, layer_h);
            sub_for_frame.borrow_mut().resize(LAYER_W as i32, layer_h as i32);

            let island = island_for_frame.borrow();
            sub_for_frame.borrow().draw(|canvas| {
                island.draw(canvas, LAYER_W as f32, layer_h as f32, aw, ah, ay);
            });
        });

        // Initial draw
        {
            let init_h = (2.0 + ih).ceil() as i32 + 4;
            sub.borrow_mut().resize(LAYER_W as i32, init_h);
            let island = self.island.borrow();
            sub.borrow().draw(|canvas| {
                island.draw(canvas, LAYER_W as f32, init_h as f32, iw, ih, 2.0);
            });
        }

        self.island_surface = Some(sub);
        layer.base_surface().wl_surface().commit();
    }

    fn on_configure(&mut self, _ctx: &AppContext, _configure: WindowConfigure, _serial: u32) {}
    fn on_keyboard_event(&mut self, _ctx: &AppContext, key: u32, state: wl_keyboard::KeyState, _serial: u32) {
        if state != wl_keyboard::KeyState::Pressed { return; }
        if key == keycodes::Q { std::process::exit(0); }

        // T: toggle Open / Reduced
        if key == 20 {
            let v = *self.open.borrow();
            *self.open.borrow_mut() = !v;
            self.update();
        }

        // N: test notification (5s)
        if key == 49 {
            self.visualizer.borrow_mut().notifications.push(NotificationInfo {
                title: "Test Notification".to_string(),
                body: "Something happened just now in your system".to_string(),
                expires_at: Instant::now() + Duration::from_secs(5),
            });
            self.update();
        }
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[smithay_client_toolkit::seat::pointer::PointerEvent]) {
        let mut needs_update = false;
        for event in events {
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    // Only hover when pointer is actually within the pill bounds
                    let ay = *self.anim_y.borrow() as f64;
                    let ah = *self.anim_h.borrow() as f64;
                    let aw = *self.anim_w.borrow() as f64;
                    let ox = (LAYER_W as f64 - aw) / 2.0;
                    let (px, py) = event.position;
                    let over_pill = px >= ox && px <= ox + aw && py >= ay && py <= ay + ah;
                    let was_hovered = *self.hovered.borrow();
                    if over_pill != was_hovered {
                        *self.hovered.borrow_mut() = over_pill;
                        needs_update = true;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    *self.hovered.borrow_mut() = false;
                    needs_update = true;
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    // Left click: toggle open
                    let v = *self.open.borrow();
                    *self.open.borrow_mut() = !v;
                    needs_update = true;
                }
                _ => {}
            }
        }
        if needs_update {
            self.update();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Dynamic Island — Feature Plugin System");
    println!("- Hover to expand, T to toggle open, N for test notification, Q to quit");

    let audio_level = start_pipewire_level_monitor();
    let playback_info = start_playerctl_monitor();

    let app = MusicNotchApp {
        layer_surface: None,
        island_surface: None,
        island: Rc::new(RefCell::new(Island::new())),
        visualizer: Rc::new(RefCell::new(VisualizerState::new(audio_level, playback_info))),
        anim_w: Rc::new(RefCell::new(28.0)),
        anim_h: Rc::new(RefCell::new(28.0)),
        anim_y: Rc::new(RefCell::new(2.0)),
        target_w: Rc::new(RefCell::new(28.0)),
        target_h: Rc::new(RefCell::new(28.0)),
        target_y: Rc::new(RefCell::new(2.0)),
        hovered: Rc::new(RefCell::new(false)),
        open: Rc::new(RefCell::new(false)),
    };

    AppRunner::new(app).run()?;
    Ok(())
}
