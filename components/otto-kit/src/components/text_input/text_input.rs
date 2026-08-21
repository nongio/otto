use skia_safe::Canvas;

use crate::common::Renderable;

use super::renderer::TextInputRenderer;
use super::state::{Movement, TextInputState};
use super::style::TextInputStyle;

/// Caret blink period in seconds (half on, half off).
pub const CARET_BLINK_PERIOD: f32 = 1.06;

/// Modifier state for a key press, in the terms this widget cares about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyMods {
    pub shift: bool,
    pub ctrl: bool,
}

impl KeyMods {
    pub fn shift(shift: bool) -> Self {
        Self { shift, ctrl: false }
    }
}

/// A key press, already translated out of whatever the host uses (xkb keysyms
/// in the compositor, wayland keycodes in a client) so this component stays
/// free of input-stack dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputKey {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Escape,
    SelectAll,
    Copy,
    Cut,
    /// Paste request — the host supplies the clipboard text.
    Paste(String),
    /// A printable character produced by the keymap.
    Char(char),
    /// Committed text from an input method.
    Text(String),
}

/// What the host should do after a key was handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputResponse {
    /// Key was not for this field — let the host handle it.
    Ignored,
    /// State changed, redraw.
    Changed,
    /// Caret/selection moved but the value is the same.
    Moved,
    /// Enter — the host should commit `value`.
    Commit,
    /// Escape — the host should discard the edit.
    Cancel,
    /// Put this text on the clipboard (copy or cut).
    Clipboard(String),
}

/// A single-line editable text field: value, caret, selection, and the drawing
/// that goes with them.
///
/// The widget owns no input plumbing. The host feeds it translated key presses
/// and pointer positions, and draws it via [`Renderable`] or by calling
/// [`TextInputRenderer::render`] directly.
#[derive(Debug, Clone)]
pub struct TextInput {
    pub state: TextInputState,
    pub style: TextInputStyle,
    /// Box size the field is laid out in, needed for hit-testing and scroll.
    pub width: f32,
    pub height: f32,
    /// Blink phase in seconds, advanced by the host via [`Self::tick`].
    blink_elapsed: f32,
    /// Offset a drag selection started from.
    drag_origin: Option<usize>,
}

impl TextInput {
    pub fn new(value: impl Into<String>, style: TextInputStyle) -> Self {
        Self {
            state: TextInputState::new(value),
            style,
            width: 0.0,
            height: 0.0,
            blink_elapsed: 0.0,
            drag_origin: None,
        }
    }

    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.set_size(width, height);
        self
    }

    /// Focused with the whole value selected — how an in-place rename starts.
    pub fn editing(value: impl Into<String>, style: TextInputStyle) -> Self {
        let mut input = Self::new(value, style);
        input.state.set_focused(true);
        input.state.select_all();
        input
    }

    pub fn set_size(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
        TextInputRenderer::ensure_caret_visible(&mut self.state, &self.style, self.width);
    }

    pub fn value(&self) -> &str {
        self.state.value()
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.state.set_value(value);
        self.after_edit();
    }

    /// Advance the caret blink by `delta` seconds.
    pub fn tick(&mut self, delta: f32) {
        self.blink_elapsed = (self.blink_elapsed + delta) % CARET_BLINK_PERIOD;
    }

    /// Is the caret in its visible phase? Typing resets the phase so the caret
    /// is always solid right after a keystroke.
    pub fn caret_visible(&self) -> bool {
        self.state.focused() && self.blink_elapsed < CARET_BLINK_PERIOD / 2.0
    }

    // === Keyboard ===

    /// Feed a key press. Returns what the host should do next.
    pub fn on_key(&mut self, key: TextInputKey, mods: KeyMods) -> TextInputResponse {
        if !self.state.focused() {
            return TextInputResponse::Ignored;
        }
        let word = if mods.ctrl {
            Movement::Word
        } else {
            Movement::Char
        };
        self.blink_elapsed = 0.0;

        match key {
            TextInputKey::Left => {
                self.state.move_left(word, mods.shift);
                self.after_move()
            }
            TextInputKey::Right => {
                self.state.move_right(word, mods.shift);
                self.after_move()
            }
            TextInputKey::Home => {
                self.state.move_left(Movement::Line, mods.shift);
                self.after_move()
            }
            TextInputKey::End => {
                self.state.move_right(Movement::Line, mods.shift);
                self.after_move()
            }
            TextInputKey::Backspace => {
                self.state.backspace(word);
                self.after_edit()
            }
            TextInputKey::Delete => {
                self.state.delete(word);
                self.after_edit()
            }
            TextInputKey::SelectAll => {
                self.state.select_all();
                self.after_move()
            }
            TextInputKey::Copy => match self.state.selected_text() {
                Some(text) => TextInputResponse::Clipboard(text.to_string()),
                None => TextInputResponse::Ignored,
            },
            TextInputKey::Cut => match self.state.cut() {
                Some(text) => {
                    self.after_edit();
                    TextInputResponse::Clipboard(text)
                }
                None => TextInputResponse::Ignored,
            },
            TextInputKey::Paste(text) => {
                self.state.insert_str(&text);
                self.after_edit()
            }
            TextInputKey::Char(c) => {
                if c.is_control() {
                    return TextInputResponse::Ignored;
                }
                self.state.insert_char(c);
                self.after_edit()
            }
            TextInputKey::Text(text) => {
                self.state.insert_str(&text);
                self.after_edit()
            }
            TextInputKey::Enter => TextInputResponse::Commit,
            TextInputKey::Escape => TextInputResponse::Cancel,
        }
    }

    // === Pointer ===

    /// Pointer press at box-local `x`. `click_count` is 1 for a single click,
    /// 2 for a double click (select word), 3 or more for a triple click
    /// (select all). Shift-click extends the current selection.
    pub fn on_pointer_down(&mut self, x: f32, click_count: u32, shift: bool) {
        let offset = self.offset_at(x);
        self.blink_elapsed = 0.0;
        match click_count {
            0 | 1 => {
                self.state.set_caret(offset, shift);
                self.drag_origin = Some(offset);
            }
            2 => {
                self.state.select_word_at(offset);
                self.drag_origin = None;
            }
            _ => {
                self.state.select_all();
                self.drag_origin = None;
            }
        }
        TextInputRenderer::ensure_caret_visible(&mut self.state, &self.style, self.width);
    }

    /// Pointer moved to box-local `x` while the button is held — extend the
    /// selection from where the drag started.
    pub fn on_pointer_drag(&mut self, x: f32) {
        if self.drag_origin.is_none() {
            return;
        }
        let offset = self.offset_at(x);
        self.state.set_caret(offset, true);
        TextInputRenderer::ensure_caret_visible(&mut self.state, &self.style, self.width);
    }

    pub fn on_pointer_up(&mut self) {
        self.drag_origin = None;
    }

    /// Byte offset under box-local `x`.
    pub fn offset_at(&self, x: f32) -> usize {
        TextInputRenderer::hit_test_offset(&self.state, &self.style, self.width, x)
    }

    // === Drawing ===

    pub fn render_at(&self, canvas: &Canvas, width: f32, height: f32) {
        TextInputRenderer::render(
            canvas,
            &self.state,
            &self.style,
            width,
            height,
            self.caret_visible(),
        );
    }

    fn after_move(&mut self) -> TextInputResponse {
        TextInputRenderer::ensure_caret_visible(&mut self.state, &self.style, self.width);
        TextInputResponse::Moved
    }

    fn after_edit(&mut self) -> TextInputResponse {
        TextInputRenderer::ensure_caret_visible(&mut self.state, &self.style, self.width);
        TextInputResponse::Changed
    }
}

impl Renderable for TextInput {
    fn render(&self, canvas: &Canvas) {
        self.render_at(canvas, self.width, self.height);
    }

    fn intrinsic_size(&self) -> Option<(f32, f32)> {
        Some((self.width, self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: &str) -> TextInput {
        TextInput::editing(value, TextInputStyle::default()).with_size(200.0, 24.0)
    }

    #[test]
    fn typing_over_the_initial_selection_replaces_it() {
        let mut i = input("Workspace 1");
        assert_eq!(
            i.on_key(TextInputKey::Char('M'), KeyMods::default()),
            TextInputResponse::Changed
        );
        assert_eq!(i.value(), "M");
    }

    #[test]
    fn enter_commits_and_escape_cancels() {
        let mut i = input("name");
        assert_eq!(
            i.on_key(TextInputKey::Enter, KeyMods::default()),
            TextInputResponse::Commit
        );
        assert_eq!(
            i.on_key(TextInputKey::Escape, KeyMods::default()),
            TextInputResponse::Cancel
        );
        assert_eq!(i.value(), "name");
    }

    #[test]
    fn ctrl_arrows_move_by_word() {
        let mut i = input("one two three");
        i.on_key(TextInputKey::Home, KeyMods::default());
        i.on_key(
            TextInputKey::Right,
            KeyMods {
                shift: false,
                ctrl: true,
            },
        );
        assert_eq!(i.state.caret(), 3);
    }

    #[test]
    fn copy_and_cut_hand_text_to_the_host() {
        let mut i = input("hello");
        i.state.select_range(0..2);
        assert_eq!(
            i.on_key(TextInputKey::Copy, KeyMods::default()),
            TextInputResponse::Clipboard("he".into())
        );
        assert_eq!(i.value(), "hello");
        i.state.select_range(0..2);
        assert_eq!(
            i.on_key(TextInputKey::Cut, KeyMods::default()),
            TextInputResponse::Clipboard("he".into())
        );
        assert_eq!(i.value(), "llo");
    }

    #[test]
    fn password_fields_refuse_copy_and_cut() {
        let mut i = input("hunter2");
        i.state.password = true;
        i.state.select_all();
        assert_eq!(
            i.on_key(TextInputKey::Copy, KeyMods::default()),
            TextInputResponse::Ignored
        );
        assert_eq!(
            i.on_key(TextInputKey::Cut, KeyMods::default()),
            TextInputResponse::Ignored
        );
        assert_eq!(i.value(), "hunter2");
    }

    #[test]
    fn unfocused_fields_ignore_keys() {
        let mut i = input("abc");
        i.state.set_focused(false);
        assert_eq!(
            i.on_key(TextInputKey::Char('z'), KeyMods::default()),
            TextInputResponse::Ignored
        );
        assert_eq!(i.value(), "abc");
    }

    #[test]
    fn double_click_selects_word_triple_selects_all() {
        let mut i = input("one two");
        let x = TextInputRenderer::caret_x(&i.state, &i.style, i.width, 5);
        i.on_pointer_down(x, 2, false);
        assert_eq!(i.state.selected_text(), Some("two"));
        i.on_pointer_down(x, 3, false);
        assert_eq!(i.state.selected_text(), Some("one two"));
    }

    #[test]
    fn drag_extends_the_selection() {
        let mut i = input("abcdef");
        let start = TextInputRenderer::caret_x(&i.state, &i.style, i.width, 1);
        let end = TextInputRenderer::caret_x(&i.state, &i.style, i.width, 4);
        i.on_pointer_down(start, 1, false);
        i.on_pointer_drag(end);
        assert_eq!(i.state.selection(), 1..4);
        i.on_pointer_up();
        // After release, moving the pointer no longer changes the selection.
        i.on_pointer_drag(start);
        assert_eq!(i.state.selection(), 1..4);
    }

    #[test]
    fn caret_blinks_and_typing_resets_the_phase() {
        let mut i = input("a");
        assert!(i.caret_visible());
        i.tick(CARET_BLINK_PERIOD * 0.6);
        assert!(!i.caret_visible());
        i.on_key(TextInputKey::Char('b'), KeyMods::default());
        assert!(i.caret_visible());
    }
}
