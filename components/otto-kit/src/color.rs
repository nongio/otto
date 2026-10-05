//! Hex colour literals, read the one way across the desktop.
//!
//! Every config file, command line and setting that takes a colour takes the
//! CSS forms: `#RGB`, `#RRGGBB` and `#RRGGBBAA`, alpha last. Three digits
//! expand the way CSS expands them, each digit doubled, so `#f0a` and
//! `#ff00aa` are the same colour.

use skia_safe::Color;

/// Read a `#RGB`, `#RRGGBB` or `#RRGGBBAA` literal.
///
/// The leading `#` is required: it is what tells a colour apart from a palette
/// name, and without it `abc` would be a valid colour as well as an unknown
/// name. Surrounding whitespace is ignored. Anything else is `None`, so a typo
/// leaves the caller's default rather than painting black.
pub fn parse_hex(text: &str) -> Option<Color> {
    parse_digits(text.trim().strip_prefix('#')?)
}

/// [`parse_hex`], with the `#` optional — for the places that have always
/// taken bare digits (`background_color = "1a1a2e"`, a command-line flag).
pub fn parse_hex_lenient(text: &str) -> Option<Color> {
    let text = text.trim();
    parse_digits(text.strip_prefix('#').unwrap_or(text))
}

fn parse_digits(digits: &str) -> Option<Color> {
    // Checked up front: it keeps the byte slicing below on char boundaries,
    // and `from_str_radix` alone would take a leading `+`.
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    match digits.len() {
        3 => {
            let nibble = |i: usize| {
                u8::from_str_radix(&digits[i..i + 1], 16)
                    .ok()
                    .map(|v| v * 17)
            };
            Some(Color::from_argb(0xFF, nibble(0)?, nibble(1)?, nibble(2)?))
        }
        6 => Some(Color::from_argb(0xFF, byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color::from_argb(byte(6)?, byte(0)?, byte(2)?, byte(4)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_three_css_forms() {
        assert_eq!(
            parse_hex("#0A84FF"),
            Some(Color::from_argb(255, 10, 132, 255))
        );
        assert_eq!(parse_hex("#f0a"), parse_hex("#ff00aa"));
        assert_eq!(
            parse_hex("#FF000080"),
            Some(Color::from_argb(0x80, 255, 0, 0))
        );
        assert_eq!(parse_hex("  #fff  "), Some(Color::WHITE));
    }

    #[test]
    fn rejects_what_is_not_a_hash_prefixed_literal() {
        for text in [
            "", "#", "ff00aa", "#ff00a", "#ff00aaa", "#gggggg", "blue", "#+ff", "#ééé",
        ] {
            assert!(parse_hex(text).is_none(), "`{text}` parsed as a colour");
        }
    }

    #[test]
    fn lenient_form_takes_bare_digits() {
        assert_eq!(parse_hex_lenient("1a1a2e"), parse_hex("#1a1a2e"));
        assert_eq!(parse_hex_lenient("#1a1a2e"), parse_hex("#1a1a2e"));
        assert_eq!(parse_hex_lenient("ff000080"), parse_hex("#ff000080"));
        assert!(parse_hex_lenient("blue").is_none());
    }
}
