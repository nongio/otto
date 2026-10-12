//! The text-recognition badge in the corner of a Peek preview.
//!
//! One dot, two states: recognising (it breathes) and recognised (the
//! picture's words can be selected). Files draws it on the Peek panel and
//! Preview on its window's content, so it lives with the session both embed.

use otto_kit::icons;
use otto_kit::theme::Theme;
use skia_safe::{Canvas, Color, Paint, Point, Rect};

/// The badge that says the picture's text has been recognised and can be
/// selected, in the panel's bottom-right corner.
fn badge_rect(panel: Rect) -> Rect {
    const D: f32 = 22.0;
    const INSET: f32 = 10.0;
    Rect::from_xywh(panel.right - INSET - D, panel.bottom - INSET - D, D, D)
}

/// The badge while the recogniser is still reading the picture, in the same
/// dot as the finished one so the two read as one thing changing rather than
/// two appearing. It breathes: recognition takes seconds on a big screenshot,
/// and a still glyph for that long looks like a result rather than a wait.
pub fn draw_working(canvas: &Canvas, theme: &Theme, panel: Rect, opacity: f32, phase: f32) {
    // A slow breath, never all the way out: the badge stays legible at the
    // bottom of it, so what pulses is attention rather than presence.
    let breath = 0.65 + 0.35 * (phase * std::f32::consts::TAU / WORKING_BADGE_PERIOD).sin();
    draw_peek_badge(
        canvas,
        theme,
        panel,
        opacity,
        breath,
        &[
            "content-loading-symbolic",
            "process-working-symbolic",
            "view-refresh-symbolic",
        ],
        draw_working_fallback,
    );
}

/// How long one breath of the working badge takes, in seconds.
const WORKING_BADGE_PERIOD: f32 = 1.6;

/// A dot in the same idiom as the close button, carrying a text-selection
/// glyph. Its presence is the message: the picture has words in it, and they
/// can be selected.
pub fn draw_text(canvas: &Canvas, theme: &Theme, panel: Rect, opacity: f32) {
    draw_peek_badge(
        canvas,
        theme,
        panel,
        opacity,
        1.0,
        &[
            "font-x-generic-symbolic",
            "insert-text-symbolic",
            "text-x-generic-symbolic",
        ],
        draw_text_fallback,
    );
}

/// The badge both states share: a dot in the same idiom as the close button,
/// carrying whichever symbolic glyph the caller asked for. `weight` scales it
/// against the panel's own chrome opacity, which is what the working badge
/// breathes on.
fn draw_peek_badge(
    canvas: &Canvas,
    theme: &Theme,
    panel: Rect,
    opacity: f32,
    weight: f32,
    names: &[&str],
    fallback: fn(&Canvas, &Theme, Rect, Point, f32),
) {
    let opacity = opacity * weight;
    let badge = badge_rect(panel);
    let centre = Point::new(badge.center_x(), badge.center_y());

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(fade(theme.fill_secondary, opacity));
    canvas.draw_circle(centre, badge.width() / 2.0, &paint);

    let glyph = 12.0;
    let dst = Rect::from_xywh(centre.x - glyph / 2.0, centre.y - glyph / 2.0, glyph, glyph);
    if let Some(image) = icons::cached_icon_chain(names, glyph as i32) {
        // Symbolic art recoloured to the theme's text tone, as the sidebar
        // does for its own glyphs.
        let mut tint = Paint::default();
        tint.set_color_filter(skia_safe::color_filters::blend(
            fade(theme.text_secondary, opacity),
            skia_safe::BlendMode::SrcIn,
        ));
        canvas.draw_image_rect(&image, None, dst, &tint);
    } else {
        fallback(canvas, theme, dst, centre, opacity);
    }
}

/// No theme art for the finished badge: a serif "T", the plainest sign for
/// text.
fn draw_text_fallback(canvas: &Canvas, theme: &Theme, dst: Rect, centre: Point, opacity: f32) {
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(skia_safe::paint::Style::Stroke);
    stroke.set_stroke_width(1.5);
    stroke.set_stroke_cap(skia_safe::PaintCap::Round);
    stroke.set_color(fade(theme.text_secondary, opacity));
    canvas.draw_line(
        (dst.left + 2.0, dst.top + 2.0),
        (dst.right - 2.0, dst.top + 2.0),
        &stroke,
    );
    canvas.draw_line(
        (centre.x, dst.top + 2.0),
        (centre.x, dst.bottom - 2.0),
        &stroke,
    );
}

/// No theme art for the working badge: three dots in a row, which is what
/// the themes draw for this anyway.
fn draw_working_fallback(canvas: &Canvas, theme: &Theme, dst: Rect, centre: Point, opacity: f32) {
    let mut dot = Paint::default();
    dot.set_anti_alias(true);
    dot.set_color(fade(theme.text_secondary, opacity));
    let r = 1.3;
    let gap = (dst.width() - 2.0 * r) / 2.0;
    for i in -1..=1 {
        canvas.draw_circle((centre.x + i as f32 * gap, centre.y), r, &dot);
    }
}

/// A colour at a fraction of its own alpha. Scaled, never replaced: the
/// theme's fills are translucent by design and forcing one opaque to fade it
/// in would make it darker at the end than it is at rest.
fn fade(color: Color, opacity: f32) -> Color {
    Color::from_argb(
        (color.a() as f32 * opacity) as u8,
        color.r(),
        color.g(),
        color.b(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw one badge state into its own bitmap and hand back the pixels of
    /// the badge's own square, which is all either state touches.
    fn badge_pixels(draw: impl FnOnce(&Canvas, &Theme, Rect)) -> Vec<u8> {
        let panel = Rect::from_wh(200.0, 200.0);
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((panel.width() as i32, panel.height() as i32))
                .unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let theme = Theme::light();
        draw(surface.canvas(), &theme, panel);

        let badge = badge_rect(panel);
        let image = surface.image_snapshot();
        let info = skia_safe::ImageInfo::new_n32_premul(
            (badge.width() as i32, badge.height() as i32),
            None,
        );
        let mut out = vec![0u8; (badge.width() * badge.height() * 4.0) as usize];
        let row = badge.width() as usize * 4;
        assert!(image.read_pixels(
            &info,
            &mut out,
            row,
            (badge.left as i32, badge.top as i32),
            skia_safe::image::CachingHint::Allow,
        ));
        out
    }

    fn drawn(pixels: &[u8]) -> usize {
        pixels.chunks(4).filter(|p| p[3] != 0).count()
    }

    /// The working state puts something in the corner as soon as the
    /// recogniser starts — the wait is the thing it exists to show.
    #[test]
    fn the_working_badge_draws_in_the_corner() {
        let pixels =
            badge_pixels(|canvas, theme, panel| draw_working(canvas, theme, panel, 1.0, 0.0));
        assert!(drawn(&pixels) > 100, "nothing drawn in the badge's square");
    }

    /// And it breathes: half a period apart the same badge is not the same
    /// pixels, which is what makes it read as a wait rather than a result.
    #[test]
    fn the_working_badge_breathes() {
        let early = badge_pixels(|canvas, theme, panel| {
            draw_working(canvas, theme, panel, 1.0, WORKING_BADGE_PERIOD / 4.0)
        });
        let later = badge_pixels(|canvas, theme, panel| {
            draw_working(canvas, theme, panel, 1.0, WORKING_BADGE_PERIOD * 3.0 / 4.0)
        });
        assert_ne!(early, later, "the badge looks the same across a breath");
    }

    /// The two states are told apart by their glyph, not only by being there.
    #[test]
    fn the_finished_badge_is_not_the_working_one() {
        let working =
            badge_pixels(|canvas, theme, panel| draw_working(canvas, theme, panel, 1.0, 0.0));
        let finished = badge_pixels(|canvas, theme, panel| draw_text(canvas, theme, panel, 1.0));
        assert!(drawn(&finished) > 100);
        assert_ne!(working, finished, "both states draw the same badge");
    }
}
