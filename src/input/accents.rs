//! Press and hold a letter for its accented forms.
//!
//! A held key that has accented forms does not repeat. The press reaches the
//! application at once and its release follows straight after, so the letter
//! is typed without delay and the application, which repeats keys itself,
//! sees a tap. If the key is still down once the repeat delay has passed, a
//! row of the letter's forms appears at the caret, each with a number. A
//! number, or the arrows and Return, picks one: `otto-emoji --replace`
//! presses Backspace and types the choice through a virtual keyboard of its
//! own, so the choice arrives whatever the keyboard layout. Escape closes the
//! row, and any other key closes it and goes through as usual.
//!
//! Which key it was is read from the keysym the layout produces, not from
//! the key's position, so the row follows the active layout. A key whose
//! character has no forms (anything outside the table below: punctuation,
//! which editors and drawn rules repeat, and every non-Latin script) repeats
//! as it always did.
//!
//! The hold belongs to the press and the window that took it: losing the
//! keyboard focus, or the keys (a VT switch), ends it and puts the row away.
//!
//! The row is only offered where the focused application reports a caret
//! through text-input-v3. That is where text is typed, and it keeps a game
//! that walks on a held key out of it.

use smithay::{
    backend::input::{InputTime, KeyState},
    input::keyboard::{FilterResult, Keycode, Keysym, ModifiersState},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        RegistrationToken,
    },
    utils::SERIAL_COUNTER as SCOUNTER,
    wayland::{seat::WaylandFocus, text_input::TextInputSeat},
};
use std::time::Duration;
use tracing::{debug, warn};

use crate::{config::Config, input::KeyAction, state::Backend, Otto};

/// The program that types the choice.
const TYPIST: &str = "otto-emoji";

/// The accented forms of `c`, most used first. Upper case follows from lower
/// case; a form with no single upper-case letter (ß) is left out.
pub fn forms(c: char) -> Option<Vec<char>> {
    if let Some(forms) = lower_forms(c) {
        return Some(forms.chars().collect());
    }
    let lower = c.to_lowercase().next().filter(|l| *l != c)?;
    let forms: Vec<char> = lower_forms(lower)?
        .chars()
        .filter_map(|form| {
            let mut upper = form.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => Some(u),
                _ => None,
            }
        })
        .collect();
    (!forms.is_empty()).then_some(forms)
}

fn lower_forms(c: char) -> Option<&'static str> {
    Some(match c {
        'a' => "àáâäæãåā",
        'c' => "çćč",
        'd' => "ďđð",
        'e' => "èéêëēėę",
        'g' => "ğ",
        'i' => "îïíīįì",
        'l' => "łľ",
        'n' => "ñń",
        'o' => "ôöòóœøōõ",
        'r' => "ř",
        's' => "ßśš",
        't' => "ťþ",
        'u' => "ûüùúūů",
        'y' => "ÿý",
        'z' => "žźż",
        _ => return None,
    })
}

/// A press-and-hold in progress, and the row once it is up.
#[derive(Debug, Default)]
pub struct AccentState {
    /// The key whose release the application has already been sent, until
    /// the real one comes and is dropped.
    held: Option<Keycode>,
    /// Opens the row if `held` is still down when it fires.
    timer: Option<RegistrationToken>,
    /// A press the filter let through that starts a hold once delivered.
    starting: Option<(Keycode, char)>,
    row: Option<Row>,
}

#[derive(Debug)]
struct Row {
    forms: Vec<char>,
    selected: Option<usize>,
    scale: f32,
}

/// Where the row goes: the caret and the output's size, both in physical
/// pixels relative to the output, and the output's scale.
struct Placement {
    caret_px: (f32, f32, f32, f32),
    output_px: (f32, f32),
    scale: f32,
}

impl<BackendData: Backend> Otto<BackendData> {
    /// End the hold and put the row away, for when the press it belongs to
    /// can no longer be followed: the focus moved, or the keys were lost.
    ///
    /// Returns the key whose release the application already had, if any.
    pub(crate) fn reset_accent_hold(&mut self) -> Option<Keycode> {
        self.cancel_accent_timer();
        self.accent.starting = None;
        self.close_accent_row();
        self.accent.held.take()
    }

    /// Close the row without picking, as a click elsewhere does.
    pub(crate) fn close_accent_row(&mut self) {
        if self.accent.row.take().is_some() {
            self.workspaces.accent_picker.hide();
        }
    }

    fn cancel_accent_timer(&mut self) {
        if let Some(token) = self.accent.timer.take() {
            self.handle.remove(token);
        }
    }
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Offer a key to the accent picker, from inside the keyboard filter.
    ///
    /// `Some` is what the filter returns for it; `None` lets the key go on
    /// through the filter. A press that starts a hold is let through and
    /// remembered, for [`Self::accent_key_delivered`] to finish.
    pub(crate) fn accent_key(
        &mut self,
        keycode: Keycode,
        state: KeyState,
        keysym: Keysym,
        modifiers: &ModifiersState,
        suppressed_keys: &mut Vec<Keysym>,
    ) -> Option<FilterResult<KeyAction>> {
        if state == KeyState::Released {
            if self.accent.held != Some(keycode) {
                return None;
            }
            // The application had this release when the key went down.
            self.accent.held = None;
            self.cancel_accent_timer();
            return Some(FilterResult::Intercept(KeyAction::None));
        }

        // Any press ends a hold in progress.
        self.cancel_accent_timer();

        if let Some(row) = &self.accent.row {
            let count = row.forms.len();
            let selected = row.selected;
            let picked = match keysym {
                Keysym::Escape => {
                    self.close_accent_row();
                    suppressed_keys.push(keysym);
                    return Some(FilterResult::Intercept(KeyAction::None));
                }
                Keysym::Left | Keysym::Right => {
                    let next = match (keysym, selected) {
                        (Keysym::Left, Some(i)) => i.saturating_sub(1),
                        (Keysym::Left, None) => count - 1,
                        (_, Some(i)) => (i + 1).min(count - 1),
                        (_, None) => 0,
                    };
                    self.select_accent(next);
                    suppressed_keys.push(keysym);
                    return Some(FilterResult::Intercept(KeyAction::None));
                }
                Keysym::Return | Keysym::KP_Enter => selected,
                _ => digit(keycode, keysym)
                    .filter(|d| (1..=count).contains(d))
                    .map(|d| d - 1),
            };
            if let Some(index) = picked {
                self.pick_accent(index);
                suppressed_keys.push(keysym);
                return Some(FilterResult::Intercept(KeyAction::None));
            }
            // Shift, Caps Lock and the like leave the row up.
            if keysym.is_modifier_key() {
                return None;
            }
            self.close_accent_row();
        }

        if modifiers.ctrl || modifiers.alt || modifiers.logo {
            return None;
        }
        let character = smithay::input::keyboard::xkb::keysym_to_utf32(keysym);
        let character = char::from_u32(character).filter(|c| forms(*c).is_some())?;
        if !Config::with(|c| c.input.press_and_hold_accents) || self.accent_placement().is_none() {
            return None;
        }
        self.accent.starting = Some((keycode, character));
        None
    }

    /// After a key went through the filter: if its press started a hold and
    /// reached the application, send its release now and wait for the row.
    pub(crate) fn accent_key_delivered(&mut self, delivered: bool, time: InputTime) {
        let Some((keycode, character)) = self.accent.starting.take() else {
            return;
        };
        // Something later in the filter (a shortcut) kept the press.
        if !delivered {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        keyboard.input_forward(
            self,
            keycode,
            KeyState::Released,
            SCOUNTER.next_serial(),
            time,
            false,
        );
        self.accent.held = Some(keycode);

        let delay =
            Duration::from_millis(Config::with(|c| c.keyboard_repeat_delay).max(200) as u64);
        match self
            .handle
            .insert_source(Timer::from_duration(delay), move |_, _, state| {
                state.accent.timer = None;
                if state.accent.held == Some(keycode) {
                    state.open_accent_row(character);
                }
                TimeoutAction::Drop
            }) {
            Ok(token) => self.accent.timer = Some(token),
            Err(err) => warn!(err = %err.error, "could not time the accent hold"),
        }
    }

    fn open_accent_row(&mut self, character: char) {
        let (Some(forms), Some(placement)) = (forms(character), self.accent_placement()) else {
            return;
        };
        self.workspaces.accent_picker.show(
            &forms,
            None,
            placement.caret_px,
            placement.output_px,
            placement.scale,
        );
        self.accent.row = Some(Row {
            forms,
            selected: None,
            scale: placement.scale,
        });
    }

    fn select_accent(&mut self, index: usize) {
        if let Some(row) = &mut self.accent.row {
            row.selected = Some(index);
            self.workspaces
                .accent_picker
                .select(&row.forms, row.selected, row.scale);
        }
    }

    fn pick_accent(&mut self, index: usize) {
        let Some(choice) = self
            .accent
            .row
            .as_ref()
            .and_then(|r| r.forms.get(index).copied())
        else {
            return;
        };
        self.close_accent_row();
        let program = typist_program();
        debug!(%choice, %program, "replacing the held letter");
        let mut command = self.program_command(&program, &["--replace".into(), choice.to_string()]);
        match command.spawn() {
            Ok(child) => crate::input::actions::reap_in_background(&program, child),
            Err(err) => warn!(%program, %err, "could not type the accent"),
        }
    }

    /// Where the row would go for the focused application, or `None` when
    /// it reports no caret. Only on the primary output, where the overlay is.
    fn accent_placement(&self) -> Option<Placement> {
        let (caret_surface, _) = self.seat.text_input().cursor_rectangle()?;
        let focus = self.seat.get_keyboard()?.current_focus()?;
        if focus.wl_surface().as_deref() != Some(&caret_surface) {
            return None;
        }
        let caret = self.text_cursor_rectangle()?;
        let output = self.workspaces.primary_output()?;
        let geometry = self.workspaces.output_geometry(output)?;
        if !geometry.contains(caret.loc) {
            return None;
        }
        let scale = output.current_scale().fractional_scale() as f32;
        let x_px = (caret.loc.x - geometry.loc.x) as f32 * scale;
        let y_px = (caret.loc.y - geometry.loc.y) as f32 * scale;
        Some(Placement {
            caret_px: (
                x_px,
                y_px,
                caret.size.w as f32 * scale,
                caret.size.h as f32 * scale,
            ),
            output_px: (
                geometry.size.w as f32 * scale,
                geometry.size.h as f32 * scale,
            ),
            scale,
        })
    }
}

/// The typist beside the running compositor when there is one, so an
/// uninstalled build runs its own; otherwise found through `$PATH`.
fn typist_program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(TYPIST)))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| TYPIST.to_string())
}

/// The number a digit key stands for, on the top row or the keypad.
///
/// The digit the layout types comes first. Failing that, the top row counts
/// by position (evdev `KEY_1`..`KEY_9`), since AZERTY types `&`, `é`, … there
/// without Shift.
fn digit(keycode: Keycode, keysym: Keysym) -> Option<usize> {
    let raw = keysym.raw();
    if (Keysym::_1.raw()..=Keysym::_9.raw()).contains(&raw) {
        return Some((raw - Keysym::_0.raw()) as usize);
    }
    if (Keysym::KP_1.raw()..=Keysym::KP_9.raw()).contains(&raw) {
        return Some((raw - Keysym::KP_0.raw()) as usize);
    }
    // xkb keycodes are evdev codes plus 8; KEY_1 is evdev 2.
    let evdev = keycode.raw().checked_sub(8)?;
    (2..=10).contains(&evdev).then(|| evdev as usize - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_have_their_forms() {
        assert_eq!(forms('e').unwrap()[1], 'é');
        assert_eq!(forms('n'), Some(vec!['ñ', 'ń']));
        assert!(forms('x').is_none());
        assert!(forms('ж').is_none());
    }

    #[test]
    fn upper_case_follows_lower_case() {
        assert_eq!(forms('E').unwrap()[1], 'É');
        // ß has no single upper-case letter.
        assert_eq!(forms('S'), Some(vec!['Ś', 'Š']));
    }

    /// An xkb keycode from an evdev one.
    fn key(evdev: u32) -> Keycode {
        Keycode::new(evdev + 8)
    }

    // evdev codes.
    const KEY_1: u32 = 2;
    const KEY_2: u32 = 3;
    const KEY_9: u32 = 10;
    const KEY_0: u32 = 11;
    const KEY_A: u32 = 30;
    const KEY_KP8: u32 = 72;

    #[test]
    fn digits_from_both_rows() {
        assert_eq!(digit(key(KEY_1), Keysym::_1), Some(1));
        assert_eq!(digit(key(KEY_KP8), Keysym::KP_8), Some(8));
        assert_eq!(digit(key(KEY_0), Keysym::_0), None);
        assert_eq!(digit(key(KEY_A), Keysym::a), None);
    }

    #[test]
    fn top_row_counts_by_position_on_any_layout() {
        // AZERTY, unshifted.
        assert_eq!(digit(key(KEY_1), Keysym::ampersand), Some(1));
        assert_eq!(digit(key(KEY_2), Keysym::eacute), Some(2));
        assert_eq!(digit(key(KEY_9), Keysym::ccedilla), Some(9));
        assert_eq!(digit(key(KEY_0), Keysym::agrave), None);
        // The keypad without Num Lock is not a digit.
        assert_eq!(digit(key(KEY_KP8), Keysym::KP_Up), None);
    }

    #[test]
    fn the_digit_the_layout_types_wins_over_position() {
        // A layout that types 3 on the key in the 1 position.
        assert_eq!(digit(key(KEY_1), Keysym::_3), Some(3));
    }

    #[test]
    fn punctuation_keeps_repeating() {
        for c in ['-', '.', '\'', '"', '!', '?', '$'] {
            assert!(forms(c).is_none(), "{c:?} has no forms");
        }
    }
}
