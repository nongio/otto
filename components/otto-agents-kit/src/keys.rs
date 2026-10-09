//! The keys the launcher's field and list answer to.
//!
//! The launcher's agents mode and the side canvas's session list both put a
//! search field over a list of rows, and both answer the same keys the same
//! way: the arrows, Tab and Ctrl+N/P walk the rows, the page keys walk a
//! page of them, and everything else edits the field. The walking and the
//! editing live here so the two cannot drift apart. What Enter, Escape and
//! the other keys with a meaning of their own do is up to each host, which
//! checks them before handing the key to [`edit_field`].

// Rust guideline compliant 2026-02-21

use otto_kit::clipboard;
use otto_kit::components::text_input::{self, KeyMods, TextInput, TextInputKey, TextInputResponse};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

use crate::rows::MAX_ROWS;

/// The letter of a Ctrl combination, as `'a'` for Ctrl+A.
///
/// Ctrl combinations arrive as control characters rather than with a
/// modifier flag, which is enough to recognise them by. Return, newline and
/// Tab are keys of their own, not combinations.
pub fn control_char(event: &KeyEvent) -> Option<char> {
    event
        .utf8
        .as_deref()
        .and_then(|text| text.chars().next())
        .filter(|c| (*c as u32) < 0x20 && *c != '\r' && *c != '\n' && *c != '\t')
        .map(|c| char::from(c as u8 + 0x60))
}

/// How far a key moves the highlight through the rows, if it is one of the
/// keys that walk them.
///
/// Down, Ctrl+N and Tab go one row down; Up, Ctrl+P, Shift+Tab one row up;
/// Page Down and Page Up a page ([`MAX_ROWS`] rows) either way. `shift` is
/// whether Shift is held, which turns Tab around.
pub fn list_step(keysym: Keysym, control: Option<char>, shift: bool) -> Option<isize> {
    let page = MAX_ROWS as isize;
    match (keysym, control) {
        (Keysym::Down, _) | (_, Some('n')) => Some(1),
        (Keysym::Up, _) | (_, Some('p')) => Some(-1),
        (Keysym::Tab, _) => Some(if shift { -1 } else { 1 }),
        (Keysym::ISO_Left_Tab, _) => Some(-1),
        (Keysym::Page_Down, _) => Some(page),
        (Keysym::Page_Up, _) => Some(-page),
        _ => None,
    }
}

/// The row `delta` rows from `selected`, in a list of `count`.
///
/// Wrapping, because a list that stops at the end makes someone check where
/// the end was. An empty list stays at `selected`.
pub fn wrap(selected: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return selected;
    }
    let count = count as isize;
    (selected as isize + delta).rem_euclid(count) as usize
}

/// What an edit did to the field, for the host to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldEdit {
    /// The text changed: filter again.
    Changed,
    /// Only the caret or the selection moved: draw again.
    Moved,
    /// Enter, which the field does not handle itself.
    Commit,
    /// Escape, which the field does not handle itself.
    Cancel,
    /// Nothing to act on: a key the field has no use for, or one it took
    /// without changing anything.
    None,
}

/// Put `text` on the clipboard, so that it is still there afterwards.
///
/// The offer is made here first, which is what makes a paste work while the
/// program is still up. But a Wayland selection dies with the client that
/// made it, and the launcher or the canvas is one keystroke from closing, so
/// `wl-copy`, which forks and stays to serve the offer, is handed the same
/// text and takes the selection over. Without it the copy still works until
/// the program goes, which is better than refusing to copy at all. `serial`
/// is the key press that asked for the copy.
pub fn copy_to_clipboard(text: &str, serial: u32) {
    clipboard::set_text(text, serial);
    if let Err(err) = std::process::Command::new("wl-copy").arg(text).spawn() {
        tracing::debug!(%err, "wl-copy is not available: the copy lasts as long as the program");
    }
}

/// Edit `input` with a key press.
///
/// Beyond the keys every otto-kit field shares (see
/// [`text_input::key_for`]): Ctrl+U clears the field, the fastest way to
/// start a different search; Ctrl+A selects everything in it; Ctrl+C and
/// Ctrl+X copy and cut to the system clipboard and Ctrl+V pastes from it;
/// Ctrl+W deletes the word before the caret. `control` is
/// [`control_char`] of the event, `shift` whether Shift is held, and
/// `serial` the press, for the clipboard.
pub fn edit_field(
    input: &mut TextInput,
    event: &KeyEvent,
    control: Option<char>,
    shift: bool,
    serial: u32,
) -> FieldEdit {
    match control {
        Some('u') => {
            input.set_value("");
            return FieldEdit::Changed;
        }
        Some('a') => {
            input.on_key(TextInputKey::SelectAll, KeyMods::default());
            return FieldEdit::Moved;
        }
        Some(letter @ ('c' | 'x')) => {
            let cut = letter == 'x';
            let key = if cut {
                TextInputKey::Cut
            } else {
                TextInputKey::Copy
            };
            if let TextInputResponse::Clipboard(text) = input.on_key(key, KeyMods::default()) {
                copy_to_clipboard(&text, serial);
            }
            return if cut {
                FieldEdit::Changed
            } else {
                FieldEdit::Moved
            };
        }
        Some('v') => {
            return match clipboard::text() {
                Some(text) => {
                    input.on_key(TextInputKey::Paste(text), KeyMods::default());
                    FieldEdit::Changed
                }
                None => FieldEdit::None,
            };
        }
        Some('w') => {
            input.on_key(TextInputKey::Backspace, KeyMods::word());
            return FieldEdit::Changed;
        }
        _ => {}
    }

    // Everything else is the field's, with the keys every otto-kit field
    // shares: Alt or Ctrl with the arrows and Backspace for a word at a
    // time, Alt+B/F/D, Ctrl+E/K, Cmd with the arrows for the ends.
    let mods = KeyMods {
        shift,
        ..KeyMods::from(otto_kit::AppContext::current_modifiers())
    };
    let Some((key, mods)) = text_input::key_for(event.keysym, event.utf8.as_deref(), mods) else {
        return FieldEdit::None;
    };
    match input.on_key(key, mods) {
        TextInputResponse::Changed => FieldEdit::Changed,
        TextInputResponse::Moved => FieldEdit::Moved,
        TextInputResponse::Commit => FieldEdit::Commit,
        TextInputResponse::Cancel => FieldEdit::Cancel,
        TextInputResponse::Ignored | TextInputResponse::Clipboard(_) => FieldEdit::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_wraps_at_either_end() {
        assert_eq!(wrap(0, -1, 3), 2);
        assert_eq!(wrap(2, 1, 3), 0);
        assert_eq!(wrap(1, 1, 3), 2);
    }

    #[test]
    fn walking_an_empty_list_stays_put() {
        assert_eq!(wrap(0, 1, 0), 0);
    }

    #[test]
    fn a_page_is_the_rows_the_card_shows() {
        assert_eq!(
            list_step(Keysym::Page_Down, None, false),
            Some(MAX_ROWS as isize)
        );
        assert_eq!(
            list_step(Keysym::Page_Up, None, false),
            Some(-(MAX_ROWS as isize))
        );
    }

    #[test]
    fn shift_turns_tab_around() {
        assert_eq!(list_step(Keysym::Tab, None, false), Some(1));
        assert_eq!(list_step(Keysym::Tab, None, true), Some(-1));
        assert_eq!(list_step(Keysym::ISO_Left_Tab, None, false), Some(-1));
    }

    #[test]
    fn ctrl_n_and_p_walk_as_the_arrows_do() {
        assert_eq!(list_step(Keysym::n, Some('n'), false), Some(1));
        assert_eq!(list_step(Keysym::p, Some('p'), false), Some(-1));
        assert_eq!(list_step(Keysym::a, None, false), None);
    }
}
