use skia_safe::{Canvas, Color, Paint, PaintStyle, RRect, Rect};

use crate::common::Renderable;
use crate::input::keycodes;
use crate::typography::TextStyle;

/// Visual state of the input field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFieldState {
    Normal,
    Focused,
    Disabled,
}

/// A text input field with cursor, selection, and keyboard handling
pub struct InputField {
    // Layout
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Content
    text: String,
    placeholder: String,
    cursor_pos: usize,
    selection_start: Option<usize>,

    // Scroll offset for text wider than the field
    scroll_offset: f32,

    // Visual state
    state: InputFieldState,
    text_style: TextStyle,
    text_color: Color,
    placeholder_color: Color,
    background_color: Color,
    border_color: Color,
    focused_border_color: Color,
    cursor_color: Color,
    selection_color: Color,
    corner_radius: f32,
    padding_horizontal: f32,
    padding_vertical: f32,

    // Password masking
    masked: bool,
    mask_char: char,

    // Cursor blink state — toggled externally via `tick_cursor_blink`
    cursor_visible: bool,
    blink_counter: u32,
}

impl InputField {
    /// Create a new empty input field
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 0.0, // auto-calculated

            text: String::new(),
            placeholder: String::new(),
            cursor_pos: 0,
            selection_start: None,
            scroll_offset: 0.0,

            state: InputFieldState::Normal,
            text_style: crate::typography::styles::BODY,
            text_color: Color::from_rgb(17, 24, 39),
            placeholder_color: Color::from_rgb(156, 163, 175),
            background_color: Color::WHITE,
            border_color: Color::from_rgb(209, 213, 219),
            focused_border_color: Color::from_rgb(59, 130, 246),
            cursor_color: Color::from_rgb(59, 130, 246),
            selection_color: Color::from_argb(0x40, 59, 130, 246),
            corner_radius: 6.0,
            padding_horizontal: 10.0,
            padding_vertical: 8.0,

            masked: false,
            mask_char: '•',

            cursor_visible: true,
            blink_counter: 0,
        }
    }

    // ── Builder methods ──────────────────────────────────────────────

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self.cursor_pos = self.text.len();
        self
    }

    pub fn with_text_style(mut self, style: TextStyle) -> Self {
        self.text_style = style;
        self
    }

    pub fn with_text_color(mut self, color: Color) -> Self {
        self.text_color = color;
        self
    }

    pub fn with_background(mut self, color: Color) -> Self {
        self.background_color = color;
        self
    }

    pub fn with_border_color(mut self, color: Color) -> Self {
        self.border_color = color;
        self
    }

    pub fn with_focused_border_color(mut self, color: Color) -> Self {
        self.focused_border_color = color;
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.padding_horizontal = horizontal;
        self.padding_vertical = vertical;
        self
    }

    pub fn with_state(mut self, state: InputFieldState) -> Self {
        self.state = state;
        self
    }

    /// Enable password masking — text is displayed as bullet characters.
    pub fn with_mask(mut self) -> Self {
        self.masked = true;
        self
    }

    /// Enable password masking with a custom mask character.
    pub fn with_mask_char(mut self, ch: char) -> Self {
        self.masked = true;
        self.mask_char = ch;
        self
    }

    pub fn build(self) -> Self {
        self
    }

    // ── Accessors ────────────────────────────────────────────────────

    /// Current text content
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Set the text content and move cursor to the end
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor_pos = self.text.len();
        self.clear_selection();
        self.ensure_cursor_visible();
    }

    /// Current cursor byte-position in the text
    pub fn cursor_pos(&self) -> usize {
        self.cursor_pos
    }

    /// Current visual state
    pub fn state(&self) -> InputFieldState {
        self.state
    }

    /// Whether the field currently has focus
    pub fn is_focused(&self) -> bool {
        self.state == InputFieldState::Focused
    }

    /// Whether password masking is enabled
    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// Returns the display string — masked bullets or the real text.
    fn display_text(&self) -> String {
        if self.masked {
            self.mask_char.to_string().repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }

    /// Returns the display prefix up to the given byte position.
    /// For masked mode, maps the byte position to the equivalent masked char count.
    fn display_prefix(&self, byte_pos: usize) -> String {
        if self.masked {
            let char_count = self.text[..byte_pos].chars().count();
            self.mask_char.to_string().repeat(char_count)
        } else {
            self.text[..byte_pos].to_string()
        }
    }

    // ── Focus management ─────────────────────────────────────────────

    /// Give keyboard focus to this field
    pub fn focus(&mut self) {
        if self.state != InputFieldState::Disabled {
            self.state = InputFieldState::Focused;
            self.cursor_visible = true;
            self.blink_counter = 0;
        }
    }

    /// Remove keyboard focus from this field
    pub fn blur(&mut self) {
        if self.state == InputFieldState::Focused {
            self.state = InputFieldState::Normal;
            self.clear_selection();
        }
    }

    // ── Cursor blink ─────────────────────────────────────────────────

    /// Call periodically (e.g. every frame or every 16 ms) to animate the cursor blink.
    /// The cursor toggles visibility roughly every 30 ticks (~500 ms at 60 fps).
    /// Returns `true` if the visibility changed (i.e. a redraw is needed).
    pub fn tick_cursor_blink(&mut self) -> bool {
        if self.state != InputFieldState::Focused {
            return false;
        }
        self.blink_counter += 1;
        if self.blink_counter >= 30 {
            self.blink_counter = 0;
            self.cursor_visible = !self.cursor_visible;
            return true;
        }
        false
    }

    // ── Keyboard input handling ──────────────────────────────────────

    /// Handle a key press. `utf8` is the character text produced by XKB (if any).
    /// Returns `true` if the text content changed.
    ///
    /// This variant ignores modifiers — selection and shortcuts won't work.
    /// Prefer [`handle_key_mod`] when modifier state is available.
    pub fn handle_key(&mut self, keycode: u32, utf8: Option<&str>) -> bool {
        self.handle_key_mod(keycode, utf8, false, false)
    }

    /// Handle a key press with modifier state.
    ///
    /// - `shift`: extend/create selection on cursor-movement keys
    /// - `ctrl`: word-level movement (Left/Right), select-all (A), cut/copy placeholders
    ///
    /// Returns `true` if the text content changed.
    pub fn handle_key_mod(
        &mut self,
        keycode: u32,
        utf8: Option<&str>,
        shift: bool,
        ctrl: bool,
    ) -> bool {
        if self.state != InputFieldState::Focused {
            return false;
        }

        // Reset cursor blink on every keypress
        self.cursor_visible = true;
        self.blink_counter = 0;

        let changed = match keycode {
            keycodes::BACKSPACE => {
                if ctrl {
                    self.delete_word_backward()
                } else {
                    self.delete_backward()
                }
            }
            keycodes::DELETE => {
                if ctrl {
                    self.delete_word_forward()
                } else {
                    self.delete_forward()
                }
            }
            keycodes::LEFT => {
                if ctrl {
                    self.move_cursor_word_left(shift);
                } else {
                    self.move_cursor_left(shift);
                }
                false
            }
            keycodes::RIGHT => {
                if ctrl {
                    self.move_cursor_word_right(shift);
                } else {
                    self.move_cursor_right(shift);
                }
                false
            }
            keycodes::HOME => {
                self.move_to(0, shift);
                false
            }
            keycodes::END => {
                self.move_to(self.text.len(), shift);
                false
            }
            _ => {
                // Ctrl+A → select all
                if ctrl && keycode == keycodes::A {
                    self.select_all();
                    return false;
                }

                // Insert printable characters (replaces selection if any)
                if let Some(ch) = utf8 {
                    if !ch.is_empty() && !ch.chars().all(|c| c.is_control()) {
                        self.insert_text(ch);
                        return true;
                    }
                }
                false
            }
        };

        changed
    }

    /// Select all text
    pub fn select_all(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.selection_start = Some(0);
        self.cursor_pos = self.text.len();
    }

    // ── Pointer hit-testing ──────────────────────────────────────────

    /// Returns `true` if the point (in the parent's coordinate space) is inside this field.
    pub fn hit_test(&self, px: f32, py: f32) -> bool {
        let (width, height) = self.dimensions();
        px >= self.x && px <= self.x + width && py >= self.y && py <= self.y + height
    }

    /// Move the cursor to the position nearest to the given x coordinate.
    pub fn place_cursor_at_x(&mut self, px: f32) {
        let font = self.text_style.font();
        let display = self.display_text();
        let content_x = self.x + self.padding_horizontal;
        let relative_x = px - content_x + self.scroll_offset;

        if relative_x <= 0.0 {
            self.cursor_pos = 0;
        } else {
            // Walk character boundaries to find the nearest position
            let mut best_pos = 0;
            for (i, _) in self.text.char_indices() {
                let prefix = self.display_prefix(i);
                let (w, _) = font.measure_str(&prefix, None);
                if w > relative_x {
                    break;
                }
                best_pos = i;
            }
            // Check end
            let (full_w, _) = font.measure_str(&display, None);
            if relative_x >= full_w {
                best_pos = self.text.len();
            }
            self.cursor_pos = best_pos;
        }
        self.clear_selection();
        self.cursor_visible = true;
        self.blink_counter = 0;
    }

    // ── Private helpers ──────────────────────────────────────────────

    fn insert_text(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor_pos, s);
        self.cursor_pos += s.len();
        self.ensure_cursor_visible();
    }

    fn delete_backward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor_pos == 0 {
            return false;
        }
        let prev = self.prev_char_boundary(self.cursor_pos);
        self.text.drain(prev..self.cursor_pos);
        self.cursor_pos = prev;
        self.ensure_cursor_visible();
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor_pos >= self.text.len() {
            return false;
        }
        let next = self.next_char_boundary(self.cursor_pos);
        self.text.drain(self.cursor_pos..next);
        self.ensure_cursor_visible();
        true
    }

    fn delete_selection(&mut self) -> bool {
        if let Some(sel) = self.selection_start.take() {
            let (start, end) = if sel < self.cursor_pos {
                (sel, self.cursor_pos)
            } else {
                (self.cursor_pos, sel)
            };
            self.text.drain(start..end);
            self.cursor_pos = start;
            self.ensure_cursor_visible();
            return true;
        }
        false
    }

    /// Move the cursor to `target`, optionally extending the selection.
    fn move_to(&mut self, target: usize, extend_selection: bool) {
        if extend_selection {
            if self.selection_start.is_none() {
                self.selection_start = Some(self.cursor_pos);
            }
        } else {
            self.clear_selection();
        }
        self.cursor_pos = target;
        self.ensure_cursor_visible();
    }

    fn move_cursor_left(&mut self, extend_selection: bool) {
        if self.cursor_pos > 0 {
            let target = self.prev_char_boundary(self.cursor_pos);
            self.move_to(target, extend_selection);
        } else if !extend_selection {
            self.clear_selection();
        }
    }

    fn move_cursor_right(&mut self, extend_selection: bool) {
        if self.cursor_pos < self.text.len() {
            let target = self.next_char_boundary(self.cursor_pos);
            self.move_to(target, extend_selection);
        } else if !extend_selection {
            self.clear_selection();
        }
    }

    fn move_cursor_word_left(&mut self, extend_selection: bool) {
        let target = self.word_boundary_left(self.cursor_pos);
        self.move_to(target, extend_selection);
    }

    fn move_cursor_word_right(&mut self, extend_selection: bool) {
        let target = self.word_boundary_right(self.cursor_pos);
        self.move_to(target, extend_selection);
    }

    fn delete_word_backward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor_pos == 0 {
            return false;
        }
        let target = self.word_boundary_left(self.cursor_pos);
        self.text.drain(target..self.cursor_pos);
        self.cursor_pos = target;
        self.ensure_cursor_visible();
        true
    }

    fn delete_word_forward(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor_pos >= self.text.len() {
            return false;
        }
        let target = self.word_boundary_right(self.cursor_pos);
        self.text.drain(self.cursor_pos..target);
        self.ensure_cursor_visible();
        true
    }

    fn clear_selection(&mut self) {
        self.selection_start = None;
    }

    /// Find the previous char boundary before `pos`
    fn prev_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.saturating_sub(1);
        while p > 0 && !self.text.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// Find the next char boundary after `pos`
    fn next_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos + 1;
        while p < self.text.len() && !self.text.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    /// Find the start of the word to the left of `pos`
    fn word_boundary_left(&self, pos: usize) -> usize {
        let bytes = self.text.as_bytes();
        let mut p = pos;
        // Skip whitespace
        while p > 0 && bytes[p - 1].is_ascii_whitespace() {
            p -= 1;
        }
        // Skip word characters
        while p > 0 && !bytes[p - 1].is_ascii_whitespace() {
            p -= 1;
        }
        p
    }

    /// Find the end of the word to the right of `pos`
    fn word_boundary_right(&self, pos: usize) -> usize {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        let mut p = pos;
        // Skip word characters
        while p < len && !bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        // Skip whitespace
        while p < len && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        p
    }

    /// Adjust scroll_offset so the cursor is visible within the text area
    fn ensure_cursor_visible(&mut self) {
        let font = self.text_style.font();
        let text_area_width = self.width - self.padding_horizontal * 2.0;

        let cursor_prefix = self.display_prefix(self.cursor_pos);
        let (cursor_x, _) = font.measure_str(&cursor_prefix, None);

        // Scroll right if cursor is past the visible area
        if cursor_x - self.scroll_offset > text_area_width {
            self.scroll_offset = cursor_x - text_area_width;
        }
        // Scroll left if cursor is before the visible area
        if cursor_x < self.scroll_offset {
            self.scroll_offset = cursor_x;
        }
    }

    fn dimensions(&self) -> (f32, f32) {
        let height = if self.height > 0.0 {
            self.height
        } else {
            let font = self.text_style.font();
            font.size() + self.padding_vertical * 2.0
        };
        (self.width, height)
    }
}

impl Default for InputField {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for InputField {
    fn render(&self, canvas: &Canvas) {
        let (width, height) = self.dimensions();
        let font = self.text_style.font();

        let rect = Rect::from_xywh(self.x, self.y, width, height);
        let rrect = RRect::new_rect_xy(rect, self.corner_radius, self.corner_radius);

        // Background
        let mut bg_paint = Paint::default();
        bg_paint.set_anti_alias(true);
        bg_paint.set_color(if self.state == InputFieldState::Disabled {
            Color::from_rgb(243, 244, 246)
        } else {
            self.background_color
        });
        canvas.draw_rrect(rrect, &bg_paint);

        // Border
        let mut border_paint = Paint::default();
        border_paint.set_anti_alias(true);
        border_paint.set_style(PaintStyle::Stroke);
        border_paint.set_stroke_width(1.0);
        border_paint.set_color(if self.state == InputFieldState::Focused {
            self.focused_border_color
        } else {
            self.border_color
        });
        canvas.draw_rrect(rrect, &border_paint);

        // Clip to content area
        let content_rect = Rect::from_xywh(
            self.x + self.padding_horizontal,
            self.y,
            width - self.padding_horizontal * 2.0,
            height,
        );

        canvas.save();
        canvas.clip_rect(content_rect, None, Some(true));

        let text_x = self.x + self.padding_horizontal - self.scroll_offset;
        let text_y = self.y + (height + font.size()) / 2.0 - font.size() * 0.15;

        let display = self.display_text();

        // Selection highlight
        if let Some(sel_start) = self.selection_start {
            let (start, end) = if sel_start < self.cursor_pos {
                (sel_start, self.cursor_pos)
            } else {
                (self.cursor_pos, sel_start)
            };

            let start_display = self.display_prefix(start);
            let end_display = self.display_prefix(end);
            let (start_x, _) = font.measure_str(&start_display, None);
            let (end_x, _) = font.measure_str(&end_display, None);

            let sel_rect = Rect::from_xywh(
                text_x + start_x,
                self.y + self.padding_vertical * 0.5,
                end_x - start_x,
                height - self.padding_vertical,
            );

            let mut sel_paint = Paint::default();
            sel_paint.set_color(self.selection_color);
            sel_paint.set_anti_alias(true);
            canvas.draw_rect(sel_rect, &sel_paint);
        }

        // Text or placeholder
        let mut text_paint = Paint::default();
        text_paint.set_anti_alias(true);

        if self.text.is_empty() {
            text_paint.set_color(self.placeholder_color);
            canvas.draw_str(&self.placeholder, (text_x, text_y), &font, &text_paint);
        } else {
            text_paint.set_color(if self.state == InputFieldState::Disabled {
                self.placeholder_color
            } else {
                self.text_color
            });
            canvas.draw_str(&display, (text_x, text_y), &font, &text_paint);
        }

        // Cursor
        if self.state == InputFieldState::Focused && self.cursor_visible {
            let cursor_display = self.display_prefix(self.cursor_pos);
            let (cursor_x_offset, _) = font.measure_str(&cursor_display, None);

            let cursor_x = text_x + cursor_x_offset;
            let cursor_top = self.y + self.padding_vertical * 0.5;
            let cursor_bottom = self.y + height - self.padding_vertical * 0.5;

            let mut cursor_paint = Paint::default();
            cursor_paint.set_color(self.cursor_color);
            cursor_paint.set_stroke_width(1.5);
            cursor_paint.set_anti_alias(true);

            canvas.draw_line(
                (cursor_x, cursor_top),
                (cursor_x, cursor_bottom),
                &cursor_paint,
            );
        }

        canvas.restore();
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        Some(self.dimensions())
    }
}
