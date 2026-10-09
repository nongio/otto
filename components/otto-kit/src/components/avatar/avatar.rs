// Rust guideline compliant 2026-02-21

use skia_safe::{
    Canvas, ClipOp, Color, FilterMode, MipmapMode, Paint, PaintStyle, PathBuilder, Point, RRect,
    Rect, SamplingOptions,
};

use crate::typography::TextStyle;

/// Grounds for the initials, chosen to keep white text above 4.5:1 contrast.
const GROUNDS: [Color; 8] = [
    Color::from_rgb(0x2F, 0x7D, 0x6D),
    Color::from_rgb(0x1D, 0x63, 0xC8),
    Color::from_rgb(0x8A, 0x4F, 0xB8),
    Color::from_rgb(0xB0, 0x4A, 0x2E),
    Color::from_rgb(0x9C, 0x3D, 0x6B),
    Color::from_rgb(0x4A, 0x6B, 0x1F),
    Color::from_rgb(0x8A, 0x5A, 0x14),
    Color::from_rgb(0x3E, 0x55, 0x8F),
];

/// Ground behind a nameless avatar's silhouette.
const NAMELESS_GROUND: Color = Color::from_rgb(0x70, 0x70, 0x70);

/// Up to two initials for `name`: the first letter of its first and last words.
///
/// Returns an empty string for a name with no letters, which [`draw`] shows as
/// the silhouette instead.
///
/// # Examples
///
/// ```
/// use otto_kit::components::avatar::initials;
/// assert_eq!(initials("Ana López"), "AL");
/// assert_eq!(initials("guest"), "G");
/// assert_eq!(initials("  "), "");
/// ```
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|word| word.chars().next().is_some_and(char::is_alphanumeric))
        .collect();
    let first = |word: &&str| word.chars().next().into_iter().flat_map(char::to_uppercase);
    match words.as_slice() {
        [] => String::new(),
        [only] => first(only).collect(),
        [head, .., tail] => first(head).chain(first(tail)).collect(),
    }
}

/// The ground color for `name`'s initials, stable for the same name.
///
/// A small FNV-1a hash picks one of a fixed set of grounds, so a person keeps
/// their color across runs and machines.
pub fn ground_color(name: &str) -> Color {
    let hash = name.bytes().fold(0x811C_9DC5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    GROUNDS[hash as usize % GROUNDS.len()]
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

/// Initials text style scaled to the avatar: roughly a third of its diameter.
fn initials_style(diameter: f32) -> TextStyle {
    let mut style = crate::typography::styles::BODY_EMPHASIZED;
    style.size = (diameter * 0.36).max(8.0);
    style
}

/// A head and shoulders in white, clipped by the caller to the circle.
fn draw_silhouette(canvas: &Canvas, rect: Rect) {
    let d = rect.width().min(rect.height());
    let cx = rect.center_x();
    let cy = rect.center_y();
    let mut stroke = fill(Color::WHITE);
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width((d * 0.08).max(1.2));
    stroke.set_stroke_cap(skia_safe::paint::Cap::Round);
    canvas.draw_circle(Point::new(cx, cy - d * 0.08), d * 0.16, &stroke);
    let mut shoulders = PathBuilder::new();
    shoulders.move_to((cx - d * 0.28, cy + d * 0.32));
    shoulders.cubic_to(
        (cx - d * 0.28, cy + d * 0.14),
        (cx + d * 0.28, cy + d * 0.14),
        (cx + d * 0.28, cy + d * 0.32),
    );
    canvas.draw_path(&shoulders.detach(), &stroke);
}

/// Draw the avatar for `name` as the largest circle centered in `rect`.
///
/// `picture` is drawn cropped to fill the circle when given. Without one, the
/// name's [`initials`] are drawn on its [`ground_color`]; a name with no
/// initials gets a neutral silhouette.
pub fn draw(canvas: &Canvas, rect: Rect, picture: Option<&skia_safe::Image>, name: &str) {
    let d = rect.width().min(rect.height());
    let circle = Rect::from_xywh(rect.center_x() - d / 2.0, rect.center_y() - d / 2.0, d, d);

    canvas.save();
    canvas.clip_rrect(RRect::new_oval(circle), ClipOp::Intersect, true);
    if let Some(image) = picture {
        // Cover: scale the shorter side to the circle and crop the rest.
        let (w, h) = (image.width() as f32, image.height() as f32);
        let side = w.min(h);
        let src = Rect::from_xywh((w - side) / 2.0, (h - side) / 2.0, side, side);
        canvas.draw_image_rect_with_sampling_options(
            image,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
            circle,
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
            &Paint::default(),
        );
    } else {
        let letters = initials(name);
        if letters.is_empty() {
            canvas.draw_rect(circle, &fill(NAMELESS_GROUND));
            draw_silhouette(canvas, circle);
        } else {
            canvas.draw_rect(circle, &fill(ground_color(name)));
            let style = initials_style(d);
            let width = style.font().measure_str(&letters, None).0;
            use crate::common::Renderable;
            crate::components::label::Label::new(&letters)
                .with_style(style)
                .with_color(Color::WHITE)
                .centered_on(circle.center_x() - width / 2.0, circle.center_y())
                .render(canvas);
        }
    }
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_the_first_and_last_words() {
        assert_eq!(initials("Riccardo Canalicchio"), "RC");
        assert_eq!(initials("Mary Ann de la Cruz"), "MC");
        assert_eq!(initials("élodie"), "É");
        assert_eq!(initials("— ·"), "");
    }

    #[test]
    fn a_name_keeps_its_color() {
        assert_eq!(ground_color("Ana López"), ground_color("Ana López"));
    }

    #[test]
    fn draw_smoke_test_does_not_panic() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((64, 64)).unwrap();
        let rect = Rect::from_wh(64.0, 64.0);
        draw(surface.canvas(), rect, None, "Ana López");
        draw(surface.canvas(), rect, None, "");
    }
}
