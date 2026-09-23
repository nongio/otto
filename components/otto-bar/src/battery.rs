//! The battery indicator: a glyph that fills with the charge.
//!
//! Drawn by the bar rather than published as a tray icon — see `power.rs` for
//! why. Everything here reads from `config::battery_config()`, so the shape,
//! the colours, and whether the percentage is written at all are the
//! configuration file's business, not this module's.

use otto_kit::prelude::*;
use otto_kit::typography;
use skia_safe::{Canvas, Color, Paint, PathBuilder, RRect, Rect, TextBlob};

use crate::config::{battery_config, BatteryVisibility, PercentagePosition};
use crate::power::{self, Battery};

/// Gap between the percentage text and the glyph, when the text is beside it.
const TEXT_GAP: f32 = 5.0;

/// Width of the cap — the nub on the positive terminal.
const CAP_WIDTH: f32 = 2.0;

/// Gap between the body and the cap.
const CAP_GAP: f32 = 1.5;

/// Thickness of the body outline.
const STROKE: f32 = 1.2;

/// Inset from the outline to the fill.
const FILL_INSET: f32 = 1.6;

/// Whether the indicator is drawn at all.
pub fn visible() -> bool {
    match battery_config().visibility {
        BatteryVisibility::Never => false,
        BatteryVisibility::Always => true,
        // A desktop has no battery to report, and an outline that never fills
        // is worse than nothing.
        BatteryVisibility::Auto => power::battery().present,
    }
}

/// The percentage as it is written: no decimals, because a battery that says
/// 87.4% is reporting precision it does not have.
fn percentage_text(battery: &Battery) -> String {
    format!("{}%", battery.percentage.round() as i64)
}

fn percentage_font() -> skia_safe::Font {
    // Smaller than the clock: it sits inside a 13pt glyph, and at the clock's
    // size three digits do not fit between the terminals.
    typography::get_font_with_fallback(
        "Inter",
        skia_safe::FontStyle::new(
            skia_safe::font_style::Weight::SEMI_BOLD,
            skia_safe::font_style::Width::NORMAL,
            skia_safe::font_style::Slant::Upright,
        ),
        9.0,
    )
}

/// Total width the indicator occupies, including any text beside it.
pub fn width() -> f32 {
    if !visible() {
        return 0.0;
    }
    let cfg = battery_config();
    let glyph = cfg.width + CAP_GAP + CAP_WIDTH;

    match cfg.percentage {
        PercentagePosition::Beside => {
            let text = percentage_text(&power::battery());
            let font = percentage_font();
            glyph + TEXT_GAP + font.measure_str(&text, None).0
        }
        // Inside costs nothing: it is drawn over the fill.
        _ => glyph,
    }
}

/// The colour of the fill at this charge.
fn fill_color(battery: &Battery, theme: &Theme) -> Color {
    let cfg = battery_config();
    if !cfg.colored {
        return theme.text_primary;
    }
    if battery.state.is_charging() {
        return cfg.color_charging;
    }
    if battery.percentage <= cfg.critical_level {
        cfg.color_critical
    } else if battery.percentage <= cfg.low_level {
        cfg.color_low
    } else {
        cfg.color_normal
    }
}

/// Draw the indicator with its left edge at `x`, centred in `height`.
/// Returns the width drawn, which is `width()`.
pub fn draw(canvas: &Canvas, x: f32, height: f32, theme: &Theme) -> f32 {
    if !visible() {
        return 0.0;
    }

    let cfg = battery_config();
    let battery = power::battery();
    let text = percentage_text(&battery);
    let font = percentage_font();

    let mut glyph_x = x;
    if cfg.percentage == PercentagePosition::Beside {
        let text_width = font.measure_str(&text, None).0;
        let (_, metrics) = font.metrics();
        let baseline = (height + metrics.cap_height) / 2.0;
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.text_primary);
        if let Some(blob) = TextBlob::new(&text, &font) {
            canvas.draw_text_blob(&blob, (glyph_x, baseline), &paint);
        }
        glyph_x += text_width + TEXT_GAP;
    }

    let top = ((height - cfg.height) / 2.0).round();
    let body = Rect::from_xywh(glyph_x, top, cfg.width, cfg.height);
    let radius = cfg.height * 0.3;

    // Outline. Dimmer than the text around it: the shape is a container, and
    // a full-strength border competes with the fill it is meant to frame.
    let mut outline = Paint::default();
    outline.set_anti_alias(true);
    outline.set_style(skia_safe::paint::Style::Stroke);
    outline.set_stroke_width(STROKE);
    outline.set_color(with_alpha(theme.text_primary, 0.55));
    canvas.draw_round_rect(body, radius, radius, &outline);

    // The cap.
    let cap_height = cfg.height * 0.42;
    let cap = Rect::from_xywh(
        body.right + CAP_GAP,
        top + (cfg.height - cap_height) / 2.0,
        CAP_WIDTH,
        cap_height,
    );
    let mut cap_paint = Paint::default();
    cap_paint.set_anti_alias(true);
    cap_paint.set_color(with_alpha(theme.text_primary, 0.55));
    canvas.draw_round_rect(cap, CAP_WIDTH / 2.0, CAP_WIDTH / 2.0, &cap_paint);

    // The fill. Clamped to the track so a battery reporting 0 still shows a
    // sliver of colour rather than nothing, and one reporting 101 — which
    // some firmware does — does not overflow the outline.
    let track = Rect::from_xywh(
        body.left + FILL_INSET,
        top + FILL_INSET,
        cfg.width - FILL_INSET * 2.0,
        cfg.height - FILL_INSET * 2.0,
    );
    let fraction = (battery.percentage / 100.0).clamp(0.0, 1.0) as f32;
    let filled = (track.width() * fraction).max(if battery.present { 1.5 } else { 0.0 });
    let fill_rect = Rect::from_xywh(track.left, track.top, filled, track.height());

    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(fill_color(&battery, theme));
    let fill_radius = (track.height() * 0.3).min(filled / 2.0);
    canvas.draw_round_rect(fill_rect, fill_radius, fill_radius, &fill);

    // A bolt while charge is going in, a plug while the cable is in but the
    // battery is full or held at a limit — so a plugged-in machine never
    // reads the same as one running on its battery.
    let mark = if battery.state.is_charging() {
        Some(Mark::Bolt)
    } else if battery.state.is_plugged() {
        Some(Mark::Plug)
    } else {
        None
    };

    // With the percentage inside, the mark sits beside the digits rather than
    // over them: drawn over, both become unreadable. The sign goes to make
    // room — "100" and a plug fit in the body where "100%" and a plug do not.
    let mut mark_cx = body.center_x();
    if cfg.percentage == PercentagePosition::Inside {
        let text = match mark {
            Some(_) => format!("{}", battery.percentage.round() as i64),
            None => text,
        };
        let text_width = font.measure_str(&text, None).0;
        let mark_width = mark.map_or(0.0, |mark| MARK_GAP + mark.width(body));
        let left = body.left + (body.width() - text_width - mark_width) / 2.0;
        draw_percentage_inside(canvas, &text, &font, left, body, theme);
        if let Some(mark) = mark {
            mark_cx = left + text_width + MARK_GAP + mark.width(body) / 2.0;
        }
    }
    match mark {
        Some(Mark::Bolt) => draw_bolt(canvas, mark_cx, body, theme),
        Some(Mark::Plug) => draw_plug(canvas, mark_cx, body, theme),
        None => {}
    }

    width()
}

/// What the glyph says about the charger, drawn over the fill.
#[derive(Clone, Copy)]
enum Mark {
    Bolt,
    Plug,
}

/// Gap between the digits and the mark beside them.
const MARK_GAP: f32 = 1.0;

impl Mark {
    fn height(self, body: Rect) -> f32 {
        match self {
            Mark::Bolt => body.height() * 0.72,
            Mark::Plug => body.height() * 0.66,
        }
    }

    fn width(self, body: Rect) -> f32 {
        match self {
            Mark::Bolt => self.height(body) * 0.5,
            Mark::Plug => self.height(body) * 0.62,
        }
    }
}

/// The percentage over the fill, its left edge at `x`.
fn draw_percentage_inside(
    canvas: &Canvas,
    text: &str,
    font: &skia_safe::Font,
    x: f32,
    body: Rect,
    theme: &Theme,
) {
    let (_, metrics) = font.metrics();
    let baseline = body.top + (body.height() + metrics.cap_height) / 2.0;

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.text_primary);

    // A thin halo in the bar's own background colour, so the digits stay
    // legible where they cross the edge of the fill — half the text over
    // colour and half over the blur is the one case plain text fails.
    let mut halo = Paint::default();
    halo.set_anti_alias(true);
    halo.set_style(skia_safe::paint::Style::Stroke);
    halo.set_stroke_width(2.0);
    halo.set_color(with_alpha(contrast_against(theme.text_primary), 0.45));

    if let Some(blob) = TextBlob::new(text, font) {
        canvas.draw_text_blob(&blob, (x, baseline), &halo);
        canvas.draw_text_blob(&blob, (x, baseline), &paint);
    }
}

/// The charging bolt, centred on `cx`.
fn draw_bolt(canvas: &Canvas, cx: f32, body: Rect, theme: &Theme) {
    let h = Mark::Bolt.height(body);
    let w = Mark::Bolt.width(body);
    let cy = body.center_y();
    let (left, top) = (cx - w / 2.0, cy - h / 2.0);

    let mut builder = PathBuilder::new();
    builder.move_to((left + w * 0.62, top));
    builder.line_to((left + w * 0.05, top + h * 0.58));
    builder.line_to((left + w * 0.45, top + h * 0.58));
    builder.line_to((left + w * 0.38, top + h));
    builder.line_to((left + w * 0.95, top + h * 0.42));
    builder.line_to((left + w * 0.55, top + h * 0.42));
    builder.close();
    let path = builder.detach();

    // Outlined in the background colour for the same reason the percentage
    // is: the bolt crosses the boundary between fill and empty track.
    let mut halo = Paint::default();
    halo.set_anti_alias(true);
    halo.set_style(skia_safe::paint::Style::Stroke);
    halo.set_stroke_width(1.6);
    halo.set_color(with_alpha(contrast_against(theme.text_primary), 0.55));
    canvas.draw_path(&path, &halo);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.text_primary);
    canvas.draw_path(&path, &paint);
}

/// A mains plug, prongs up, centred on `cx`.
fn draw_plug(canvas: &Canvas, cx: f32, body: Rect, theme: &Theme) {
    let h = Mark::Plug.height(body);
    let w = Mark::Plug.width(body);
    let top = body.center_y() - h / 2.0;

    let prong_h = h * 0.28;
    let prong_w = (w * 0.16).max(1.0);
    let head_top = top + prong_h;
    let head_h = h * 0.44;
    let cord_w = (w * 0.22).max(1.0);

    let mut builder = PathBuilder::new();
    for x in [cx - w * 0.24, cx + w * 0.24] {
        builder.add_rect(
            Rect::from_xywh(x - prong_w / 2.0, top, prong_w, prong_h + 0.5),
            None,
            None,
        );
    }
    builder.add_rrect(
        RRect::new_rect_xy(
            Rect::from_xywh(cx - w / 2.0, head_top, w, head_h),
            w * 0.2,
            w * 0.2,
        ),
        None,
        None,
    );
    builder.add_rect(
        Rect::from_xywh(
            cx - cord_w / 2.0,
            head_top + head_h - 0.5,
            cord_w,
            top + h - (head_top + head_h) + 0.5,
        ),
        None,
        None,
    );
    let path = builder.detach();

    // The same halo as the bolt, for the same reason.
    let mut halo = Paint::default();
    halo.set_anti_alias(true);
    halo.set_style(skia_safe::paint::Style::Stroke);
    halo.set_stroke_width(1.6);
    halo.set_color(with_alpha(contrast_against(theme.text_primary), 0.55));
    canvas.draw_path(&path, &halo);

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.text_primary);
    canvas.draw_path(&path, &paint);
}

/// Black against a light foreground, white against a dark one — the colour
/// the bar's background is, near enough, without reaching for the material.
fn contrast_against(color: Color) -> Color {
    let luminance =
        0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32;
    if luminance > 128.0 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::from_argb(
        (alpha.clamp(0.0, 1.0) * 255.0) as u8,
        color.r(),
        color.g(),
        color.b(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::ChargeState;

    fn battery(percentage: f64, state: ChargeState) -> Battery {
        Battery {
            present: true,
            percentage,
            state,
            seconds_left: 0,
        }
    }

    #[test]
    fn the_percentage_is_written_without_decimals() {
        assert_eq!(
            percentage_text(&battery(87.4, ChargeState::Discharging)),
            "87%"
        );
        assert_eq!(
            percentage_text(&battery(99.6, ChargeState::Charging)),
            "100%"
        );
        assert_eq!(
            percentage_text(&battery(0.0, ChargeState::Discharging)),
            "0%"
        );
    }

    #[test]
    fn a_light_foreground_haloes_against_black() {
        assert_eq!(contrast_against(Color::WHITE), Color::BLACK);
        assert_eq!(contrast_against(Color::BLACK), Color::WHITE);
    }

    #[test]
    fn alpha_is_applied_without_disturbing_the_colour() {
        let faded = with_alpha(Color::from_rgb(0x34, 0xC7, 0x59), 0.5);
        assert_eq!(faded.a(), 127);
        assert_eq!((faded.r(), faded.g(), faded.b()), (0x34, 0xC7, 0x59));
    }
}
