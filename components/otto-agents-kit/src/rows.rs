//! The rows of a list and the search field over it, as the launcher and the
//! side canvas paint them.
//!
//! A row is an icon, an activity dot or a checkbox, then a title with an
//! optional second line, and the name of its source at the end. Everything
//! here paints with Skia onto a canvas the host provides, so the same rows
//! read the same wherever they are listed.

use std::cell::RefCell;
use std::collections::HashMap;

use otto_kit::components::scroll::RowLayout;
use otto_kit::components::text_input::TextInputStyle;
use otto_kit::icons::named_icon_sized;
use otto_kit::skia::{
    self, Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect, SamplingOptions,
};
use otto_kit::theme::Theme;
use otto_kit::typography::{get_font_with_fallback, styles};

use crate::item::{Activity, Item};

/// Height of one row.
pub const ROW_H: f32 = 46.0;
/// How many rows are on screen at once. Past this the list scrolls, because a
/// list taller than this stops being scannable and the card starts to be the
/// screen.
pub const MAX_ROWS: usize = 8;
/// Side of a row's icon.
const ICON: f32 = 28.0;
/// The activity dot beside a row with no icon, such as an agent session.
const DOT_RADIUS: f32 = 4.0;
/// How far a row's content and highlight sit in from the list's edges.
const ROW_INSET: f32 = 8.0;
/// Corner radius of the selection's highlight.
pub const HIGHLIGHT_RADIUS: f32 = 9.0;

/// Icons decoded for rows, by icon theme name.
///
/// Kept for as long as the list is: decoding the same icon on every keystroke
/// is the one thing that would make typing feel slow. Misses are remembered
/// too, so an app whose icon the theme does not have is not looked up again.
/// Behind a cell because painting only borrows whoever holds it.
#[derive(Default)]
pub struct RowIcons(RefCell<HashMap<String, Option<Image>>>);

/// Paint the rows of `items` that fall inside `band`, in the list's content
/// coordinates — row 0 at the top — each [`ROW_H`] tall and `width` wide.
/// `labels` name each item's source, as the badge at the row's end.
///
/// The launcher's list paints its rows with this, and so does anything else
/// that lists the same things and should look like it.
pub fn paint_item_rows(
    canvas: &Canvas,
    band: Rect,
    items: &[&Item],
    labels: &[&'static str],
    width: f32,
    dark: bool,
    icons: &RowIcons,
) {
    let title_font = row_font(15.0);
    let subtitle_font = row_font(11.5);
    let badge_font = row_font(10.5);
    let (title_color, subtitle_color) = (row_title_color(dark), row_subtitle_color(dark));
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let layout = RowLayout::new(ROW_H, items.len());
    for index in layout.visible(band) {
        let item = items[index];
        let icon = item
            .icon
            .as_deref()
            .and_then(|name| resolve_icon(&mut icons.0.borrow_mut(), name));
        let dot = item.activity.map(|activity| match activity {
            Activity::Working => theme.accent,
            Activity::Idle => theme.text_tertiary,
            Activity::Waiting => theme.accent_yellow,
        });
        let check = item.checked.map(|checked| Check {
            checked,
            accent: theme.accent,
            outline: theme.text_tertiary,
        });
        let draw = draw_row(
            icon,
            dot,
            check,
            item.title.clone(),
            item.subtitle.clone(),
            labels.get(item.origin.source).copied().unwrap_or(""),
            title_font.clone(),
            subtitle_font.clone(),
            badge_font.clone(),
            title_color,
            subtitle_color,
        );
        let row = layout.rect(index, width);
        canvas.save();
        canvas.translate((row.left, row.top));
        draw(canvas, row.width(), row.height());
        canvas.restore();
    }
}

/// The hairline between the field and the rows.
pub fn divider_color(dark: bool) -> Color {
    if dark {
        Color::from_argb(36, 255, 255, 255)
    } else {
        Color::from_argb(24, 0, 0, 0)
    }
}

/// The selected row's wash, drawn under it.
pub fn row_highlight_color(dark: bool) -> Color {
    if dark {
        Color::from_argb(46, 255, 255, 255)
    } else {
        Color::from_argb(20, 0, 0, 0)
    }
}

/// Where the highlight goes for row `index` of a list `width` wide, in the
/// list's content coordinates.
pub fn row_highlight_rect(index: usize, width: f32) -> Rect {
    Rect::from_xywh(
        ROW_INSET,
        index as f32 * ROW_H + 2.0,
        width - ROW_INSET * 2.0,
        ROW_H - 4.0,
    )
}

/// The colour of a row's title.
pub fn row_title_color(dark: bool) -> Color {
    if dark {
        Color::from_argb(240, 255, 255, 255)
    } else {
        Color::from_argb(240, 12, 12, 14)
    }
}

/// The colour of a row's second line, its badge, and a list's message.
pub fn row_subtitle_color(dark: bool) -> Color {
    if dark {
        Color::from_argb(150, 255, 255, 255)
    } else {
        Color::from_argb(140, 0, 0, 0)
    }
}

/// The body face at `size`, as rows set their text.
pub fn row_font(size: f32) -> Font {
    get_font_with_fallback(styles::BODY.family, FontStyle::normal(), size)
}

/// A checkbox in a row's icon place: an outlined box, or one filled with the
/// accent and ticked.
#[derive(Clone, Copy)]
struct Check {
    checked: bool,
    accent: Color,
    outline: Color,
}

impl Check {
    /// Side of the box.
    const SIZE: f32 = 16.0;

    fn draw(&self, canvas: &Canvas, centre: (f32, f32)) {
        let half = Self::SIZE / 2.0;
        let rect = Rect::from_xywh(centre.0 - half, centre.1 - half, Self::SIZE, Self::SIZE);
        let radius = 4.0;
        if !self.checked {
            let mut paint = Paint::new(Color4f::from(self.outline), None);
            paint.set_anti_alias(true);
            paint.set_style(skia::PaintStyle::Stroke);
            paint.set_stroke_width(1.5);
            canvas.draw_round_rect(rect.with_inset((0.75, 0.75)), radius, radius, &paint);
            return;
        }
        let mut fill = Paint::new(Color4f::from(self.accent), None);
        fill.set_anti_alias(true);
        canvas.draw_round_rect(rect, radius, radius, &fill);
        let mut tick = skia::PathBuilder::new();
        tick.move_to((centre.0 - 4.0, centre.1 + 0.2));
        tick.line_to((centre.0 - 1.2, centre.1 + 3.0));
        tick.line_to((centre.0 + 4.2, centre.1 - 3.2));
        let mut paint = Paint::new(Color4f::from(Color::WHITE), None);
        paint.set_anti_alias(true);
        paint.set_style(skia::PaintStyle::Stroke);
        paint.set_stroke_width(2.0);
        paint.set_stroke_cap(skia::paint::Cap::Round);
        paint.set_stroke_join(skia::paint::Join::Round);
        canvas.draw_path(&tick.detach(), &paint);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    icon: Option<Image>,
    dot: Option<Color>,
    check: Option<Check>,
    title: String,
    subtitle: Option<String>,
    badge: &'static str,
    title_font: Font,
    subtitle_font: Font,
    badge_font: Font,
    title_color: Color,
    subtitle_color: Color,
) -> impl Fn(&Canvas, f32, f32) -> Rect + Send + Sync {
    move |canvas, width, height| {
        let mut paint = Paint::new(Color4f::from(title_color), None);
        paint.set_anti_alias(true);

        if let Some(image) = &icon {
            let top = (height - ICON) / 2.0;
            let left = ROW_INSET + 8.0;
            canvas.draw_image_rect_with_sampling_options(
                image,
                None,
                Rect::from_xywh(left, top, ICON, ICON),
                SamplingOptions::default(),
                &paint,
            );
        }

        if let Some(color) = dot {
            // In the icon's place, centred where a full-size icon would be.
            let centre = (ROW_INSET + 8.0 + ICON / 2.0, height / 2.0);
            let mut dot_paint = Paint::new(Color4f::from(color), None);
            dot_paint.set_anti_alias(true);
            canvas.draw_circle(centre, DOT_RADIUS, &dot_paint);
        }

        if let Some(check) = check {
            check.draw(canvas, (ROW_INSET + 8.0 + ICON / 2.0, height / 2.0));
        }

        // The badge is measured first: the title is clipped to what is left,
        // so a long window title cannot run underneath it.
        let mut badge_paint = Paint::new(Color4f::from(subtitle_color), None);
        badge_paint.set_anti_alias(true);
        let badge_width = if badge.is_empty() {
            0.0
        } else {
            badge_font.measure_str(badge, Some(&badge_paint)).0
        };
        if !badge.is_empty() {
            canvas.draw_str(
                badge,
                (width - ROW_INSET - 8.0 - badge_width, height / 2.0 + 4.0),
                &badge_font,
                &badge_paint,
            );
        }

        let text_x = ROW_INSET + 8.0 + ICON + 12.0;
        let text_width = (width - ROW_INSET - 20.0 - badge_width - text_x).max(0.0);
        canvas.save();
        canvas.clip_rect(
            Rect::from_xywh(text_x, 0.0, text_width, height),
            None,
            Some(true),
        );

        match &subtitle {
            Some(subtitle) if !subtitle.is_empty() => {
                canvas.draw_str(&title, (text_x, height / 2.0 - 1.0), &title_font, &paint);
                let mut sub = Paint::new(Color4f::from(subtitle_color), None);
                sub.set_anti_alias(true);
                canvas.draw_str(
                    subtitle,
                    (text_x, height / 2.0 + 14.0),
                    &subtitle_font,
                    &sub,
                );
            }
            _ => {
                canvas.draw_str(&title, (text_x, height / 2.0 + 5.0), &title_font, &paint);
            }
        }
        canvas.restore();

        Rect::from_wh(width, height)
    }
}

/// Decode an icon once and keep it. Misses are remembered too — an app whose
/// icon the theme does not have must not be looked up again on every keystroke.
fn resolve_icon(cache: &mut HashMap<String, Option<Image>>, name: &str) -> Option<Image> {
    cache
        .entry(name.to_string())
        .or_insert_with(|| named_icon_sized(name, (ICON * 2.0) as i32))
        .clone()
}

/// The query field's look: no box of its own, because it already sits in one.
pub fn field_style(dark: bool) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(if dark { Theme::dark() } else { Theme::light() });
    style.text_style = styles::TITLE_3;
    style.text_style.size = 19.0;
    style.horizontal_padding = 20.0;
    style.corner_radius = 0.0;
    style.focus_ring_width = 0.0;
    style.background = Color::TRANSPARENT;
    style.text_color = if dark {
        Color::from_argb(245, 255, 255, 255)
    } else {
        Color::from_argb(245, 10, 10, 12)
    };
    style.placeholder_color = if dark {
        Color::from_argb(110, 255, 255, 255)
    } else {
        Color::from_argb(100, 0, 0, 0)
    };
    style
}
