//! Sidebar glyphs.
//!
//! otto-kit bundles only a handful of icons and none of the ones the panes
//! need, so they are drawn here: one stroke weight, one 16pt box, rounded
//! caps. Placeholders — replace with real icon assets once the set exists.

use otto_kit::prelude::*;
use skia_safe::{paint::Cap, paint::Join, PaintStyle, PathBuilder, Point, RRect};

/// Draw `name` centred on (`cx`, `cy`) at `size` points.
pub fn draw(canvas: &Canvas, name: &str, cx: f32, cy: f32, size: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(size / 11.0);
    paint.set_stroke_cap(Cap::Round);
    paint.set_stroke_join(Join::Round);
    paint.set_color(color);

    canvas.save();
    canvas.translate((cx, cy));
    // Every glyph is drawn in a 16×16 box centred on the origin.
    canvas.scale((size / 16.0, size / 16.0));

    match name {
        "settings" => settings(canvas, &paint),
        "monitor" => monitor(canvas, &paint),
        "dock" => dock(canvas, &paint),
        "keyboard" => keyboard(canvas, &paint),
        "pointer" => pointer(canvas, &paint, color),
        "sound" => sound(canvas, &paint),
        "battery" => battery(canvas, &paint),
        "lock" => lock(canvas, &paint),
        _ => {
            canvas.draw_circle(Point::new(0.0, 0.0), 2.5, &paint);
        }
    }

    canvas.restore();
}

/// Three horizontal sliders.
fn settings(canvas: &Canvas, paint: &Paint) {
    for (i, y) in [-5.0_f32, 0.0, 5.0].iter().enumerate() {
        canvas.draw_line(Point::new(-7.0, *y), Point::new(7.0, *y), paint);
        let knob_x = [-2.0, 3.0, -4.0][i];
        canvas.draw_circle(Point::new(knob_x, *y), 2.0, paint);
    }
}

/// Screen on a stand.
fn monitor(canvas: &Canvas, paint: &Paint) {
    let screen = Rect::from_ltrb(-7.0, -6.5, 7.0, 3.0);
    canvas.draw_rrect(RRect::new_rect_xy(screen, 1.5, 1.5), paint);
    canvas.draw_line(Point::new(0.0, 3.0), Point::new(0.0, 6.0), paint);
    canvas.draw_line(Point::new(-4.0, 6.5), Point::new(4.0, 6.5), paint);
}

/// A strip of tiles along the bottom edge.
fn dock(canvas: &Canvas, paint: &Paint) {
    let tray = Rect::from_ltrb(-7.5, 1.0, 7.5, 6.5);
    canvas.draw_rrect(RRect::new_rect_xy(tray, 2.0, 2.0), paint);
    for x in [-4.0_f32, 0.0, 4.0] {
        canvas.draw_line(Point::new(x, 2.8), Point::new(x, 4.7), paint);
    }
}

/// Key grid with a spacebar.
fn keyboard(canvas: &Canvas, paint: &Paint) {
    let body = Rect::from_ltrb(-7.5, -5.0, 7.5, 5.0);
    canvas.draw_rrect(RRect::new_rect_xy(body, 2.0, 2.0), paint);
    for x in [-4.5_f32, -1.5, 1.5, 4.5] {
        canvas.draw_line(Point::new(x, -2.2), Point::new(x, -2.2), paint);
        canvas.draw_line(Point::new(x, 0.4), Point::new(x, 0.4), paint);
    }
    canvas.draw_line(Point::new(-3.0, 2.8), Point::new(3.0, 2.8), paint);
}

/// Arrow cursor. Filled, so it reads at 16pt.
fn pointer(canvas: &Canvas, paint: &Paint, color: Color) {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(-4.0, -6.5));
    builder.line_to(Point::new(5.5, 1.5));
    builder.line_to(Point::new(0.5, 2.0));
    builder.line_to(Point::new(-1.5, 6.5));
    builder.close();

    let mut fill = paint.clone();
    fill.set_style(PaintStyle::Fill);
    fill.set_color(color);
    canvas.draw_path(&builder.detach(), &fill);
}

/// Speaker with two waves.
fn sound(canvas: &Canvas, paint: &Paint) {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(-6.0, -2.0));
    builder.line_to(Point::new(-3.5, -2.0));
    builder.line_to(Point::new(-0.5, -5.5));
    builder.line_to(Point::new(-0.5, 5.5));
    builder.line_to(Point::new(-3.5, 2.0));
    builder.line_to(Point::new(-6.0, 2.0));
    builder.close();
    canvas.draw_path(&builder.detach(), paint);

    for (r, sweep) in [(3.0_f32, 90.0_f32), (5.5, 100.0)] {
        let arc = Rect::from_ltrb(2.0 - r, -r, 2.0 + r, r);
        canvas.draw_arc(arc, -sweep / 2.0, sweep, false, paint);
    }
}

/// Battery with a bolt.
fn battery(canvas: &Canvas, paint: &Paint) {
    let body = Rect::from_ltrb(-7.0, -4.0, 5.0, 4.0);
    canvas.draw_rrect(RRect::new_rect_xy(body, 2.0, 2.0), paint);
    let cap = Rect::from_ltrb(5.5, -1.8, 7.0, 1.8);
    canvas.draw_rrect(RRect::new_rect_xy(cap, 0.8, 0.8), paint);

    let mut bolt = PathBuilder::new();
    bolt.move_to(Point::new(-0.5, -2.4));
    bolt.line_to(Point::new(-3.0, 0.3));
    bolt.line_to(Point::new(-1.2, 0.3));
    bolt.line_to(Point::new(-1.8, 2.4));
    bolt.line_to(Point::new(0.8, -0.4));
    bolt.line_to(Point::new(-1.0, -0.4));
    bolt.close();
    canvas.draw_path(&bolt.detach(), paint);
}

/// Padlock, shackle closed.
fn lock(canvas: &Canvas, paint: &Paint) {
    let body = Rect::from_ltrb(-5.5, -1.0, 5.5, 6.5);
    canvas.draw_rrect(RRect::new_rect_xy(body, 2.0, 2.0), paint);
    let shackle = Rect::from_ltrb(-3.5, -6.5, 3.5, 0.5);
    canvas.draw_arc(shackle, 180.0, 180.0, false, paint);
}
