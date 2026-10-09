use skia_safe::{Canvas, Paint, RRect, Rect};

use crate::components::label::TextAlign;

use super::state::TextInputState;
use super::style::TextInputStyle;

/// Character used to mask the value in password mode.
const MASK_CHAR: char = '•';

/// The dictation equaliser, in ems of the field's font: the whole row of
/// bars, one bar, and the space before it.
const BARS_WIDTH_EM: f32 = 0.9;
const BAR_WIDTH_EM: f32 = 0.12;
const BARS_GAP_EM: f32 = 0.15;

/// Stateless drawing and geometry for a text input.
///
/// Everything here is a free function over `(state, style)` so consumers can
/// hit-test without owning a widget instance — the same shape as
/// [`ContextMenuRenderer`](crate::components::context_menu::ContextMenuRenderer).
pub struct TextInputRenderer;

impl TextInputRenderer {
    /// The string actually drawn: the value, the mask in password mode, or the
    /// placeholder when the value is empty.
    pub fn display_text(state: &TextInputState) -> String {
        if Self::shows_placeholder(state) {
            return state.placeholder.clone();
        }
        if state.password {
            return MASK_CHAR.to_string().repeat(state.value().chars().count());
        }
        state.value().to_string()
    }

    /// Whether the placeholder is drawn: the value is empty and nothing is
    /// being dictated into it.
    fn shows_placeholder(state: &TextInputState) -> bool {
        state.is_empty() && state.dictation.is_none()
    }

    /// Width of what dictation draws at the caret: the pending words, then
    /// the equaliser. Zero when not dictating.
    fn mark_width(state: &TextInputState, style: &TextInputStyle) -> f32 {
        let Some(mark) = &state.dictation else {
            return 0.0;
        };
        let em = style.font().size();
        Self::measure(style, &mark.pending) + em * (BARS_GAP_EM + BARS_WIDTH_EM)
    }

    /// Draw the pending words dimmed at `x`, then the equaliser after them,
    /// as tall as the line.
    fn draw_mark(
        canvas: &Canvas,
        mark: &super::state::DictationMark,
        style: &TextInputStyle,
        font: &skia_safe::Font,
        x: f32,
        baseline: f32,
        metrics: &skia_safe::FontMetrics,
    ) {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(style.placeholder_color);
        canvas.draw_str(&mark.pending, (x, baseline), font, &paint);

        let em = font.size();
        let count = mark.levels.len().max(1) as f32;
        let bar_w = em * BAR_WIDTH_EM;
        let left = x + Self::measure(style, &mark.pending) + em * BARS_GAP_EM;
        let step = (em * BARS_WIDTH_EM - bar_w) / (count - 1.0).max(1.0);
        let top = baseline + metrics.ascent;
        let bottom = baseline + metrics.descent;
        let middle = (top + bottom) / 2.0;
        let tallest = bottom - top;
        paint.set_color(style.caret_color);
        for (i, level) in mark.levels.iter().enumerate() {
            let h = (tallest * level.clamp(0.0, 1.0)).max(bar_w);
            let rect = Rect::from_xywh(left + step * i as f32, middle - h / 2.0, bar_w, h);
            canvas.draw_rrect(RRect::new_rect_xy(rect, bar_w / 2.0, bar_w / 2.0), &paint);
        }
    }

    /// Display text of `value[..offset]` — what sits left of a caret at
    /// `offset`. Placeholders never have a caret, so this ignores them.
    fn display_prefix(state: &TextInputState, offset: usize) -> String {
        let prefix = &state.value()[..offset.min(state.value().len())];
        if state.password {
            MASK_CHAR.to_string().repeat(prefix.chars().count())
        } else {
            prefix.to_string()
        }
    }

    /// Advance width of `text` in the style's font.
    pub fn measure(style: &TextInputStyle, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        style.font().measure_str(text, None).0
    }

    /// Full width of the drawn text.
    pub fn text_width(state: &TextInputState, style: &TextInputStyle) -> f32 {
        Self::measure(style, &Self::display_text(state)) + Self::mark_width(state, style)
    }

    /// Left edge of the text run inside a box of `width`, accounting for
    /// alignment and horizontal scroll. Text wider than the box always aligns
    /// left so scrolling stays meaningful.
    pub fn text_origin_x(state: &TextInputState, style: &TextInputStyle, width: f32) -> f32 {
        let padding = style.scaled_horizontal_padding();
        let inner = (width - padding * 2.0).max(0.0);
        let text_width = Self::text_width(state, style);
        let aligned = if text_width > inner {
            padding
        } else {
            match style.align {
                TextAlign::Left => padding,
                TextAlign::Center => padding + (inner - text_width) / 2.0,
                TextAlign::Right => padding + inner - text_width,
            }
        };
        aligned - state.scroll_px
    }

    /// X position of the caret (or of any offset) inside a box of `width`.
    pub fn caret_x(
        state: &TextInputState,
        style: &TextInputStyle,
        width: f32,
        offset: usize,
    ) -> f32 {
        let past_mark = if offset > state.caret() {
            Self::mark_width(state, style)
        } else {
            0.0
        };
        Self::text_origin_x(state, style, width)
            + Self::measure(style, &Self::display_prefix(state, offset))
            + past_mark
    }

    /// The caret's box, in box-local points: where [`Self::render`] paints it,
    /// and what a client hands the compositor through
    /// `zwp_text_input_v3::set_cursor_rectangle`.
    ///
    /// Answered whether or not the caret is in its blink-on phase and whether
    /// or not a selection is hiding it. What an input method or a panel wants
    /// to know is where the insertion point *is*, not whether it happens to be
    /// painted this frame — a completion list that flickered with the blink
    /// would be unusable.
    pub fn caret_rect(
        state: &TextInputState,
        style: &TextInputStyle,
        width: f32,
        height: f32,
    ) -> Rect {
        let (_, metrics) = style.font().metrics();
        let baseline = Self::baseline(&metrics, height);
        let x = Self::caret_x(state, style, width, state.caret());
        Rect::from_ltrb(
            x,
            baseline + metrics.ascent,
            x + style.scaled_caret_width(),
            baseline + metrics.descent,
        )
    }

    /// Where the text sits vertically: the line's own box centred in the
    /// field's, rather than the glyphs' — so fields holding different strings
    /// still line up with each other.
    fn baseline(metrics: &skia_safe::FontMetrics, height: f32) -> f32 {
        (height - (metrics.ascent + metrics.descent)) / 2.0
    }

    /// Byte offset nearest to `x` (in box-local points) — click to place caret,
    /// drag to select. Returns a `char`-boundary offset into the value.
    pub fn hit_test_offset(
        state: &TextInputState,
        style: &TextInputStyle,
        width: f32,
        x: f32,
    ) -> usize {
        let value = state.value();
        if value.is_empty() {
            return 0;
        }
        let origin = Self::text_origin_x(state, style, width);
        let target = x - origin;
        if target <= 0.0 {
            return 0;
        }

        // Walk the chars, accumulating advances, and stop at the boundary whose
        // midpoint the pointer has passed — that is the offset a click snaps to.
        let mut prev_x = 0.0;
        let mut prev_offset = 0;
        for (offset, ch) in value.char_indices() {
            let next_offset = offset + ch.len_utf8();
            let next_x = Self::measure(style, &Self::display_prefix(state, next_offset));
            if target < (prev_x + next_x) / 2.0 {
                return prev_offset;
            }
            prev_x = next_x;
            prev_offset = next_offset;
        }
        value.len()
    }

    /// Scroll the text so the caret stays inside the box. Call after any change
    /// to the value or the caret, before drawing.
    pub fn ensure_caret_visible(state: &mut TextInputState, style: &TextInputStyle, width: f32) {
        let padding = style.scaled_horizontal_padding();
        let inner = (width - padding * 2.0).max(0.0);
        let text_width = Self::text_width(state, style);

        if text_width <= inner {
            state.scroll_px = 0.0;
            return;
        }

        // Caret x relative to the start of the text run.
        let caret_in_text = Self::measure(style, &Self::display_prefix(state, state.caret()));
        // While dictating, what has to stay in view runs to the equaliser.
        let caret_end = caret_in_text + Self::mark_width(state, style);
        let caret_width = style.scaled_caret_width();
        let mut scroll = state.scroll_px;
        if caret_in_text - scroll < 0.0 {
            scroll = caret_in_text;
        } else if caret_end - scroll > inner - caret_width {
            scroll = caret_end - inner + caret_width;
        }
        state.scroll_px = scroll.clamp(0.0, text_width - inner);
    }

    /// The completion drawn after the caret, or empty when there is none to
    /// draw. Only with the caret at the end of the value and nothing selected:
    /// a suggestion for text somewhere else is noise, and one drawn over a
    /// selection is unreadable.
    pub fn ghost_text(state: &TextInputState) -> &str {
        let at_end = state.caret() == state.value().len() && state.selection().is_empty();
        if state.password || state.is_empty() || !at_end || state.dictation.is_some() {
            return "";
        }
        &state.ghost
    }

    fn draw_ghost(
        canvas: &Canvas,
        state: &TextInputState,
        style: &TextInputStyle,
        font: &skia_safe::Font,
        origin_x: f32,
        baseline: f32,
    ) {
        let ghost = Self::ghost_text(state);
        if ghost.is_empty() {
            return;
        }
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(style.placeholder_color);
        let x = origin_x + Self::text_width(state, style);
        canvas.draw_str(ghost, (x, baseline), font, &paint);
    }

    /// Draw the field into a box of `width` x `height` at the canvas origin.
    ///
    /// `caret_visible` drives the blink — pass `true` for a steady caret.
    pub fn render(
        canvas: &Canvas,
        state: &TextInputState,
        style: &TextInputStyle,
        width: f32,
        height: f32,
        caret_visible: bool,
    ) {
        let bounds = Rect::from_xywh(0.0, 0.0, width, height);
        let radius = style.scaled_corner_radius();
        let mut paint = Paint::default();
        paint.set_anti_alias(true);

        if style.background.a() > 0 {
            paint.set_color(style.background);
            canvas.draw_rrect(RRect::new_rect_xy(bounds, radius, radius), &paint);
        }

        if state.focused() && style.focus_ring_width > 0.0 && style.focus_ring_color.a() > 0 {
            let w = style.scaled_focus_ring_width();
            let inset = bounds.with_inset((w / 2.0, w / 2.0));
            let mut ring = Paint::default();
            ring.set_anti_alias(true);
            ring.set_style(skia_safe::paint::Style::Stroke);
            ring.set_stroke_width(w);
            ring.set_color(style.focus_ring_color);
            canvas.draw_rrect(RRect::new_rect_xy(inset, radius, radius), &ring);
        }

        let font = style.font();
        let (_, metrics) = font.metrics();
        let baseline = Self::baseline(&metrics, height);
        let origin_x = Self::text_origin_x(state, style, width);

        canvas.save();
        // Clip so scrolled text and the selection never bleed past the box.
        let padding = style.scaled_horizontal_padding();
        canvas.clip_rect(
            Rect::from_xywh(
                padding.min(bounds.width()),
                0.0,
                (width - padding * 2.0).max(0.0),
                height,
            ),
            None,
            Some(true),
        );

        let text = Self::display_text(state);
        let is_placeholder = Self::shows_placeholder(state);
        let selection = state.selection();

        // Selection highlight sits behind the glyphs.
        if !is_placeholder && !selection.is_empty() {
            let start_x = Self::caret_x(state, style, width, selection.start);
            let end_x = Self::caret_x(state, style, width, selection.end);
            let top = baseline + metrics.ascent;
            let bottom = baseline + metrics.descent;
            paint.set_color(style.selection_color);
            canvas.draw_rect(Rect::from_ltrb(start_x, top, end_x, bottom), &paint);
        }

        if is_placeholder {
            if !text.is_empty() {
                paint.set_color(style.placeholder_color);
                canvas.draw_str(&text, (origin_x, baseline), &font, &paint);
            }
        } else if let (Some(mark), true) = (&state.dictation, selection.is_empty()) {
            // Dictating: the text before the caret, the mark, then the rest.
            let before = Self::display_prefix(state, state.caret());
            let after = text[before.len().min(text.len())..].to_string();
            let at = origin_x + Self::measure(style, &before);
            paint.set_color(style.text_color);
            canvas.draw_str(&before, (origin_x, baseline), &font, &paint);
            Self::draw_mark(canvas, mark, style, &font, at, baseline, &metrics);
            paint.set_color(style.text_color);
            canvas.draw_str(
                &after,
                (at + Self::mark_width(state, style), baseline),
                &font,
                &paint,
            );
        } else if selection.is_empty() {
            paint.set_color(style.text_color);
            canvas.draw_str(&text, (origin_x, baseline), &font, &paint);
            Self::draw_ghost(canvas, state, style, &font, origin_x, baseline);
        } else {
            // Three runs so the selected glyphs can take their own color.
            let before = Self::display_prefix(state, selection.start);
            let selected = {
                let full = Self::display_prefix(state, selection.end);
                full[before.len()..].to_string()
            };
            let after = {
                let full = Self::display_text(state);
                let end = Self::display_prefix(state, selection.end);
                full[end.len()..].to_string()
            };

            paint.set_color(style.text_color);
            canvas.draw_str(&before, (origin_x, baseline), &font, &paint);
            paint.set_color(style.selected_text_color);
            canvas.draw_str(
                &selected,
                (
                    Self::caret_x(state, style, width, selection.start),
                    baseline,
                ),
                &font,
                &paint,
            );
            paint.set_color(style.text_color);
            canvas.draw_str(
                &after,
                (Self::caret_x(state, style, width, selection.end), baseline),
                &font,
                &paint,
            );
        }

        // Caret: hidden while a selection is active, like every other field,
        // and while dictating, when the equaliser stands in for it.
        if state.focused() && caret_visible && selection.is_empty() && state.dictation.is_none() {
            paint.set_color(style.caret_color);
            canvas.draw_rect(Self::caret_rect(state, style, width, height), &paint);
        }

        canvas.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextInputStyle {
        TextInputStyle::default().with_align(TextAlign::Left)
    }

    /// A completion is only drawn where it reads as one: after what is being
    /// typed, with nothing selected and nothing masked.
    #[test]
    fn a_completion_is_offered_only_at_the_end_of_the_text() {
        let mut s = TextInputState::new("/conf");
        s.ghost = "igure-otto".into();
        assert_eq!(TextInputRenderer::ghost_text(&s), "igure-otto");

        let mut moved = s.clone();
        moved.set_caret(2, false);
        assert_eq!(
            TextInputRenderer::ghost_text(&moved),
            "",
            "the caret is not where the completion would land"
        );

        let mut selecting = s.clone();
        selecting.select_all();
        assert_eq!(TextInputRenderer::ghost_text(&selecting), "");

        let mut secret = s.clone();
        secret.password = true;
        assert_eq!(
            TextInputRenderer::ghost_text(&secret),
            "",
            "a masked field gives nothing away"
        );

        let mut empty = TextInputState::default();
        empty.ghost = "igure-otto".into();
        assert_eq!(
            TextInputRenderer::ghost_text(&empty),
            "",
            "an empty field shows its placeholder, not a completion"
        );
    }

    /// What one keystroke costs to draw, with and without a completion
    /// behind it. Ignored by default — it is a measurement, not an assertion.
    ///
    ///     cargo test -p otto-kit --lib field_paint_cost -- --ignored --nocapture
    #[test]
    #[ignore = "a measurement, run it by hand"]
    fn field_paint_cost() {
        use std::time::Instant;

        let style = style();
        let (w, h) = (600.0_f32, 44.0_f32);
        let mut surface = skia_safe::surfaces::raster_n32_premul((w as i32, h as i32))
            .expect("an offscreen surface");

        let mut plain = TextInputState::new("/configure-otto put the dock on the left");
        plain.set_focused(true);
        let mut ghosted = TextInputState::new("/conf");
        ghosted.set_focused(true);
        ghosted.ghost = "igure-otto".into();

        let runs = 2000;
        for (name, state) in [("no ghost", &plain), ("with ghost", &ghosted)] {
            // Warm the font cache and the raster surface.
            for _ in 0..50 {
                TextInputRenderer::render(surface.canvas(), state, &style, w, h, true);
            }
            let start = Instant::now();
            for _ in 0..runs {
                TextInputRenderer::render(surface.canvas(), state, &style, w, h, true);
            }
            let each = start.elapsed() / runs;
            println!("{name:>12}: {each:?} per paint");
        }

        let start = Instant::now();
        for _ in 0..runs {
            let _ = TextInputRenderer::measure(&style, "/configure-otto");
        }
        println!("     measure: {:?} per call", start.elapsed() / runs);

        let start = Instant::now();
        for _ in 0..runs {
            let _ = style.font();
        }
        println!("  style.font: {:?} per call", start.elapsed() / runs);
    }

    #[test]
    fn hit_test_snaps_to_the_nearest_boundary() {
        let s = TextInputState::new("hello");
        let style = style();
        let width = 200.0;

        assert_eq!(
            TextInputRenderer::hit_test_offset(&s, &style, width, -50.0),
            0
        );
        assert_eq!(
            TextInputRenderer::hit_test_offset(&s, &style, width, 1000.0),
            5
        );

        // Just past the middle of the third glyph lands on offset 3.
        let x2 = TextInputRenderer::caret_x(&s, &style, width, 2);
        let x3 = TextInputRenderer::caret_x(&s, &style, width, 3);
        let mid = (x2 + x3) / 2.0;
        assert_eq!(
            TextInputRenderer::hit_test_offset(&s, &style, width, mid + 0.5),
            3
        );
        assert_eq!(
            TextInputRenderer::hit_test_offset(&s, &style, width, mid - 0.5),
            2
        );
    }

    #[test]
    fn hit_test_returns_char_boundaries() {
        let s = TextInputState::new("héllo");
        let style = style();
        for i in 0..40 {
            let offset = TextInputRenderer::hit_test_offset(&s, &style, 200.0, i as f32 * 3.0);
            assert!(s.value().is_char_boundary(offset), "offset {offset}");
        }
    }

    #[test]
    fn caret_x_is_monotonic() {
        let s = TextInputState::new("abcdef");
        let style = style();
        let mut last = f32::NEG_INFINITY;
        for offset in 0..=s.value().len() {
            let x = TextInputRenderer::caret_x(&s, &style, 200.0, offset);
            assert!(x >= last);
            last = x;
        }
    }

    #[test]
    fn short_text_does_not_scroll() {
        let mut s = TextInputState::new("hi");
        let style = style();
        s.scroll_px = 40.0;
        TextInputRenderer::ensure_caret_visible(&mut s, &style, 200.0);
        assert_eq!(s.scroll_px, 0.0);
    }

    #[test]
    fn long_text_scrolls_to_keep_the_caret_in_view() {
        let mut s = TextInputState::new("long text ".repeat(20));
        let style = style();
        let width = 100.0;
        TextInputRenderer::ensure_caret_visible(&mut s, &style, width);
        assert!(s.scroll_px > 0.0);
        let caret_x = TextInputRenderer::caret_x(&s, &style, width, s.caret());
        assert!(caret_x <= width, "caret at {caret_x} outside {width}");

        s.set_caret(0, false);
        TextInputRenderer::ensure_caret_visible(&mut s, &style, width);
        assert_eq!(s.scroll_px, 0.0);
    }

    /// While dictating, the pending words and the equaliser sit at the
    /// caret: they widen the run, push the text after the caret along and
    /// hide the placeholder, without becoming part of the value.
    #[test]
    fn a_dictation_mark_takes_room_at_the_caret() {
        let style = style();
        let mut s = TextInputState::new("ab").with_placeholder("Ask");
        s.set_caret(1, false);
        let plain_end = TextInputRenderer::caret_x(&s, &style, 400.0, 2);
        s.dictation = Some(super::super::state::DictationMark {
            pending: " hello".into(),
            levels: vec![0.5; 5],
        });
        assert!(TextInputRenderer::caret_x(&s, &style, 400.0, 2) > plain_end);
        assert_eq!(
            TextInputRenderer::caret_x(&s, &style, 400.0, 1),
            TextInputRenderer::caret_x(&TextInputState::new("ab"), &style, 400.0, 1)
        );
        assert_eq!(s.value(), "ab");

        let mut empty = TextInputState::default().with_placeholder("Ask");
        empty.dictation = Some(Default::default());
        assert_eq!(TextInputRenderer::display_text(&empty), "");
    }

    #[test]
    fn password_mode_masks_the_display_text() {
        let s = TextInputState::new("abc").with_password(true);
        assert_eq!(TextInputRenderer::display_text(&s), "•••");
    }

    #[test]
    fn placeholder_shows_when_empty() {
        let s = TextInputState::default().with_placeholder("Name");
        assert_eq!(TextInputRenderer::display_text(&s), "Name");
    }
}
