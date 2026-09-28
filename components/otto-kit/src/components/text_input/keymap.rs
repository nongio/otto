//! The keys every text field answers to, in one place.
//!
//! Hosts receive keys in their own terms (a `wl_keyboard` event in a client,
//! the seat's xkb state in the compositor) but both end in an xkb keysym, some
//! text, and a set of held modifiers. [`key_for`] turns that into the edit
//! [`TextInput::on_key`](super::TextInput::on_key) understands, so a field in
//! the launcher moves the caret the same way as one in Files or Settings.
//!
//! What a field answers to, beyond plain typing and the arrows:
//!
//! | Keys                           | Edit                                  |
//! |--------------------------------|---------------------------------------|
//! | Ctrl/Alt + ←/→                 | a word left/right (Shift extends)     |
//! | Alt+B / Alt+F                  | a word left/right, as in a terminal   |
//! | Cmd + ←/→, Home/End, Ctrl+E    | start/end of the value                |
//! | Ctrl/Alt + Backspace, Ctrl+W   | delete the word before the caret      |
//! | Ctrl/Alt + Delete, Alt+D       | delete the word after the caret       |
//! | Cmd+Backspace, Ctrl+U          | delete to the start                   |
//! | Cmd+Delete, Ctrl+K             | delete to the end                     |
//! | Ctrl+A                         | select all                            |
//! | Ctrl+C / Ctrl+X                | copy / cut                            |
//!
//! Paste needs the clipboard, which only the host has, so Ctrl+V comes back
//! as `None` for the host to answer. So does any key a host gives a meaning
//! of its own first — a host checks its own keys before calling this.

use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};

use super::text_input::{KeyMods, TextInputKey};

/// The held modifiers as a client's keyboard reports them.
impl From<Modifiers> for KeyMods {
    fn from(modifiers: Modifiers) -> Self {
        Self {
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            logo: modifiers.logo,
        }
    }
}

/// Translate a key press into a field edit, with the modifiers the edit is
/// made with. `text` is what the keymap produced for the key, if the host has
/// it; without it the keysym's own character is used. `None` means the key is
/// not an edit — a shortcut, a bare modifier, or a key the field has no use
/// for.
pub fn key_for(
    keysym: Keysym,
    text: Option<&str>,
    mods: KeyMods,
) -> Option<(TextInputKey, KeyMods)> {
    let shift = KeyMods {
        shift: mods.shift,
        ..KeyMods::default()
    };
    let word = KeyMods {
        ctrl: true,
        ..shift
    };
    let line = KeyMods {
        logo: true,
        ..shift
    };

    let edit = match keysym {
        Keysym::Left | Keysym::KP_Left => (TextInputKey::Left, mods),
        Keysym::Right | Keysym::KP_Right => (TextInputKey::Right, mods),
        Keysym::Home | Keysym::KP_Home => (TextInputKey::Home, mods),
        Keysym::End | Keysym::KP_End => (TextInputKey::End, mods),
        Keysym::BackSpace => (TextInputKey::Backspace, mods),
        Keysym::Delete | Keysym::KP_Delete => (TextInputKey::Delete, mods),
        Keysym::Return | Keysym::KP_Enter => (TextInputKey::Enter, mods),
        Keysym::Escape => (TextInputKey::Escape, mods),
        _ => {
            let letter = keysym.key_char().map(|c| c.to_ascii_lowercase());
            if mods.alt && !mods.ctrl && !mods.logo {
                // The readline word keys. Shift still extends a selection,
                // which a terminal has no use for but a field does.
                match letter {
                    Some('b') => (TextInputKey::Left, word),
                    Some('f') => (TextInputKey::Right, word),
                    Some('d') => (TextInputKey::Delete, word),
                    _ => return None,
                }
            } else if mods.ctrl && !mods.alt && !mods.logo {
                match letter {
                    Some('a') => (TextInputKey::SelectAll, KeyMods::default()),
                    Some('c') => (TextInputKey::Copy, KeyMods::default()),
                    Some('x') => (TextInputKey::Cut, KeyMods::default()),
                    Some('e') => (TextInputKey::End, line),
                    Some('w') => (TextInputKey::Backspace, KeyMods::word()),
                    Some('u') => (TextInputKey::Backspace, KeyMods::line()),
                    Some('k') => (TextInputKey::Delete, KeyMods::line()),
                    _ => return None,
                }
            } else if mods.ctrl || mods.alt || mods.logo {
                return None;
            } else {
                let typed: String = match text {
                    Some(text) => text.chars().filter(|c| !c.is_control()).collect(),
                    None => keysym
                        .key_char()
                        .filter(|c| !c.is_control())
                        .into_iter()
                        .collect(),
                };
                if typed.is_empty() {
                    return None;
                }
                (TextInputKey::Text(typed), KeyMods::default())
            }
        }
    };
    Some(edit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_input::{TextInput, TextInputStyle};

    fn alt() -> KeyMods {
        KeyMods {
            alt: true,
            ..KeyMods::default()
        }
    }

    fn ctrl() -> KeyMods {
        KeyMods {
            ctrl: true,
            ..KeyMods::default()
        }
    }

    fn logo() -> KeyMods {
        KeyMods {
            logo: true,
            ..KeyMods::default()
        }
    }

    /// Drive a field with the caret at `caret` through one key.
    fn press(
        value: &str,
        caret: usize,
        keysym: Keysym,
        text: Option<&str>,
        mods: KeyMods,
    ) -> TextInput {
        let mut input = TextInput::new(value, TextInputStyle::default());
        input.state.set_focused(true);
        input.state.set_caret(caret, false);
        if let Some((key, mods)) = key_for(keysym, text, mods) {
            input.on_key(key, mods);
        }
        input
    }

    #[test]
    fn alt_arrows_move_by_word() {
        assert_eq!(
            press("one two three", 0, Keysym::Right, None, alt())
                .state
                .caret(),
            3
        );
        assert_eq!(
            press("one two three", 13, Keysym::Left, None, alt())
                .state
                .caret(),
            8
        );
    }

    #[test]
    fn ctrl_arrows_move_by_word() {
        assert_eq!(
            press("one two three", 0, Keysym::Right, None, ctrl())
                .state
                .caret(),
            3
        );
    }

    #[test]
    fn cmd_arrows_reach_the_ends() {
        assert_eq!(
            press("one two three", 5, Keysym::Left, None, logo())
                .state
                .caret(),
            0
        );
        assert_eq!(
            press("one two three", 5, Keysym::Right, None, logo())
                .state
                .caret(),
            13
        );
    }

    #[test]
    fn shift_alt_arrow_extends_by_word() {
        let mods = KeyMods {
            shift: true,
            ..alt()
        };
        let input = press("one two three", 0, Keysym::Right, None, mods);
        assert_eq!(input.state.selection(), 0..3);
    }

    #[test]
    fn readline_word_keys() {
        assert_eq!(
            press("one two three", 13, Keysym::b, Some("b"), alt())
                .state
                .caret(),
            8
        );
        assert_eq!(
            press("one two three", 0, Keysym::f, Some("f"), alt())
                .state
                .caret(),
            3
        );
        assert_eq!(
            press("one two three", 4, Keysym::d, Some("d"), alt()).value(),
            "one  three"
        );
    }

    #[test]
    fn word_deletes() {
        assert_eq!(
            press("one two", 7, Keysym::BackSpace, None, alt()).value(),
            "one "
        );
        assert_eq!(press("one two", 7, Keysym::w, None, ctrl()).value(), "one ");
        assert_eq!(
            press("one two", 0, Keysym::Delete, None, ctrl()).value(),
            " two"
        );
    }

    #[test]
    fn line_deletes_and_moves() {
        assert_eq!(press("one two", 4, Keysym::u, None, ctrl()).value(), "two");
        assert_eq!(press("one two", 4, Keysym::k, None, ctrl()).value(), "one ");
        assert_eq!(
            press("one two", 4, Keysym::BackSpace, None, logo()).value(),
            "two"
        );
        assert_eq!(
            press("one two", 0, Keysym::e, None, ctrl()).state.caret(),
            7
        );
    }

    #[test]
    fn modified_letters_never_type() {
        assert_eq!(key_for(Keysym::z, Some("z"), alt()), None);
        assert_eq!(key_for(Keysym::z, Some("\u{1a}"), ctrl()), None);
        assert_eq!(key_for(Keysym::v, Some("\u{16}"), ctrl()), None);
    }

    #[test]
    fn plain_text_types() {
        assert_eq!(
            press("", 0, Keysym::A, Some("A"), KeyMods::shift(true)).value(),
            "A"
        );
        assert_eq!(
            press("", 0, Keysym::eacute, None, KeyMods::default()).value(),
            "é"
        );
    }
}
