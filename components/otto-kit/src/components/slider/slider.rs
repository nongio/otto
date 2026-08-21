use skia_safe::{BlurStyle, Canvas, Color, MaskFilter, Paint, PaintStyle, Point, RRect, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::styles;

/// Knob radius the visual design was tuned at.
pub const KNOB_RADIUS: f32 = 8.0;
/// Track thickness.
pub const TRACK_THICKNESS: f32 = 4.0;
/// Gap between the track's right edge and the readout, when one is given.
const READOUT_GAP: f32 = 12.0;

/// Interaction state, set by the caller from pointer tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderInteraction {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

/// Fraction along the track, 0.0..=1.0, for `value` within `[min, max]`.
/// An inverted or zero-width range clamps to 0.0 rather than dividing by
/// zero or going negative.
pub fn fraction(value: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

/// Centre of the knob for `value`, in `rect`'s coordinate space. Shared by
/// [`draw`] and the hit-test helpers so the painted knob and the one hit-test
/// reasons about are always the same knob.
pub fn knob_center(rect: Rect, value: f32, min: f32, max: f32) -> Point {
    Point::new(
        rect.left + rect.width() * fraction(value, min, max),
        rect.center_y(),
    )
}

/// Is `(x, y)` on the knob?
pub fn hit_test_knob(rect: Rect, value: f32, min: f32, max: f32, x: f32, y: f32) -> bool {
    let c = knob_center(rect, value, min, max);
    let dx = x - c.x;
    let dy = y - c.y;
    dx * dx + dy * dy <= KNOB_RADIUS * KNOB_RADIUS
}

/// Is `(x, y)` anywhere on the track — the click-to-jump target? Padded
/// vertically to the knob's radius so the strip is as easy to hit as the
/// knob itself.
pub fn hit_test_track(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && (rect.center_y() - y).abs() <= KNOB_RADIUS
}

/// Map an x coordinate (in `rect`'s space) to the value it represents,
/// clamped to `[min, max]` and, when `step` is given, snapped to that
/// increment. Shared by click-to-jump on the track and by dragging, so both
/// land on the same value for the same pointer position.
pub fn value_at(rect: Rect, min: f32, max: f32, step: Option<f32>, x: f32) -> f32 {
    if rect.width() <= 0.0 || max <= min {
        return min;
    }
    let t = ((x - rect.left) / rect.width()).clamp(0.0, 1.0);
    let raw = min + t * (max - min);
    let snapped = match step {
        Some(step) if step > 0.0 => (raw / step).round() * step,
        _ => raw,
    };
    snapped.clamp(min, max)
}

/// Draw the slider into `rect`: track, filled portion, knob, and — when
/// `readout` is `Some` — a right-hand label.
///
/// `readout` is the caller's already-formatted string ("24 px", "200%",
/// "0.50", ...): the same value reads differently depending on the setting,
/// so formatting it is not this component's job.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    value: f32,
    min: f32,
    max: f32,
    readout: Option<&str>,
    interaction: SliderInteraction,
    theme: &Theme,
) {
    let disabled = interaction == SliderInteraction::Disabled;
    let t = fraction(value, min, max);
    let cy = rect.center_y();

    let track_alpha = if disabled { 0.5 } else { 1.0 };
    let track = Rect::from_xywh(
        rect.left,
        cy - TRACK_THICKNESS / 2.0,
        rect.width(),
        TRACK_THICKNESS,
    );
    canvas.draw_rrect(
        RRect::new_rect_xy(track, TRACK_THICKNESS / 2.0, TRACK_THICKNESS / 2.0),
        &fill(scale_alpha(theme.fill_secondary, track_alpha)),
    );

    let filled = Rect::from_xywh(
        rect.left,
        cy - TRACK_THICKNESS / 2.0,
        rect.width() * t,
        TRACK_THICKNESS,
    );
    let mut fill_color = theme.accent;
    if disabled {
        fill_color = scale_alpha(fill_color, 0.5);
    }
    canvas.draw_rrect(
        RRect::new_rect_xy(filled, TRACK_THICKNESS / 2.0, TRACK_THICKNESS / 2.0),
        &fill(fill_color),
    );

    let knob = knob_center(rect, value, min, max);
    // Pressed shrinks the knob a hair, same restrained language as the
    // toggle's pressed state; hovered gets a ring instead of a colour shift.
    let knob_r = if interaction == SliderInteraction::Pressed {
        KNOB_RADIUS * 0.92
    } else {
        KNOB_RADIUS
    };

    if !disabled {
        let mut shadow = fill(Color::from_argb(0x38, 0, 0, 0));
        shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 2.0, false));
        canvas.draw_circle(Point::new(knob.x, knob.y + 1.0), knob_r, &shadow);
    }
    let knob_fill = if disabled {
        scale_alpha(Color::WHITE, 0.6)
    } else {
        Color::WHITE
    };
    canvas.draw_circle(knob, knob_r, &fill(knob_fill));
    canvas.draw_circle(knob, knob_r, &stroke(theme.fill_tertiary, 0.5));
    if interaction == SliderInteraction::Hovered {
        canvas.draw_circle(knob, knob_r + 1.5, &stroke(theme.fill_primary, 1.0));
    }

    if let Some(readout) = readout {
        let color = if disabled {
            theme.text_tertiary
        } else {
            theme.text_secondary
        };
        text_centered_y(
            canvas,
            readout,
            rect.right + READOUT_GAP,
            cy,
            styles::SUBHEADLINE,
            color,
        );
    }
}

/// Text drawn so its optical centre sits on `cy` — same trick the
/// `otto-settings` prototype used, kept local since it is a drawing detail
/// of this one readout, not a general `Label` feature.
fn text_centered_y(
    canvas: &Canvas,
    text: &str,
    x: f32,
    cy: f32,
    style: crate::typography::TextStyle,
    color: Color,
) {
    Label::new(text)
        .with_style(style)
        .with_color(color)
        .centered_on(x, cy)
        .render(canvas);
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

fn scale_alpha(color: Color, factor: f32) -> Color {
    let a = (color.a() as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color::from_argb(a, color.r(), color.g(), color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_xywh(0.0, 0.0, 160.0, 24.0)
    }

    #[test]
    fn fraction_clamps_and_handles_degenerate_ranges() {
        assert_eq!(fraction(5.0, 0.0, 10.0), 0.5);
        assert_eq!(fraction(-5.0, 0.0, 10.0), 0.0);
        assert_eq!(fraction(50.0, 0.0, 10.0), 1.0);
        assert_eq!(fraction(5.0, 10.0, 10.0), 0.0);
    }

    #[test]
    fn value_at_maps_x_across_the_track() {
        let r = rect();
        assert_eq!(value_at(r, 0.0, 100.0, None, r.left), 0.0);
        assert_eq!(value_at(r, 0.0, 100.0, None, r.right), 100.0);
        assert_eq!(
            value_at(r, 0.0, 100.0, None, r.left + r.width() / 2.0),
            50.0
        );
    }

    #[test]
    fn value_at_snaps_to_step() {
        let r = rect();
        // Roughly 37% across a 0..100 range should snap to the nearest 10.
        let x = r.left + r.width() * 0.37;
        assert_eq!(value_at(r, 0.0, 100.0, Some(10.0), x), 40.0);
    }

    #[test]
    fn value_at_clamps_out_of_range_x() {
        let r = rect();
        assert_eq!(value_at(r, 0.0, 100.0, None, r.left - 50.0), 0.0);
        assert_eq!(value_at(r, 0.0, 100.0, None, r.right + 50.0), 100.0);
    }

    #[test]
    fn hit_test_knob_follows_the_value() {
        let r = rect();
        let c = knob_center(r, 100.0, 0.0, 100.0);
        assert!(hit_test_knob(r, 100.0, 0.0, 100.0, c.x, c.y));
        assert!(!hit_test_knob(r, 0.0, 0.0, 100.0, c.x, c.y));
    }

    #[test]
    fn hit_test_track_covers_the_whole_strip() {
        let r = rect();
        assert!(hit_test_track(r, r.left, r.center_y()));
        assert!(hit_test_track(r, r.right, r.center_y()));
        assert!(!hit_test_track(r, r.left - 1.0, r.center_y()));
    }
}
