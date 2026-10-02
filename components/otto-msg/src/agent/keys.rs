//! The agent's keyboard layout: a keymap made of exactly the keys it needs.
//!
//! A virtual keyboard sends keycodes, and the client turns them into text
//! with the keymap the keyboard uploaded. Rather than guess the user's
//! layout, the agent's keyboard has one key per symbol it types, plus the
//! four modifiers, and uploads a new keymap only when a symbol is missing.

use xkbcommon::xkb;

/// The modifiers, always the first four keys, with their bit in the
/// `depressed` mask of `zwp_virtual_keyboard_v1.modifiers`.
const MODIFIERS: [(&str, &str, u32); 4] = [
    ("Shift_L", "Shift", 0x01),
    ("Control_L", "Control", 0x04),
    ("Alt_L", "Mod1", 0x08),
    ("Super_L", "Mod4", 0x40),
];

/// xkb keycodes stop at 255 and start at 8, and the first four are the
/// modifiers.
const MAX_KEYS: usize = 247;

/// A key to press: the keycode the virtual keyboard sends.
pub type Key = u32;

/// The symbols the agent's keyboard has keys for, in key order after the
/// modifiers.
#[derive(Default)]
pub struct Layout {
    symbols: Vec<xkb::Keysym>,
}

/// A combination such as `ctrl+shift+t`: modifier bits and one symbol.
#[derive(Debug, PartialEq, Eq)]
pub struct Combo {
    pub modifiers: u32,
    pub keysym: xkb::Keysym,
}

impl Layout {
    /// The key for `keysym`, adding it if it is missing. Returns whether the
    /// keymap changed, so the caller uploads it before pressing.
    pub fn key_for(&mut self, keysym: xkb::Keysym) -> (Key, bool) {
        if let Some(index) = self.symbols.iter().position(|known| *known == keysym) {
            return (Self::key_at(index), false);
        }
        if self.symbols.len() == MAX_KEYS {
            self.symbols.clear();
        }
        self.symbols.push(keysym);
        (Self::key_at(self.symbols.len() - 1), true)
    }

    /// The evdev keycode of a modifier, by its bit.
    pub fn modifier_key(bit: u32) -> Option<Key> {
        MODIFIERS
            .iter()
            .position(|(_, _, mask)| *mask == bit)
            .map(|index| index as Key)
    }

    /// Wayland keys are evdev codes, xkb's minus 8; the modifiers take 0-3.
    fn key_at(index: usize) -> Key {
        (MODIFIERS.len() + index) as Key
    }

    /// The keymap, in the xkb text format `zwp_virtual_keyboard_v1.keymap`
    /// takes.
    pub fn keymap(&self) -> String {
        let total = MODIFIERS.len() + self.symbols.len();
        let mut keycodes = String::new();
        let mut symbols = String::new();
        for index in 0..total {
            keycodes.push_str(&format!("    <K{index}> = {};\n", index + 8));
        }
        for (index, (name, modifier, _)) in MODIFIERS.iter().enumerate() {
            symbols.push_str(&format!("    key <K{index}> {{ [ {name} ] }};\n"));
            symbols.push_str(&format!("    modifier_map {modifier} {{ <K{index}> }};\n"));
        }
        for (offset, keysym) in self.symbols.iter().enumerate() {
            let index = MODIFIERS.len() + offset;
            symbols.push_str(&format!(
                "    key <K{index}> {{ [ {} ] }};\n",
                symbol_name(*keysym)
            ));
        }
        format!(
            "xkb_keymap {{\n\
             xkb_keycodes \"otto-agent\" {{\n    minimum = 8;\n    maximum = {};\n{keycodes}}};\n\
             xkb_types \"otto-agent\" {{ include \"complete\" }};\n\
             xkb_compat \"otto-agent\" {{ include \"complete\" }};\n\
             xkb_symbols \"otto-agent\" {{\n{symbols}}};\n\
             }};\n",
            total + 7
        )
    }
}

/// The name xkb writes `keysym` with, or its number when it has none.
fn symbol_name(keysym: xkb::Keysym) -> String {
    let name = xkb::keysym_get_name(keysym);
    if name.is_empty() || name.starts_with("0x") {
        format!("0x{:x}", keysym.raw())
    } else {
        name
    }
}

/// The symbol that types `c`: a new line is Return and a tab is Tab.
pub fn keysym_for_char(c: char) -> Option<xkb::Keysym> {
    let keysym = match c {
        '\n' | '\r' => xkb::keysym_from_name("Return", xkb::KEYSYM_NO_FLAGS),
        '\t' => xkb::keysym_from_name("Tab", xkb::KEYSYM_NO_FLAGS),
        c => xkb::utf32_to_keysym(c as u32),
    };
    (keysym.raw() != 0).then_some(keysym)
}

/// Read a combination: modifiers and a key name joined with `+`, such as
/// `Return`, `ctrl+s`, `alt+Tab` or `ctrl+shift+t`. Key names are xkb's,
/// in any case; a single character stands for itself.
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let mut parts: Vec<&str> = text.split('+').collect();
    // `ctrl++` is Control with the plus key.
    if text.ends_with("++") {
        parts.pop();
        parts.pop();
        parts.push("+");
    }
    let Some((key, modifiers)) = parts.split_last() else {
        return Err("no key given".to_string());
    };
    let mut bits = 0;
    for modifier in modifiers {
        bits |= match modifier.to_ascii_lowercase().as_str() {
            "shift" => 0x01,
            "ctrl" | "control" => 0x04,
            "alt" => 0x08,
            "super" | "logo" | "meta" | "cmd" => 0x40,
            other => return Err(format!("unknown modifier '{other}'")),
        };
    }
    let mut chars = key.chars();
    let keysym = match (chars.next(), chars.next()) {
        (Some(c), None) => keysym_for_char(c),
        _ => Some(xkb::keysym_from_name(key, xkb::KEYSYM_CASE_INSENSITIVE))
            .filter(|keysym| keysym.raw() != 0),
    }
    .ok_or_else(|| format!("unknown key '{key}'"))?;
    Ok(Combo {
        modifiers: bits,
        keysym,
    })
}

/// The modifier bits set in `mask`, lowest first.
pub fn modifier_bits(mask: u32) -> impl Iterator<Item = u32> {
    MODIFIERS
        .iter()
        .map(|(_, _, bit)| *bit)
        .filter(move |bit| mask & bit != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiles(text: &str) -> bool {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_string(
            &context,
            text.to_string(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .is_some()
    }

    #[test]
    fn a_keymap_with_what_was_typed_compiles() {
        let mut layout = Layout::default();
        for c in "Héllo, wörld! ✓\n\t".chars() {
            layout.key_for(keysym_for_char(c).unwrap());
        }
        assert!(compiles(&layout.keymap()), "{}", layout.keymap());
    }

    #[test]
    fn an_empty_keymap_compiles() {
        assert!(compiles(&Layout::default().keymap()));
    }

    #[test]
    fn a_symbol_already_there_keeps_its_key() {
        let mut layout = Layout::default();
        let a = keysym_for_char('a').unwrap();
        let (first, changed) = layout.key_for(a);
        assert!(changed);
        assert_eq!(first, 4, "the modifiers come first");
        assert_eq!(layout.key_for(a), (first, false));
    }

    #[test]
    fn combinations_read_modifiers_and_a_key() {
        let combo = parse_combo("ctrl+shift+t").unwrap();
        assert_eq!(combo.modifiers, 0x05);
        assert_eq!(combo.keysym, keysym_for_char('t').unwrap());
        assert_eq!(parse_combo("Return").unwrap().modifiers, 0);
        assert_eq!(
            parse_combo("return").unwrap().keysym,
            xkb::keysym_from_name("Return", xkb::KEYSYM_NO_FLAGS)
        );
        assert_eq!(
            parse_combo("ctrl++").unwrap().keysym,
            keysym_for_char('+').unwrap()
        );
        assert!(parse_combo("hyper+x").is_err());
        assert!(parse_combo("ctrl+NoSuchKey").is_err());
    }

    #[test]
    fn modifiers_have_keys_of_their_own() {
        assert_eq!(Layout::modifier_key(0x04), Some(1));
        assert_eq!(modifier_bits(0x45).collect::<Vec<_>>(), vec![0x01, 0x04, 0x40]);
    }
}
