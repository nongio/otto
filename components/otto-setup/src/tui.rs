//! The interactive setup: the harnesses to tick, the steps that follows
//! from, a preview of each step exactly as it will write or run it, and the
//! results once applied.
//!
//! The look and the terminal are `otto-term-kit`'s, shared with
//! `otto-msg doctor`.

// Rust guideline compliant 2026-02-21

use std::io;

use otto_agents_client::harness::Harness;
use otto_term_kit::style::{BOLD, DIM, GREEN, RED, REVERSE, YELLOW};
use otto_term_kit::{printable, Key, Spans, Terminal};

use crate::apply::{self, Outcome};
use crate::plan::{self, Readiness, State, Step, HARNESSES};
use crate::probe::Facts;

/// How often the loop wakes without input to notice a resize.
const RESIZE_POLL_MS: i32 = 250;

/// Width of the title column.
const TITLE_WIDTH: usize = 18;

/// What is on screen.
#[derive(Debug, Clone, Copy)]
enum View {
    /// The harnesses, the steps and Apply.
    List,
    /// One step's detail, scrolled down by `scroll` lines.
    Preview { step: usize, scroll: usize },
    /// What Apply did; `cursor` is an outcome.
    Results { cursor: usize },
    /// One outcome's log.
    Log { index: usize, scroll: usize },
}

/// One row of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Harness(Harness),
    Step(usize),
    Apply,
}

struct App {
    facts: Facts,
    picks: Vec<Harness>,
    steps: Vec<Step>,
    outcomes: Vec<Outcome>,
    cursor: usize,
    view: View,
    status: String,
}

/// Runs the interface until the user quits.
///
/// # Errors
///
/// Fails when the terminal cannot be set up, read or written.
pub fn run(facts: Facts) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let picks = plan::initial_picks(&facts);
    let steps = plan::steps(&facts, &picks);
    let mut app = App {
        facts,
        picks,
        steps,
        outcomes: Vec::new(),
        cursor: 0,
        view: View::List,
        status: "Nothing changes until you apply.".into(),
    };
    let mut drawn_size = None;
    let mut dirty = true;
    loop {
        let size = otto_term_kit::size();
        if dirty || drawn_size != Some(size) {
            terminal.draw(&app.frame(size))?;
            drawn_size = Some(size);
            dirty = false;
        }
        for key in terminal.keys(RESIZE_POLL_MS)? {
            dirty = true;
            match app.handle(key, size) {
                Flow::Continue => {}
                Flow::Quit => return Ok(()),
                Flow::Apply => {
                    app.status = "Applying…".into();
                    terminal.draw(&app.frame(size))?;
                    app.apply();
                }
            }
        }
    }
}

/// What the loop does after a key.
enum Flow {
    Continue,
    Quit,
    Apply,
}

impl App {
    fn rows(&self) -> Vec<Row> {
        HARNESSES
            .into_iter()
            .map(Row::Harness)
            .chain((0..self.steps.len()).map(Row::Step))
            .chain(std::iter::once(Row::Apply))
            .collect()
    }

    fn todo(&self) -> usize {
        self.steps.iter().filter(|s| s.state == State::Todo).count()
    }

    fn replan(&mut self) {
        self.steps = plan::steps(&self.facts, &self.picks);
    }

    fn apply(&mut self) {
        self.outcomes = apply::run(&self.steps);
        self.facts = Facts::read();
        self.picks.retain(|h| !self.facts.configured.contains(h));
        self.replan();
        let failed = self.outcomes.iter().any(|o| !o.ok);
        self.status = if failed {
            "Something did not work: open it for what it printed.".into()
        } else {
            "Done. Try it: run otto-ask, or bind it to a key.".into()
        };
        self.view = View::Results { cursor: 0 };
    }

    fn handle(&mut self, key: Key, (_, rows): (usize, usize)) -> Flow {
        if key == Key::Interrupt {
            return Flow::Quit;
        }
        let page = rows.saturating_sub(3).max(1);
        match self.view {
            View::List => return self.handle_list(key),
            View::Preview { step, scroll } => {
                let total = self.preview_lines(step).len();
                match scrolled(key, scroll, page, total) {
                    Some(scroll) => self.view = View::Preview { step, scroll },
                    None => self.view = View::List,
                }
            }
            View::Results { cursor } => {
                let last = self.outcomes.len().saturating_sub(1);
                match key {
                    Key::Up | Key::Char('k') => {
                        self.view = View::Results {
                            cursor: cursor.saturating_sub(1),
                        }
                    }
                    Key::Down | Key::Char('j') => {
                        self.view = View::Results {
                            cursor: (cursor + 1).min(last),
                        }
                    }
                    Key::Enter | Key::Char('v') if !self.outcomes.is_empty() => {
                        self.view = View::Log {
                            index: cursor,
                            scroll: 0,
                        }
                    }
                    Key::Esc | Key::Char('b') => self.view = View::List,
                    Key::Char('q') => return Flow::Quit,
                    _ => {}
                }
            }
            View::Log { index, scroll } => {
                let total = self.log_lines(index).len();
                match scrolled(key, scroll, page, total) {
                    Some(scroll) => self.view = View::Log { index, scroll },
                    None => self.view = View::Results { cursor: index },
                }
            }
        }
        Flow::Continue
    }

    fn handle_list(&mut self, key: Key) -> Flow {
        let rows = self.rows();
        let last = rows.len() - 1;
        match key {
            Key::Up | Key::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            Key::Down | Key::Char('j') => self.cursor = (self.cursor + 1).min(last),
            Key::Home | Key::Char('g') => self.cursor = 0,
            Key::End | Key::Char('G') => self.cursor = last,
            Key::Char(' ') | Key::Char('x') | Key::Enter
                if matches!(rows[self.cursor], Row::Harness(_)) =>
            {
                if let Row::Harness(harness) = rows[self.cursor] {
                    self.toggle(harness);
                }
            }
            Key::Enter | Key::Char('v') => match rows[self.cursor] {
                Row::Step(step) => {
                    self.view = View::Preview { step, scroll: 0 };
                }
                Row::Apply => return self.try_apply(),
                Row::Harness(_) => {}
            },
            Key::Char('a') => return self.try_apply(),
            Key::Char('r') if !self.outcomes.is_empty() => {
                self.view = View::Results { cursor: 0 };
            }
            Key::Esc | Key::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    fn toggle(&mut self, harness: Harness) {
        match plan::readiness(&self.facts, harness) {
            Readiness::Configured => {
                self.status =
                    "Already in your agent list. Settings › Agents changes or removes it.".into();
            }
            Readiness::Missing(program) => {
                self.status = format!(
                    "{program} is not on your PATH. Install {} and log in, then run otto-setup again.",
                    harness.name().unwrap_or(program)
                );
            }
            Readiness::Installed(_) => {
                if let Some(at) = self.picks.iter().position(|h| *h == harness) {
                    self.picks.remove(at);
                } else {
                    self.picks.push(harness);
                    // Keep the list order, so the file does too.
                    self.picks
                        .sort_by_key(|h| HARNESSES.iter().position(|x| x == h));
                }
                self.replan();
            }
        }
    }

    fn try_apply(&mut self) -> Flow {
        if self.todo() == 0 {
            self.status = "Nothing to do.".into();
            return Flow::Continue;
        }
        Flow::Apply
    }

    fn preview_lines(&self, step: usize) -> Vec<String> {
        let detail = &self.steps[step].detail;
        if detail.is_empty() {
            return vec![self.steps[step].summary.clone()];
        }
        detail.lines().map(printable).collect()
    }

    fn log_lines(&self, index: usize) -> Vec<String> {
        self.outcomes[index].log.lines().map(printable).collect()
    }

    fn frame(&self, (cols, rows): (usize, usize)) -> String {
        let lines = match self.view {
            View::List => self.list_lines(cols, rows),
            View::Preview { step, scroll } => self.scroll_frame(
                Some(self.steps[step].title),
                &self.preview_lines(step),
                scroll,
                cols,
                rows,
            ),
            View::Results { cursor } => self.results_lines(cursor, cols, rows),
            View::Log { index, scroll } => self.scroll_frame(
                Some(self.outcomes[index].title),
                &self.log_lines(index),
                scroll,
                cols,
                rows,
            ),
        };
        otto_term_kit::compose(&lines, rows)
    }

    fn header(&self, cols: usize, crumb: Option<&str>) -> String {
        let todo = self.todo();
        let right = if todo == 0 {
            (GREEN, "all set ".to_owned())
        } else {
            (YELLOW, format!("{todo} to do "))
        };
        let crumb = crumb.map_or_else(|| "Agents".to_owned(), |c| format!("Agents  ›  {c}"));
        otto_term_kit::header(cols, "setup", Some(&crumb), Some((right.0, &right.1)))
    }

    fn list_lines(&self, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![self.header(cols, None), otto_term_kit::rule(cols)];
        lines.push(plain(
            cols,
            " Choose the agents Otto can run. Otto uses their own login.",
        ));
        lines.push(String::new());
        lines.push(plain_styled(cols, BOLD, " Agents"));
        let items = self.rows();
        for (i, row) in items.iter().enumerate() {
            let selected = self.cursor == i;
            let base = if selected { REVERSE } else { "" };
            let mut line = Spans::new(cols);
            line.push(base, if selected { " ▸ " } else { "   " });
            match *row {
                Row::Harness(harness) => {
                    let name = harness.name().unwrap_or_default();
                    let (mark, mark_style, note, note_style) =
                        match plan::readiness(&self.facts, harness) {
                            Readiness::Configured => {
                                ("[✓]", GREEN, "in your agent list".to_owned(), DIM)
                            }
                            Readiness::Installed(path) => {
                                let mark = if self.picks.contains(&harness) {
                                    "[x]"
                                } else {
                                    "[ ]"
                                };
                                (mark, GREEN, self.facts.shown(&path), DIM)
                            }
                            Readiness::Missing(program) => {
                                ("[ ]", DIM, format!("{program} not found"), YELLOW)
                            }
                        };
                    line.push(&format!("{base}{mark_style}"), mark);
                    line.push(base, &format!(" {name:<TITLE_WIDTH$}"));
                    line.push(&format!("{base}{note_style}"), &note);
                }
                Row::Step(index) => {
                    let step = &self.steps[index];
                    let (glyph, style) = match step.state {
                        State::Done => (" ✓ ", GREEN),
                        State::Todo => (" ● ", YELLOW),
                        State::Blocked => (" ! ", RED),
                    };
                    line.push(&format!("{base}{style}"), glyph);
                    line.push(base, &format!(" {:<TITLE_WIDTH$}", step.title));
                    line.push(&format!("{base}{DIM}"), &printable(&step.summary));
                }
                Row::Apply => {
                    let style = if selected {
                        format!("{REVERSE}{BOLD}")
                    } else {
                        BOLD.to_owned()
                    };
                    line.push(&style, "Apply ");
                    if selected {
                        line.fill(&style);
                    }
                }
            }
            if selected {
                line.fill(REVERSE);
            }
            lines.push(line.finish());
            // Headings and gaps between the groups.
            if i + 1 == HARNESSES.len() {
                lines.push(String::new());
                lines.push(plain_styled(cols, BOLD, " Steps"));
            } else if i + 2 == items.len() {
                lines.push(String::new());
            }
        }
        lines.push(String::new());
        lines.push(plain(cols, &format!(" {}", self.status)));
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let mut help = " ↑↓ move  space tick  enter/v preview  a apply".to_owned();
        if !self.outcomes.is_empty() {
            help.push_str("  r results");
        }
        help.push_str("  q quit");
        lines.push(plain_styled(cols, DIM, &help));
        lines
    }

    fn results_lines(&self, cursor: usize, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![
            self.header(cols, Some("Results")),
            otto_term_kit::rule(cols),
        ];
        lines.push(String::new());
        for (i, outcome) in self.outcomes.iter().enumerate() {
            let selected = cursor == i;
            let base = if selected { REVERSE } else { "" };
            let mut line = Spans::new(cols);
            line.push(base, if selected { " ▸ " } else { "   " });
            let (glyph, style) = if outcome.ok {
                (" ✓ ", GREEN)
            } else {
                (" ✗ ", RED)
            };
            line.push(&format!("{base}{style}"), glyph);
            line.push(base, &format!(" {:<TITLE_WIDTH$}", outcome.title));
            let last = outcome
                .log
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or_default();
            line.push(&format!("{base}{DIM}"), &printable(last.trim()));
            if selected {
                line.fill(REVERSE);
            }
            lines.push(line.finish());
        }
        lines.push(String::new());
        lines.push(plain(cols, &format!(" {}", self.status)));
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        lines.push(plain_styled(
            cols,
            DIM,
            " ↑↓ move  enter/v what it printed  b back  q quit",
        ));
        lines
    }

    fn scroll_frame(
        &self,
        crumb: Option<&str>,
        content: &[String],
        scroll: usize,
        cols: usize,
        rows: usize,
    ) -> Vec<String> {
        let mut lines = vec![self.header(cols, crumb), otto_term_kit::rule(cols)];
        let page = rows.saturating_sub(3).max(1);
        for text in content.iter().skip(scroll).take(page) {
            let style = if text.starts_with('#') { DIM } else { "" };
            lines.push(plain_styled(cols, style, &format!(" {text}")));
        }
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let first = if content.is_empty() { 0 } else { scroll + 1 };
        let last = (scroll + page).min(content.len());
        let mut help = Spans::new(cols);
        help.push(DIM, " ↑↓ scroll  space/b page  q back");
        help.push_right(DIM, &format!("lines {first}–{last} of {} ", content.len()));
        lines.push(help.finish());
        lines
    }
}

/// The new scroll position after `key`, or `None` to go back.
fn scrolled(key: Key, scroll: usize, page: usize, total: usize) -> Option<usize> {
    let max = total.saturating_sub(page);
    let scroll = match key {
        Key::Up | Key::Char('k') => scroll.saturating_sub(1),
        Key::Down | Key::Char('j') => scroll + 1,
        Key::PageUp | Key::Char('b') => scroll.saturating_sub(page),
        Key::PageDown | Key::Char(' ') | Key::Char('f') => scroll + page,
        Key::Home | Key::Char('g') => 0,
        Key::End | Key::Char('G') => max,
        Key::Esc | Key::Enter | Key::Char('q') | Key::Char('v') => return None,
        _ => scroll,
    };
    Some(scroll.min(max))
}

fn plain(cols: usize, text: &str) -> String {
    plain_styled(cols, "", text)
}

fn plain_styled(cols: usize, style: &str, text: &str) -> String {
    let mut line = Spans::new(cols);
    line.push(style, text);
    line.finish()
}
