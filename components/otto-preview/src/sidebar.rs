//! The pages sidebar: a column of page thumbnails down the window's leading
//! edge, under the toolbar, for a document of pages.
//!
//! It opens by itself for a document of more than one page and stays shut
//! for one of a single page; the toolbar's sidebar button overrides that
//! either way. Geometry and drawing live side by side, as the chrome's do,
//! so a press is tested against the rects that were painted.

// Rust guideline compliant 2026-02-21

use otto_kit::common::Renderable;
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, PaintStyle, Point, RRect};

use crate::viewer::Viewer;

/// The sidebar's width, its hairline included.
pub const WIDTH: f32 = 148.0;
/// A thumbnail's width; its height follows the page.
pub const THUMB_W: f32 = 96.0;
/// Above the first thumbnail and below the last.
const MARGIN: f32 = 14.0;
/// Under a thumbnail, holding its page number.
const LABEL_H: f32 = 22.0;
/// Between one thumbnail's label and the next thumbnail.
const GAP: f32 = 10.0;
/// The ring around the page showing, outside the thumbnail.
const RING: f32 = 3.0;

/// Where every thumbnail sits, in window coordinates, scrolled.
#[derive(Debug, Clone)]
pub struct SidebarLayout {
    /// The sidebar's whole box.
    pub rect: Rect,
    /// Each page's thumbnail, 1-based page first.
    pub thumbs: Vec<(u32, Rect)>,
    /// The column's whole height, thumbnails and margins.
    pub column: f32,
    /// How far the column may scroll: its height past the box's.
    pub max_scroll: f32,
}

impl SidebarLayout {
    /// The page whose thumbnail, or its label, is under a window-local point.
    pub fn page_at(&self, at: Point) -> Option<u32> {
        if !self.rect.contains(at) {
            return None;
        }
        self.thumbs
            .iter()
            .find(|(_, rect)| {
                Rect::from_ltrb(rect.left, rect.top, rect.right, rect.bottom + LABEL_H)
                    .contains(at)
            })
            .map(|(page, _)| *page)
    }

    /// The pages whose thumbnails are in the box, or near enough to it to be
    /// worth having ready.
    pub fn pages_in_view(&self, ahead: usize) -> std::ops::RangeInclusive<u32> {
        let visible: Vec<u32> = self
            .thumbs
            .iter()
            .filter(|(_, rect)| rect.bottom + LABEL_H >= self.rect.top && rect.top <= self.rect.bottom)
            .map(|(page, _)| *page)
            .collect();
        let first = visible.first().copied().unwrap_or(1);
        let last = visible.last().copied().unwrap_or(first);
        let pages = self.thumbs.len() as u32;
        first.saturating_sub(ahead as u32).max(1)..=(last + ahead as u32).min(pages.max(1))
    }

    /// The scroll that brings `page`'s thumbnail fully into the box from
    /// `scroll`, or `scroll` itself when it is already there.
    pub fn scroll_to_show(&self, page: u32, scroll: f32) -> f32 {
        let Some((_, rect)) = self.thumbs.iter().find(|(p, _)| *p == page) else {
            return scroll;
        };
        let top = rect.top - MARGIN / 2.0;
        let bottom = rect.bottom + LABEL_H;
        let target = if top < self.rect.top {
            scroll - (self.rect.top - top)
        } else if bottom > self.rect.bottom {
            scroll + (bottom - self.rect.bottom)
        } else {
            scroll
        };
        target.clamp(0.0, self.max_scroll)
    }
}

/// Lay the column out in `rect`, scrolled by `scroll`, for pages of the given
/// sizes.
pub fn layout(rect: Rect, pages: &[(f32, f32)], scroll: f32) -> SidebarLayout {
    let left = rect.left + (rect.width() - THUMB_W) / 2.0;
    let mut y = MARGIN;
    let mut thumbs = Vec::with_capacity(pages.len());
    for (index, (width, height)) in pages.iter().enumerate() {
        let aspect = if *width > 0.0 { height / width } else { 1.4 };
        let thumb_h = (THUMB_W * aspect).clamp(THUMB_W * 0.25, THUMB_W * 4.0);
        let top = rect.top + y - scroll;
        thumbs.push((index as u32 + 1, Rect::from_xywh(left, top, THUMB_W, thumb_h)));
        y += thumb_h + LABEL_H + GAP;
    }
    let column = y - GAP + MARGIN;
    SidebarLayout {
        rect,
        thumbs,
        column,
        max_scroll: (column - rect.height()).max(0.0),
    }
}

/// Paint the sidebar, when it is open.
pub fn draw(canvas: &Canvas, viewer: &Viewer, theme: &Theme) {
    let Some(layout) = viewer.sidebar() else {
        return;
    };
    let rect = layout.rect;
    let showing = viewer.page_status().map(|(page, _)| page);

    canvas.save();
    canvas.clip_rect(rect, None, true);

    let mut ground = Paint::default();
    ground.set_color(theme.fill_quaternary);
    canvas.draw_rect(rect, &ground);

    let mut paper = Paint::default();
    paper.set_anti_alias(true);
    paper.set_color(Color::WHITE);
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(theme.fill_tertiary);
    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_style(PaintStyle::Stroke);
    ring.set_stroke_width(2.0);
    ring.set_color(theme.accent);

    for (page, thumb) in &layout.thumbs {
        if thumb.bottom + LABEL_H < rect.top || thumb.top > rect.bottom {
            continue;
        }
        let current = showing == Some(*page);
        match viewer.thumbs.get(page) {
            Some(image) => {
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    *thumb,
                    otto_kit::skia::SamplingOptions::from(
                        otto_kit::skia::CubicResampler::mitchell(),
                    ),
                    &Paint::default(),
                );
            }
            None => {
                canvas.draw_rect(*thumb, &paper);
            }
        }
        canvas.draw_rect(*thumb, &edge);
        if current {
            let around = thumb.with_outset((RING, RING));
            canvas.draw_rrect(RRect::new_rect_xy(around, 4.0, 4.0), &ring);
        }
        Label::new(page.to_string())
            .with_style(styles::CAPTION_1)
            .with_color(if current {
                theme.text_primary
            } else {
                theme.text_secondary
            })
            .centered_at(thumb.center_x(), thumb.bottom + LABEL_H / 2.0 + 1.0)
            .render(canvas);
    }
    // The column's scrollbar, faded in and out like every other one.
    viewer.sidebar_scroll.render(canvas, theme, |_, _| {});
    canvas.restore();

    // The hairline between the column and the document.
    let mut line = Paint::default();
    line.set_color(theme.fill_tertiary);
    canvas.draw_rect(
        Rect::from_ltrb(rect.right - 1.0, rect.top, rect.right, rect.bottom),
        &line,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(pages: usize) -> Vec<(f32, f32)> {
        vec![(612.0, 792.0); pages]
    }

    #[test]
    fn a_short_document_does_not_scroll() {
        let layout = layout(Rect::from_xywh(0.0, 80.0, WIDTH, 600.0), &letter(2), 0.0);
        assert_eq!(layout.max_scroll, 0.0);
        assert_eq!(layout.thumbs.len(), 2);
    }

    #[test]
    fn a_press_on_a_thumbnail_names_its_page() {
        let layout = layout(Rect::from_xywh(0.0, 80.0, WIDTH, 600.0), &letter(3), 0.0);
        let (_, second) = layout.thumbs[1];
        let at = Point::new(second.center_x(), second.center_y());
        assert_eq!(layout.page_at(at), Some(2));
        assert_eq!(layout.page_at(Point::new(WIDTH + 10.0, second.center_y())), None);
    }

    #[test]
    fn a_page_below_the_box_is_scrolled_up_into_it() {
        let rect = Rect::from_xywh(0.0, 80.0, WIDTH, 300.0);
        let layout = layout(rect, &letter(20), 0.0);
        let scroll = layout.scroll_to_show(10, 0.0);
        assert!(scroll > 0.0);
        let moved = super::layout(rect, &letter(20), scroll);
        let (_, thumb) = moved.thumbs[9];
        assert!(thumb.top >= rect.top && thumb.bottom + LABEL_H <= rect.bottom + 0.5);
        // Already showing: nothing moves.
        assert_eq!(moved.scroll_to_show(10, scroll), scroll);
    }
}
