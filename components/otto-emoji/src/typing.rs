//! Typing a pick into whichever window has the keyboard.
//!
//! The picker cannot write into another application's text field: Wayland
//! gives a client no way to reach another. What it can do is be a keyboard.
//! `zwp-virtual-keyboard-v1` lets a client press keys on the seat, and a key
//! means whatever the keymap says it means — so the picker uploads a keymap of
//! its own, one key per codepoint of the emoji, and taps them in order. The
//! compositor hands the focused client that keymap before the keys, and the
//! client turns each key into the character it names, exactly as it would
//! for a physical keyboard with an emoji on every keycap. This is how `wtype`
//! types Unicode, and every toolkit that takes a keyboard takes it.
//!
//! It runs on a connection of its own, after the picker's surface is gone:
//! the keys have to land where the keyboard was *before* the picker took it,
//! and that is where the compositor sends it back once the picker lets go.
//!
//! There is a second route, [`press_paste`], for applications where typing
//! cannot work. Chromium — and so Electron, and every HTML input — turns a
//! key into a character from its keysym rather than from the UTF-8 the
//! keymap provides, and truncates that keysym to sixteen bits on the way:
//! an emoji at U+1F600 arrives as U+F600, in the Private Use Area. Nothing
//! above the Basic Multilingual Plane survives, and that is where the emoji
//! are. This is a Chromium bug (electron/electron#49894), not a compositor's
//! to fix, and every picker on Wayland lands on the same answer: put the text
//! on the clipboard and press paste.

use std::io::Write;
use std::os::fd::AsFd;
use std::time::Instant;

use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat::WlSeat};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

#[derive(Default)]
struct Globals {
    seat: Option<WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(Globals: ignore WlSeat);
delegate_noop!(Globals: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ZwpVirtualKeyboardV1);

/// The evdev codes the synthetic keys are put on: the printable keys of a
/// standard keyboard — the digit row, and the three letter rows.
///
/// The choice is not free, and this is the whole reason the picker types into
/// Chromium at all. A key's *meaning* comes from the keymap, but Chromium and
/// everything built on it (Electron, and so every HTML input) decide what a
/// key **is** from the raw evdev code, through a fixed table, before they look
/// at the keymap. A code outside that table is dropped, and a code that maps
/// to a non-printable key — evdev 1 is Escape — yields no text for anything
/// but plain ASCII. Put the keys on codes the table calls printable and the
/// keysym is honoured, whatever it is.
///
/// Verified against Chromium: on evdev 1 an emoji produced nothing, on 200
/// nothing at all arrived, and on these the character lands.
const PRINTABLE_KEYS: [u32; 46] = [
    // 1234567890-=
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, //
    // qwertyuiop[]
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, //
    // asdfghjkl;'`
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, //
    // \zxcvbnm,./
    43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
];

/// How many codepoints can be typed in one go.
pub const MAX_CODEPOINTS: usize = PRINTABLE_KEYS.len();

/// An xkb keymap with one key per character of `text`, in order.
///
/// Keycodes start at 8 in xkb and at 0 on the wire, so the first character
/// is key 1 here and `key(1)` on the virtual keyboard. `U+XXXX` keysyms are
/// how xkb spells an arbitrary codepoint, so nothing in the text needs a
/// name of its own.
pub fn keymap_for(text: &str) -> String {
    let count = text.chars().count().clamp(1, MAX_CODEPOINTS);
    let mut keymap = String::new();
    keymap.push_str("xkb_keymap {\n");
    // xkb keycodes are evdev codes plus eight.
    keymap.push_str("xkb_keycodes \"otto-emoji\" {\n  minimum = 8;\n  maximum = 255;\n");
    for (index, code) in PRINTABLE_KEYS.iter().take(count).enumerate() {
        keymap.push_str(&format!("  <K{}> = {};\n", index + 1, code + 8));
    }
    keymap.push_str("};\n");
    keymap.push_str(
        "xkb_types \"otto-emoji\" {\n  type \"ONE_LEVEL\" {\n    modifiers = none;\n    level_name[Level1] = \"Any\";\n  };\n};\n",
    );
    keymap.push_str("xkb_compatibility \"otto-emoji\" {\n};\n");
    keymap.push_str("xkb_symbols \"otto-emoji\" {\n");
    for (index, c) in text.chars().take(count).enumerate() {
        keymap.push_str(&format!(
            "  key <K{}> {{ [ U{:04X} ] }};\n",
            index + 1,
            c as u32
        ));
    }
    keymap.push_str("};\n};\n");
    keymap
}

/// Type `text` into the focused window. Blocks for the few round trips it
/// takes, which is a handful of milliseconds.
pub fn type_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());

    let mut globals = Globals::default();
    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    let seat = globals.seat.clone().ok_or("the compositor has no seat")?;
    let manager = globals
        .manager
        .clone()
        .ok_or("the compositor does not offer zwp-virtual-keyboard-v1")?;

    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
    upload(&keyboard, &keymap_for(text))?;

    let started = Instant::now();
    for key in PRINTABLE_KEYS.iter().take(text.chars().count()) {
        let time = started.elapsed().as_millis() as u32;
        keyboard.key(time, *key, wl_keyboard::KeyState::Pressed as u32);
        keyboard.key(time, *key, wl_keyboard::KeyState::Released as u32);
    }
    // The keys are requests like any other; a round trip is what makes sure
    // the compositor has them before the keyboard — and the process — goes.
    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    keyboard.destroy();
    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// Standard evdev codes for the paste chord.
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_V: u32 = 47;
/// xkb real modifier bits, as every standard keymap lays them out.
const MOD_SHIFT: u32 = 1 << 0;
const MOD_CONTROL: u32 = 1 << 2;

/// Press the paste shortcut in the focused window: Ctrl+V, or Ctrl+Shift+V
/// when `shift` is set, which is what terminals want.
///
/// Rides on the standard US layout rather than a keymap of its own — the
/// chord is made of real keys that mean the same everywhere. The modifier
/// state is sent explicitly through the protocol's own `modifiers` request,
/// so the window sees Ctrl held rather than a stray `v`.
pub fn press_paste(shift: bool) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());

    let mut globals = Globals::default();
    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    let seat = globals.seat.clone().ok_or("the compositor has no seat")?;
    let manager = globals
        .manager
        .clone()
        .ok_or("the compositor does not offer zwp-virtual-keyboard-v1")?;

    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
    upload(&keyboard, &standard_keymap()?)?;

    let started = Instant::now();
    let now = || started.elapsed().as_millis() as u32;
    let pressed = wl_keyboard::KeyState::Pressed as u32;
    let released = wl_keyboard::KeyState::Released as u32;
    let mut mods = MOD_CONTROL;
    if shift {
        mods |= MOD_SHIFT;
    }

    keyboard.key(now(), KEY_LEFTCTRL, pressed);
    if shift {
        keyboard.key(now(), KEY_LEFTSHIFT, pressed);
    }
    keyboard.modifiers(mods, 0, 0, 0);
    keyboard.key(now(), KEY_V, pressed);
    keyboard.key(now(), KEY_V, released);
    if shift {
        keyboard.key(now(), KEY_LEFTSHIFT, released);
    }
    keyboard.key(now(), KEY_LEFTCTRL, released);
    keyboard.modifiers(0, 0, 0, 0);

    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    keyboard.destroy();
    queue
        .roundtrip(&mut globals)
        .map_err(|err| err.to_string())?;
    Ok(())
}

/// The US layout as text, for the paste chord to be decoded against.
fn standard_keymap() -> Result<String, String> {
    use xkbcommon::xkb;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap =
        xkb::Keymap::new_from_names(&context, "", "", "us", "", None, xkb::COMPILE_NO_FLAGS)
            .ok_or("could not compile the standard keymap")?;
    Ok(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
}

/// Whether every character of `text` is inside the Basic Multilingual Plane,
/// and so survives Chromium's keysym truncation. Such text can be typed
/// anywhere; anything else needs the paste route in Chromium.
pub fn fits_in_bmp(text: &str) -> bool {
    text.chars().all(|c| (c as u32) <= 0xFFFF)
}

/// Hand the keymap over as a sealed memory file, which is how `wl_keyboard`
/// keymaps travel.
fn upload(keyboard: &ZwpVirtualKeyboardV1, keymap: &str) -> Result<(), String> {
    let memfd = memfd::MemfdOptions::default()
        .close_on_exec(true)
        .create("otto-emoji-keymap")
        .map_err(|err| err.to_string())?;
    let mut file = memfd.as_file();
    file.write_all(keymap.as_bytes())
        .and_then(|_| file.write_all(&[0]))
        .map_err(|err| err.to_string())?;
    keyboard.keymap(
        wl_keyboard::KeymapFormat::XkbV1 as u32,
        memfd.as_file().as_fd(),
        keymap.len() as u32 + 1,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keymap_has_one_key_per_codepoint() {
        let keymap = keymap_for("👋🏽");
        assert!(keymap.contains(&format!("<K1> = {};", PRINTABLE_KEYS[0] + 8)));
        assert!(keymap.contains(&format!("<K2> = {};", PRINTABLE_KEYS[1] + 8)));
        assert!(!keymap.contains("<K3>"));
        assert!(keymap.contains("key <K1> { [ U1F44B ] };"));
        assert!(keymap.contains("key <K2> { [ U1F3FD ] };"));
    }

    #[test]
    fn bmp_is_told_from_the_rest() {
        assert!(fits_in_bmp("hello ❤ ★ é"));
        assert!(!fits_in_bmp("😀"));
        assert!(!fits_in_bmp("👋🏽"), "a skin tone is non-BMP too");
    }

    #[test]
    fn the_standard_keymap_compiles_and_has_the_paste_chord() {
        use xkbcommon::xkb;
        let text = standard_keymap().expect("keymap");
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            text,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("libxkbcommon accepts it");
        let state = xkb::State::new(&keymap);
        assert_eq!(state.key_get_utf8(xkb::Keycode::new(KEY_V + 8)), "v");
        let ctrl = state.key_get_one_sym(xkb::Keycode::new(KEY_LEFTCTRL + 8));
        assert_eq!(ctrl, xkb::keysyms::KEY_Control_L.into());
    }

    /// Chromium reads the raw evdev code before the keymap and drops keys it
    /// does not recognise, or yields no text for ones it calls non-printable.
    /// Every code the picker uses has to be a printable key on a normal
    /// keyboard — never Escape, a modifier, or something off the table.
    #[test]
    fn the_keys_are_all_printable_ones() {
        // Escape, Backspace, Tab, Enter, Ctrl, the shifts, Alt, Caps, and
        // everything above the main block.
        const NEVER: [u32; 9] = [1, 14, 15, 28, 29, 42, 54, 56, 58];
        for code in PRINTABLE_KEYS {
            assert!(
                !NEVER.contains(&code),
                "evdev {code} is not a printable key"
            );
            assert!(
                (2..=53).contains(&code),
                "evdev {code} is off the main block"
            );
        }
        let mut sorted = PRINTABLE_KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PRINTABLE_KEYS.len(), "a code is used twice");
    }

    /// The point of the exercise: libxkbcommon has to accept what is built,
    /// and map the keys back to the characters they were made from.
    #[test]
    fn xkbcommon_compiles_the_keymap() {
        use xkbcommon::xkb;
        let text = "❤️🇬🇧";
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            keymap_for(text),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("a keymap libxkbcommon accepts");
        let state = xkb::State::new(&keymap);
        // The wire codes the picker actually sends, turned into xkb keycodes
        // the same way a client does.
        let typed: String = PRINTABLE_KEYS
            .iter()
            .take(text.chars().count())
            .map(|code| state.key_get_utf8(xkb::Keycode::new(code + 8)))
            .collect();
        assert_eq!(typed, text);
    }
}
