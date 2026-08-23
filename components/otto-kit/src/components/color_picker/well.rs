//! Pure drawing half: the closed colour well only.
//!
//! No `AppContext`, no `AppRunner`, no wayland-client — same constraint as
//! [`dropdown::field`](crate::components::dropdown::field), and for the same
//! reason: the compositor draws `accent_color`-style settings server-side.
//! The picker popup that opens from this is the client half, in
//! [`super::popup`].
//!
//! Visual design lifted from the approved prototype in
//! `otto-settings/src/widgets.rs`'s `color_well` function: swatch, then its
//! hex value. This adds interaction states and a hit-test helper that reads
//! the same `rect`.

use skia_safe::{Canvas, Color, Paint, PaintStyle, RRect, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::styles;

/// Height the visual design was tuned at.
pub const HEIGHT: f32 = 24.0;
/// Swatch side length.
pub const SWATCH_SIZE: f32 = 22.0;
/// Gap between the swatch and the hex label.
const GAP: f32 = 10.0;

/// Interaction state, set by the caller from pointer tracking and from
/// [`super::popup::ColorPickerPopup::is_open`]. `Open` mirrors
/// [`DropdownInteraction::Open`](crate::components::dropdown::DropdownInteraction::Open) —
/// the well reads as active for as long as the popup is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellInteraction {
    Normal,
    Hovered,
    Pressed,
    Open,
    Disabled,
}

/// `#RRGGBB` for `color`, alpha dropped — the well never shows transparency.
pub fn hex_string(color: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

/// Total width the well needs to draw `color` at [`HEIGHT`]: swatch, gap,
/// and the hex label sized to its text. Callers build the well's `rect` from
/// this — [`draw`] and [`hit_test`] both just take that rect, so a caller
/// that mis-sizes it gets a mismatch between the two immediately rather than
/// a control that silently clips.
pub fn measure(color: Color) -> f32 {
    let hex = hex_string(color);
    SWATCH_SIZE + GAP + styles::BODY.font().measure_str(&hex, None).0
}

/// Is `(x, y)` within the well? Reads the same `rect` [`draw`] paints into.
pub fn hit_test(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// Draw the closed well into `rect`: swatch on the left, hex value on the
/// right, vertically centred. `rect` should be at least
/// [`measure`]`(color)` wide and [`HEIGHT`] tall — a narrower rect clips the
/// hex label rather than overflowing it, the same clip-not-overflow
/// convention [`dropdown::field::draw`](crate::components::dropdown::field::draw) uses.
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    color: Color,
    interaction: WellInteraction,
    theme: &Theme,
) {
    let disabled = interaction == WellInteraction::Disabled;
    let cy = rect.center_y();

    let swatch = Rect::from_xywh(rect.left, cy - SWATCH_SIZE / 2.0, SWATCH_SIZE, SWATCH_SIZE);
    let rrect = RRect::new_rect_xy(swatch, 5.0, 5.0);

    let swatch_color = if disabled {
        scale_alpha(color, 0.5)
    } else {
        color
    };
    canvas.draw_rrect(rrect, &fill(swatch_color));

    // Open reads the same way a dropdown field's border does: the accent
    // colour, for as long as the popup is up, not just on the initial click.
    let (border_color, border_width) = if interaction == WellInteraction::Open {
        (theme.accent, 1.5)
    } else if interaction == WellInteraction::Hovered {
        (theme.fill_primary, 1.0)
    } else {
        (theme.fill_primary, 0.7)
    };
    canvas.draw_rrect(rrect, &stroke(border_color, border_width));

    if interaction == WellInteraction::Pressed {
        // A faint dark overlay reads as pressed without disturbing the hue
        // shown in the swatch — darkening the swatch itself would be
        // indistinguishable from picking a darker colour.
        canvas.draw_rrect(rrect, &fill(Color::from_argb(0x28, 0, 0, 0)));
    }

    // The hex *is* the value this control shows — it reads at full strength,
    // like a field's text, not as the secondary annotation it used to be.
    let text_color = if disabled {
        scale_alpha(theme.text_primary, 0.5)
    } else {
        theme.text_primary
    };
    Label::new(hex_string(color))
        .with_style(styles::BODY)
        .with_color(text_color)
        .centered_on(swatch.right + GAP, cy)
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

    fn rect(color: Color) -> Rect {
        Rect::from_xywh(10.0, 10.0, measure(color), HEIGHT)
    }

    #[test]
    fn hex_string_drops_alpha() {
        assert_eq!(
            hex_string(Color::from_argb(0x11, 0xAA, 0xBB, 0xCC)),
            "#AABBCC"
        );
    }

    #[test]
    fn hit_test_matches_the_measured_rect() {
        let color = Color::from_rgb(0x0A, 0x84, 0xFF);
        let r = rect(color);
        assert!(hit_test(r, r.left, r.top));
        assert!(hit_test(r, r.right, r.bottom));
        assert!(!hit_test(r, r.left - 1.0, r.top));
        assert!(!hit_test(r, r.left, r.bottom + 1.0));
    }

    #[test]
    fn measure_grows_with_the_hex_label() {
        // "#FFFFFF" and "#000000" are the same length, but this guards the
        // general shape: measure is never narrower than the swatch alone.
        assert!(measure(Color::from_rgb(255, 255, 255)) > SWATCH_SIZE);
    }

    #[test]
    fn draw_does_not_panic_across_every_interaction_state_and_theme() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 60)).expect("surface");
        let canvas = surface.canvas();
        let color = Color::from_rgb(0x0A, 0x84, 0xFF);
        let r = rect(color);
        for theme in [Theme::light(), Theme::dark()] {
            for state in [
                WellInteraction::Normal,
                WellInteraction::Hovered,
                WellInteraction::Pressed,
                WellInteraction::Open,
                WellInteraction::Disabled,
            ] {
                draw(canvas, r, color, state, &theme);
            }
        }
    }
}
