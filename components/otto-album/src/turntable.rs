//! The deck the record sits on: plinth, platter and mat, speed markings, the
//! strobe dots around the platter edge, and the play/pause control.

use skia_safe::{BlurStyle, Color, Color4f, MaskFilter, Paint, Point, RRect, Rect};

const PLINTH: Color = Color::new(0xFF_23_22_26);
const PLINTH_TOP: Color = Color::new(0xFF_36_35_3A);
const MAT: Color = Color::new(0xFF_16_15_18);

/// Cast plastic is never perfectly smooth: a fine noise over the plinth, plus
/// a couple of long scuffs, so the slab reads as a used object rather than a
/// flat fill.
fn plinth_texture(canvas: &skia_safe::Canvas, plinth: Rect) {
    canvas.save();
    canvas.clip_rrect(
        RRect::new_rect_xy(plinth, 10.0, 10.0),
        skia_safe::ClipOp::Intersect,
        true,
    );

    // Grain.
    if let Some(noise) = skia_safe::shaders::fractal_noise((0.9, 0.9), 2, 3.0, None) {
        let mut grain = Paint::default();
        grain.set_shader(noise);
        grain.set_blend_mode(skia_safe::BlendMode::SoftLight);
        grain.set_alpha(56);
        canvas.draw_rect(plinth, &grain);
    }

    // A coarser, stretched pass reads as moulding texture rather than dust.
    if let Some(noise) = skia_safe::shaders::turbulence((0.02, 0.4), 3, 7.0, None) {
        let mut cast = Paint::default();
        cast.set_shader(noise);
        cast.set_blend_mode(skia_safe::BlendMode::Overlay);
        cast.set_alpha(26);
        canvas.draw_rect(plinth, &cast);
    }

    // Wear: a few faint scuffs, brighter where the light catches them.
    let mut scuff = Paint::default();
    scuff.set_anti_alias(true);
    scuff.set_style(skia_safe::paint::Style::Stroke);
    let mut seed: u32 = 0x51ED_2701;
    for i in 0..7 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let fx = ((seed >> 8) % 1000) as f32 / 1000.0;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let fy = ((seed >> 8) % 1000) as f32 / 1000.0;
        let x = plinth.left() + fx * plinth.width();
        let y = plinth.top() + fy * plinth.height();
        let len = 14.0 + (i as f32 * 9.0);
        scuff.set_stroke_width(if i % 3 == 0 { 1.1 } else { 0.7 });
        scuff.set_color(Color::from_argb(
            if i % 2 == 0 { 20 } else { 12 },
            255,
            252,
            246,
        ));
        canvas.draw_line((x, y), (x + len, y - len * 0.22), &scuff);
    }

    canvas.restore();
}

/// Draw the deck under the record. `platter_r` is the platter's radius, a
/// little wider than the record itself.
pub fn deck(canvas: &skia_safe::Canvas, plinth: Rect, center: Point, platter_r: f32) {
    // Plinth: a dark slab with a lit top face.
    let mut body = Paint::default();
    body.set_anti_alias(true);
    body.set_shader(skia_safe::gradient_shader::linear(
        (
            (plinth.left(), plinth.top()),
            (plinth.left(), plinth.bottom()),
        ),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[PLINTH_TOP, PLINTH]),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_rrect(RRect::new_rect_xy(plinth, 10.0, 10.0), &body);

    plinth_texture(canvas, plinth);

    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::paint::Style::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(Color::from_argb(46, 255, 255, 255));
    canvas.draw_rrect(RRect::new_rect_xy(plinth, 10.0, 10.0), &edge);

    // Platter: metal rim, rubber mat on top.
    let mut rim = Paint::default();
    rim.set_anti_alias(true);
    rim.set_shader(skia_safe::gradient_shader::linear(
        (
            (center.x - platter_r, center.y - platter_r),
            (center.x + platter_r, center.y + platter_r),
        ),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_rgb(0x8E, 0x8D, 0x92),
            Color::from_rgb(0x4A, 0x49, 0x4E),
        ]),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_circle(center, platter_r, &rim);

    let mut mat = Paint::new(Color4f::from(MAT), None);
    mat.set_anti_alias(true);
    canvas.draw_circle(center, platter_r * 0.94, &mat);

    // Strobe dots around the rim: the row that stands still at the right speed.
    let mut dot = Paint::new(Color4f::new(0.85, 0.83, 0.78, 0.55), None);
    dot.set_anti_alias(true);
    for i in 0..48 {
        let a = i as f32 / 48.0 * std::f32::consts::TAU;
        let r = platter_r * 0.972;
        canvas.draw_circle(
            Point::new(center.x + a.cos() * r, center.y + a.sin() * r),
            platter_r * 0.006,
            &dot,
        );
    }

    // The record's shadow on the mat.
    // The record sits proud of the mat: a soft throw plus a tight contact ring.
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_color(Color::from_argb(110, 0, 0, 0));
    shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 10.0, false));
    canvas.draw_circle(
        Point::new(center.x + 5.0, center.y + 8.0),
        platter_r * 0.90,
        &shadow,
    );
    let mut contact = Paint::default();
    contact.set_anti_alias(true);
    contact.set_color(Color::from_argb(150, 0, 0, 0));
    contact.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 3.0, false));
    canvas.draw_circle(
        Point::new(center.x + 1.5, center.y + 2.5),
        platter_r * 0.885,
        &contact,
    );
}

/// The play/pause control on the plinth. Returns nothing — hit testing lives in
/// [`play_button_hit`] so the layout owns the geometry.
pub fn play_button(canvas: &skia_safe::Canvas, c: Point, r: f32, playing: bool, hovered: bool) {
    let face = Rect::from_xywh(c.x - r, c.y - r, r * 2.0, r * 2.0);
    // Square, with just enough rounding to look moulded rather than cut.
    let radius = r * 0.12;

    // The well the cap sits in.
    let mut well = Paint::default();
    well.set_anti_alias(true);
    well.set_color(Color::from_argb(180, 8, 8, 10));
    canvas.draw_rrect(
        RRect::new_rect_xy(face.with_offset((0.0, 1.5)), radius, radius),
        &well,
    );

    let mut cap = Paint::default();
    cap.set_anti_alias(true);
    let top = if hovered { 0x5E } else { 0x4C };
    cap.set_shader(skia_safe::gradient_shader::linear(
        ((c.x, c.y - r), (c.x, c.y + r)),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_rgb(top, top, top + 4),
            Color::from_rgb(0x2A, 0x2A, 0x2E),
        ]),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ));
    let inset = face.with_inset((r * 0.06, r * 0.06));
    canvas.draw_rrect(RRect::new_rect_xy(inset, radius, radius), &cap);

    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(skia_safe::paint::Style::Stroke);
    ring.set_stroke_width(1.0);
    ring.set_color(Color::from_argb(70, 255, 255, 255));
    canvas.draw_rrect(RRect::new_rect_xy(inset, radius, radius), &ring);

    // Glyph: a pause bar pair while playing, a triangle while stopped.
    let mut glyph = Paint::new(
        Color4f::new(0.98, 0.86, 0.60, if playing { 1.0 } else { 0.85 }),
        None,
    );
    glyph.set_anti_alias(true);
    let s = r * 0.62;
    if playing {
        let bw = s * 0.30;
        let gap = s * 0.26;
        canvas.draw_rrect(
            RRect::new_rect_xy(
                Rect::from_xywh(c.x - gap / 2.0 - bw, c.y - s / 2.0, bw, s),
                1.5,
                1.5,
            ),
            &glyph,
        );
        canvas.draw_rrect(
            RRect::new_rect_xy(
                Rect::from_xywh(c.x + gap / 2.0, c.y - s / 2.0, bw, s),
                1.5,
                1.5,
            ),
            &glyph,
        );
    } else {
        let mut tri = skia_safe::PathBuilder::new();
        tri.move_to((c.x - s * 0.30, c.y - s * 0.52));
        tri.line_to((c.x + s * 0.46, c.y));
        tri.line_to((c.x - s * 0.30, c.y + s * 0.52));
        tri.close();
        canvas.draw_path(&tri.detach(), &glyph);
    }
}

/// The maker's mark silkscreened on the plinth: Otto's two dots and the
/// wordmark, small and low-contrast the way real deck branding is.
pub fn badge(canvas: &skia_safe::Canvas, at: Point, scale: f32) {
    let mut ink = Paint::new(Color4f::new(0.86, 0.85, 0.83, 0.72), None);
    ink.set_anti_alias(true);

    let r = 3.4 * scale;
    let gap = 5.2 * scale;
    canvas.draw_circle(Point::new(at.x + r, at.y), r, &ink);
    canvas.draw_circle(Point::new(at.x + r * 3.0 + gap, at.y), r, &ink);

    let font = otto_kit::typography::TextStyle {
        family: "Inter",
        weight: 600,
        size: 11.0 * scale,
    }
    .font();
    ink.set_alpha_f(0.62);
    canvas.draw_str("otto", (at.x, at.y + 17.0 * scale), &font, &ink);
}

/// Speed markings beside the control.
pub fn speed_plate(canvas: &skia_safe::Canvas, at: Point, active_45: bool) {
    let mut ink = Paint::new(Color4f::new(0.78, 0.77, 0.74, 0.75), None);
    ink.set_anti_alias(true);
    let font = otto_kit::typography::TextStyle {
        family: "Inter",
        weight: 600,
        size: 9.0,
    }
    .font();
    let (a, b) = if active_45 { (0.35, 0.9) } else { (0.9, 0.35) };
    ink.set_alpha_f(a);
    canvas.draw_str("33", (at.x, at.y), &font, &ink);
    ink.set_alpha_f(b);
    canvas.draw_str("45", (at.x + 18.0, at.y), &font, &ink);
}
