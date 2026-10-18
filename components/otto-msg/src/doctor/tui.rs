//! The interactive chooser: a list of sections to tick, and a preview of
//! each one exactly as the report will carry it.
//!
//! Colours are the terminal's own palette (bold, dim, reverse video and the
//! standard green and cyan), so they follow its light or dark theme.

// Rust guideline compliant 2026-02-21

use std::io;

use crate::doctor::redact::Redactor;
use crate::doctor::report::{self, Redaction};
use crate::doctor::sections::Section;
use crate::doctor::term::{self, Key, Terminal};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const REVERSE: &str = "\x1b[7m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";

/// How often the loop wakes without input to notice a resize.
const RESIZE_POLL_MS: i32 = 250;

/// Width of the title column in the section list.
const TITLE_WIDTH: usize = 18;

/// How the chooser ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Leave without writing anything.
    Quit,
    /// Write the report with the current choices.
    Write,
}

/// What is on screen.
#[derive(Debug, Clone, Copy)]
enum View {
    /// The section list; `cursor` is a section index, or `sections.len()`
    /// for the "Create report" button.
    List,
    /// One section's full text, scrolled down by `scroll` lines.
    Preview { index: usize, scroll: usize },
}

/// The chooser's state.
struct App<'a> {
    sections: &'a mut [Section],
    redactor: &'a Redactor,
    redact: &'a mut bool,
    cursor: usize,
    view: View,
    status: String,
}

/// Runs the chooser until the user writes the report or quits.
///
/// Changes `sections` (which are included) and `redact` in place.
///
/// # Errors
///
/// Fails when the terminal cannot be set up, read or written.
pub fn run(
    sections: &mut [Section],
    redactor: &Redactor,
    redact: &mut bool,
) -> io::Result<Outcome> {
    let mut terminal = Terminal::enter()?;
    let mut app = App {
        sections,
        redactor,
        redact,
        cursor: 0,
        view: View::List,
        status: "The preview shows exactly what the report will contain.".into(),
    };
    let mut drawn_size = None;
    let mut dirty = true;
    loop {
        let size = term::size();
        if dirty || drawn_size != Some(size) {
            terminal.draw(&app.frame(size))?;
            drawn_size = Some(size);
            dirty = false;
        }
        for key in terminal.keys(RESIZE_POLL_MS)? {
            dirty = true;
            if let Some(outcome) = app.handle(key, size) {
                return Ok(outcome);
            }
        }
    }
}

impl App<'_> {
    fn redaction(&self) -> Redaction<'_> {
        if *self.redact {
            Redaction::On(self.redactor)
        } else {
            Redaction::Off
        }
    }

    /// Reacts to `key`; `Some` when the chooser is done.
    fn handle(&mut self, key: Key, (_, rows): (usize, usize)) -> Option<Outcome> {
        if key == Key::Interrupt {
            return Some(Outcome::Quit);
        }
        if key == Key::Char('r') {
            *self.redact = !*self.redact;
            self.status = if *self.redact {
                "Redaction on: home folder, user and host names, IP and MAC addresses are replaced."
            } else {
                "Redaction off: the report keeps names and addresses as they are."
            }
            .into();
            return None;
        }
        match self.view {
            View::List => self.handle_list(key),
            View::Preview { index, scroll } => {
                let page = rows.saturating_sub(3).max(1);
                let total = self.preview_lines(index).len();
                let max = total.saturating_sub(page);
                let scroll = match key {
                    Key::Up | Key::Char('k') => scroll.saturating_sub(1),
                    Key::Down | Key::Char('j') => scroll + 1,
                    Key::PageUp | Key::Char('b') => scroll.saturating_sub(page),
                    Key::PageDown | Key::Char(' ') | Key::Char('f') => scroll + page,
                    Key::Home | Key::Char('g') => 0,
                    Key::End | Key::Char('G') => max,
                    Key::Char('x') | Key::Char('t') => {
                        let section = &mut self.sections[index];
                        section.include = !section.include;
                        scroll
                    }
                    Key::Esc | Key::Enter | Key::Char('q') | Key::Char('v') => {
                        self.view = View::List;
                        return None;
                    }
                    _ => scroll,
                };
                self.view = View::Preview {
                    index,
                    scroll: scroll.min(max),
                };
                None
            }
        }
    }

    fn handle_list(&mut self, key: Key) -> Option<Outcome> {
        let last = self.sections.len();
        match key {
            Key::Up | Key::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            Key::Down | Key::Char('j') => self.cursor = (self.cursor + 1).min(last),
            Key::Home | Key::Char('g') => self.cursor = 0,
            Key::End | Key::Char('G') => self.cursor = last,
            Key::Char(' ') | Key::Char('x') if self.cursor < last => {
                let section = &mut self.sections[self.cursor];
                section.include = !section.include;
            }
            Key::Char('a') => {
                let all = self.sections.iter().all(|s| s.include);
                for section in self.sections.iter_mut() {
                    section.include = !all;
                }
            }
            Key::Enter | Key::Char('v') if self.cursor < last => {
                self.view = View::Preview {
                    index: self.cursor,
                    scroll: 0,
                };
            }
            Key::Enter | Key::Char('w') => {
                if self.sections.iter().any(|s| s.include) {
                    return Some(Outcome::Write);
                }
                self.status = "Choose at least one section first.".into();
            }
            Key::Esc | Key::Char('q') => return Some(Outcome::Quit),
            _ => {}
        }
        None
    }

    /// The preview text of section `index`, one entry per screen line.
    fn preview_lines(&self, index: usize) -> Vec<String> {
        report::render_section(&self.sections[index], self.redaction())
            .lines()
            .map(printable)
            .collect()
    }

    /// The whole screen for a terminal of `cols`×`rows`.
    fn frame(&self, (cols, rows): (usize, usize)) -> String {
        let lines = match self.view {
            View::List => self.list_lines(cols, rows),
            View::Preview { index, scroll } => self.preview_frame(index, scroll, cols, rows),
        };
        let mut frame = String::new();
        for (row, line) in lines.iter().take(rows).enumerate() {
            frame.push_str(&format!("\x1b[{};1H{line}{RESET}\x1b[K", row + 1));
        }
        frame.push_str(&format!("\x1b[{};1H\x1b[J", lines.len().min(rows) + 1));
        frame
    }

    /// The header row: the mark, the title and, on the right, the redaction
    /// state.
    fn header(&self, cols: usize, crumb: Option<&str>) -> String {
        let mut left = Spans::new(cols);
        left.push(CYAN, " ● ● ");
        left.push(BOLD, "Otto");
        left.push(DIM, "  doctor");
        if let Some(crumb) = crumb {
            left.push(DIM, "  ›  ");
            left.push("", crumb);
        }
        let right = if *self.redact {
            "redaction on "
        } else {
            "redaction off "
        };
        let right_style = if *self.redact { GREEN } else { BOLD };
        left.push_right(right_style, right);
        left.finish()
    }

    fn rule(cols: usize) -> String {
        format!("{DIM}{}{RESET}", "─".repeat(cols))
    }

    fn list_lines(&self, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![self.header(cols, None), Self::rule(cols)];
        let mut intro = Spans::new(cols);
        intro.push(
            "",
            " Choose what goes into the bug report. Nothing is uploaded.",
        );
        lines.push(intro.finish());
        lines.push(String::new());
        let redaction = self.redaction();
        for (i, section) in self.sections.iter().enumerate() {
            let selected = self.cursor == i;
            let mut row = Spans::new(cols);
            let base = if selected { REVERSE } else { "" };
            row.push(base, if selected { " ▸ " } else { "   " });
            if section.include {
                row.push(&format!("{base}{GREEN}"), "[x]");
            } else {
                row.push(&format!("{base}{DIM}"), "[ ]");
            }
            row.push(base, &format!(" {:<TITLE_WIDTH$}", section.title));
            row.push(
                &format!("{base}{DIM}"),
                &printable(&redaction.apply(&section.summary)),
            );
            if selected {
                row.fill(REVERSE);
            }
            lines.push(row.finish());
        }
        lines.push(String::new());
        let mut button = Spans::new(cols);
        let on_button = self.cursor == self.sections.len();
        let style = if on_button {
            format!("{REVERSE}{BOLD}")
        } else {
            BOLD.to_owned()
        };
        button.push(&style, if on_button { " ▸ " } else { "   " });
        button.push(&style, "Create report ");
        if on_button {
            button.fill(&style);
        }
        lines.push(button.finish());
        lines.push(String::new());
        let mut status = Spans::new(cols);
        status.push("", &format!(" {}", self.status));
        lines.push(status.finish());

        // The help line sits on the last row.
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let mut help = Spans::new(cols);
        help.push(
            DIM,
            " ↑↓ move  space toggle  enter/v preview  a all  r redaction  w write  q quit",
        );
        lines.push(help.finish());
        lines
    }

    fn preview_frame(&self, index: usize, scroll: usize, cols: usize, rows: usize) -> Vec<String> {
        let section = &self.sections[index];
        let mut lines = vec![self.header(cols, Some(section.title)), Self::rule(cols)];
        let content = self.preview_lines(index);
        let page = rows.saturating_sub(3).max(1);
        for text in content.iter().skip(scroll).take(page) {
            let mut row = Spans::new(cols);
            let style = if text.starts_with('#') { BOLD } else { "" };
            row.push(style, &format!(" {text}"));
            lines.push(row.finish());
        }
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let first = if content.is_empty() { 0 } else { scroll + 1 };
        let last = (scroll + page).min(content.len());
        let included = if section.include {
            "included"
        } else {
            "not included"
        };
        let mut help = Spans::new(cols);
        help.push(
            DIM,
            " ↑↓ scroll  space/b page  g/G ends  x include  r redaction  q back",
        );
        help.push_right(
            DIM,
            &format!("{included} · lines {first}–{last} of {} ", content.len()),
        );
        lines.push(help.finish());
        lines
    }
}

/// `text` with tabs expanded and control characters made visible, so file
/// content cannot move the cursor or change colours.
fn printable(text: &str) -> String {
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
/// this tool shows; a wide character in a log line can push that line's end
/// past the edge, where it is clipped because wrapping is off.
struct Spans {
    cols: usize,
    used: usize,
    text: String,
}

impl Spans {
    fn new(cols: usize) -> Self {
        Self {
            cols,
            used: 0,
            text: String::new(),
        }
    }

    /// Appends `piece` in `style`, cut with an ellipsis if it overflows.
    fn push(&mut self, style: &str, piece: &str) {
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
    fn push_right(&mut self, style: &str, piece: &str) {
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
    fn fill(&mut self, style: &str) {
        let gap = self.cols.saturating_sub(self.used);
        self.text.push_str(style);
        self.text.push_str(&" ".repeat(gap));
        self.used = self.cols;
    }

    fn finish(self) -> String {
        self.text
    }
}
