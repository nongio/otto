//! Key presses, read from the keyboards' evdev nodes, turned into the label
//! the key overlay shows — `Super+Ctrl+Alt+Shift+K` for a chord, `K` for a
//! single key.
//!
//! Labels come from the physical key (evdev keycode) on a US layout, not from
//! the active xkb keymap: the overlay shows which keys the hands pressed, the
//! same on every layout.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// How long a label stays up once every key has been let go.
const LINGER: Duration = Duration::from_millis(1200);

// linux/input-event-codes.h — the modifier keys, by side.
const KEY_LEFTCTRL: u16 = 29;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_LEFTALT: u16 = 56;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;

/// Modifiers in the order they're written — Super first, as Otto's shortcut
/// config writes `Logo+Shift+B` — with the name for each.
const MODIFIERS: [(&[u16], &str); 4] = [
    (&[KEY_LEFTMETA, KEY_RIGHTMETA], "Super"),
    (&[KEY_LEFTCTRL, KEY_RIGHTCTRL], "Ctrl"),
    (&[KEY_LEFTALT, KEY_RIGHTALT], "Alt"),
    (&[KEY_LEFTSHIFT, KEY_RIGHTSHIFT], "Shift"),
];

/// The same modifiers as symbols, in the order symbols are written.
const MODIFIER_SYMBOLS: [(&[u16], &str); 4] = [
    (&[KEY_LEFTCTRL, KEY_RIGHTCTRL], "⌃"),
    (&[KEY_LEFTALT, KEY_RIGHTALT], "⌥"),
    (&[KEY_LEFTSHIFT, KEY_RIGHTSHIFT], "⇧"),
    (&[KEY_LEFTMETA, KEY_RIGHTMETA], "⌘"),
];

/// How chords are written on the overlay.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum KeyStyle {
    /// `Super+Shift+K`
    #[default]
    Words,
    /// `⇧⌘K`
    Symbols,
}

impl KeyStyle {
    pub fn parse(spec: &str) -> Option<Self> {
        match spec {
            "words" => Some(Self::Words),
            "symbols" => Some(Self::Symbols),
            _ => None,
        }
    }
}

fn is_modifier(code: u16) -> bool {
    MODIFIERS.iter().any(|(codes, _)| codes.contains(&code))
}

/// What a non-modifier key is called on the overlay, or `None` for keys it
/// doesn't show (media keys, brightness, and the like).
fn key_label(code: u16) -> Option<&'static str> {
    const LETTERS: [(u16, &str); 26] = [
        (30, "A"),
        (48, "B"),
        (46, "C"),
        (32, "D"),
        (18, "E"),
        (33, "F"),
        (34, "G"),
        (35, "H"),
        (23, "I"),
        (36, "J"),
        (37, "K"),
        (38, "L"),
        (50, "M"),
        (49, "N"),
        (24, "O"),
        (25, "P"),
        (16, "Q"),
        (19, "R"),
        (31, "S"),
        (20, "T"),
        (22, "U"),
        (47, "V"),
        (17, "W"),
        (45, "X"),
        (21, "Y"),
        (44, "Z"),
    ];
    if let Some((_, label)) = LETTERS.iter().find(|(c, _)| *c == code) {
        return Some(label);
    }
    Some(match code {
        1 => "Esc",
        2 => "1",
        3 => "2",
        4 => "3",
        5 => "4",
        6 => "5",
        7 => "6",
        8 => "7",
        9 => "8",
        10 => "9",
        11 => "0",
        12 => "-",
        13 => "=",
        14 => "⌫",
        15 => "⇥",
        26 => "[",
        27 => "]",
        28 | 96 => "↩",
        39 => ";",
        40 => "'",
        41 => "`",
        43 => "\\",
        51 => ",",
        52 => ".",
        53 => "/",
        57 => "Space",
        58 => "⇪",
        59 => "F1",
        60 => "F2",
        61 => "F3",
        62 => "F4",
        63 => "F5",
        64 => "F6",
        65 => "F7",
        66 => "F8",
        67 => "F9",
        68 => "F10",
        87 => "F11",
        88 => "F12",
        102 => "Home",
        103 => "↑",
        104 => "Page Up",
        105 => "←",
        106 => "→",
        107 => "End",
        108 => "↓",
        109 => "Page Down",
        111 => "⌦",
        _ => return None,
    })
}

#[derive(Default)]
pub struct KeyState {
    style: KeyStyle,
    /// Every key currently down that the overlay knows about, modifiers
    /// included. A label never expires while any of these is held.
    held: HashSet<u16>,
    /// The label on screen, if any.
    label: Option<String>,
    /// Whether `label` is modifiers alone, still waiting for the key that
    /// completes the chord.
    modifiers_only: bool,
    /// When the label last changed or a key was let go — the linger timer
    /// starts here.
    touched_at: Option<Instant>,
}

impl KeyState {
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn new(style: KeyStyle) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    /// The held modifiers, then `key` if there is one, in the overlay's
    /// style.
    fn chord(&self, key: Option<&str>) -> String {
        let held = |table: &[(&[u16], &'static str)]| -> Vec<&'static str> {
            table
                .iter()
                .filter(|(codes, _)| codes.iter().any(|c| self.held.contains(c)))
                .map(|(_, name)| *name)
                .collect()
        };
        match self.style {
            KeyStyle::Words => {
                let mut parts = held(&MODIFIERS);
                parts.extend(key);
                parts.join("+")
            }
            KeyStyle::Symbols => {
                let symbols = held(&MODIFIER_SYMBOLS).concat();
                match key {
                    // A key name made of words reads better set apart.
                    Some(key) if !symbols.is_empty() && key.chars().count() > 1 => {
                        format!("{symbols} {key}")
                    }
                    Some(key) => format!("{symbols}{key}"),
                    None => symbols,
                }
            }
        }
    }

    /// Feed one `EV_KEY` event: `value` is 1 for press, 0 for release, 2 for
    /// autorepeat. Returns whether the label changed.
    pub fn apply(&mut self, code: u16, value: i32) -> bool {
        if value == 2 {
            return false;
        }
        let pressed = value == 1;
        let now = Instant::now();

        if is_modifier(code) {
            if pressed {
                self.held.insert(code);
            } else {
                self.held.remove(&code);
            }
            self.touched_at = Some(now);
            // Modifiers show as they go down, so a chord builds up on screen;
            // once a key completes it, letting the modifiers go leaves the
            // whole chord up rather than peeling it back.
            let names = self.chord(None);
            if (pressed || self.modifiers_only) && !names.is_empty() {
                let changed = self.label.as_deref() != Some(names.as_str());
                self.label = Some(names);
                self.modifiers_only = true;
                return changed;
            }
            return false;
        }

        let Some(key) = key_label(code) else {
            return false;
        };
        self.touched_at = Some(now);
        if !pressed {
            self.held.remove(&code);
            return false;
        }
        self.held.insert(code);
        self.label = Some(self.chord(Some(key)));
        self.modifiers_only = false;
        true
    }

    /// Forget every held key and clear the label — for when the overlay is
    /// switched off, so nothing is stuck down when it comes back.
    pub fn reset(&mut self) {
        *self = Self::new(self.style);
    }

    /// Clear the label if it has lingered long enough. Returns whether it
    /// was cleared.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.label.is_none() || !self.held.is_empty() {
            return false;
        }
        match self.touched_at {
            Some(at) if now.duration_since(at) >= LINGER => {
                self.label = None;
                self.modifiers_only = false;
                true
            }
            _ => false,
        }
    }

    /// How long until `expire` has something to do, if a label is up.
    pub fn time_to_expiry(&self, now: Instant) -> Option<Duration> {
        if self.label.is_none() || !self.held.is_empty() {
            return None;
        }
        let at = self.touched_at?;
        Some(LINGER.saturating_sub(now.duration_since(at)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_builds_and_stays_after_release() {
        let mut keys = KeyState::new(KeyStyle::Words);
        keys.apply(KEY_LEFTMETA, 1);
        keys.apply(KEY_LEFTALT, 1);
        keys.apply(KEY_LEFTSHIFT, 1);
        assert_eq!(keys.label(), Some("Super+Alt+Shift"));
        keys.apply(37, 1);
        assert_eq!(keys.label(), Some("Super+Alt+Shift+K"));
        keys.apply(37, 0);
        keys.apply(KEY_LEFTSHIFT, 0);
        keys.apply(KEY_LEFTALT, 0);
        keys.apply(KEY_LEFTMETA, 0);
        assert_eq!(keys.label(), Some("Super+Alt+Shift+K"));
    }

    #[test]
    fn symbols_style() {
        let mut keys = KeyState::new(KeyStyle::Symbols);
        keys.apply(KEY_LEFTMETA, 1);
        keys.apply(KEY_LEFTSHIFT, 1);
        keys.apply(37, 1);
        assert_eq!(keys.label(), Some("⇧⌘K"));
        keys.apply(37, 0);
        keys.apply(57, 1);
        assert_eq!(keys.label(), Some("⇧⌘ Space"));
    }
}
