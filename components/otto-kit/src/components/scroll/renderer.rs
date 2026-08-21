use skia_safe::{Canvas, Color4f, Contains, Paint, Point, RRect, Rect};

use crate::theme::Theme;

use super::state::{Axis, ScrollState};

/// Scrollbar thumb width while it is a passive indicator, in points.
const THUMB_WIDTH: f32 = 6.0;
/// Thumb width once the pointer is on it and it is meant to be grabbed.
const THUMB_WIDTH_EXPANDED: f32 = 10.0;
/// Gap between the thumb and the viewport's trailing edge.
const THUMB_MARGIN: f32 = 3.0;
/// A thumb never shrinks below this, or a very tall pane would leave nothing
/// to grab.
const THUMB_MIN_LENGTH: f32 = 24.0;
/// Width of the strip along the trailing edge that counts as "on the
/// scrollbar" — wider than the thin thumb, so a thin bar is still easy to
/// hover and grab.
const GUTTER_WIDTH: f32 = 14.0;
/// How much of the track shows through behind the thumb.
const TRACK_ALPHA: f32 = 0.6;

/// How much stronger the thumb is than the fill token it is derived from.
/// `fill_secondary` is tuned for large flat areas; at a 6pt handle the same
/// alpha reads as a smudge rather than something you could grab.
const THUMB_ALPHA_BOOST: f32 = 1.9;
/// The thumb squashes against an end while the content is rubber-banded past
/// it, but never below this fraction of its length.
const SQUASH_FLOOR: f32 = 0.4;

/// Stateless drawing and geometry for a [`ScrollState`] — the same shape as
/// [`TextInputRenderer`](crate::components::text_input::TextInputRenderer):
/// every function is a free function over `(state, ..)` so a caller can
/// hit-test without owning a [`ScrollView`](super::ScrollView).
pub struct ScrollRenderer;

impl ScrollRenderer {
    /// The scrollbar only exists once content overflows the viewport.
    pub fn scrollbar_visible(state: &ScrollState) -> bool {
        state.scrollable()
    }

    /// Scrollbar thumb rect, in the same (canvas-local) space as the
    /// viewport — `None` when there is nothing to scroll.
    ///
    /// Thumb length is proportional to how much of the content the viewport
    /// shows; its position is proportional to how far through the content
    /// the offset is. Its thickness follows
    /// [`ScrollState::scrollbar_expansion`], and while the content is
    /// rubber-banded past an end the thumb squashes against that end by the
    /// overscroll distance — the same cue the content itself is giving.
    /// [`Self::draw`] and [`Self::hit_test_thumb`] both read this so the
    /// painted thumb and the grabbable area cannot drift apart.
    ///
    /// A horizontal state lays the same thumb along the viewport's bottom
    /// edge instead of its right one.
    pub fn thumb_rect(state: &ScrollState) -> Option<Rect> {
        if !Self::scrollbar_visible(state) {
            return None;
        }
        let viewport = state.viewport();
        let track_len = state.viewport_length();
        let content_length = state.content_length().max(f32::EPSILON);
        let ratio = (track_len / content_length).clamp(0.0, 1.0);
        let thumb_len = (track_len * ratio).clamp(THUMB_MIN_LENGTH.min(track_len), track_len);
        let travel = (track_len - thumb_len).max(0.0);
        let progress = (state.offset() / state.max_offset().max(f32::EPSILON)).clamp(0.0, 1.0);

        let over = state.overscroll();
        let squashed = (thumb_len - over.abs()).max(thumb_len * SQUASH_FLOOR);
        // Distance from the track's start to the thumb's leading edge: pinned
        // to whichever end the content is hanging off, proportional otherwise.
        let start = if over < 0.0 {
            0.0
        } else if over > 0.0 {
            track_len - squashed
        } else {
            travel * progress
        };

        let thickness =
            THUMB_WIDTH + (THUMB_WIDTH_EXPANDED - THUMB_WIDTH) * state.scrollbar_expansion();
        Some(match state.axis() {
            Axis::Vertical => Rect::from_xywh(
                viewport.right - thickness - THUMB_MARGIN,
                viewport.top + start,
                thickness,
                squashed,
            ),
            Axis::Horizontal => Rect::from_xywh(
                viewport.left + start,
                viewport.bottom - thickness - THUMB_MARGIN,
                squashed,
                thickness,
            ),
        })
    }

    /// The strip along the viewport's trailing edge that the scrollbar lives
    /// in — what counts as hovering the bar. `None` when there is nothing to
    /// scroll.
    pub fn gutter_rect(state: &ScrollState) -> Option<Rect> {
        if !Self::scrollbar_visible(state) {
            return None;
        }
        let viewport = state.viewport();
        Some(match state.axis() {
            Axis::Vertical => Rect::from_ltrb(
                viewport.right - GUTTER_WIDTH,
                viewport.top,
                viewport.right,
                viewport.bottom,
            ),
            Axis::Horizontal => Rect::from_ltrb(
                viewport.left,
                viewport.bottom - GUTTER_WIDTH,
                viewport.right,
                viewport.bottom,
            ),
        })
    }

    /// Is `(x, y)`, in canvas-local space, in the scrollbar's gutter?
    pub fn hit_test_gutter(state: &ScrollState, x: f32, y: f32) -> bool {
        Self::gutter_rect(state).is_some_and(|rect| rect.contains(Point::new(x, y)))
    }

    /// Is `(x, y)`, in canvas-local space, over the scrollbar thumb? The
    /// thumb's rows count across the whole gutter, so a thin, un-expanded bar
    /// is still grabbable without pixel-hunting.
    pub fn hit_test_thumb(state: &ScrollState, x: f32, y: f32) -> bool {
        let (Some(thumb), Some(gutter)) = (Self::thumb_rect(state), Self::gutter_rect(state))
        else {
            return false;
        };
        // The thumb's own extent along the scrolling axis, widened to the
        // whole gutter across it.
        let grab = match state.axis() {
            Axis::Vertical => Rect::from_ltrb(
                gutter.left.min(thumb.left),
                thumb.top,
                gutter.right,
                thumb.bottom,
            ),
            Axis::Horizontal => Rect::from_ltrb(
                thumb.left,
                gutter.top.min(thumb.top),
                thumb.right,
                gutter.bottom,
            ),
        };
        grab.contains(Point::new(x, y))
    }

    /// Map a point in viewport (canvas-local) space into content-local
    /// space, so a caller can hit-test its own content while scrolled.
    pub fn viewport_to_content(state: &ScrollState, x: f32, y: f32) -> (f32, f32) {
        let viewport = state.viewport();
        let (x, y) = (x - viewport.left, y - viewport.top);
        match state.axis() {
            Axis::Vertical => (x, y + state.offset()),
            Axis::Horizontal => (x + state.offset(), y),
        }
    }

    /// The band of content the viewport is showing, in content-local
    /// coordinates — what [`Self::draw`] hands its content closure.
    pub fn visible_content_rect(state: &ScrollState) -> Rect {
        let viewport = state.viewport();
        let (left, top) = match state.axis() {
            Axis::Vertical => (0.0, state.offset()),
            Axis::Horizontal => (state.offset(), 0.0),
        };
        Rect::from_xywh(left, top, viewport.width(), viewport.height())
    }

    /// Clip to the viewport, translate by the scroll offset, let `content`
    /// paint at content-local coordinates (origin at the content's top-left,
    /// x and y increasing right and down), then draw the scrollbar over the
    /// result.
    ///
    /// `content` is handed the region it is being asked for, in that same
    /// content-local space — the band the viewport is currently showing,
    /// which [`Self::visible_content_rect`] offsets along the scrolling
    /// axis. It is a
    /// *permission to skip*, not an obligation: the clip and the translation
    /// are still in force, so a closure that ignores the rect and paints the
    /// whole content renders exactly as it did before — it just pays for draw
    /// ops that are thrown away. A closure that honours it may omit anything
    /// lying entirely outside the rect, and must not use it as a clip
    /// substitute: nothing guarantees the rect is pixel-aligned with the
    /// viewport, and while the content is rubber-banded past an end the band
    /// runs outside `0 .. content_length`.
    ///
    /// The scrollbar is an overlay: it is painted at
    /// [`ScrollState::scrollbar_opacity`], which a
    /// [`ScrollView`](super::ScrollView) fades in while scrolling and out
    /// again when idle. A caller that never animates the state gets a state
    /// whose opacity is whatever it set — `0` by default, so a plain
    /// `ScrollState` draws no bar.
    pub fn draw(
        canvas: &Canvas,
        state: &ScrollState,
        theme: &Theme,
        content: impl FnOnce(&Canvas, Rect),
    ) {
        let viewport = state.viewport();
        let visible = Self::visible_content_rect(state);

        canvas.save();
        canvas.clip_rect(viewport, None, Some(true));
        canvas.translate((viewport.left - visible.left, viewport.top - visible.top));
        content(canvas, visible);
        canvas.restore();

        let opacity = state.scrollbar_opacity();
        if opacity <= 0.0 {
            return;
        }
        let Some(thumb) = Self::thumb_rect(state) else {
            return;
        };
        let radius = thumb.width().min(thumb.height()) / 2.0;

        // A faint track the full length of the viewport, so the thumb reads
        // as sitting in a channel rather than floating.
        let track = match state.axis() {
            Axis::Vertical => {
                Rect::from_xywh(thumb.left, viewport.top, thumb.width(), viewport.height())
            }
            Axis::Horizontal => {
                Rect::from_xywh(viewport.left, thumb.top, viewport.width(), thumb.height())
            }
        };
        let mut track_paint = Paint::default();
        track_paint.set_anti_alias(true);
        track_paint.set_color4f(faded(theme.fill_tertiary, opacity * TRACK_ALPHA), None);
        canvas.draw_rrect(RRect::new_rect_xy(track, radius, radius), &track_paint);

        let mut thumb_paint = Paint::default();
        thumb_paint.set_anti_alias(true);
        let mut thumb_color = faded(theme.fill_secondary, opacity);
        thumb_color.a = (thumb_color.a * THUMB_ALPHA_BOOST).min(opacity);
        thumb_paint.set_color4f(thumb_color, None);
        canvas.draw_rrect(RRect::new_rect_xy(thumb, radius, radius), &thumb_paint);
    }
}

/// A theme colour scaled by a fade factor, keeping whatever alpha the theme
/// already gave it.
fn faded(color: skia_safe::Color, fade: f32) -> Color4f {
    let mut c = Color4f::from(color);
    c.a *= fade.clamp(0.0, 1.0);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn horizontal(content_length: f32) -> ScrollState {
        let mut s = ScrollState::on_axis(Axis::Horizontal, Rect::from_xywh(0.0, 0.0, 300.0, 200.0));
        s.set_content_length(content_length);
        s
    }

    #[test]
    fn a_horizontal_thumb_runs_along_the_bottom_edge() {
        let mut s = horizontal(900.0);
        let at_start = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(at_start.left, 0.0);
        assert_eq!(at_start.bottom, 200.0 - THUMB_MARGIN);
        assert_eq!(at_start.height(), THUMB_WIDTH);
        // Viewport is a third of the content, so is the thumb.
        assert!(
            (at_start.width() - 100.0).abs() < 1.0,
            "{}",
            at_start.width()
        );

        s.set_offset(s.max_offset());
        let at_end = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(at_end.right, 300.0);
        assert!(at_end.left > at_start.left);
    }

    #[test]
    fn a_horizontal_gutter_is_the_bottom_strip() {
        let s = horizontal(900.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        assert!(ScrollRenderer::hit_test_gutter(&s, 10.0, 195.0));
        assert!(!ScrollRenderer::hit_test_gutter(&s, 10.0, 100.0));
        // Anywhere in the gutter on the thumb's columns grabs it, even above
        // the thin bar itself.
        assert!(ScrollRenderer::hit_test_thumb(&s, thumb.center_x(), 190.0));
        assert!(!ScrollRenderer::hit_test_thumb(&s, 290.0, 190.0));
    }

    #[test]
    fn a_horizontal_view_scrolls_content_sideways() {
        let mut s =
            ScrollState::on_axis(Axis::Horizontal, Rect::from_xywh(20.0, 40.0, 300.0, 200.0));
        s.set_content_length(1000.0);
        s.set_offset(60.0);
        assert_eq!(
            ScrollRenderer::visible_content_rect(&s),
            Rect::from_ltrb(60.0, 0.0, 360.0, 200.0)
        );
        assert_eq!(
            ScrollRenderer::viewport_to_content(&s, 25.0, 55.0),
            (65.0, 15.0)
        );
    }

    #[test]
    fn a_horizontal_thumb_squashes_against_the_end_it_is_pulled_past() {
        let mut s = horizontal(900.0);
        let resting = ScrollRenderer::thumb_rect(&s).unwrap();

        s.set_offset_overscrolled(-30.0);
        let pulled = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(pulled.left, 0.0);
        assert!(pulled.width() < resting.width());

        s.set_offset_overscrolled(s.max_offset() + 30.0);
        let pushed = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(pushed.right, 300.0);
        assert!(pushed.width() < resting.width());
    }

    fn state(content_length: f32) -> ScrollState {
        let mut s = ScrollState::new(Rect::from_xywh(0.0, 0.0, 300.0, 200.0));
        s.set_content_length(content_length);
        s
    }

    #[test]
    fn no_thumb_when_content_fits() {
        assert!(ScrollRenderer::thumb_rect(&state(100.0)).is_none());
        assert!(!ScrollRenderer::scrollbar_visible(&state(100.0)));
    }

    #[test]
    fn thumb_tracks_offset_from_top_to_bottom() {
        let mut s = state(1000.0);
        let at_top = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(at_top.top, 0.0);

        s.set_offset(s.max_offset());
        let at_bottom = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(at_bottom.bottom, 200.0);
        assert!(at_bottom.top > at_top.top);
    }

    #[test]
    fn thumb_length_matches_viewport_content_ratio() {
        // Viewport is half the content, so the thumb should be roughly half
        // the track, comfortably clear of the minimum-length floor.
        let s = state(400.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        assert!((thumb.height() - 100.0).abs() < 1.0, "{}", thumb.height());
    }

    #[test]
    fn very_tall_content_still_leaves_a_grabbable_thumb() {
        let s = state(100_000.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(thumb.height(), THUMB_MIN_LENGTH);
    }

    #[test]
    fn hit_test_matches_the_painted_thumb() {
        let s = state(1000.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        assert!(ScrollRenderer::hit_test_thumb(
            &s,
            thumb.center_x(),
            thumb.center_y()
        ));
        assert!(!ScrollRenderer::hit_test_thumb(&s, 0.0, 0.0));
    }

    #[test]
    fn viewport_to_content_accounts_for_offset() {
        let mut s = ScrollState::new(Rect::from_xywh(20.0, 40.0, 300.0, 200.0));
        s.set_content_length(1000.0);
        s.set_offset(60.0);
        assert_eq!(
            ScrollRenderer::viewport_to_content(&s, 20.0, 40.0),
            (0.0, 60.0)
        );
        assert_eq!(
            ScrollRenderer::viewport_to_content(&s, 25.0, 55.0),
            (5.0, 75.0)
        );
    }

    #[test]
    fn the_thumb_squashes_against_the_end_it_is_pulled_past() {
        let mut s = state(400.0);
        let resting = ScrollRenderer::thumb_rect(&s).unwrap();

        s.set_offset_overscrolled(-30.0);
        let pulled = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(pulled.top, 0.0, "stays pinned to the top edge");
        assert!(pulled.height() < resting.height());

        s.set_offset_overscrolled(s.max_offset() + 30.0);
        let pushed = ScrollRenderer::thumb_rect(&s).unwrap();
        assert_eq!(pushed.bottom, 200.0);
        assert!(pushed.height() < resting.height());
    }

    #[test]
    fn a_huge_overscroll_leaves_a_visible_thumb() {
        let mut s = state(400.0);
        s.set_offset_overscrolled(-5000.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        assert!(thumb.height() > 0.0);
    }

    #[test]
    fn the_thumb_widens_with_expansion() {
        let mut s = state(1000.0);
        let thin = ScrollRenderer::thumb_rect(&s).unwrap();
        s.set_scrollbar_expansion(1.0);
        let wide = ScrollRenderer::thumb_rect(&s).unwrap();
        assert!(wide.width() > thin.width());
        // Both hug the same trailing edge; only the left edge moves.
        assert_eq!(thin.right, 300.0 - THUMB_MARGIN);
        assert_eq!(wide.right, 300.0 - THUMB_MARGIN);
    }

    #[test]
    fn the_content_closure_is_handed_the_scrolled_band() {
        let mut s = ScrollState::new(Rect::from_xywh(20.0, 40.0, 300.0, 200.0));
        s.set_content_length(1000.0);
        s.set_offset(120.0);

        let mut surface = skia_safe::surfaces::raster_n32_premul((400, 400)).unwrap();
        let mut asked = Rect::new_empty();
        ScrollRenderer::draw(surface.canvas(), &s, &Theme::light(), |_, rect| {
            asked = rect
        });

        assert_eq!(asked, Rect::from_ltrb(0.0, 120.0, 300.0, 320.0));
    }

    #[test]
    fn a_closure_that_ignores_the_band_still_draws_clipped_and_translated() {
        let mut s = ScrollState::new(Rect::from_xywh(0.0, 0.0, 100.0, 100.0));
        s.set_content_length(400.0);
        s.set_offset(100.0);

        let mut surface = skia_safe::surfaces::raster_n32_premul((200, 200)).unwrap();
        surface.canvas().clear(skia_safe::Color::BLACK);
        ScrollRenderer::draw(surface.canvas(), &s, &Theme::light(), |canvas, _| {
            let mut paint = Paint::default();
            paint.set_color(skia_safe::Color::WHITE);
            canvas.draw_rect(Rect::from_xywh(0.0, 0.0, 100.0, 400.0), &paint);
        });

        let mut read = |x, y| {
            let mut pixel = [0u8; 4];
            let info = skia_safe::ImageInfo::new(
                (1, 1),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
                None,
            );
            assert!(surface.read_pixels(&info, &mut pixel, 4, (x, y)));
            pixel[0]
        };
        // The second hundred points of the content are what the viewport
        // shows, and nothing escapes the viewport.
        assert_eq!(read(50, 50), 0xFF);
        assert_eq!(read(50, 150), 0x00);
    }

    #[test]
    fn the_gutter_is_grabbable_even_where_the_thumb_is_thin() {
        let s = state(1000.0);
        let thumb = ScrollRenderer::thumb_rect(&s).unwrap();
        // A point in the gutter, left of the thin thumb, on its rows.
        assert!(ScrollRenderer::hit_test_thumb(&s, 288.0, thumb.center_y()));
        assert!(ScrollRenderer::hit_test_gutter(&s, 288.0, 10.0));
        assert!(!ScrollRenderer::hit_test_gutter(&s, 100.0, 10.0));
    }
}
