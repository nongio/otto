use std::hash::{Hash, Hasher};
use std::ops::Range;

/// Caret movement granularity, shared by keyboard handling in consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movement {
    /// One grapheme (approximated as one `char`) left/right.
    Char,
    /// To the start/end of the adjacent word.
    Word,
    /// To the start/end of the whole value.
    Line,
}

/// Editing state of a single-line text field.
///
/// Offsets are **byte** offsets into `value` and are always kept on `char`
/// boundaries. `anchor` is where a selection started, `caret` is where it
/// currently ends — `anchor == caret` means an empty selection (just a caret).
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputState {
    value: String,
    anchor: usize,
    caret: usize,
    focused: bool,
    /// Shown when `value` is empty.
    pub placeholder: String,
    /// Mask the glyphs and refuse to hand the text out (lock screen, greeter).
    pub password: bool,
    /// Maximum length in `char`s. Inserts that would exceed it are truncated.
    pub max_chars: Option<usize>,
    /// Horizontal scroll offset in points, kept so the caret stays visible when
    /// the text is wider than the box. Owned by the renderer.
    pub scroll_px: f32,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            value: String::new(),
            anchor: 0,
            caret: 0,
            focused: false,
            placeholder: String::new(),
            password: false,
            max_chars: None,
            scroll_px: 0.0,
        }
    }
}

/// Hashed so scene-graph hosts (lay-rs views) can key a redraw off the whole
/// editing state, caret and selection included.
impl Hash for TextInputState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        self.anchor.hash(state);
        self.caret.hash(state);
        self.focused.hash(state);
        self.placeholder.hash(state);
        self.password.hash(state);
        self.max_chars.hash(state);
        self.scroll_px.to_bits().hash(state);
    }
}

impl TextInputState {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let caret = value.len();
        Self {
            value,
            anchor: caret,
            caret,
            ..Default::default()
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn with_password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_chars = Some(max_chars);
        self
    }

    /// Start focused with everything selected — the "rename in place" default.
    pub fn with_all_selected(mut self) -> Self {
        self.focused = true;
        self.select_all();
        self
    }

    // === Value ===

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replace the whole value, clamping the caret to the end.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        let end = self.value.len();
        self.anchor = end;
        self.caret = end;
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    // === Focus ===

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    // === Selection ===

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// Selected byte range, normalized so `start <= end`.
    pub fn selection(&self) -> Range<usize> {
        let (start, end) = if self.anchor <= self.caret {
            (self.anchor, self.caret)
        } else {
            (self.caret, self.anchor)
        };
        start..end
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }

    /// The selected text, or `None` in password mode — a masked field must not
    /// leak its buffer through copy/cut.
    pub fn selected_text(&self) -> Option<&str> {
        if self.password {
            return None;
        }
        let sel = self.selection();
        if sel.is_empty() {
            None
        } else {
            Some(&self.value[sel])
        }
    }

    /// Place the caret at `offset`. With `extend` the anchor stays put, growing
    /// the selection (shift-click, shift-arrows, drag).
    pub fn set_caret(&mut self, offset: usize, extend: bool) {
        let offset = self.clamp_boundary(offset);
        self.caret = offset;
        if !extend {
            self.anchor = offset;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.value.len();
    }

    pub fn select_range(&mut self, range: Range<usize>) {
        self.anchor = self.clamp_boundary(range.start);
        self.caret = self.clamp_boundary(range.end);
    }

    /// Select the word under `offset` (double-click). Falls back to a caret
    /// placement when the offset sits in a run of separators.
    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.clamp_boundary(offset);
        let start = self.word_start(offset);
        let end = self.word_end(offset);
        self.anchor = start;
        self.caret = end;
    }

    // === Movement ===

    pub fn move_left(&mut self, movement: Movement, extend: bool) {
        // Collapsing a selection leftwards lands on its start, not one char
        // back from the caret.
        if self.has_selection() && !extend {
            let start = self.selection().start;
            if movement == Movement::Char {
                self.set_caret(start, false);
                return;
            }
        }
        let target = match movement {
            Movement::Char => self.prev_boundary(self.caret),
            Movement::Word => self.prev_word_boundary(self.caret),
            Movement::Line => 0,
        };
        self.set_caret(target, extend);
    }

    pub fn move_right(&mut self, movement: Movement, extend: bool) {
        if self.has_selection() && !extend {
            let end = self.selection().end;
            if movement == Movement::Char {
                self.set_caret(end, false);
                return;
            }
        }
        let target = match movement {
            Movement::Char => self.next_boundary(self.caret),
            Movement::Word => self.next_word_boundary(self.caret),
            Movement::Line => self.value.len(),
        };
        self.set_caret(target, extend);
    }

    // === Editing ===

    /// Insert text at the caret, replacing any selection. Honors `max_chars`.
    pub fn insert_str(&mut self, text: &str) {
        // Single-line field: newlines and control chars never enter the buffer.
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() && !self.has_selection() {
            return;
        }
        self.delete_selection();

        let text = match self.max_chars {
            Some(max) => {
                let room = max.saturating_sub(self.value.chars().count());
                text.chars().take(room).collect::<String>()
            }
            None => text,
        };
        if text.is_empty() {
            return;
        }
        self.value.insert_str(self.caret, &text);
        let caret = self.caret + text.len();
        self.anchor = caret;
        self.caret = caret;
    }

    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut buf));
    }

    /// Backspace. With an active selection it just removes the selection.
    pub fn backspace(&mut self, movement: Movement) {
        if self.delete_selection() {
            return;
        }
        let start = match movement {
            Movement::Char => self.prev_boundary(self.caret),
            Movement::Word => self.prev_word_boundary(self.caret),
            Movement::Line => 0,
        };
        if start == self.caret {
            return;
        }
        self.value.replace_range(start..self.caret, "");
        self.anchor = start;
        self.caret = start;
    }

    /// Forward delete.
    pub fn delete(&mut self, movement: Movement) {
        if self.delete_selection() {
            return;
        }
        let end = match movement {
            Movement::Char => self.next_boundary(self.caret),
            Movement::Word => self.next_word_boundary(self.caret),
            Movement::Line => self.value.len(),
        };
        if end == self.caret {
            return;
        }
        self.value.replace_range(self.caret..end, "");
        self.anchor = self.caret;
    }

    /// Remove the selection if there is one. Returns whether anything changed.
    pub fn delete_selection(&mut self) -> bool {
        let sel = self.selection();
        if sel.is_empty() {
            return false;
        }
        let start = sel.start;
        self.value.replace_range(sel, "");
        self.anchor = start;
        self.caret = start;
        true
    }

    /// Cut: the selected text, removed from the buffer. `None` in password mode
    /// (the selection is left untouched there).
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_text()?.to_string();
        self.delete_selection();
        Some(text)
    }

    // === Boundaries ===

    fn clamp_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.value.len());
        if self.value.is_char_boundary(offset) {
            offset
        } else {
            // Walk back to the start of the char containing `offset`.
            (0..=offset)
                .rev()
                .find(|o| self.value.is_char_boundary(*o))
                .unwrap_or(0)
        }
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.value[..self.clamp_boundary(offset)]
            .chars()
            .next_back()
            .map(|c| offset - c.len_utf8())
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let offset = self.clamp_boundary(offset);
        self.value[offset..]
            .chars()
            .next()
            .map(|c| offset + c.len_utf8())
            .unwrap_or(offset)
    }

    /// Start of the word run containing (or preceding) `offset`: skip
    /// separators leftwards, then the word itself.
    fn prev_word_boundary(&self, offset: usize) -> usize {
        let mut o = self.clamp_boundary(offset);
        while o > 0 && !is_word_char(self.char_before(o)) {
            o = self.prev_boundary(o);
        }
        while o > 0 && is_word_char(self.char_before(o)) {
            o = self.prev_boundary(o);
        }
        o
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        let len = self.value.len();
        let mut o = self.clamp_boundary(offset);
        while o < len && !is_word_char(self.char_at(o)) {
            o = self.next_boundary(o);
        }
        while o < len && is_word_char(self.char_at(o)) {
            o = self.next_boundary(o);
        }
        o
    }

    fn word_start(&self, offset: usize) -> usize {
        let mut o = offset;
        while o > 0 && is_word_char(self.char_before(o)) {
            o = self.prev_boundary(o);
        }
        o
    }

    fn word_end(&self, offset: usize) -> usize {
        let len = self.value.len();
        let mut o = offset;
        while o < len && is_word_char(self.char_at(o)) {
            o = self.next_boundary(o);
        }
        o
    }

    fn char_before(&self, offset: usize) -> Option<char> {
        self.value[..offset].chars().next_back()
    }

    fn char_at(&self, offset: usize) -> Option<char> {
        self.value[offset..].chars().next()
    }
}

fn is_word_char(c: Option<char>) -> bool {
    matches!(c, Some(c) if c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(value: &str) -> TextInputState {
        TextInputState::new(value)
    }

    #[test]
    fn new_places_caret_at_end() {
        let s = state("hello");
        assert_eq!(s.caret(), 5);
        assert!(!s.has_selection());
    }

    #[test]
    fn select_all_then_typing_replaces() {
        let mut s = state("Workspace 1");
        s.select_all();
        assert_eq!(s.selected_text(), Some("Workspace 1"));
        s.insert_str("Mail");
        assert_eq!(s.value(), "Mail");
        assert_eq!(s.caret(), 4);
        assert!(!s.has_selection());
    }

    #[test]
    fn shift_arrows_extend_from_anchor() {
        let mut s = state("abcdef");
        s.set_caret(3, false);
        s.move_right(Movement::Char, true);
        s.move_right(Movement::Char, true);
        assert_eq!(s.selection(), 3..5);
        s.move_left(Movement::Char, true);
        assert_eq!(s.selection(), 3..4);
    }

    #[test]
    fn plain_arrow_collapses_selection_to_its_edge() {
        let mut s = state("abcdef");
        s.select_range(1..4);
        s.move_left(Movement::Char, false);
        assert_eq!(s.caret(), 1);
        s.select_range(1..4);
        s.move_right(Movement::Char, false);
        assert_eq!(s.caret(), 4);
    }

    #[test]
    fn word_movement_skips_separators() {
        let mut s = state("one two  three");
        s.set_caret(0, false);
        s.move_right(Movement::Word, false);
        assert_eq!(s.caret(), 3);
        s.move_right(Movement::Word, false);
        assert_eq!(s.caret(), 7);
        s.move_left(Movement::Word, false);
        assert_eq!(s.caret(), 4);
    }

    #[test]
    fn double_click_selects_a_word() {
        let mut s = state("one two three");
        s.select_word_at(5);
        assert_eq!(s.selected_text(), Some("two"));
    }

    #[test]
    fn backspace_word_removes_the_run() {
        let mut s = state("hello world");
        s.backspace(Movement::Word);
        assert_eq!(s.value(), "hello ");
    }

    #[test]
    fn delete_forward_at_end_is_a_noop() {
        let mut s = state("ab");
        s.delete(Movement::Char);
        assert_eq!(s.value(), "ab");
    }

    #[test]
    fn editing_is_char_boundary_safe() {
        let mut s = state("héllo — ok");
        s.set_caret(1, false);
        s.move_right(Movement::Char, false);
        assert_eq!(s.caret(), 3); // 'é' is two bytes
        s.move_left(Movement::Char, false);
        assert_eq!(s.caret(), 1);
        s.set_caret(usize::MAX, false);
        assert_eq!(s.caret(), s.value().len());
        s.backspace(Movement::Char);
        assert_eq!(s.value(), "héllo — o");
    }

    #[test]
    fn max_chars_truncates_inserts() {
        let mut s = TextInputState::new("abc").with_max_chars(5);
        s.insert_str("defgh");
        assert_eq!(s.value(), "abcde");
    }

    #[test]
    fn control_chars_never_enter_the_buffer() {
        let mut s = state("");
        s.insert_str("a\nb\tc");
        assert_eq!(s.value(), "abc");
    }

    #[test]
    fn password_mode_hides_the_selection() {
        let mut s = TextInputState::new("hunter2").with_password(true);
        s.select_all();
        assert_eq!(s.selected_text(), None);
        assert_eq!(s.cut(), None);
        assert_eq!(s.value(), "hunter2");
    }
}
