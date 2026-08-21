//! Pure drawing half: the closed field only.
//!
//! No `AppContext`, no `AppRunner`, no wayland-client — this has to draw from
//! a bare Skia canvas, because the compositor paints server-side otto-kit
//! components the same way it paints a titlebar. The open menu itself is the
//! client half, in [`super::menu`].
//!
//! Visual design lifted from the approved prototype in
//! `otto-settings/src/widgets.rs`'s `select` function: rounded box, current
//! value clipped to the field, chevron pair. This just adds interaction
//! states and a hit-test helper that reads the same `rect`.

use skia_safe::{Canvas, ClipOp, Color, Paint, PaintStyle, Path, PathBuilder, Point, RRect, Rect};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::theme::Theme;
use crate::typography::styles;

/// Height the visual design was tuned at. Callers may draw at other heights
/// — the geometry below is all relative to `rect` — but this is the
/// reference, matching the settings prototype's `CONTROL_H`.
pub const HEIGHT: f32 = 24.0;

const CORNER_RADIUS: f32 = 6.0;
/// Reserved width on the right for the chevron pair; the label clips before it.
const CHEVRON_GUTTER: f32 = 26.0;

/// Interaction state, set by the caller from pointer tracking and from
/// [`super::menu::DropdownMenu::is_open`]. `Open` is distinct from `Pressed`
/// — the field reads as active for as long as the menu is up, not just for
/// the initial click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropdownInteraction {
    Normal,
    Hovered,
    Pressed,
    Open,
    Disabled,
}

/// Is `(x, y)` within the field? Reads the same `rect` [`draw`] paints into,
/// so hit-testing cannot drift from what is on screen.
pub fn hit_test(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// Draw the closed field into `rect`: rounded box, `label` clipped before the
/// chevron gutter, chevron pair.
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    label: &str,
    interaction: DropdownInteraction,
    theme: &Theme,
) {
    let disabled = interaction == DropdownInteraction::Disabled;
    let open = interaction == DropdownInteraction::Open;

    let mut bg = theme.fill_tertiary;
    bg = match interaction {
        DropdownInteraction::Hovered => lighten(bg),
        DropdownInteraction::Pressed => darken(bg),
        _ => bg,
    };
    if disabled {
        bg = scale_alpha(bg, 0.5);
    }

    let rrect = RRect::new_rect_xy(rect, CORNER_RADIUS, CORNER_RADIUS);
    canvas.draw_rrect(rrect, &fill(bg));

    // An open menu keeps the field reading as active the whole time it's up,
    // the way a native pop-up button's border picks up the accent colour
    // rather than just flashing on the initial click.
    let (border_color, border_width) = if open {
        (theme.accent, 1.5)
    } else {
        (theme.fill_secondary, 1.0)
    };
    canvas.draw_rrect(rrect, &stroke(border_color, border_width));

    canvas.save();
    canvas.clip_rect(
        Rect::from_ltrb(
            rect.left,
            rect.top,
            rect.right - CHEVRON_GUTTER,
            rect.bottom,
        ),
        ClipOp::Intersect,
        true,
    );
    let text_color = if disabled {
        scale_alpha(theme.text_primary, 0.5)
    } else {
        theme.text_primary
    };
    Label::new(label)
        .with_style(styles::SUBHEADLINE)
        .with_color(text_color)
        .centered_on(rect.left + 9.0, rect.center_y())
        .render(canvas);
    canvas.restore();

    draw_chevrons(canvas, rect, disabled, theme);
}

fn draw_chevrons(canvas: &Canvas, rect: Rect, disabled: bool, theme: &Theme) {
    let cx = rect.right - 13.0;
    let cy = rect.center_y();
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(cx - 3.5, cy - 2.0));
    builder.line_to(Point::new(cx, cy - 5.5));
    builder.line_to(Point::new(cx + 3.5, cy - 2.0));
    builder.move_to(Point::new(cx - 3.5, cy + 2.0));
    builder.line_to(Point::new(cx, cy + 5.5));
    builder.line_to(Point::new(cx + 3.5, cy + 2.0));

    let color = if disabled {
        scale_alpha(theme.text_secondary, 0.5)
    } else {
        theme.text_secondary
    };
    let mut chevron = stroke(color, 1.4);
    chevron.set_stroke_cap(skia_safe::paint::Cap::Round);
    chevron.set_stroke_join(skia_safe::paint::Join::Round);
    let path: Path = builder.detach();
    canvas.draw_path(&path, &chevron);
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

fn lighten(color: Color) -> Color {
    mix_toward(color, 255, 0.12)
}

fn darken(color: Color) -> Color {
    mix_toward(color, 0, 0.12)
}

fn mix_toward(color: Color, target: u8, t: f32) -> Color {
    let mix = |c: u8| (c as f32 + (target as f32 - c as f32) * t).round() as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_xywh(10.0, 10.0, 160.0, HEIGHT)
    }

    #[test]
    fn hit_test_matches_the_drawn_rect() {
        let r = rect();
        assert!(hit_test(r, 15.0, 15.0));
        assert!(!hit_test(r, 5.0, 15.0));
        assert!(!hit_test(r, 15.0, 100.0));
    }

    #[test]
    fn hit_test_is_inclusive_of_edges() {
        let r = rect();
        assert!(hit_test(r, r.left, r.top));
        assert!(hit_test(r, r.right, r.bottom));
    }

    #[test]
    fn draw_does_not_panic_across_every_interaction_state_and_theme() {
        // Guards against a canvas-only regression: this has to run with
        // nothing but a raster surface, no AppContext, no wayland-client.
        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 60)).expect("surface");
        let canvas = surface.canvas();
        let r = rect();
        for theme in [Theme::light(), Theme::dark()] {
            for state in [
                DropdownInteraction::Normal,
                DropdownInteraction::Hovered,
                DropdownInteraction::Pressed,
                DropdownInteraction::Open,
                DropdownInteraction::Disabled,
            ] {
                draw(canvas, r, "Documents", state, &theme);
            }
        }
    }
}
