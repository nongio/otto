//! The terminal interfaces of Otto's command-line tools.
//!
//! `otto-msg doctor` and `otto-setup` share one look: a header with Otto's
//! mark and the tool's name, a rule, a list to move through, and a help line
//! on the last row. This crate is that look and the terminal under it, hand
//! rolled over termios and ANSI escapes rather than a terminal UI crate.
//!
//! Colours are the terminal's own palette (bold, dim, reverse video and the
//! standard colours), so they follow its light or dark theme.

// Rust guideline compliant 2026-02-21

// A `pub` item nothing outside the crate can reach is an accident: say
// `pub(crate)` or `pub(super)`, so the public API is what lib.rs exports.
#![warn(unreachable_pub)]

mod line;
mod term;

pub use line::{printable, Spans};
pub use term::{is_tty, size, Key, Terminal};

/// Styles, as SGR escapes.
pub mod style {
    /// Back to plain text.
    pub const RESET: &str = "\x1b[0m";
    /// Bold.
    pub const BOLD: &str = "\x1b[1m";
    /// Dim, for secondary text.
    pub const DIM: &str = "\x1b[2m";
    /// Reverse video, for the selected row.
    pub const REVERSE: &str = "\x1b[7m";
    /// Red, for what failed.
    pub const RED: &str = "\x1b[31m";
    /// Green, for what is on or done.
    pub const GREEN: &str = "\x1b[32m";
    /// Yellow, for what still needs doing.
    pub const YELLOW: &str = "\x1b[33m";
    /// Cyan, for Otto's mark.
    pub const CYAN: &str = "\x1b[36m";
}

use style::{BOLD, CYAN, DIM, RESET};

/// The header row: Otto's mark, the tool's name, an optional breadcrumb and,
/// flush right, an optional `(style, text)`.
pub fn header(cols: usize, tool: &str, crumb: Option<&str>, right: Option<(&str, &str)>) -> String {
    let mut line = Spans::new(cols);
    line.push(CYAN, " ● ● ");
    line.push(BOLD, "Otto");
    line.push(DIM, &format!("  {tool}"));
    if let Some(crumb) = crumb {
        line.push(DIM, "  ›  ");
        line.push("", crumb);
    }
    if let Some((style, text)) = right {
        line.push_right(style, text);
    }
    line.finish()
}

/// A dim rule across the whole width, under the header.
pub fn rule(cols: usize) -> String {
    format!("{DIM}{}{RESET}", "─".repeat(cols))
}

/// The whole screen from its `lines`, one per row from the top, with
/// whatever is below them cleared. Lines past `rows` are dropped.
pub fn compose(lines: &[String], rows: usize) -> String {
    let mut frame = String::new();
    for (row, line) in lines.iter().take(rows).enumerate() {
        frame.push_str(&format!("\x1b[{};1H{line}{RESET}\x1b[K", row + 1));
    }
    frame.push_str(&format!("\x1b[{};1H\x1b[J", lines.len().min(rows) + 1));
    frame
}
