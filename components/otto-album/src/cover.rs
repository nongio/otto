//! Example album artwork drawn with Skia, so the window has a real cover to
//! render before any player is connected: a vintage LP — aged paper sleeve
//! with ring wear, the record itself with grooves and a sheen, and a printed
//! centre label.

use otto_kit::typography::TextStyle;
use skia_safe::{surfaces, Color, Color4f, Image, Paint, Point, Rect, TileMode};

const PAPER: Color = Color::new(0xFF_E7_D8_B8);
const PAPER_DARK: Color = Color::new(0xFF_C9_B4_8C);
const VINYL: Color = Color::new(0xFF_11_10_12);
const LABEL: Color = Color::new(0xFF_C6_4A_1E);
const LABEL_INK: Color = Color::new(0xFF_F4_E6_CB);

/// The album art shipped with the app: the *Time Out* sleeve, decoded from the
/// bundled JPEG. Falls back to `example_cover` if it cannot be decoded.
pub fn bundled_cover() -> Option<Image> {
    const BYTES: &[u8] = include_bytes!("../resources/example-cover.jpg");
    Image::from_encoded(skia_safe::Data::new_copy(BYTES))
}

/// The record label shipped with the app: the Unknown Pleasures side-A label,
/// cropped from the Cover Art Archive scan of the vinyl.
pub fn bundled_label() -> Option<Image> {
    const BYTES: &[u8] = include_bytes!("../resources/example-label.jpg");
    Image::from_encoded(skia_safe::Data::new_copy(BYTES))
}

/// Draw the example cover `size × size`.
pub fn example_cover(size: i32) -> Image {
    let mut surface =
        surfaces::raster_n32_premul((size, size)).expect("raster surface for the example cover");
    let canvas = surface.canvas();
    let s = size as f32;
    let c = Point::new(s * 0.5, s * 0.52);
    let disc_r = s * 0.38;

    aged_paper(canvas, s);
    ring_wear(canvas, c, disc_r, s);
    record(canvas, c, disc_r, s);
    label(canvas, c, disc_r, s);
    sleeve_title(canvas, s);
    grain(canvas, size);

    surface.image_snapshot()
}

/// Warm paper with a vignette, as if photographed under a lamp.
fn aged_paper(canvas: &skia_safe::Canvas, s: f32) {
    let mut paint = Paint::default();
    paint.set_shader(skia_safe::gradient_shader::linear(
        ((0.0, 0.0), (s, s)),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[PAPER, PAPER_DARK]),
        None,
        TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_rect(Rect::from_wh(s, s), &paint);

    let mut vignette = Paint::default();
    vignette.set_shader(skia_safe::gradient_shader::radial(
        Point::new(s * 0.5, s * 0.45),
        s * 0.72,
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_argb(0, 0, 0, 0),
            Color::from_argb(90, 60, 40, 20),
        ]),
        None,
        TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_rect(Rect::from_wh(s, s), &vignette);
}

/// The circular impression an LP leaves on the sleeve it lived in.
fn ring_wear(canvas: &skia_safe::Canvas, c: Point, disc_r: f32, s: f32) {
    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(skia_safe::paint::Style::Stroke);
    for (r, alpha, w) in [
        (disc_r * 1.06, 46u8, 0.010),
        (disc_r * 1.10, 26, 0.018),
        (disc_r * 0.62, 20, 0.008),
    ] {
        ring.set_color(Color::from_argb(alpha, 90, 66, 40));
        ring.set_stroke_width(s * w);
        canvas.draw_circle(c, r, &ring);
    }
}

/// The record: body, groove bands, and a diagonal sheen across it.
fn record(canvas: &skia_safe::Canvas, c: Point, disc_r: f32, s: f32) {
    let mut body = Paint::new(Color4f::from(VINYL), None);
    body.set_anti_alias(true);
    canvas.draw_circle(c, disc_r, &body);

    // Grooves: closely spaced rings, with a few wider band separations.
    let mut groove = Paint::default();
    groove.set_anti_alias(true);
    groove.set_style(skia_safe::paint::Style::Stroke);
    let inner = disc_r * 0.40;
    let mut r = inner;
    let mut i = 0;
    while r < disc_r * 0.985 {
        let band_edge = i % 26 == 0;
        groove.set_stroke_width(s * if band_edge { 0.0035 } else { 0.0016 });
        groove.set_color(Color::from_argb(
            if band_edge { 70 } else { 34 },
            235,
            228,
            214,
        ));
        canvas.draw_circle(c, r, &groove);
        r += s * 0.0055;
        i += 1;
    }

    // Sheen: a soft diagonal highlight, clipped to the disc.
    canvas.save();
    let mut clip = skia_safe::PathBuilder::new();
    clip.add_circle(c, disc_r, None);
    canvas.clip_path(&clip.detach(), None, true);
    let mut sheen = Paint::default();
    sheen.set_shader(skia_safe::gradient_shader::linear(
        ((c.x - disc_r, c.y - disc_r), (c.x + disc_r, c.y + disc_r)),
        skia_safe::gradient_shader::GradientShaderColors::Colors(&[
            Color::from_argb(0, 255, 255, 255),
            Color::from_argb(46, 255, 250, 235),
            Color::from_argb(0, 255, 255, 255),
            Color::from_argb(26, 255, 250, 235),
            Color::from_argb(0, 255, 255, 255),
        ]),
        Some(&[0.18f32, 0.32, 0.46, 0.68, 0.84][..]),
        TileMode::Clamp,
        None,
        None,
    ));
    canvas.draw_circle(c, disc_r, &sheen);
    canvas.restore();

    // Outer edge.
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::paint::Style::Stroke);
    edge.set_stroke_width(s * 0.004);
    edge.set_color(Color::from_argb(80, 255, 245, 225));
    canvas.draw_circle(c, disc_r, &edge);
}

/// Printed centre label, spindle hole included.
fn label(canvas: &skia_safe::Canvas, c: Point, disc_r: f32, s: f32) {
    let label_r = disc_r * 0.38;
    let mut paint = Paint::new(Color4f::from(LABEL), None);
    paint.set_anti_alias(true);
    canvas.draw_circle(c, label_r, &paint);

    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(skia_safe::paint::Style::Stroke);
    ring.set_stroke_width(s * 0.004);
    ring.set_color(Color::from_argb(150, 244, 230, 203));
    canvas.draw_circle(c, label_r * 0.86, &ring);

    let mut text = Paint::new(Color4f::from(LABEL_INK), None);
    text.set_anti_alias(true);
    let title = TextStyle {
        family: "Inter",
        weight: 700,
        size: s * 0.040,
    }
    .font();
    let small = TextStyle {
        family: "Inter",
        weight: 500,
        size: s * 0.024,
    }
    .font();
    centered(canvas, "TAKE FIVE", &title, c.x, c.y - s * 0.012, &text);
    centered(
        canvas,
        "DAVE BRUBECK QUARTET",
        &small,
        c.x,
        c.y + s * 0.020,
        &text,
    );
    centered(canvas, "33⅓ RPM", &small, c.x, c.y + s * 0.052, &text);

    // Spindle hole.
    let mut hole = Paint::new(Color4f::from(PAPER_DARK), None);
    hole.set_anti_alias(true);
    canvas.draw_circle(c, s * 0.016, &hole);
    let mut shadow = Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_style(skia_safe::paint::Style::Stroke);
    shadow.set_stroke_width(s * 0.004);
    shadow.set_color(Color::from_argb(120, 40, 26, 14));
    canvas.draw_circle(c, s * 0.016, &shadow);
}

/// Sleeve lettering, letterpressed at the top.
fn sleeve_title(canvas: &skia_safe::Canvas, s: f32) {
    let mut text = Paint::new(Color4f::from(Color::new(0xFF_2A_1E_14)), None);
    text.set_anti_alias(true);
    let heading = TextStyle {
        family: "Inter",
        weight: 700,
        size: s * 0.062,
    }
    .font();
    let sub = TextStyle {
        family: "Inter",
        weight: 500,
        size: s * 0.028,
    }
    .font();
    canvas.draw_str("TIME OUT", (s * 0.07, s * 0.095), &heading, &text);
    text.set_color(Color::from_argb(190, 42, 30, 20));
    canvas.draw_str(
        "THE DAVE BRUBECK QUARTET  ·  LONG PLAYING",
        (s * 0.072, s * 0.128),
        &sub,
        &text,
    );
}

/// Paper grain, so the flat fills read as print rather than vector.
fn grain(canvas: &skia_safe::Canvas, size: i32) {
    let mut paint = Paint::default();
    let mut seed: u32 = 0x9E37_79B9;
    for _ in 0..(size as u32 * 26) {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let x = ((seed >> 8) % size as u32) as f32;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let y = ((seed >> 8) % size as u32) as f32;
        let a = ((seed >> 25) % 16) as u8;
        let dark = (seed >> 3) & 1 == 0;
        paint.set_color(if dark {
            Color::from_argb(a, 60, 40, 20)
        } else {
            Color::from_argb(a, 255, 250, 235)
        });
        canvas.draw_rect(Rect::from_xywh(x, y, 1.0, 1.0), &paint);
    }
}

fn centered(
    canvas: &skia_safe::Canvas,
    text: &str,
    font: &skia_safe::Font,
    cx: f32,
    baseline: f32,
    paint: &Paint,
) {
    let w = font.measure_str(text, Some(paint)).0;
    canvas.draw_str(text, (cx - w / 2.0, baseline), font, paint);
}

#[cfg(test)]
mod tests {
    /// Dev helper: `COVER_OUT=/tmp/cover.png cargo test -p otto-album dump`
    /// writes the example cover to a PNG so the artwork can be eyeballed
    /// without running the app. A no-op when COVER_OUT is unset.
    #[test]
    fn dump() {
        let Ok(out) = std::env::var("COVER_OUT") else {
            return;
        };
        let img = super::example_cover(600);
        let data = img
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .expect("encode cover");
        std::fs::write(out, data.as_bytes()).expect("write cover");
    }
}
