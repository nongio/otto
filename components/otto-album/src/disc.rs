//! The record itself, drawn peeking out from behind the sleeve: black vinyl
//! with groove bands, a diagonal sheen, and a printed centre label.

use crate::track::Track;
use skia_safe::{Color, Color4f, Paint, Point};

const VINYL: Color = Color::new(0xFF_0E_0D_10);

/// Draw the record centred at `c` with radius `r`, turned by `angle` radians
/// and lit from `light` (canvas pixel coordinates).
pub fn draw(
    canvas: &skia_safe::Canvas,
    c: Point,
    r: f32,
    track: &Track,
    label_color: Color,
    angle: f32,
    light: (f32, f32),
) {
    match crate::vinyl::surface((c.x, c.y), r, r * 0.32, angle, light) {
        Some(shader) => {
            let mut paint = Paint::default();
            paint.set_anti_alias(true);
            paint.set_shader(shader);
            canvas.draw_circle(c, r, &paint);
        }
        None => {
            let mut paint = Paint::new(Color4f::from(VINYL), None);
            paint.set_anti_alias(true);
            canvas.draw_circle(c, r, &paint);
        }
    }

    // The label turns with the record.
    canvas.save();
    canvas.rotate(angle.to_degrees(), Some(c));
    label(canvas, c, r, track, label_color);
    canvas.restore();

    spindle(canvas, c, r);
}

fn label(canvas: &skia_safe::Canvas, c: Point, r: f32, track: &Track, color: Color) {
    let label_r = r * 0.32;

    // Best: a scan of the actual label. Next best: the cover art, printed on
    // the label the way picture-labels are. Last: a plain paper label.
    let printed = track.label.as_ref().or(track.cover.as_ref());

    if let Some(image) = printed {
        canvas.save();
        let mut clip = skia_safe::PathBuilder::new();
        clip.add_circle(c, label_r, None);
        canvas.clip_path(&clip.detach(), None, true);

        let (iw, ih) = (image.width() as f32, image.height() as f32);
        let side = iw.min(ih);
        let src = skia_safe::Rect::from_xywh((iw - side) / 2.0, (ih - side) / 2.0, side, side);
        let dst =
            skia_safe::Rect::from_xywh(c.x - label_r, c.y - label_r, label_r * 2.0, label_r * 2.0);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        canvas.draw_image_rect(
            image,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
            dst,
            &paint,
        );

        // Cover art is not printed at label brightness; knock it back so the
        // record still reads as vinyl with a label on it.
        if track.label.is_none() {
            let mut wash = Paint::new(Color4f::new(0.0, 0.0, 0.0, 0.18), None);
            wash.set_anti_alias(true);
            canvas.draw_circle(c, label_r, &wash);
        }
        canvas.restore();

        // Seat it into the vinyl with a soft shadow at the paper's edge.
        let mut seam = Paint::default();
        seam.set_anti_alias(true);
        seam.set_style(skia_safe::paint::Style::Stroke);
        seam.set_stroke_width(r * 0.012);
        seam.set_color(Color::from_argb(90, 0, 0, 0));
        canvas.draw_circle(c, label_r, &seam);
        return;
    }

    let mut paint = Paint::new(Color4f::from(color), None);
    paint.set_anti_alias(true);
    canvas.draw_circle(c, label_r, &paint);

    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(skia_safe::paint::Style::Stroke);
    ring.set_stroke_width(r * 0.008);
    ring.set_color(Color::from_argb(120, 255, 255, 255));
    canvas.draw_circle(c, label_r * 0.86, &ring);
}

/// The spindle hole stays put while the record turns around it.
fn spindle(canvas: &skia_safe::Canvas, c: Point, r: f32) {
    let mut hole = Paint::new(Color4f::new(0.90, 0.90, 0.91, 1.0), None);
    hole.set_anti_alias(true);
    canvas.draw_circle(c, r * 0.045, &hole);
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_style(skia_safe::paint::Style::Stroke);
    shadow.set_stroke_width(r * 0.012);
    shadow.set_color(Color::from_argb(130, 18, 12, 8));
    canvas.draw_circle(c, r * 0.045, &shadow);
}
