# Accents on hold

**Status:** draft  
**Related specs:** [emoji-picker.md](emoji-picker.md) (typing through a virtual keyboard, the caret)

## Summary

Holding a key whose character has accented or related forms shows those forms in a row at the text cursor; picking one replaces the character the key typed. It is how accented letters are typed without a dead key, Compose or a second layout.

## Goals

- A held letter with forms (`e`, `a`, `n`, …) shows its forms at the caret after the repeat delay, numbered from 1.
- A number, or the arrows and Return, picks a form, and the picked form replaces the typed letter in the focused application.
- Typing speed is unaffected: a tap types its letter as soon as the key goes down.
- Works with any Latin keyboard layout, and in Chromium-based applications.

## Non-Goals

- Per-language ordering of the forms (one table for every layout).
- Non-Latin scripts, dead keys, Compose, input methods.
- Picking with the pointer.
- Keys outside the table: they keep repeating. Punctuation is left out of it, so a held `-` still draws a rule and editor motions (`.`, `$`, …) still repeat.

## Behavior

- **Eligible key:** a press with no Ctrl, Alt or Super held, whose character (as produced by the active layout, Shift and Caps Lock included) is in the table, while the application with keyboard focus has reported a text cursor through text-input-v3, and the setting `input.press_and_hold_accents` is on (default).
- **Press of an eligible key:** the application receives the press and, immediately after, a release. The character is typed once and never auto-repeats. The real release, when it comes, is not delivered.
- **Hold:** if the key is still down after the repeat delay (`keyboard_repeat_delay`, at least 200 ms) and no other key was pressed meanwhile, the row appears above the caret, centred on it, kept on screen, and below the caret when there is no room above. No form is selected.
- **While the row is up:**
  - `1`–`9` picks the form with that number, if there is one: a key that types that digit (top row or keypad), or else the top-row key in that position (evdev `KEY_1`–`KEY_9`), so AZERTY picks without Shift.
  - Left/Right moves the selection; Return picks the selected form, or with nothing selected closes the row and is delivered.
  - Escape closes the row and is not delivered.
  - Modifier keys leave the row up.
  - Any other key closes the row and is handled as usual (it may start a new hold).
  - A pointer button press closes the row.
  - None of the keys the row handles reach the application, press or release.
- **End of the hold:** a keyboard focus change (another window mapping, a click elsewhere, the focus going to no window) or losing the keys (`release_all_keys`: a VT switch, the host window losing focus) ends the hold: the row closes, the pending timer is cancelled, and the key is no longer held. A digit typed after a focus change goes to the newly focused window untouched. The held key's release, already sent, is not sent again when the keys are released.
- **Pick:** the row closes, and the application receives Backspace followed by the picked character, typed through a virtual keyboard with a keymap of its own (`otto-emoji --replace`).
- **Upper case:** a held upper-case letter offers the upper-case forms; a form with no single upper-case letter (ß) is left out.

## Constraints & Edge Cases

- Applications repeat keys themselves from `wl_keyboard.repeat_info`, so the compositor cannot cancel a repeat already started: the early release is what stops it.
- Applications with no caret (games, X11 applications, terminals without text-input) get the key untouched, so a held key still repeats or moves a character.
- The caret must belong to the surface with keyboard focus and lie on the primary output, where the overlay is drawn.
- Backspace is sent on evdev 14, which every layout and Chromium's fixed key table read as Backspace; the character goes on a printable keycode with a `U+XXXX` keysym, which Chromium honours for characters in the Basic Multilingual Plane (every form in the table is).

## Rationale

- Forward-then-replace rather than hold-back-then-decide: holding every eligible press back until its release would add latency to every letter and reorder overlapping keystrokes in fast typing.
- Typing through a separate virtual-keyboard client rather than the seat keymap: the compositor cannot send one client a keymap of its own, and swapping the seat's keymap would reach every client.
- Not through text-input `commit_string`: it reaches only clients with text-input enabled, and it would need the compositor to act as the seat's input method, which otto-dictate may already be.
- The caret as the gate: it is where text is typed, and it keeps games that walk on a held key out.

## Open Questions

- Order the forms by the active layout's language.
- Show the row on outputs other than the primary.
