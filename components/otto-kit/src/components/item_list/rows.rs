//! The rows of a list and the search field over it, as the launcher and the
//! side canvas paint them.
//!
//! A row is an icon, an activity dot or a checkbox, then a title with an
//! optional second line, and the name of its source at the end. Everything
//! here paints with Skia onto a canvas the host provides, so the same rows
//! read the same wherever they are listed.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::components::scroll::RowLayout;
use crate::components::text_input::TextInputStyle;
use crate::icons::named_icon_sized;
use crate::skia::{
    self, Canvas, Color, Color4f, Font, FontStyle, Image, Paint, Rect, SamplingOptions,
};
use crate::theme::Theme;
use crate::typography::{get_font_with_fallback, styles};

use super::item::{Activity, Item};

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
/// Room either side of a pill's word, and its height — the mode pill's in the
/// ask log.
const PILL_PAD_X: f32 = 7.0;
const PILL_H: f32 = 16.0;
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
    paint_item_rows_styled(
        canvas,
        band,
        items,
        labels,
        width,
        dark,
        icons,
        &RowStyle::default(),
    );
}

/// Draws a row's icon: given the item's index, the centre of the icon's
/// place, its side, and the colour the row's text is in.
pub type RowIconPainter<'a> = &'a dyn Fn(&Canvas, usize, (f32, f32), f32, Color);

/// What a list asks of its rows beyond what the items themselves carry.
///
/// The default is the launcher's look: the host paints the selection under the
/// rows itself (the launcher slides one highlight between them), and nothing in
/// a title is marked.
#[derive(Default)]
pub struct RowStyle<'a> {
    /// The row painted as selected, filled with `selection` and its text and
    /// icon drawn in `on_selection`. With no `selection` the host is drawing
    /// the highlight itself, and the row is painted as any other.
    pub selected: Option<usize>,
    pub selection: Option<Color>,
    pub on_selection: Option<Color>,
    /// What was typed. The characters of each title it matched are drawn in
    /// `mark`, so a list says *why* a row is there — except on the selected
    /// row, whose text is all `on_selection`.
    pub query: &'a str,
    pub mark: Option<Color>,
    /// Draws a row's icon in place of the icon theme's, for a list whose
    /// things are not applications — the settings search marks a row with its
    /// pane's glyph.
    pub icon: Option<RowIconPainter<'a>>,
}

/// [`paint_item_rows`], with a list's own [`RowStyle`].
#[allow(clippy::too_many_arguments)]
pub fn paint_item_rows_styled(
    canvas: &Canvas,
    band: Rect,
    items: &[&Item],
    labels: &[&'static str],
    width: f32,
    dark: bool,
    icons: &RowIcons,
    style: &RowStyle,
) {
    let fonts = RowFonts {
        title: row_font(15.0),
        subtitle: row_font(11.5),
        badge: row_font(10.5),
    };
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let layout = RowLayout::new(ROW_H, items.len());
    for index in layout.visible(band) {
        let item = items[index];
        let selected = style.selection.is_some() && style.selected == Some(index);
        let (title_color, subtitle_color) = match (selected, style.on_selection) {
            (true, Some(on)) => (on, on.with_a(200)),
            _ => (row_title_color(dark), row_subtitle_color(dark)),
        };
        let row = layout.rect(index, width);
        canvas.save();
        canvas.translate((row.left, row.top));

        if let (true, Some(fill)) = (selected, style.selection) {
            let mut paint = Paint::new(Color4f::from(fill), None);
            paint.set_anti_alias(true);
            let highlight = row_highlight_rect(0, row.width());
            canvas.draw_round_rect(highlight, HIGHLIGHT_RADIUS, HIGHLIGHT_RADIUS, &paint);
        }

        let centre = (ROW_INSET + 8.0 + ICON / 2.0, row.height() / 2.0);
        match style.icon {
            Some(draw) => draw(canvas, index, centre, ICON, title_color),
            None => {
                let icon = item
                    .icon
                    .as_deref()
                    .and_then(|name| resolve_icon(&mut icons.0.borrow_mut(), name));
                if let Some(image) = &icon {
                    let mut paint = Paint::new(Color4f::from(title_color), None);
                    paint.set_anti_alias(true);
                    canvas.draw_image_rect_with_sampling_options(
                        image,
                        None,
                        Rect::from_xywh(centre.0 - ICON / 2.0, centre.1 - ICON / 2.0, ICON, ICON),
                        SamplingOptions::default(),
                        &paint,
                    );
                }
            }
        }
        if let Some(activity) = item.activity {
            // In the icon's place, centred where a full-size icon would be.
            let color = match activity {
                Activity::Working => theme.accent,
                Activity::Idle => theme.text_tertiary,
                Activity::Waiting => theme.accent_yellow,
            };
            let mut dot = Paint::new(Color4f::from(color), None);
            dot.set_anti_alias(true);
            canvas.draw_circle(centre, DOT_RADIUS, &dot);
        }
        if let Some(checked) = item.checked {
            Check {
                checked,
                accent: theme.accent,
                outline: theme.text_tertiary,
            }
            .draw(canvas, centre);
        }

        // Marks only where they would show: on the selected row every
        // character is already the selection's colour. The words a title
        // starts with are what it was found by, when it was; the letters a
        // looser match walked through, when not.
        let marked = match style.mark {
            Some(mark) if !selected && !style.query.trim().is_empty() => {
                crate::matching::word_positions(&item.title, style.query)
                    .or_else(|| crate::matching::positions(&item.title, style.query))
                    .map(|at| (at, mark))
            }
            _ => None,
        };
        paint_text(
            canvas,
            (row.width(), row.height()),
            item,
            labels.get(item.origin.source).copied().unwrap_or(""),
            theme.fill_secondary,
            &fonts,
            (title_color, subtitle_color),
            marked,
        );
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

/// The faces a row sets its text in.
struct RowFonts {
    title: Font,
    subtitle: Font,
    badge: Font,
}

/// A row's text: the badge at its end, then the title — with the characters
/// in `marked` drawn in their colour — and the second line under it.
///
/// An item with a [`Item::pill`] wears it in place of `badge`: its word in
/// the title's colour, on a pill filled with `pill_fill`.
#[allow(clippy::too_many_arguments)]
fn paint_text(
    canvas: &Canvas,
    (width, height): (f32, f32),
    item: &Item,
    badge: &str,
    pill_fill: Color,
    fonts: &RowFonts,
    (title_color, subtitle_color): (Color, Color),
    marked: Option<(Vec<usize>, Color)>,
) {
    let (badge, fill, badge_color) = match item.pill.as_deref() {
        Some(pill) => (pill, Some(pill_fill), title_color),
        None => (badge, None, subtitle_color),
    };
    // The badge is measured first: the title is clipped to what is left, so a
    // long window title cannot run underneath it.
    let mut badge_paint = Paint::new(Color4f::from(badge_color), None);
    badge_paint.set_anti_alias(true);
    // A pill's text sits inside it, with room either side.
    let padding = if fill.is_some() { PILL_PAD_X } else { 0.0 };
    let badge_width = if badge.is_empty() {
        0.0
    } else {
        fonts.badge.measure_str(badge, Some(&badge_paint)).0 + 2.0 * padding
    };
    if !badge.is_empty() {
        let left = width - ROW_INSET - 8.0 - badge_width;
        if let Some(fill) = fill {
            let mut fill = Paint::new(Color4f::from(fill), None);
            fill.set_anti_alias(true);
            let rect = Rect::from_xywh(left, (height - PILL_H) / 2.0, badge_width, PILL_H);
            canvas.draw_round_rect(rect, PILL_H / 2.0, PILL_H / 2.0, &fill);
        }
        canvas.draw_str(
            badge,
            (left + padding, height / 2.0 + 4.0),
            &fonts.badge,
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

    let subtitle = item.subtitle.as_deref().filter(|s| !s.is_empty());
    let title_y = if subtitle.is_some() {
        height / 2.0 - 1.0
    } else {
        height / 2.0 + 5.0
    };
    draw_marked(
        canvas,
        &item.title,
        (text_x, title_y),
        &fonts.title,
        title_color,
        marked,
    );
    if let Some(subtitle) = subtitle {
        let mut sub = Paint::new(Color4f::from(subtitle_color), None);
        sub.set_anti_alias(true);
        canvas.draw_str(
            subtitle,
            (text_x, height / 2.0 + 14.0),
            &fonts.subtitle,
            &sub,
        );
    }
    canvas.restore();
}

/// Draw `text` with the characters at `marked` in the mark's colour: one run
/// per stretch of the same colour, each starting where the last one ended.
fn draw_marked(
    canvas: &Canvas,
    text: &str,
    origin: (f32, f32),
    font: &Font,
    color: Color,
    marked: Option<(Vec<usize>, Color)>,
) {
    let mut plain = Paint::new(Color4f::from(color), None);
    plain.set_anti_alias(true);
    let Some((marked, mark)) = marked.filter(|(at, _)| !at.is_empty()) else {
        canvas.draw_str(text, origin, font, &plain);
        return;
    };
    let mut accent = Paint::new(Color4f::from(mark), None);
    accent.set_anti_alias(true);

    let mut x = origin.0;
    let mut run = String::new();
    let mut run_marked = false;
    let chars = text.chars().enumerate();
    for (at, c) in chars {
        let is_marked = marked.binary_search(&at).is_ok();
        if is_marked != run_marked && !run.is_empty() {
            let paint = if run_marked { &accent } else { &plain };
            canvas.draw_str(&run, (x, origin.1), font, paint);
            x += font.measure_str(&run, Some(paint)).0;
            run.clear();
        }
        run_marked = is_marked;
        run.push(c);
    }
    if !run.is_empty() {
        let paint = if run_marked { &accent } else { &plain };
        canvas.draw_str(&run, (x, origin.1), font, paint);
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
