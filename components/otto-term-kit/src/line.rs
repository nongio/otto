//! One screen line built from styled pieces.

// Rust guideline compliant 2026-02-21

use crate::style::RESET;

/// `text` with tabs expanded and control characters made visible, so file
/// content cannot move the cursor or change colours.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' => out.push_str("    "),
            c if c.is_control() => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// One screen line built from styled pieces, cut at the terminal width.
///
/// Width is counted in characters, which matches the terminal for the text
/// these tools show; a wide character in a log line can push that line's end
/// past the edge, where it is clipped because wrapping is off.
#[derive(Debug)]
pub struct Spans {
    cols: usize,
    used: usize,
    text: String,
}

impl Spans {
    /// An empty line `cols` wide.
    pub fn new(cols: usize) -> Self {
        Self {
            cols,
            used: 0,
            text: String::new(),
        }
    }

    /// Appends `piece` in `style`, cut with an ellipsis if it overflows.
    pub fn push(&mut self, style: &str, piece: &str) {
        let room = self.cols.saturating_sub(self.used);
        if room == 0 {
            return;
        }
        let count = piece.chars().count();
        let shown: String = if count > room {
            piece
                .chars()
                .take(room.saturating_sub(1))
                .chain(std::iter::once('…'))
                .collect()
        } else {
            piece.to_owned()
        };
        self.used += shown.chars().count();
        self.text.push_str(RESET);
        self.text.push_str(style);
        self.text.push_str(&shown);
    }

    /// Appends `piece` flush with the right edge, if it fits.
    pub fn push_right(&mut self, style: &str, piece: &str) {
        let width = piece.chars().count();
        if self.used + width + 1 > self.cols {
            return;
        }
        let gap = self.cols - self.used - width;
        self.text.push_str(RESET);
        self.text.push_str(&" ".repeat(gap));
        self.used += gap;
        self.push(style, piece);
    }

    /// Pads to the full width in `style`, so a highlight spans the row.
    pub fn fill(&mut self, style: &str) {
        let gap = self.cols.saturating_sub(self.used);
        self.text.push_str(style);
        self.text.push_str(&" ".repeat(gap));
        self.used = self.cols;
    }

    /// The line, ready for [`crate::compose`].
    pub fn finish(self) -> String {
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_at_the_width_with_an_ellipsis() {
        let mut line = Spans::new(5);
        line.push("", "abcdefgh");
        assert_eq!(line.finish(), format!("{RESET}abcd…"));
    }

    #[test]
    fn control_characters_are_made_visible() {
        assert_eq!(printable("a\tb\x1b[2J"), "a    b\u{fffd}[2J");
    }
}
