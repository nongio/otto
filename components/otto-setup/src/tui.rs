//! The interactive setup, on three pages: the agents to add and who they
//! are, the extras (Node.js, the ACP adapters, the chat bridge), and the
//! steps that follow with Apply. Every row opens its instructions, or a
//! preview of exactly what it will write or run.
//!
//! The look and the terminal are `otto-term-kit`'s, shared with
//! `otto-msg doctor`.

// Rust guideline compliant 2026-02-21

use std::io;

use otto_agents_client::harness::Harness;
use otto_term_kit::style::{BOLD, DIM, GREEN, RED, REVERSE, YELLOW};
use otto_term_kit::{printable, Key, Spans, Terminal};

use crate::apply::{self, Outcome};
use crate::guide;
use crate::plan::{self, Bridge, Choices, Readiness, State, Step, HARNESSES};
use crate::probe::Facts;

/// How often the loop wakes without input to notice a resize.
const RESIZE_POLL_MS: i32 = 250;

/// Width of the title column.
const TITLE_WIDTH: usize = 18;

/// The pages, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Agents,
    Extras,
    Review,
}

impl Page {
    const ALL: [Page; 3] = [Page::Agents, Page::Extras, Page::Review];

    fn title(self) -> &'static str {
        match self {
            Page::Agents => "Agents",
            Page::Extras => "Extras",
            Page::Review => "Review",
        }
    }

    fn intro(self) -> &'static str {
        match self {
            Page::Agents => " Tick the agents Otto should run. Enter shows how to install one.",
            Page::Extras => " What the agents can use. Enter shows what is there and what to do.",
            Page::Review => " What Apply does. Enter shows exactly what a step writes or runs.",
        }
    }

    fn step(self, by: isize) -> Page {
        let at = Page::ALL.iter().position(|p| *p == self).unwrap_or(0) as isize;
        Page::ALL[(at + by).clamp(0, Page::ALL.len() as isize - 1) as usize]
    }
}

/// One row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Harness(Harness),
    Instructions,
    Node,
    Adapters,
    Bridge,
    Step(usize),
    Apply,
}

/// What is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// A page of rows.
    List,
    /// A row's text, scrolled down by `scroll` lines.
    Text { row: Row, scroll: usize },
    /// What Apply did; `cursor` is an outcome.
    Results { cursor: usize },
    /// One outcome's log.
    Log { index: usize, scroll: usize },
}

struct App {
    facts: Facts,
    choices: Choices,
    steps: Vec<Step>,
    outcomes: Vec<Outcome>,
    page: Page,
    cursor: usize,
    view: View,
    status: String,
}

/// What the loop does after a key.
enum Flow {
    Continue,
    Quit,
    Apply,
}

/// Runs the interface until the user quits.
///
/// # Errors
///
/// Fails when the terminal cannot be set up, read or written.
pub fn run(facts: Facts) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let choices = Choices::initial(&facts);
    let steps = plan::steps(&facts, &choices);
    let mut app = App {
        facts,
        choices,
        steps,
        outcomes: Vec::new(),
        page: Page::Agents,
        cursor: 0,
        view: View::List,
        status: "Nothing changes until you apply. Tab for the next page.".into(),
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

impl App {
    fn rows(&self) -> Vec<Row> {
        match self.page {
            Page::Agents => HARNESSES
                .into_iter()
                .map(Row::Harness)
                .chain([Row::Instructions])
                .collect(),
            Page::Extras => vec![Row::Node, Row::Adapters, Row::Bridge],
            Page::Review => (0..self.steps.len())
                .map(Row::Step)
                .chain([Row::Apply])
                .collect(),
        }
    }

    fn todo(&self) -> usize {
        self.steps.iter().filter(|s| s.state == State::Todo).count()
    }

    fn replan(&mut self) {
        self.steps = plan::steps(&self.facts, &self.choices);
    }

    fn apply(&mut self) {
        self.outcomes = apply::run(&self.steps);
        self.facts = Facts::read();
        let facts = &self.facts;
        self.choices.picks.retain(|h| !facts.configured.contains(h));
        self.choices.bridge = false;
        self.replan();
        let failed = self.outcomes.iter().any(|o| !o.ok);
        self.status = if failed {
            "Something did not work: open it for what it printed.".into()
        } else {
            "Done. Try it: run otto-ask, or bind it to a key.".into()
        };
        self.view = View::Results { cursor: 0 };
    }

    fn go(&mut self, page: Page) {
        if page != self.page {
            self.page = page;
            self.cursor = 0;
        }
    }

    fn handle(&mut self, key: Key, (_, rows): (usize, usize)) -> Flow {
        if key == Key::Interrupt {
            return Flow::Quit;
        }
        let page = rows.saturating_sub(4).max(1);
        match self.view {
            View::List => return self.handle_list(key),
            View::Text { row, scroll } => {
                let total = self.text(row).lines().count();
                self.view = match scrolled(key, scroll, page, total) {
                    Some(scroll) => View::Text { row, scroll },
                    None => View::List,
                };
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
                let total = self.outcomes[index].log.lines().count();
                self.view = match scrolled(key, scroll, page, total) {
                    Some(scroll) => View::Log { index, scroll },
                    None => View::Results { cursor: index },
                };
            }
        }
        Flow::Continue
    }

    fn handle_list(&mut self, key: Key) -> Flow {
        let rows = self.rows();
        let last = rows.len() - 1;
        let row = rows[self.cursor.min(last)];
        match key {
            Key::Up | Key::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            Key::Down | Key::Char('j') => self.cursor = (self.cursor + 1).min(last),
            Key::Home | Key::Char('g') => self.cursor = 0,
            Key::End | Key::Char('G') => self.cursor = last,
            Key::Left | Key::Right if row == Row::Instructions => {
                self.cycle_persona(if key == Key::Left { -1 } else { 1 });
            }
            Key::Tab | Key::Right => self.go(self.page.step(1)),
            Key::BackTab | Key::Left => self.go(self.page.step(-1)),
            Key::Char(' ') | Key::Char('x') => self.toggle(row),
            Key::Enter | Key::Char('v') => match row {
                Row::Apply => return self.try_apply(),
                Row::Instructions if key == Key::Enter => self.cycle_persona(1),
                _ => self.view = View::Text { row, scroll: 0 },
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

    fn cycle_persona(&mut self, by: isize) {
        let personas = Choices::personas(&self.facts);
        let at = personas
            .iter()
            .position(|p| *p == self.choices.persona)
            .unwrap_or(0) as isize;
        let len = personas.len() as isize;
        self.choices.persona = personas[(at + by).rem_euclid(len) as usize].clone();
        self.replan();
    }

    fn toggle(&mut self, row: Row) {
        match row {
            Row::Harness(harness) => match plan::readiness(&self.facts, harness) {
                Readiness::Configured => {
                    self.status =
                        "Already in your agent list. Settings › Agents changes or removes it."
                            .into();
                }
                Readiness::Missing(missing) => {
                    self.status = format!(
                        "{} missing. Enter shows how to install it.",
                        missing.join(", ")
                    );
                }
                Readiness::Ready(_) => {
                    let picks = &mut self.choices.picks;
                    if let Some(at) = picks.iter().position(|h| *h == harness) {
                        picks.remove(at);
                    } else {
                        picks.push(harness);
                        // Keep the list order, so the file does too.
                        picks.sort_by_key(|h| HARNESSES.iter().position(|x| x == h));
                    }
                    self.replan();
                }
            },
            Row::Instructions => self.cycle_persona(1),
            Row::Bridge => match plan::bridge(&self.facts) {
                Bridge::Ready | Bridge::Off => {
                    self.choices.bridge = !self.choices.bridge;
                    self.replan();
                }
                Bridge::Running => {
                    self.status =
                        "Already running. Settings › Agents › Chat bridge turns it off.".into();
                }
                Bridge::Missing(_) | Bridge::Unavailable(_) => {
                    self.status = "Not set up yet. Enter shows the steps.".into();
                }
            },
            Row::Node | Row::Adapters => {
                self.status = "Setup does not install software. Enter shows how.".into();
            }
            Row::Step(_) | Row::Apply => {}
        }
    }

    fn try_apply(&mut self) -> Flow {
        if self.todo() == 0 {
            self.status = "Nothing to do.".into();
            return Flow::Continue;
        }
        Flow::Apply
    }

    /// The instructions or preview a row opens.
    fn text(&self, row: Row) -> String {
        match row {
            Row::Harness(harness) => guide::harness(&self.facts, harness),
            Row::Instructions => guide::instructions(&self.facts, &self.choices.persona),
            Row::Node => guide::node(&self.facts),
            Row::Adapters => guide::adapters(&self.facts),
            Row::Bridge => guide::bridge(&self.facts),
            Row::Step(index) => {
                let step = &self.steps[index];
                if step.detail.is_empty() {
                    step.summary.clone()
                } else {
                    step.detail.clone()
                }
            }
            Row::Apply => String::new(),
        }
    }

    fn row_title(&self, row: Row) -> String {
        match row {
            Row::Harness(h) => h.name().unwrap_or_default().to_owned(),
            Row::Instructions => "Instructions".into(),
            Row::Node => "Node.js".into(),
            Row::Adapters => "ACP adapters".into(),
            Row::Bridge => "Chat bridge".into(),
            Row::Step(index) => self.steps[index].title.into(),
            Row::Apply => "Apply".into(),
        }
    }

    fn frame(&self, (cols, rows): (usize, usize)) -> String {
        let lines = match self.view {
            View::List => self.list_lines(cols, rows),
            View::Text { row, scroll } => {
                let content: Vec<String> = self.text(row).lines().map(printable).collect();
                self.scroll_frame(&self.row_title(row), &content, scroll, cols, rows)
            }
            View::Results { cursor } => self.results_lines(cursor, cols, rows),
            View::Log { index, scroll } => {
                let content: Vec<String> =
                    self.outcomes[index].log.lines().map(printable).collect();
                self.scroll_frame(self.outcomes[index].title, &content, scroll, cols, rows)
            }
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
        otto_term_kit::header(cols, "setup", crumb, Some((right.0, &right.1)))
    }

    /// The page tabs, the current one highlighted.
    fn tabs(&self, cols: usize) -> String {
        let mut line = Spans::new(cols);
        line.push("", " ");
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if i > 0 {
                line.push(DIM, " · ");
            }
            let style = if page == self.page {
                format!("{REVERSE}{BOLD}")
            } else {
                DIM.to_owned()
            };
            line.push(&style, &format!(" {}. {} ", i + 1, page.title()));
        }
        line.finish()
    }

    fn list_lines(&self, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![
            self.header(cols, None),
            otto_term_kit::rule(cols),
            self.tabs(cols),
            String::new(),
            plain(cols, self.page.intro()),
            String::new(),
        ];
        let items = self.rows();
        for (i, row) in items.iter().enumerate() {
            if *row == Row::Instructions || *row == Row::Apply {
                lines.push(String::new());
            }
            lines.push(self.row_line(*row, self.cursor == i, cols));
        }
        lines.push(String::new());
        lines.push(plain(cols, &format!(" {}", self.status)));
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let help = match self.page {
            Page::Agents => " ↑↓ move  space tick  ←→ instructions  enter how-to  tab next  q quit",
            Page::Extras => " ↑↓ move  space tick  enter what to do  tab next  q quit",
            Page::Review => " ↑↓ move  enter preview  a apply  tab back  q quit",
        };
        let mut help = help.to_owned();
        if !self.outcomes.is_empty() {
            help.push_str("  r results");
        }
        lines.push(plain_styled(cols, DIM, &help));
        lines
    }

    fn row_line(&self, row: Row, selected: bool, cols: usize) -> String {
        let base = if selected { REVERSE } else { "" };
        let mut line = Spans::new(cols);
        line.push(base, if selected { " ▸ " } else { "   " });
        let title = self.row_title(row);
        let (mark, mark_style, note, note_style): (&str, &str, String, &str) = match row {
            Row::Harness(harness) => match plan::readiness(&self.facts, harness) {
                Readiness::Configured => ("[✓]", GREEN, "in your agent list".into(), DIM),
                Readiness::Ready(path) => {
                    let mark = if self.choices.picks.contains(&harness) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    (mark, GREEN, self.facts.shown(&path), DIM)
                }
                Readiness::Missing(missing) => {
                    ("[ ]", DIM, format!("needs {}", missing.join(", ")), YELLOW)
                }
            },
            Row::Instructions => (
                " ⇄ ",
                "",
                format!(
                    "‹ {} ›  for the agents added now",
                    guide::persona_label(&self.choices.persona)
                ),
                "",
            ),
            Row::Node => match &self.facts.node_version {
                Some(version) => (
                    " ✓ ",
                    GREEN,
                    format!("{version} · npx {}", self.found("npx")),
                    DIM,
                ),
                None => (
                    " ✗ ",
                    YELLOW,
                    "not installed · enter for how".into(),
                    YELLOW,
                ),
            },
            Row::Adapters => {
                let adapters: Vec<_> = HARNESSES.iter().filter_map(|h| h.adapter()).collect();
                let have: Vec<&str> = adapters
                    .iter()
                    .filter(|a| self.facts.found.contains_key(a.bin))
                    .map(|a| a.bin)
                    .collect();
                if have.len() == adapters.len() {
                    (" ✓ ", GREEN, "all installed".into(), DIM)
                } else if have.is_empty() {
                    (
                        " · ",
                        DIM,
                        "none installed: Claude Code and Codex use npx".into(),
                        DIM,
                    )
                } else {
                    (" · ", DIM, format!("installed: {}", have.join(", ")), DIM)
                }
            }
            Row::Bridge => match plan::bridge(&self.facts) {
                Bridge::Running => ("[✓]", GREEN, "running".into(), DIM),
                Bridge::Off | Bridge::Ready => {
                    let mark = if self.choices.bridge { "[x]" } else { "[ ]" };
                    let note = if plan::bridge(&self.facts) == Bridge::Off {
                        "set up, off: tick to start it"
                    } else {
                        "cc-connect ready: tick to turn it on"
                    };
                    (mark, GREEN, note.into(), DIM)
                }
                Bridge::Missing(missing) => (
                    "[ ]",
                    DIM,
                    format!("needs {} · enter for how", missing.join(", ")),
                    YELLOW,
                ),
                Bridge::Unavailable(why) => ("[ ]", DIM, why.into(), DIM),
            },
            Row::Step(index) => {
                let step = &self.steps[index];
                let (glyph, style) = match step.state {
                    State::Done => (" ✓ ", GREEN),
                    State::Todo => (" ● ", YELLOW),
                    State::Blocked => (" ! ", RED),
                };
                (glyph, style, printable(&step.summary), DIM)
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
                return line.finish();
            }
        };
        line.push(&format!("{base}{mark_style}"), mark);
        line.push(base, &format!(" {title:<TITLE_WIDTH$}"));
        line.push(&format!("{base}{note_style}"), &note);
        if selected {
            line.fill(REVERSE);
        }
        line.finish()
    }

    fn found(&self, program: &str) -> String {
        self.facts
            .found
            .get(program)
            .map_or_else(|| "missing".to_owned(), |p| self.facts.shown(p))
    }

    fn results_lines(&self, cursor: usize, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![
            self.header(cols, Some("Results")),
            otto_term_kit::rule(cols),
            String::new(),
        ];
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
        crumb: &str,
        content: &[String],
        scroll: usize,
        cols: usize,
        rows: usize,
    ) -> Vec<String> {
        let mut lines = vec![self.header(cols, Some(crumb)), otto_term_kit::rule(cols)];
        let page = rows.saturating_sub(3).max(1);
        for text in content.iter().skip(scroll).take(page) {
            let (style, text) = match text.strip_prefix("# ") {
                Some(heading) => (BOLD, heading),
                None => ("", text.as_str()),
            };
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
        Key::Esc | Key::Enter | Key::Char('q') | Key::Char('v') | Key::Left => return None,
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
