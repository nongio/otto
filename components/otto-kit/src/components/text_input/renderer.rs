use skia_safe::{Canvas, Paint, RRect, Rect};

use crate::components::label::TextAlign;

use super::state::TextInputState;
use super::style::TextInputStyle;

/// Character used to mask the value in password mode.
const MASK_CHAR: char = '•';

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
        if state.is_empty() {
            return state.placeholder.clone();
        }
        if state.password {
            return MASK_CHAR.to_string().repeat(state.value().chars().count());
        }
        state.value().to_string()
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
        Self::measure(style, &Self::display_text(state))
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
        Self::text_origin_x(state, style, width)
            + Self::measure(style, &Self::display_prefix(state, offset))
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
        let caret_width = style.scaled_caret_width();
        let mut scroll = state.scroll_px;
        if caret_in_text - scroll < 0.0 {
            scroll = caret_in_text;
        } else if caret_in_text - scroll > inner - caret_width {
            scroll = caret_in_text - inner + caret_width;
        }
        state.scroll_px = scroll.clamp(0.0, text_width - inner);
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
        let baseline = (height - (metrics.ascent + metrics.descent)) / 2.0;
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
        let is_placeholder = state.is_empty();
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
        } else if selection.is_empty() {
            paint.set_color(style.text_color);
            canvas.draw_str(&text, (origin_x, baseline), &font, &paint);
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

        // Caret: hidden while a selection is active, like every other field.
        if state.focused() && caret_visible && selection.is_empty() {
            let x = Self::caret_x(state, style, width, state.caret());
            let top = baseline + metrics.ascent;
            let bottom = baseline + metrics.descent;
            paint.set_color(style.caret_color);
            canvas.draw_rect(
                Rect::from_ltrb(x, top, x + style.scaled_caret_width(), bottom),
                &paint,
            );
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
