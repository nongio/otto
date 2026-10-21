//! The interactive setup: a welcome that says what it is for, then three
//! pages: the agents to add and who they are, what they require (Node.js, the ACP adapters) and the optional chat bridge, and the
//! steps that follow with Apply. Every row opens its instructions, or a
//! preview of exactly what it will write or run.
//!
//! The look and the terminal are `otto-term-kit`'s, shared with
//! `otto-msg doctor`.

// Rust guideline compliant 2026-02-21

use std::io;

use otto_agents_client::harness::{self, Harness};
use otto_term_kit::style::{BOLD, DIM, GREEN, RED, REVERSE, YELLOW};
use otto_term_kit::{printable, Key, Spans, Terminal};

use crate::apply::{self, Outcome};
use crate::guide;
use crate::plan::{self, Bridge, Choices, Readiness, State, Step, Studio, HARNESSES};
use crate::probe::Facts;

/// How often the loop wakes without input to notice a resize.
const RESIZE_POLL_MS: i32 = 250;

/// Width of the title column.
const TITLE_WIDTH: usize = 18;

/// The pages, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Agents,
    Requirements,
    Review,
}

impl Page {
    const ALL: [Page; 3] = [Page::Agents, Page::Requirements, Page::Review];

    fn title(self) -> &'static str {
        match self {
            Page::Agents => "Agents",
            Page::Requirements => "Requirements",
            Page::Review => "Review",
        }
    }

    fn intro(self) -> &'static str {
        match self {
            Page::Agents => " Which agents do you want to set up? Enter shows what one needs.",
            Page::Requirements => {
                " What the agents need, and what is optional. Enter shows what to do."
            }
            Page::Review => {
                " What is there, and what Apply does. Enter shows how, or what a step runs."
            }
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
    Otto,
    Studio,
    Node,
    Adapters,
    Bridge,
    CcConnect,
    Skills,
    Profiles,
    Step(usize),
    Apply,
}

/// What is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// What setup is for, and the topics to set up; `cursor` is a topic.
    Welcome { cursor: usize },
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
        view: View::Welcome { cursor: 0 },
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
                .chain([Row::Otto, Row::Studio])
                .collect(),
            Page::Requirements => vec![Row::Node, Row::Adapters, Row::Bridge],
            Page::Review => [
                Row::Node,
                Row::Adapters,
                Row::CcConnect,
                Row::Skills,
                Row::Profiles,
            ]
            .into_iter()
            .chain((0..self.steps.len()).map(Row::Step))
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
            "Done. Try it: run otto-ask. Settings › Agents has the rest.".into()
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
            View::Welcome { cursor } => {
                let last = TOPICS.len() - 1;
                match key {
                    Key::Up | Key::Char('k') => {
                        self.view = View::Welcome {
                            cursor: cursor.saturating_sub(1),
                        }
                    }
                    Key::Down | Key::Char('j') => {
                        self.view = View::Welcome {
                            cursor: (cursor + 1).min(last),
                        }
                    }
                    // One topic so far: the agents' pages.
                    Key::Enter | Key::Tab | Key::Right | Key::Char(' ') => self.view = View::List,
                    Key::Esc | Key::Char('q') => return Flow::Quit,
                    _ => {}
                }
            }
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
            Key::Tab | Key::Right => self.go(self.page.step(1)),
            Key::BackTab | Key::Left if self.page == Page::Agents => {
                self.view = View::Welcome { cursor: 0 }
            }
            Key::BackTab | Key::Left => self.go(self.page.step(-1)),
            Key::Char(' ') | Key::Char('x') => self.toggle(row),
            Key::Enter | Key::Char('v') => match row {
                Row::Apply => return self.try_apply(),
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
                        if harness == Harness::Claude
                            && self.choices.studio
                            && plan::studio(&self.facts, &self.choices) != Studio::Ready
                        {
                            self.choices.studio = false;
                            self.status = "Studio runs on Claude Code: unticked it too.".into();
                        }
                    } else {
                        picks.push(harness);
                        // Keep the list order, so the file does too.
                        picks.sort_by_key(|h| HARNESSES.iter().position(|x| x == h));
                    }
                    self.replan();
                }
            },
            Row::Otto => {
                if self.facts.instructions.iter().any(|n| n == harness::OTTO) {
                    self.choices.otto = !self.choices.otto;
                    self.replan();
                } else {
                    self.status = "Otto's plugin is missing. Enter says more.".into();
                }
            }
            Row::Studio => self.toggle_studio(),
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
            Row::Node | Row::Adapters | Row::CcConnect => {
                self.status = "Setup does not install software. Enter shows how.".into();
            }
            Row::Skills | Row::Profiles => {
                self.status = "Apply takes care of it. Enter shows how, and how to check.".into();
            }
            Row::Step(_) | Row::Apply => {}
        }
    }

    fn toggle_studio(&mut self) {
        if self.choices.studio {
            self.choices.studio = false;
            return;
        }
        match plan::studio(&self.facts, &self.choices) {
            Studio::Unavailable => {
                self.status = "Otto's plugin has no Studio. Enter says more.".into();
            }
            Studio::Ready => self.choices.studio = true,
            Studio::NeedsClaude => {
                if matches!(
                    plan::readiness(&self.facts, Harness::Claude),
                    Readiness::Ready(_)
                ) {
                    self.choices.picks.insert(0, Harness::Claude);
                    self.choices.studio = true;
                    self.status = "Studio runs on Claude Code: ticked it too.".into();
                    self.replan();
                } else {
                    self.status =
                        "Studio runs on Claude Code, which is not installed: Enter on it shows how."
                            .into();
                }
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

    /// The instructions or preview a row opens.
    fn text(&self, row: Row) -> String {
        match row {
            Row::Harness(harness) => guide::harness(&self.facts, harness),
            Row::Otto => guide::otto(&self.facts),
            Row::Studio => guide::studio(&self.facts, &self.choices),
            Row::Node => guide::node(&self.facts),
            Row::Adapters => guide::adapters(&self.facts),
            Row::Bridge | Row::CcConnect => guide::bridge(&self.facts),
            Row::Skills => guide::skills(&self.facts),
            Row::Profiles => guide::profiles(&self.facts, &self.choices),
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
            Row::Otto => "Otto".into(),
            Row::Studio => "Studio".into(),
            Row::Node => "Node.js".into(),
            Row::Adapters => "ACP adapters".into(),
            Row::Bridge => "Chat bridge".into(),
            Row::CcConnect => "cc-connect".into(),
            Row::Skills => "Otto's skills".into(),
            Row::Profiles => "Agent profiles".into(),
            Row::Step(index) => self.steps[index].title.into(),
            Row::Apply => "Apply".into(),
        }
    }

    fn frame(&self, (cols, rows): (usize, usize)) -> String {
        let lines = match self.view {
            View::Welcome { cursor } => self.welcome_lines(cursor, cols, rows),
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
        // The pages are the agents' for now, the one topic.
        let crumb = match (self.view, crumb) {
            (View::Welcome { .. }, _) => None,
            (_, None) => Some(TOPICS[0].0.to_owned()),
            (_, Some(crumb)) => Some(format!("{}  ›  {crumb}", TOPICS[0].0)),
        };
        otto_term_kit::header(cols, "setup", crumb.as_deref(), Some((right.0, &right.1)))
    }

    fn welcome_lines(&self, cursor: usize, cols: usize, rows: usize) -> Vec<String> {
        let mut lines = vec![
            self.header(cols, None),
            otto_term_kit::rule(cols),
            String::new(),
            plain_styled(cols, BOLD, " Welcome to Otto."),
            String::new(),
        ];
        lines.extend(WELCOME.lines().map(|line| plain(cols, &format!(" {line}"))));
        lines.push(String::new());
        lines.push(plain_styled(cols, BOLD, " What do you want to set up?"));
        lines.push(String::new());
        for (i, (title, note)) in TOPICS.iter().enumerate() {
            let selected = cursor == i;
            let base = if selected { REVERSE } else { "" };
            let mut line = Spans::new(cols);
            line.push(base, if selected { " ▸ " } else { "   " });
            line.push(&format!("{base}{BOLD}"), &format!("{title:<TITLE_WIDTH$}"));
            line.push(&format!("{base}{DIM}"), &format!(" {note}"));
            if selected {
                line.fill(REVERSE);
            }
            lines.push(line.finish());
        }
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        lines.push(plain_styled(cols, DIM, " ↑↓ move  enter set up  q quit"));
        lines
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
            let heading = match (self.page, row) {
                (_, Row::Otto) => Some(" Otto in your agents"),
                (Page::Requirements, Row::Node) => Some(" Required"),
                (Page::Requirements, Row::Bridge) => Some(" Optional"),
                (Page::Review, Row::Node) => Some(" Checks"),
                (Page::Review, Row::Step(0)) => Some(" Steps"),
                _ => None,
            };
            if let Some(heading) = heading {
                if i > 0 {
                    lines.push(String::new());
                }
                lines.push(plain_styled(cols, BOLD, heading));
            }
            if *row == Row::Apply {
                lines.push(String::new());
            }
            lines.push(self.row_line(*row, self.cursor == i, cols));
        }
        // On a short terminal the gap goes first, so the help line stays.
        if lines.len() + 3 <= rows {
            lines.push(String::new());
        }
        lines.push(plain(cols, &format!(" {}", self.status)));
        while lines.len() + 1 < rows {
            lines.push(String::new());
        }
        let help = match self.page {
            Page::Agents => " ↑↓ move  space tick  enter what it needs  tab next  q quit",
            Page::Requirements => " ↑↓ move  space tick  enter what to do  tab next  q quit",
            Page::Review => " ↑↓ move  enter how-to or preview  a apply  tab back  q quit",
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
            Row::Otto => {
                let mark = if self.choices.otto { "[x]" } else { "[ ]" };
                if self.facts.instructions.iter().any(|n| n == harness::OTTO) {
                    (
                        mark,
                        GREEN,
                        "answers in Ask as Otto, knowing your desktop".into(),
                        DIM,
                    )
                } else {
                    ("[ ]", DIM, "Otto's plugin not found".into(), YELLOW)
                }
            }
            Row::Studio => {
                let mark = if self.choices.studio { "[x]" } else { "[ ]" };
                match plan::studio(&self.facts, &self.choices) {
                    Studio::Unavailable => ("[ ]", DIM, "not in Otto's plugin".into(), YELLOW),
                    Studio::Ready => (
                        mark,
                        GREEN,
                        "works on the file open in Studio, from its chat".into(),
                        DIM,
                    ),
                    Studio::NeedsClaude => (
                        mark,
                        GREEN,
                        "works on the file open in Studio · needs Claude Code".into(),
                        YELLOW,
                    ),
                }
            }
            Row::Node => match &self.facts.node_version {
                Some(version) => (
                    " ✓ ",
                    GREEN,
                    format!("{version} · npx {}", self.found("npx")),
                    DIM,
                ),
                None => (" ✗ ", RED, "not installed · enter for how".into(), RED),
            },
            Row::Adapters => {
                let adapters = plan::adapters(&self.facts, &self.choices);
                let named = |want: plan::Adapter| -> Vec<&str> {
                    adapters
                        .iter()
                        .filter(|(_, state)| *state == want)
                        .filter_map(|(h, _)| h.adapter().map(|a| a.bin))
                        .collect()
                };
                let missing = named(plan::Adapter::Missing);
                if adapters.is_empty() {
                    (" ✓ ", GREEN, "none of your agents needs one".into(), DIM)
                } else if !missing.is_empty() {
                    (
                        " ✗ ",
                        RED,
                        format!("missing {} · enter for how", missing.join(", ")),
                        RED,
                    )
                } else {
                    let mut parts: Vec<String> = named(plan::Adapter::Installed)
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                    parts.extend(
                        named(plan::Adapter::Npx)
                            .into_iter()
                            .map(|bin| format!("{bin} via npx")),
                    );
                    (" ✓ ", GREEN, parts.join(", "), DIM)
                }
            }
            Row::Bridge => match plan::bridge(&self.facts) {
                Bridge::Running => (
                    "[✓]",
                    GREEN,
                    "message your agents from Telegram · running".into(),
                    DIM,
                ),
                Bridge::Off | Bridge::Ready => {
                    let mark = if self.choices.bridge { "[x]" } else { "[ ]" };
                    let note = if plan::bridge(&self.facts) == Bridge::Off {
                        "message your agents from Telegram · set up, off"
                    } else {
                        "message your agents from Telegram · ready"
                    };
                    (mark, GREEN, note.into(), DIM)
                }
                Bridge::Missing(_) => (
                    "[ ]",
                    DIM,
                    "message your agents from Telegram · enter to set it up".into(),
                    DIM,
                ),
                Bridge::Unavailable(why) => ("[ ]", DIM, why.into(), DIM),
            },
            Row::CcConnect => match self.facts.found.get("cc-connect") {
                Some(path) => (
                    " ✓ ",
                    GREEN,
                    format!("{} · for the chat bridge", self.facts.shown(path)),
                    DIM,
                ),
                None => (
                    " · ",
                    DIM,
                    "not installed · optional, for the chat bridge".into(),
                    DIM,
                ),
            },
            Row::Skills => {
                let skills = plan::skills(&self.facts);
                let names = |pick: fn(&crate::plugins::Skill) -> bool| -> String {
                    skills
                        .iter()
                        .filter(|s| pick(s))
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if self.facts.plugins.is_none() {
                    (" ✗ ", RED, "otto-agents cannot say · enter".into(), RED)
                } else if skills.iter().any(crate::plugins::Skill::in_the_way) {
                    (
                        " ✗ ",
                        RED,
                        format!(
                            "{}: something of yours is in the way · enter",
                            names(crate::plugins::Skill::in_the_way)
                        ),
                        RED,
                    )
                } else if skills.iter().all(crate::plugins::Skill::linked) {
                    (" ✓ ", GREEN, format!("{} linked", names(|_| true)), DIM)
                } else {
                    (
                        " ● ",
                        YELLOW,
                        format!("Apply links {}", names(|s| !s.linked())),
                        YELLOW,
                    )
                }
            }
            Row::Profiles => {
                let profiles = plan::profiles(&self.facts, &self.choices);
                let behind: Vec<&str> = {
                    let mut v: Vec<&str> = profiles
                        .iter()
                        .filter(|r| !r.current())
                        .map(|r| r.vendor.as_str())
                        .collect();
                    v.dedup();
                    v
                };
                if !self.choices.otto {
                    (
                        " ✓ ",
                        GREEN,
                        "none: the new agents are the plain harness".into(),
                        DIM,
                    )
                } else if profiles.is_empty() {
                    (
                        " ✓ ",
                        GREEN,
                        "none needed: Claude Code reads Otto's plugin".into(),
                        DIM,
                    )
                } else if behind.is_empty() {
                    let mut v: Vec<&str> = profiles.iter().map(|r| r.vendor.as_str()).collect();
                    v.dedup();
                    (" ✓ ", GREEN, format!("current for {}", v.join(", ")), DIM)
                } else {
                    (
                        " ● ",
                        YELLOW,
                        format!("Apply writes {}", behind.join(", ")),
                        YELLOW,
                    )
                }
            }
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

/// What the welcome says: what setup is for, what it does and leaves to
/// the user, and where the rest of the configuration lives.
const WELCOME: &str = "\
This helps you configure Otto. It bootstraps what a part of Otto needs
from your system: it finds what is installed, sets up what it can, and
explains what it cannot do for you, such as installing a program or
signing in. Nothing changes until you apply.

For the finer configuration, there is Settings.";

/// What setup can set up, as a title and a line on what it is. More
/// dependencies come here as setup grows.
const TOPICS: [(&str, &str); 1] = [(
    "Otto Agents",
    "the agents behind Ask and Sessions, used from Otto itself",
)];

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
