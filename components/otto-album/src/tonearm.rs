//! The tonearm. It pivots from its post to the right of the platter and tracks
//! inward as the track plays: the arm's angle is the clearest read of "a record
//! is playing, and this is how far in it is".

use skia_safe::{BlurStyle, Color, Color4f, MaskFilter, Paint, Point};

/// Where the groove starts and ends, as fractions of the disc radius.
const OUTER: f32 = 0.94;
const INNER: f32 = 0.44;

pub struct Arm {
    /// Pivot post position.
    pub pivot: Point,
    /// Distance from the pivot to the stylus.
    pub length: f32,
}

impl Arm {
    /// The usual geometry: post to the lower right of the platter, arm long
    /// enough to reach past the outer groove.
    pub fn beside(center: Point, r: f32) -> Self {
        Self {
            pivot: Point::new(center.x + r * 1.16, center.y + r * 0.68),
            length: r * 1.16,
        }
    }

    /// Where the stylus sits for `progress` (0 = first groove, 1 = run-out).
    pub fn stylus(&self, center: Point, r: f32, progress: f32) -> Point {
        let track_r = r * (OUTER + (INNER - OUTER) * progress.clamp(0.0, 1.0));
        let to_pivot = Point::new(self.pivot.x - center.x, self.pivot.y - center.y);
        let d = (to_pivot.x * to_pivot.x + to_pivot.y * to_pivot.y).sqrt();
        if d < 0.001 {
            return center;
        }
        // Intersection of the groove circle with the arm's reach.
        let a = (track_r * track_r - self.length * self.length + d * d) / (2.0 * d);
        let h = (track_r * track_r - a * a).max(0.0).sqrt();
        let u = Point::new(to_pivot.x / d, to_pivot.y / d);
        let base = Point::new(center.x + u.x * a, center.y + u.y * a);
        // Two solutions; take the one above the pivot line, so the arm sweeps
        // across the record rather than around the outside.
        Point::new(base.x - (-u.y) * h, base.y - u.x * h)
    }

    /// `lift` is 0 with the stylus in the groove and 1 with the arm raised.
    pub fn draw(
        &self,
        canvas: &skia_safe::Canvas,
        center: Point,
        r: f32,
        progress: f32,
        lift: f32,
    ) {
        let stylus = self.stylus(center, r, progress);
        // Raising the arm lifts the whole tube a little and pulls its shadow
        // away from it, which is what reads as height.
        let rise = lift * r * 0.075;
        let stylus = Point::new(stylus.x, stylus.y - rise);
        let dir = normalize(Point::new(stylus.x - self.pivot.x, stylus.y - self.pivot.y));
        let perp = Point::new(-dir.y, dir.x);

        // Shadow of the arm on the record.
        let mut shadow = Paint::default();
        shadow.set_anti_alias(true);
        shadow.set_style(skia_safe::paint::Style::Stroke);
        shadow.set_stroke_width(r * 0.05);
        shadow.set_color(Color::from_argb((90.0 - 34.0 * lift) as u8, 0, 0, 0));
        shadow.set_mask_filter(MaskFilter::blur(
            BlurStyle::Normal,
            r * (0.03 + 0.05 * lift),
            false,
        ));
        let drop = 6.0 + rise * 1.8;
        canvas.draw_line(
            (self.pivot.x + 4.0, self.pivot.y + drop),
            (stylus.x + 4.0, stylus.y + drop + rise),
            &shadow,
        );

        // Counterweight, behind the pivot.
        let pivot = Point::new(self.pivot.x, self.pivot.y - rise * 0.35);
        let back = Point::new(pivot.x - dir.x * r * 0.30, pivot.y - dir.y * r * 0.30);
        let mut metal = Paint::new(Color4f::new(0.72, 0.72, 0.74, 1.0), None);
        metal.set_anti_alias(true);
        metal.set_style(skia_safe::paint::Style::Stroke);
        metal.set_stroke_width(r * 0.036);
        metal.set_stroke_cap(skia_safe::paint::Cap::Round);
        canvas.draw_line(pivot, back, &metal);

        let mut weight = Paint::new(Color4f::new(0.16, 0.16, 0.18, 1.0), None);
        weight.set_anti_alias(true);
        canvas.draw_circle(back, r * 0.088, &weight);

        // The arm tube: brushed metal, lit from the same side as everything.
        let mut tube = Paint::default();
        tube.set_anti_alias(true);
        tube.set_style(skia_safe::paint::Style::Stroke);
        tube.set_stroke_width(r * 0.050);
        tube.set_stroke_cap(skia_safe::paint::Cap::Round);
        tube.set_shader(skia_safe::gradient_shader::linear(
            (
                (pivot.x + perp.x * r * 0.04, pivot.y + perp.y * r * 0.04),
                (pivot.x - perp.x * r * 0.04, pivot.y - perp.y * r * 0.04),
            ),
            skia_safe::gradient_shader::GradientShaderColors::Colors(&[
                Color::from_rgb(0xEF, 0xEF, 0xF2),
                Color::from_rgb(0x9A, 0x9A, 0xA0),
                Color::from_rgb(0x5A, 0x5A, 0x60),
            ]),
            None,
            skia_safe::TileMode::Clamp,
            None,
            None,
        ));
        canvas.draw_line(pivot, stylus, &tube);

        // Headshell and stylus.
        let shell_back = Point::new(stylus.x - dir.x * r * 0.13, stylus.y - dir.y * r * 0.13);
        let mut shell = Paint::new(Color4f::new(0.12, 0.12, 0.14, 1.0), None);
        shell.set_anti_alias(true);
        shell.set_style(skia_safe::paint::Style::Stroke);
        shell.set_stroke_width(r * 0.070);
        shell.set_stroke_cap(skia_safe::paint::Cap::Square);
        canvas.draw_line(shell_back, stylus, &shell);

        let mut tip = Paint::new(Color4f::new(0.85, 0.85, 0.88, 1.0), None);
        tip.set_anti_alias(true);
        canvas.draw_circle(stylus, r * 0.014, &tip);

        // Pivot post.
        let mut post = Paint::new(Color4f::new(0.24, 0.24, 0.26, 1.0), None);
        post.set_anti_alias(true);
        canvas.draw_circle(pivot, r * 0.115, &post);
        let mut collar = Paint::new(Color4f::new(0.62, 0.62, 0.66, 1.0), None);
        collar.set_anti_alias(true);
        collar.set_style(skia_safe::paint::Style::Stroke);
        collar.set_stroke_width(r * 0.014);
        canvas.draw_circle(pivot, r * 0.115, &collar);
    }
}

fn normalize(p: Point) -> Point {
    let len = (p.x * p.x + p.y * p.y).sqrt().max(0.0001);
    Point::new(p.x / len, p.y / len)
}
