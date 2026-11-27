//! Segmented control: a row of mutually exclusive choices in one rounded
//! recessed track, the chosen one raised on a lighter pill with a hairline
//! edge and a soft shadow under it, its label in primary ink at medium weight.
//!
//! No state of its own and no client runtime: it draws from a bare canvas,
//! and the caller owns which segment is chosen and hands the press to
//! [`segment_at`]. A segment is a short text or one of Otto's Solar glyphs.

use skia_safe::{
    BlurStyle, Canvas, Color, Contains, MaskFilter, Paint, PaintStyle, Point, RRect, Rect,
};

use crate::common::Renderable;
use crate::components::label::Label;
use crate::icons::{self, SolarStyle};
use crate::theme::Theme;
use crate::typography::{styles, TextStyle};

/// Height the control was tuned at: the dropdown field's, so the two sit on
/// one line in a form. The geometry follows the `rect` it is given, but this
/// is the reference.
pub const HEIGHT: f32 = crate::components::dropdown::field::HEIGHT;

/// The track's corner radius.
const TRACK_RADIUS: f32 = 7.0;
/// How far the chosen segment's pill sits inside its segment.
const PILL_INSET: f32 = 1.0;
/// The pill's corner radius: the track's, less the inset, so the two curves
/// run parallel.
const PILL_RADIUS: f32 = TRACK_RADIUS - PILL_INSET;
/// Side of a glyph segment's glyph, in points.
const GLYPH_SIZE: f32 = 16.0;
/// How far the pill's shadow falls below it, and how soft it is.
const SHADOW_OFFSET: f32 = 1.0;
const SHADOW_SIGMA: f32 = 1.5;
/// The share of the theme's shadow colour the pill casts: a lift, not a
/// floating card.
const SHADOW_SHARE: f32 = 0.4;
/// The width of the pill's hairline edge.
const EDGE_WIDTH: f32 = 1.0;

/// An unchosen segment's label: the dropdown field's size, so the two
/// controls read as one set side by side.
pub const LABEL: TextStyle = styles::BODY;
/// The chosen segment's label: the same size, a step heavier, which is the
/// dropdown field's own style.
pub const LABEL_CHOSEN: TextStyle = crate::components::dropdown::field::LABEL_STYLE;

/// What a segment shows.
#[derive(Clone, Copy)]
pub enum Segment<'a> {
    /// A short label.
    Text(&'a str),
    /// One of Otto's Solar glyphs, by name. `fallback` draws a stand-in
    /// centred on the point, in the colour, where the set isn't installed or
    /// lacks the glyph; with none the segment is left empty.
    Glyph {
        name: &'a str,
        fallback: Option<fn(&Canvas, Point, Color)>,
    },
}

/// Interaction state, set by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegmentedInteraction {
    #[default]
    Normal,
    /// The chosen segment is the only one available: the others are drawn
    /// in quieter ink, unavailable rather than merely unchosen.
    Locked,
}

/// The segment at `(x, y)`, if the point is on the control. Reads the same
/// `rect` [`draw`] paints into, split evenly between `count` segments.
pub fn segment_at(rect: Rect, count: usize, x: f32, y: f32) -> Option<usize> {
    if count == 0 || !rect.contains(Point::new(x, y)) {
        return None;
    }
    let index = ((x - rect.left) / (rect.width() / count as f32)) as usize;
    Some(index.min(count - 1))
}

/// The rect of segment `index` of `count` within `rect`.
pub fn segment_rect(rect: Rect, count: usize, index: usize) -> Rect {
    let w = rect.width() / count.max(1) as f32;
    Rect::from_xywh(rect.left + w * index as f32, rect.top, w, rect.height())
}

/// A segment label's style: [`LABEL_CHOSEN`] on the chosen one, [`LABEL`]
/// otherwise. For a caller drawing its own labels over the control.
pub fn label_style(chosen: bool) -> TextStyle {
    if chosen {
        LABEL_CHOSEN
    } else {
        LABEL
    }
}

/// A segment's ink: primary on every segment, since each is a choice you can
/// read and make; the pill and the heavier weight mark the chosen one.
/// Tertiary on the others of a locked control, which can't be chosen. For a
/// caller drawing its own labels over the control.
pub fn label_color(theme: &Theme, chosen: bool, interaction: SegmentedInteraction) -> Color {
    match (chosen, interaction) {
        (true, _) => theme.text_primary,
        (false, SegmentedInteraction::Locked) => theme.text_tertiary,
        (false, SegmentedInteraction::Normal) => theme.text_primary,
    }
}

/// Draw the control into `rect`: the track, the chosen segment's raised pill
/// with its edge and shadow, and every segment's text or glyph.
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    segments: &[Segment<'_>],
    selected: Option<usize>,
    interaction: SegmentedInteraction,
    theme: &Theme,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(theme.control_track);
    canvas.draw_rrect(RRect::new_rect_xy(rect, TRACK_RADIUS, TRACK_RADIUS), &paint);

    let count = segments.len();
    if count == 0 {
        return;
    }

    if let Some(index) = selected.filter(|index| *index < count) {
        let pill = segment_rect(rect, count, index).with_inset((PILL_INSET, PILL_INSET));
        let pill_rrect = RRect::new_rect_xy(pill, PILL_RADIUS, PILL_RADIUS);
        let mut shadow = Paint::default();
        shadow.set_anti_alias(true);
        shadow.set_color(shadow_color(theme));
        shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, SHADOW_SIGMA, false));
        canvas.draw_rrect(pill_rrect.with_offset((0.0, SHADOW_OFFSET)), &shadow);
        paint.set_color(theme.control_raised);
        canvas.draw_rrect(pill_rrect, &paint);
        let mut edge = Paint::default();
        edge.set_anti_alias(true);
        edge.set_style(PaintStyle::Stroke);
        edge.set_stroke_width(EDGE_WIDTH);
        edge.set_color(theme.control_edge);
        let half = EDGE_WIDTH / 2.0;
        canvas.draw_rrect(
            RRect::new_rect_xy(
                pill.with_inset((half, half)),
                PILL_RADIUS - half,
                PILL_RADIUS - half,
            ),
            &edge,
        );
    }

    // Rasterised in the canvas's device pixels, so a glyph stays sharp on a
    // scaled output.
    let scale = canvas.local_to_device_as_3x3().scale_x().abs().max(1.0);
    let glyph_px = (GLYPH_SIZE * scale).round() as i32;
    for (index, segment) in segments.iter().enumerate() {
        let cell = segment_rect(rect, count, index);
        let center = Point::new(cell.center_x(), cell.center_y());
        let chosen = Some(index) == selected;
        let color = label_color(theme, chosen, interaction);
        match *segment {
            Segment::Text(text) => {
                Label::new(text)
                    .with_style(label_style(chosen))
                    .with_color(color)
                    .centered_at(center.x, center.y)
                    .render(canvas);
            }
            Segment::Glyph { name, fallback } => {
                match icons::solar_icon(name, glyph_px, SolarStyle::Linear, color) {
                    Some(image) => {
                        let half = GLYPH_SIZE / 2.0;
                        let dst = Rect::from_ltrb(
                            center.x - half,
                            center.y - half,
                            center.x + half,
                            center.y + half,
                        );
                        icons::draw_icon(canvas, &image, dst, &Paint::default());
                    }
                    None => {
                        if let Some(fallback) = fallback {
                            fallback(canvas, center, color);
                        }
                    }
                }
            }
        }
    }
}

/// The pill's shadow: the theme's own shadow colour, at a share of its
/// strength. The dark theme's is the stronger, which is what a lift needs to
/// show on a dark track.
fn shadow_color(theme: &Theme) -> Color {
    let alpha = (theme.shadow.a() as f32 * SHADOW_SHARE).round() as u8;
    Color::from_argb(alpha, theme.shadow.r(), theme.shadow.g(), theme.shadow.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_xywh(10.0, 10.0, 152.0, HEIGHT)
    }

    #[test]
    fn a_press_lands_on_the_segment_under_it() {
        let r = rect();
        let cy = r.center_y();
        assert_eq!(segment_at(r, 4, r.left + 1.0, cy), Some(0));
        assert_eq!(segment_at(r, 4, r.left + 38.0 + 1.0, cy), Some(1));
        assert_eq!(segment_at(r, 4, r.right - 0.5, cy), Some(3));
        assert_eq!(segment_at(r, 4, r.left - 1.0, cy), None);
        assert_eq!(segment_at(r, 4, r.center_x(), r.bottom + 1.0), None);
        assert_eq!(segment_at(r, 0, r.center_x(), cy), None);
    }

    #[test]
    fn segments_split_the_track_evenly() {
        let r = rect();
        let third = segment_rect(r, 4, 2);
        assert_eq!(third.left, r.left + 76.0);
        assert_eq!(third.width(), 38.0);
        assert_eq!(third.height(), r.height());
    }

    #[test]
    fn draw_does_not_panic_across_every_state_and_theme() {
        // Nothing but a raster surface: no AppContext, no wayland-client.
        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 60)).expect("surface");
        let canvas = surface.canvas();
        fn bar(canvas: &Canvas, at: Point, color: Color) {
            let mut paint = Paint::default();
            paint.set_color(color);
            canvas.draw_rect(Rect::from_xywh(at.x - 4.0, at.y - 1.0, 8.0, 2.0), &paint);
        }
        let segments = [
            Segment::Text("List"),
            Segment::Glyph {
                name: "go-up-symbolic",
                fallback: None,
            },
            Segment::Glyph {
                name: "no-such-glyph",
                fallback: Some(bar),
            },
        ];
        for theme in [Theme::light(), Theme::dark()] {
            for interaction in [SegmentedInteraction::Normal, SegmentedInteraction::Locked] {
                for selected in [None, Some(0), Some(2), Some(7)] {
                    draw(canvas, rect(), &segments, selected, interaction, &theme);
                }
            }
            draw(
                canvas,
                rect(),
                &[],
                Some(0),
                SegmentedInteraction::Normal,
                &theme,
            );
        }
    }

    /// The chosen segment casts a shadow: just under its pill the track is
    /// darker than under an unchosen segment.
    #[test]
    fn the_chosen_segment_casts_a_shadow() {
        let theme = Theme::light();
        let r = Rect::from_xywh(0.0, 0.0, 80.0, HEIGHT);
        let mut surface = skia_safe::surfaces::raster_n32_premul((80, 40)).expect("surface");
        surface.canvas().clear(Color::WHITE);
        draw(
            surface.canvas(),
            r,
            &[Segment::Text(""), Segment::Text("")],
            Some(0),
            SegmentedInteraction::Normal,
            &theme,
        );
        let image = surface.image_snapshot();
        let luma = |x: i32, y: i32| {
            let info = skia_safe::ImageInfo::new(
                (1, 1),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let mut px = [0u8; 4];
            assert!(image.read_pixels(
                &info,
                &mut px,
                4,
                (x, y),
                skia_safe::image::CachingHint::Allow
            ));
            px[0] as u32 + px[1] as u32 + px[2] as u32
        };
        // One pixel under the pill's bottom edge, under the chosen segment
        // and under the other one.
        let y = (HEIGHT - PILL_INSET + 0.5) as i32;
        assert!(luma(20, y) < luma(60, y));
    }

    /// The chosen pill is lighter than the track around it in both schemes,
    /// so the cue reads the same way light or dark.
    #[test]
    fn the_chosen_pill_is_lighter_than_the_track() {
        for (theme, ground) in [
            (Theme::light(), Color::from_rgb(0xF6, 0xF6, 0xF6)),
            (Theme::dark(), Color::from_rgb(0x24, 0x24, 0x26)),
        ] {
            let r = Rect::from_xywh(0.0, 0.0, 80.0, HEIGHT);
            let mut surface = skia_safe::surfaces::raster_n32_premul((80, 40)).expect("surface");
            surface.canvas().clear(ground);
            draw(
                surface.canvas(),
                r,
                &[Segment::Text(""), Segment::Text("")],
                Some(0),
                SegmentedInteraction::Normal,
                &theme,
            );
            let image = surface.image_snapshot();
            let luma = |x: i32, y: i32| {
                let info = skia_safe::ImageInfo::new(
                    (1, 1),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                );
                let mut px = [0u8; 4];
                assert!(image.read_pixels(
                    &info,
                    &mut px,
                    4,
                    (x, y),
                    skia_safe::image::CachingHint::Allow
                ));
                px[0] as u32 + px[1] as u32 + px[2] as u32
            };
            let mid = (HEIGHT / 2.0) as i32;
            assert!(luma(20, mid) > luma(60, mid) + 30);
        }
    }
}
