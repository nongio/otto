//! Push button: a labelled rounded box that acts when released over it.
//!
//! The look Otto's own dialogs and Settings rows use: a quiet fill with the
//! hairline edge every control carries, or the accent for the one button a
//! dialog's Return presses. It doesn't change under the pointer; it darkens
//! while held, and the press acts on release, so sliding off before letting
//! go takes it back. No state of its own: the caller says whether it is
//! held, as for [`toggle`](super::toggle).

// Rust guideline compliant 2026-02-21

use skia_safe::{Canvas, Color, Paint, PaintStyle, RRect, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

/// A dialog button's height.
pub const HEIGHT: f32 = 28.0;
/// The narrowest a button is, so "OK" doesn't shrink to a dot.
pub const MIN_WIDTH: f32 = 80.0;
/// The corner radius.
const RADIUS: f32 = 7.0;
/// Room either side of the label.
const PAD: f32 = 14.0;
/// The label: the size of the text around it, a step heavier, so it reads
/// as something to press.
const TEXT: TextStyle = styles::BODY_MEDIUM;

/// What a button is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// Any button.
    #[default]
    Normal,
    /// The one Return presses: drawn in the accent.
    Default,
}

/// How a button is drawn now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PushButton {
    pub role: Role,
    /// Held down with the pointer over it.
    pub pressed: bool,
    pub enabled: bool,
}

impl PushButton {
    /// An enabled button of `role`, not held.
    pub fn new(role: Role) -> Self {
        Self {
            role,
            pressed: false,
            enabled: true,
        }
    }

    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// How wide a button labelled `label` is: the label and its padding, and
/// never under [`MIN_WIDTH`].
pub fn width(label: &str) -> f32 {
    (TEXT.font().measure_str(label, None).0 + 2.0 * PAD).max(MIN_WIDTH)
}

/// A colour taken toward black by `amount` (0–1), its alpha kept: the
/// accent held down.
fn darker(color: Color, amount: f32) -> Color {
    let keep = |channel: u8| (f32::from(channel) * (1.0 - amount)).round() as u8;
    Color::from_argb(color.a(), keep(color.r()), keep(color.g()), keep(color.b()))
}

/// Paint `label` as a button in `rect`.
pub fn draw(canvas: &Canvas, rect: Rect, label: &str, button: PushButton, theme: &Theme) {
    let accent = button.role == Role::Default && button.enabled;
    let ground = match (accent, button.pressed) {
        (true, false) => theme.accent,
        (true, true) => darker(theme.accent, 0.18),
        (false, false) => theme.fill_tertiary,
        (false, true) => theme.fill_secondary,
    };
    let rrect = RRect::new_rect_xy(rect, RADIUS, RADIUS);
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(ground);
    canvas.draw_rrect(rrect, &paint);
    if !accent {
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(1.0);
        paint.set_color(theme.fill_secondary);
        canvas.draw_rrect(rrect, &paint);
    }
    let color = match (button.enabled, accent) {
        (false, _) => theme.text_tertiary,
        (true, true) => Color::WHITE,
        (true, false) => theme.text_primary,
    };
    let text = crate::typography::ellipsize(&TEXT.font(), label, rect.width() - 2.0 * PAD);
    Label::new(text)
        .with_style(TEXT)
        .with_color(color)
        .centered_at(rect.center_x(), rect.center_y())
        .render(canvas);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_is_never_narrower_than_the_least_and_grows_with_its_label() {
        assert_eq!(width("OK"), MIN_WIDTH);
        assert!(width("Save the document as something else") > MIN_WIDTH);
    }

    #[test]
    fn a_held_accent_is_darker_and_keeps_its_alpha() {
        let accent = Color::from_argb(200, 10, 132, 255);
        let held = darker(accent, 0.18);
        assert_eq!(held.a(), 200);
        assert!(held.b() < accent.b() && held.g() < accent.g());
    }
}
