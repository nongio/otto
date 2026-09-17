//! otto-launcher — type to find something, Enter to have it.
//!
//! A fullscreen overlay that takes the keyboard, filters a list of [`Item`]s
//! from one or more [`Source`]s, and activates the one that is selected. Apps
//! and open windows are the two sources it ships with; a source is a trait, so
//! files, clipboard history or a calculator are additions rather than rewrites.
//!
//! Run it and it stays up until something is picked, Escape is pressed, or the
//! click lands outside the card. It is meant to be bound to a key and started
//! fresh each time — there is no daemon, and nothing to keep warm: the desktop
//! entry scan is the only work at startup, and it is milliseconds.

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use otto_kit::accessibility::{A11yTree, Action, ActionRequest, Role};
use otto_kit::clipboard;
use otto_kit::components::scroll::{Axis, RowLayout, ScrollContent, ScrollPane};
use otto_kit::components::text_input::{
    KeyMods, TextInput, TextInputKey, TextInputResponse, CARET_BLINK_PERIOD,
};
use otto_kit::focus::FocusId;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::protocols::otto_timing_function_v1::Preset;
use otto_kit::surfaces::{LayerShellSurface, SubsurfaceSurface};
use otto_kit::CursorShape;
use otto_kit::{App, AppContext, AppRunner, ObjectId};
use skia_safe::Rect;
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind};
use wayland_client::protocol::{wl_keyboard, wl_surface};
use wayland_client::Proxy;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use otto_launcher::apps::Apps;
use otto_launcher::ask::{attached_text, Ask, Note, Status, Step, Terminal};
use otto_launcher::calc::Calculator;
use otto_launcher::log::{self as ask_log, lay_out, Block, Line as LogLine};
use otto_launcher::selection::{self, Caret, Selection, Span};
use otto_launcher::source::{rank, Item, Origin, Source};
use otto_launcher::view::{
    field_style, Palette, CARD_W, FIELD_H, HIGHLIGHT_RADIUS, LIST_TOP, LOG_LINE_H, LOG_W,
    MAX_CARD_H, MAX_ROWS, RADIUS, ROW_H,
};
use otto_launcher::windows;

/// How long the scene is kept painting after the card's height changes, so
/// its resize is seen through to the end.
const SETTLE: Duration = Duration::from_millis(220);

/// A frame the compositor never answered must not freeze the launcher.
const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

/// How the card arrives and leaves.
///
/// It scales and fades rather than sliding: the launcher appears where it is
/// going to stay. The two are deliberately out of step. The fade is quick — the
/// card should be *there* almost at once, because it is about to be typed into
/// — while the scale takes its time and springs past full size before settling,
/// which is what gives the arrival some life. Leaving is quicker than arriving,
/// and does not bounce: on the way out there is nothing to settle into.
const FADE_IN: Duration = Duration::from_millis(90);
const SCALE_IN: Duration = Duration::from_millis(340);

/// How long after a change of state the card's changes of size spring. Long
/// enough for what the new state shows — a session's conversation, the list
/// of sessions — to come back from otto-agentsd and spring in too.
const SPRING_WINDOW: Duration = Duration::from_millis(400);
const FADE_OUT: Duration = Duration::from_millis(90);
const SCALE_OUT: Duration = Duration::from_millis(110);

/// How much the scale overshoots on the way in. Enough to notice, not enough
/// to wobble.
const BOUNCE: f64 = 0.35;

/// When the launcher may stop — the longest thing the exit is waiting on.
const CLOSE: Duration = Duration::from_millis(120);

/// How long closing may wait on an ask request still on its way to otto-agentsd.
/// Past this the service is not answering, and the launcher goes anyway.
const HAND_OFF_GRACE: Duration = Duration::from_secs(3);

/// How small the card is before it arrives, and again once it has gone. Near
/// enough to full size that it reads as a swell rather than a zoom.
const OPEN_SCALE: f64 = 0.96;
const CLOSE_SCALE: f64 = 0.96;

struct Launcher {
    /// The fullscreen surface. It carries the scrim, takes the keyboard, and
    /// catches the click that lands outside the card.
    surface: Option<LayerShellSurface>,
    /// The card. A surface of its own so the compositor can frost what is
    /// behind *it* rather than behind the whole screen.
    card: Option<SubsurfaceSurface>,
    palette: Option<Palette>,
    /// Last size handed to the card's style, so an unchanged frame does not
    /// re-send it.
    card_size: (f32, f32),
    /// Last rectangle handed to the two surfaces' input regions, for the same
    /// reason.
    input_region: Option<(i32, i32, i32, i32)>,

    sources: Vec<Box<dyn Source>>,
    labels: Vec<&'static str>,
    /// Everything the sources have, which is what a query is ranked against.
    items: Vec<Item>,
    /// What is shown before anything is typed — the last few applications
    /// launched, and every window in the switcher.
    resting: Vec<Item>,
    /// The rows as displayed: answers derived from the query, then either the
    /// resting list or the ranked matches.
    rows: Vec<Item>,

    input: TextInput,
    /// Index into `matches` of the highlighted row.
    selected: usize,
    /// The result rows: a scroll pane over the card, so scrolling the list or
    /// moving the selection repaints neither the card nor the rows. `None`
    /// until the card exists.
    list: Option<ScrollPane>,
    /// Moves whenever the rows would paint differently.
    list_revision: u64,
    /// The list still has a scroll or a highlight slide in hand.
    list_busy: bool,
    /// The selection is the pointer's: it follows the row under the pointer
    /// as the list glides, with no event to say so. Cleared by the keyboard.
    follow_pointer: bool,
    /// The card is being dragged: where the pointer took hold of it, in the
    /// card's coordinates.
    dragging: Option<(f32, f32)>,
    /// Where the card subsurface was last placed on the parent. Pointer
    /// positions over the card arrive relative to this.
    card_placed: (f32, f32),

    shift: bool,
    sized: bool,
    /// Set once the launcher has the keyboard, or has been interacted with,
    /// after which losing the keyboard means "gone" rather than "not arrived
    /// yet".
    engaged: bool,

    /// Set once the card has something on it and the entrance has been
    /// started, so it is started exactly once.
    opened: bool,
    /// When the closing animation will have finished, and the launcher can
    /// stop. Input is ignored from the moment this is set.
    closing_at: Option<Instant>,

    dirty: bool,
    painted_at: Option<Instant>,
    /// The parent surface has been painted since it was last configured. It
    /// draws nothing, so once is enough: see [`Launcher::paint`].
    parent_painted: bool,
    settle_until: Option<Instant>,
    last_tick: Instant,

    /// Ask mode's connection to otto-agentsd, and the request once it is made.
    ask: Option<Ask>,
    /// The ask log, laid out for the card.
    log: Vec<LogLine>,
    /// The answer and its status as plain text, for assistive technologies.
    log_text: String,
    /// The log keeps its end in view as it grows. Scrolling up stops that,
    /// and scrolling back to the end starts it again.
    log_following: bool,
    /// The ask log's pane, above the field. `None` until the card exists.
    log_pane: Option<ScrollPane>,
    /// Moves whenever the log would paint differently.
    log_revision: u64,
    /// The log pane still has a scroll in hand.
    log_busy: bool,
    /// Every piece of text in the log, as painted, so it can be selected.
    log_spans: Vec<Span>,
    /// What is selected in the log, when anything is.
    log_selection: Option<Selection>,
    /// The selection as boxes to paint, kept beside it so a scroll or a
    /// repaint does not measure the text again.
    selection_rects: Vec<Rect>,
    /// A selection being dragged out: where the drag took hold of the text.
    selecting: Option<Caret>,
    /// Whether the pointer was last over the log's words, so the cursor is
    /// only asked for when it changes.
    over_log_text: bool,
    /// The last press in the log — when, where, and how many presses have run
    /// together — so a second picks a word and a third the line.
    last_press: Option<(u32, f32, f32, u32)>,
    /// The tool call id of the agent's question the rows answer, so a new
    /// question can start from its default answer.
    asked: Option<String>,
    /// Agents mode, until a session is picked: the rows are the sessions, and
    /// what is typed narrows them.
    picking: bool,
    /// The session opened from the list, by URI, so going back to the list
    /// highlights it again wherever it now sits.
    opened_session: Option<String>,
    /// The session to highlight once the list of sessions arrives.
    return_to: Option<String>,
    /// Down has listed the agents under the field, to send the first request
    /// to another than the default.
    choosing_agent: bool,
    /// Until when the card's changes of size spring, as the card does on
    /// opening, instead of snapping; see [`Launcher::spring`].
    spring_until: Option<Instant>,
    /// Where the field sat down the card at the last resize: how much log was
    /// above it.
    log_top: f32,
}

/// The query field's identity for assistive technologies.
const FIELD: FocusId = FocusId::from_raw(0xF1E1_D000);
/// The results list's.
const RESULTS: FocusId = FocusId::from_raw(0xF1E1_D001);
/// The ask log's.
const LOG: FocusId = FocusId::from_raw(0xF1E1_D002);

/// One result row's, by its position in the list.
fn row_focus(index: usize) -> FocusId {
    FocusId::new(format!("row-{index}"))
}

/// Which sources a run offers. Everything, by default; the single-kind scopes
/// exist so a binding can mean "switch window" specifically, the way rofi's
/// `-show window` does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Everything,
    Apps,
    Windows,
    /// What is typed is a request for an agent, handed to otto-agentsd.
    Ask,
    /// The agent sessions otto-agentsd has, to pick one and carry it on in ask
    /// mode.
    Agents,
}

impl Scope {
    /// What the field says when it is empty. It names the mode, because the
    /// two bindings mean different things and the field is the only place that
    /// says which one is up.
    fn placeholder(self) -> &'static str {
        match self {
            Scope::Everything => otto_kit::t!("launcher-search-everything"),
            Scope::Apps => otto_kit::t!("launcher-search-apps"),
            Scope::Windows => otto_kit::t!("launcher-search-windows"),
            Scope::Ask => otto_kit::t!("launcher-search-ask"),
            Scope::Agents => otto_kit::t!("launcher-search-agents"),
        }
    }
}

/// Ask mode's rows: the agents, sessions or answers to pick from…
const ASK_ROWS: usize = 0;
/// …and, under them, the files going with the next request, which are only
/// shown.
const ATTACHMENT_ROWS: usize = 1;

/// When the process started, for the startup timings. A launcher is judged on
/// how long it takes to appear, so the stages that make up that time are
/// measurable without a profiler.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn since_start() -> u128 {
    STARTED.get_or_init(Instant::now).elapsed().as_millis()
}

impl Launcher {
    /// `query` is what the field starts with — the command line's optional
    /// argument, for a binding that opens the launcher already narrowed.
    fn new(query: &str, scope: Scope) -> Self {
        let mut sources: Vec<Box<dyn Source>> = Vec::new();
        let mut labels: Vec<&'static str> = Vec::new();

        // Ask mode has no sources. Its rows are the agents to ask, which the
        // connection brings, and a badge on them would only repeat the mode.
        // Agents mode is ask mode that starts from a list of sessions.
        let ask = matches!(scope, Scope::Ask | Scope::Agents).then(|| {
            // Two kinds of row, `ASK_ROWS` and `ATTACHMENT_ROWS`, neither
            // badged.
            labels.extend(["", ""]);
            Ask::open()
        });

        if matches!(scope, Scope::Everything | Scope::Apps) {
            let apps = Apps::load(sources.len());
            tracing::debug!(ms = since_start(), "desktop entries scanned");
            labels.push(apps.label());
            sources.push(Box::new(apps));

            let calculator = Calculator::new(sources.len());
            labels.push(calculator.label());
            sources.push(Box::new(calculator));
        }

        if matches!(scope, Scope::Everything | Scope::Windows) {
            let started = Instant::now();
            let connected = windows::Windows::connect(sources.len(), scope == Scope::Windows);
            tracing::debug!(ms = started.elapsed().as_millis(), "toplevels listed");
            match connected {
                Some(windows) => {
                    labels.push(windows.label());
                    sources.push(Box::new(windows));
                }
                None => tracing::info!("no foreign-toplevel protocol; windows will not be listed"),
            }
        }

        let mut input = TextInput::editing(query, field_style(dark()));
        input.state.placeholder = scope.placeholder().to_string();
        // Without a box to be laid out in, the field scrolls the text it is
        // given out of its own (zero-width) clip and draws nothing.
        input.set_size(CARD_W, FIELD_H);

        Self {
            surface: None,
            card: None,
            palette: None,
            card_size: (0.0, 0.0),
            input_region: None,
            sources,
            labels,
            items: Vec::new(),
            resting: Vec::new(),
            rows: Vec::new(),
            input,
            selected: 0,
            list: None,
            list_revision: 0,
            list_busy: false,
            follow_pointer: false,
            dragging: None,
            card_placed: (0.0, 0.0),
            shift: false,
            sized: false,
            engaged: false,
            opened: false,
            closing_at: None,
            dirty: true,
            painted_at: None,
            parent_painted: false,
            settle_until: None,
            last_tick: Instant::now(),
            ask,
            log: Vec::new(),
            log_text: String::new(),
            log_following: true,
            log_pane: None,
            log_revision: 0,
            log_busy: false,
            log_spans: Vec::new(),
            log_selection: None,
            selection_rects: Vec::new(),
            selecting: None,
            over_log_text: false,
            last_press: None,
            asked: None,
            picking: scope == Scope::Agents,
            opened_session: None,
            return_to: None,
            choosing_agent: false,
            spring_until: None,
            log_top: 0.0,
        }
    }

    /// Ask every source for its items again, keeping the selection on the same
    /// item where that is still possible.
    fn reload(&mut self) {
        let previous = self.selected_origin();
        self.items = self
            .sources
            .iter_mut()
            .flat_map(|source| source.items())
            .collect();
        self.resting = self
            .sources
            .iter_mut()
            .flat_map(|source| source.resting())
            .collect();
        self.refilter();

        if let Some(origin) = previous {
            if let Some(position) =
                (0..self.row_count()).find(|row| self.row(*row).is_some_and(|i| i.origin == origin))
            {
                self.selected = position;
                self.scroll_to_selection();
            }
        }
    }

    fn refilter(&mut self) {
        // What is typed in ask mode is the request, so it filters nothing: the
        // rows are the agents, and the selection stays on the one picked.
        // Once a request is made, the rows are the answers to the agent's
        // question, when it asked one. The files going with the next request
        // are listed under either, until it is sent.
        if let Some(ask) = self.ask.as_ref() {
            let mut returned = false;
            let question = ask.question();
            // What the agent asked the person, when no permission question
            // comes first: its rows are the answers to the question at hand.
            let input = question.is_none().then(|| ask.input_key()).flatten();
            self.rows = if question.is_some() {
                ask.question_rows(ASK_ROWS)
            } else if input.is_some() {
                ask.input_rows(ASK_ROWS)
            } else if self.picking {
                let rows = ask.session_rows(ASK_ROWS, self.input.value());
                if let Some(resource) = self.return_to.as_deref() {
                    let found = rows
                        .iter()
                        .position(|item| ask.session_at(item.origin.index) == Some(resource));
                    if let Some(position) = found {
                        self.selected = position;
                        self.return_to = None;
                        returned = true;
                    } else if ask.sessions_listed() {
                        self.return_to = None;
                    }
                }
                rows
            } else if ask.running() || !self.choosing_agent {
                Vec::new()
            } else {
                ask.agent_rows(ASK_ROWS)
            };
            if question.is_none() && input.is_none() && !self.picking {
                self.rows.extend(ask.attachment_rows(ATTACHMENT_ROWS));
            }
            let asked = question
                .as_ref()
                .map(|q| q.tool_call_id.clone())
                .or_else(|| input.clone());
            if asked != self.asked {
                // A new question offers its narrowest allowing answer first;
                // one the agent asked the person, the answer already given,
                // or the one it suggests.
                self.selected = match question.as_ref() {
                    Some(question) => question.default_choice(),
                    None => ask.input_default_row(),
                };
                if input.is_some() {
                    if let Some(text) = ask.input_prefill() {
                        self.input.set_value(text);
                    }
                }
                match ask.input_placeholder().filter(|_| input.is_some()) {
                    Some(placeholder) => self.input.state.placeholder = placeholder.to_string(),
                    None if ask.running() && !self.picking => {
                        self.input.state.placeholder =
                            otto_kit::t!("launcher-search-ask-more").to_string()
                    }
                    None => {}
                }
                self.asked = asked;
            }
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            if returned {
                self.scroll_to_selection();
            }
            self.list_revision = self.list_revision.wrapping_add(1);
            self.dirty = true;
            self.update_completion();
            return;
        }

        let query = self.input.value().to_string();

        let mut rows: Vec<Item> = self
            .sources
            .iter_mut()
            .filter_map(|source| source.answer(&query))
            .collect();

        if query.trim().is_empty() {
            // Nothing typed: the sources' own idea of what is worth showing,
            // not everything they have.
            rows.extend(self.resting.iter().cloned());
        } else {
            rows.extend(
                rank(&self.items, &query)
                    .into_iter()
                    .filter_map(|matched| self.items.get(matched.index).cloned()),
            );
        }

        self.rows = rows;
        self.selected = 0;
        if let Some(list) = self.list.as_mut() {
            list.scroll_to(0.0);
        }
        self.list_revision = self.list_revision.wrapping_add(1);
        self.dirty = true;
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row(&self, index: usize) -> Option<&Item> {
        self.rows.get(index)
    }

    fn selected_origin(&self) -> Option<Origin> {
        Some(self.row(self.selected)?.origin)
    }

    fn move_selection(&mut self, delta: isize) {
        // While the agents are listed, the selection stays among them rather
        // than wandering onto the attached files under them.
        let count = if self.choosing_agent {
            self.rows
                .iter()
                .filter(|row| row.origin.source == ASK_ROWS)
                .count()
        } else {
            self.row_count()
        } as isize;
        if count == 0 {
            return;
        }
        // Wrapping, because a list that stops at the end makes someone check
        // where the end was.
        self.selected = (self.selected as isize + delta).rem_euclid(count) as usize;
        // The keyboard has the selection now, wherever the pointer is.
        self.follow_pointer = false;
        self.scroll_to_selection();
        self.dirty = true;
    }

    fn scroll_to_selection(&mut self) {
        if let Some(list) = self.list.as_mut() {
            let top = self.selected as f32 * ROW_H;
            list.reveal(top, top + ROW_H);
        }
    }

    /// Which result the point `(x, y)` on the card is over, through the list's
    /// scroll.
    fn list_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let list = self.list.as_ref()?;
        let point = skia_safe::Point::new(x, y);
        if !list.contains(point) {
            return None;
        }
        RowLayout::new(ROW_H, self.row_count()).index_at(list.parent_to_content(point).y)
    }

    /// Bring the list pane in line with the rows, the selection and the card.
    fn sync_list(&mut self) {
        if !self.sized {
            return;
        }
        // A fling carries on after the fingers lift, and the row under a
        // still pointer changes with it: the pane says which row that is.
        if self.follow_pointer {
            let rows = RowLayout::new(ROW_H, self.row_count());
            let hovered = self.list.as_ref().and_then(ScrollPane::hovered);
            if let Some(row) = hovered.and_then(|point| rows.index_at(point.y)) {
                self.selected = row;
            }
        }
        // An attached file is not something to pick, so it is never shown
        // picked.
        let highlighted = !self.attachment_row(self.selected);
        let (Some(list), Some(palette)) = (self.list.as_mut(), self.palette.as_ref()) else {
            return;
        };
        let viewport = palette.list_rect();
        if viewport.height() <= 0.0 {
            list.set_hidden(true);
            self.list_busy = false;
            return;
        }
        list.set_hidden(false);
        list.set_viewport(viewport);
        list.set_highlight(
            highlighted.then(|| Palette::highlight_rect(self.selected)),
            palette.highlight_color(),
            HIGHLIGHT_RADIUS,
        );
        let items: Vec<&Item> = self.rows.iter().collect();
        let content = Rows {
            palette,
            items: &items,
            labels: &self.labels,
            compact_source: self.ask.is_some().then_some(ATTACHMENT_ROWS),
            revision: self.list_revision,
        };
        self.list_busy = list.update(&content, &AppContext::current_theme());
    }

    /// Bring the log pane above the field in line with the ask log, keeping
    /// its end in view while it follows.
    fn sync_log(&mut self) {
        if !self.sized {
            return;
        }
        let (Some(pane), Some(palette)) = (self.log_pane.as_mut(), self.palette.as_ref()) else {
            return;
        };
        let viewport = palette.log_rect();
        if viewport.height() <= 0.0 {
            pane.set_hidden(true);
            self.log_busy = false;
            return;
        }
        pane.set_hidden(false);
        pane.set_viewport(viewport);
        let content = LogRows {
            palette,
            lines: &self.log,
            selection: &self.selection_rects,
            revision: self.log_revision,
        };
        self.log_busy = pane.update(&content, &AppContext::current_theme());
        let end = (ask_log::length(&self.log) - viewport.height()).max(0.0);
        if self.log_following {
            if (pane.offset() - end).abs() > 0.5 {
                pane.scroll_to(end);
                self.log_busy = true;
            }
        } else if pane.offset() >= end - 0.5 && !pane.is_animating() {
            self.log_following = true;
        }
    }

    /// Carry out the selection and leave. A source that refuses says why and
    /// the launcher stays up, because the alternative is vanishing without
    /// having done anything.
    fn activate(&mut self) {
        if self.picking {
            self.resume_selected();
            return;
        }
        if self.ask.is_some() {
            // With nothing typed, Return answers the agent's question when it
            // asked one; otherwise it sends what is typed.
            let empty = self.input.value().trim().is_empty();
            if self.asked_question() && empty {
                self.answer_selected();
            } else if self.asking_input() && empty {
                self.answer_input(Some(self.selected));
            } else if self.asking_input() && self.ask.as_ref().is_some_and(Ask::input_takes_text) {
                // What is typed answers the question, when it takes words.
                self.answer_input(None);
            } else {
                self.send_ask();
            }
            return;
        }
        let Some(origin) = self.selected_origin() else {
            return;
        };
        let Some(source) = self.sources.get_mut(origin.source) else {
            return;
        };
        match source.activate(origin.index) {
            Ok(()) => self.close(),
            Err(err) => {
                tracing::error!(%err, "could not activate the selection");
                self.dirty = true;
            }
        }
    }

    // === Asking ===

    /// Whether a request has been made, and the card is its log.
    /// Whether an agent can still be picked: ask mode, more than one agent,
    /// nothing asked yet, and no session being picked or question answered.
    /// The agent the next request would go to: the row picked in the agent
    /// list, if one is picked.
    fn chosen_agent(&self) -> Option<usize> {
        self.selected_origin()
            .filter(|origin| origin.source == ASK_ROWS)
            .map(|origin| origin.index)
    }

    /// Offer the rest of the skill name being typed, in grey after the caret.
    /// Tab takes it; typing past it, or away from it, drops it.
    fn update_completion(&mut self) {
        let agent = self.chosen_agent();
        let ghost = self
            .ask
            .as_ref()
            .filter(|ask| !self.picking && ask.question().is_none() && ask.input().is_none())
            .and_then(|ask| ask.completion(self.input.value(), agent))
            .unwrap_or_default();
        if self.input.state.ghost != ghost {
            self.input.state.ghost = ghost;
            self.dirty = true;
        }
    }

    /// Take the offered completion, with a space after it so the request
    /// carries straight on. Returns whether there was one to take.
    fn accept_completion(&mut self) -> bool {
        let ghost = std::mem::take(&mut self.input.state.ghost);
        if ghost.is_empty() {
            return false;
        }
        let completed = format!("{}{ghost} ", self.input.value());
        self.input.set_value(completed);
        self.refilter();
        true
    }

    fn can_choose_agent(&self) -> bool {
        !self.picking
            && !self.asked_question()
            && self
                .ask
                .as_ref()
                .is_some_and(|ask| !ask.running() && !ask.agent_rows(ASK_ROWS).is_empty())
    }

    fn ask_running(&self) -> bool {
        self.ask.as_ref().is_some_and(Ask::running)
    }

    /// Whether the agent's question is waiting, and the rows are its answers.
    fn asked_question(&self) -> bool {
        self.ask
            .as_ref()
            .is_some_and(|ask| ask.question().is_some())
    }

    /// Whether the agent asked the person something, and the rows are the
    /// answers to the question at hand.
    fn asking_input(&self) -> bool {
        !self.asked_question() && self.ask.as_ref().is_some_and(|ask| ask.input().is_some())
    }

    /// Whether the row at `index` is an attached file.
    fn attachment_row(&self, index: usize) -> bool {
        self.ask.is_some()
            && self
                .row(index)
                .is_some_and(|item| item.origin.source == ATTACHMENT_ROWS)
    }

    /// Attach `files` to the first request, and open the session `session`
    /// names instead of starting one, when there is one.
    fn prepare_ask(&mut self, files: Vec<PathBuf>, session: Option<&str>) {
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        ask.attach(files);
        if let Some(session) = session {
            ask.resume(session);
            self.follow_session();
        }
    }

    /// How to open a session in a terminal: the one being followed, or, while
    /// the list of sessions is up, the one highlighted in it.
    fn selected_terminal(&self) -> Option<Terminal> {
        let ask = self.ask.as_ref()?;
        if let Some(terminal) = ask.terminal() {
            return Some(terminal.clone());
        }
        if !self.picking {
            return None;
        }
        let origin = self.selected_origin()?;
        (origin.source == ASK_ROWS)
            .then(|| ask.terminal_at(origin.index))
            .flatten()
    }

    /// Open the session picked from the list.
    fn resume_selected(&mut self) {
        let Some(index) = self.selected_origin().map(|origin| origin.index) else {
            return;
        };
        self.opened_session = self
            .ask
            .as_ref()
            .and_then(|ask| ask.session_at(index))
            .map(str::to_string);
        if self.ask.as_mut().is_some_and(|ask| ask.resume_at(index)) {
            self.input.set_value("");
            self.follow_session();
        }
    }

    /// The session is chosen: from here on the launcher is its conversation,
    /// as in ask mode.
    fn follow_session(&mut self) {
        self.spring();
        self.picking = false;
        self.input.state.placeholder = otto_kit::t!("launcher-search-ask-more").to_string();
        self.rows.clear();
        self.selected = 0;
        self.log_following = true;
        self.refilter();
        self.relayout_log();
    }

    /// Spring the card's changes of size for a moment, as it springs open: the
    /// launcher is changing state, and what the new state shows may take a
    /// round trip to otto-agentsd to arrive.
    fn spring(&mut self) {
        self.spring_until = Some(Instant::now() + SPRING_WINDOW);
    }

    /// Leave the session for the list of sessions, as a back button would.
    ///
    /// The connection goes with the session and a new one lists the sessions
    /// afresh, so the one just left is there, most recently changed. Leaving
    /// costs the session nothing, as closing the launcher does not: the
    /// service owns its requests, and its questions go to a dialog.
    fn back_to_sessions(&mut self) {
        self.spring();
        self.ask = Some(Ask::open());
        self.picking = true;
        // Back where the user was, once the list arrives: the session left.
        self.return_to = self.opened_session.take();
        self.input.set_value("");
        self.input.state.placeholder = Scope::Agents.placeholder().to_string();
        self.selected = 0;
        self.asked = None;
        self.choosing_agent = false;
        self.log.clear();
        self.log_text.clear();
        self.log_revision = self.log_revision.wrapping_add(1);
        self.log_following = true;
        self.refilter();
    }

    /// Hand what is typed to the agent, and move it into the log above the
    /// field. The field empties for the next request, which queues behind
    /// whatever the agent is doing. The launcher stays up: closing it is
    /// always the user's call.
    fn send_ask(&mut self) {
        let prompt = self.input.value().trim().to_string();
        let agent = self.chosen_agent();
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        if prompt.is_empty() {
            return;
        }
        let first = !ask.running();
        ask.send(&prompt, agent);
        self.input.set_value("");
        if first {
            // The agent is chosen for the session now, so its list goes.
            self.input.state.placeholder = otto_kit::t!("launcher-search-ask-more").to_string();
            self.selected = 0;
            self.choosing_agent = false;
        }
        // The attached files went with the request, into the log.
        self.refilter();
        self.log_following = true;
        self.relayout_log();
    }

    /// Answer the agent's question with the selected row.
    fn answer_selected(&mut self) {
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        if ask.answer(self.selected) {
            self.log_following = true;
            self.refilter();
            self.relayout_log();
        }
    }

    /// Answer the question the agent asked: with the row at `row`, or with
    /// what is typed when `row` is `None`.
    fn answer_input(&mut self, row: Option<usize>) {
        let typed = self.input.value().to_string();
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        let answered = match row {
            Some(row) => ask.choose_input(row, &typed),
            None => ask.answer_input(&typed),
        };
        if answered.took_text {
            self.input.set_value("");
        }
        self.log_following = true;
        self.refilter();
        if answered.opened {
            // The link is open; what is left is saying so.
            self.selected = self.ask.as_ref().map_or(0, Ask::input_default_row);
            self.list_revision = self.list_revision.wrapping_add(1);
        }
        self.relayout_log();
    }

    /// Lay the log out again from the conversation.
    fn relayout_log(&mut self) {
        let (Some(ask), Some(palette)) = (self.ask.as_ref(), self.palette.as_ref()) else {
            return;
        };
        let Some(transcript) = ask.transcript() else {
            return;
        };
        let attached: Vec<Option<String>> = transcript
            .entries
            .iter()
            .map(|entry| attached_text(&entry.attachments))
            .collect();
        let notes: Vec<Option<String>> = transcript
            .entries
            .iter()
            .map(|entry| entry.note.as_ref().map(Note::text))
            .collect();
        let steps: Vec<Vec<String>> = transcript
            .entries
            .iter()
            .map(|entry| entry.steps.iter().map(Step::text).collect())
            .collect();
        let inputs: Vec<Vec<Vec<(String, ask_log::Style)>>> = transcript
            .entries
            .iter()
            .map(|entry| {
                entry
                    .inputs
                    .iter()
                    .map(|request| ask.input_lines(request))
                    .collect()
            })
            .collect();
        let blocks: Vec<Block> = transcript
            .entries
            .iter()
            .zip(&notes)
            .zip(&steps)
            .zip(&attached)
            .zip(&inputs)
            .map(|((((entry, note), steps), attached), inputs)| Block {
                prompt: &entry.prompt,
                attachments: attached.as_deref(),
                answer: &entry.answer,
                steps,
                inputs,
                question: entry
                    .question
                    .as_ref()
                    .map(|question| (question.title.as_str(), question.detail.as_str())),
                note: note.as_deref(),
            })
            .collect();
        let status = transcript.status.as_ref().map(Status::text);
        self.log = lay_out(&blocks, status.as_deref(), LOG_W, |text, style| {
            palette.measure_log(text, style)
        });
        self.log_text = self
            .log
            .iter()
            .map(LogLine::text)
            .collect::<Vec<_>>()
            .join("\n");
        // What can be selected, rebuilt with the lines it belongs to. A
        // selection whose text has since been laid out differently — the log
        // was cleared, or a request withdrawn — is dropped rather than left
        // highlighting whatever now sits at those coordinates.
        self.log_spans = palette.log_spans(&self.log);
        match self.log_selection {
            Some(selection) if selection.fits(&self.log_spans) => {
                // The words may have been laid out somewhere else — the card
                // is a different width, or a line above re-wrapped — so the
                // highlight is measured again against where they are now.
                self.selection_rects = selection::rects(&self.log_spans, selection);
            }
            Some(_) => {
                self.log_selection = None;
                self.selection_rects.clear();
            }
            None => {}
        }
        self.log_revision = self.log_revision.wrapping_add(1);
        self.dirty = true;
    }

    /// Select `selection` in the log — or nothing, with `None` — and repaint
    /// what changed.
    fn set_log_selection(&mut self, selection: Option<Selection>) {
        let selection = selection.filter(|selection| !selection.is_empty());
        let same = match (self.log_selection, selection) {
            (Some(before), Some(now)) => before.range() == now.range(),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.selection_rects = selection
            .map(|selection| selection::rects(&self.log_spans, selection))
            .unwrap_or_default();
        self.log_selection = selection;
        self.log_revision = self.log_revision.wrapping_add(1);
        self.dirty = true;
    }

    /// What is selected in the log, as text.
    fn selected_log_text(&self) -> Option<String> {
        let selection = self.log_selection?;
        let text = selection::text(&self.log_spans, selection);
        (!text.is_empty()).then_some(text)
    }

    /// Where a point on the card is in the log's own content coordinates,
    /// when the log is showing and the point is over it.
    fn log_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let palette = self.palette.as_ref()?;
        let pane = self.log_pane.as_ref()?;
        let viewport = palette.log_rect();
        let inside = (viewport.left..viewport.right).contains(&x)
            && (viewport.top..viewport.bottom).contains(&y);
        if viewport.height() <= 0.0 || !inside {
            return None;
        }
        Some((x - viewport.left, y - viewport.top + pane.offset()))
    }

    /// The same, for a drag that has left the log: the point is pulled back
    /// inside the pane, so a selection dragged past the last line keeps
    /// running to the end of it rather than stopping.
    fn log_point_clamped(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let palette = self.palette.as_ref()?;
        let pane = self.log_pane.as_ref()?;
        let viewport = palette.log_rect();
        if viewport.height() <= 0.0 {
            return None;
        }
        let x = x.clamp(viewport.left, viewport.right);
        let y = y.clamp(viewport.top, viewport.bottom);
        Some((x - viewport.left, y - viewport.top + pane.offset()))
    }

    /// How many presses have run together at this spot: a second within the
    /// double-press time picks out a word, a third the whole line.
    fn press_count(&mut self, time: u32, x: f32, y: f32) -> u32 {
        const DOUBLE_PRESS_MS: u32 = 400;
        const SLOP: f32 = 4.0;
        let count = match self.last_press {
            Some((last, last_x, last_y, count))
                if time.saturating_sub(last) <= DOUBLE_PRESS_MS
                    && (x - last_x).abs() <= SLOP
                    && (y - last_y).abs() <= SLOP =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_press = Some((time, x, y, count));
        count
    }

    /// Scroll the log by `delta` points, from the keyboard.
    fn scroll_log(&mut self, delta: f32) {
        let (Some(pane), Some(palette)) = (self.log_pane.as_mut(), self.palette.as_ref()) else {
            return;
        };
        let end = (ask_log::length(&self.log) - palette.log_rect().height()).max(0.0);
        let target = (pane.offset() + delta).clamp(0.0, end);
        pane.scroll_to(target);
        self.log_following = target >= end;
        self.log_busy = true;
    }

    // === Painting ===

    fn push(&mut self) {
        if !self.sized {
            return;
        }
        // "No results" answers a question. Nothing has been asked yet when the
        // query is empty, and the launcher has nothing to report.
        let empty_message = match self.ask.as_ref() {
            // Ask mode has nothing to report about an empty list, only about a
            // service that is not there to ask — and once the conversation
            // has started, the log says that.
            Some(ask) if ask.running() => None,
            // Picking a session: say when there are none to pick.
            Some(ask) if self.picking && ask.unreachable().is_none() => {
                (ask.sessions_listed() && self.rows.is_empty()).then(|| {
                    if self.input.value().trim().is_empty() {
                        otto_kit::t!("launcher-agents-none")
                    } else {
                        otto_kit::t!("launcher-no-results")
                    }
                })
            }
            Some(ask) => ask
                .unreachable()
                .map(|_| otto_kit::t!("launcher-ask-unreachable")),
            None => {
                (!self.input.value().trim().is_empty()).then(|| otto_kit::t!("launcher-no-results"))
            }
        };
        let count = self.rows.len();
        // The log is empty until there is something to show: a conversation,
        // or files waiting to go with the first request.
        let log = if self.ask.is_some() {
            ask_log::length(&self.log)
        } else {
            0.0
        };
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.set_log(log);
        palette.update(&self.input, count, empty_message);

        let size = palette.card_size();
        let log_top = palette.field_top();
        if size != self.card_size {
            // The card's height runs a transition: keep painting until it
            // lands. Nothing else in the scene animates.
            self.settle_until = Some(Instant::now() + SETTLE);
            self.card_size = size;
            // The log opening above the field springs the card open upwards,
            // as the agents do below it. Growing a line at a time as the
            // answer streams stays a plain resize.
            if self.log_top <= 0.0 && log_top > 0.0 {
                self.spring();
            }
            self.log_top = log_top;
            self.resize_card(size);
        }
        self.update_input_region();
    }

    /// Tell the compositor which part of the launcher is worth pointing at.
    ///
    /// Both surfaces have to say the same thing, and neither says it by
    /// itself. The card's buffer is the card at its tallest and is shown
    /// clipped, so with no region of its own it goes on catching the pointer
    /// over rows that are not there. And Otto takes a layer surface's own
    /// input region as the clickable area of everything under it, so a parent
    /// anchored to all four edges and asking for nothing claims the whole
    /// output: while the launcher was up, the dock beneath it stopped
    /// answering the pointer at all. Both are set to the card as drawn — the
    /// shadow outside it included, because a shadow is something to see past
    /// rather than something to click.
    ///
    /// A press outside the region now reaches whatever is under it instead of
    /// the launcher, which is why the keyboard going elsewhere is what closes
    /// the launcher — see [`Launcher::on_keyboard_leave`].
    fn update_input_region(&mut self) {
        let (Some(surface), Some(card), Some(palette)) = (
            self.surface.as_ref(),
            self.card.as_ref(),
            self.palette.as_ref(),
        ) else {
            return;
        };
        // Before the first configure the card has no place to be, and a region
        // built from a zero-sized output would be somewhere off to the left.
        if !self.sized {
            return;
        }
        let rect = palette.input_rect();
        if self.input_region == Some(rect) {
            return;
        }
        let compositor = AppContext::compositor_state();
        let (Ok(on_parent), Ok(on_card)) = (Region::new(compositor), Region::new(compositor))
        else {
            tracing::warn!("no wl_region; the launcher will take input over the whole output");
            return;
        };
        let (x, y, width, height) = rect;
        on_parent.add(x, y, width, height);
        // The card itself takes no input: the pointer arrives on the parent,
        // under it, as Files' palette takes its pointer on a catcher surface.
        // The parent never moves, so positions stay put while the card is
        // dragged — over the card they would shift under the pointer with
        // every step it moved.
        let parent = surface.wl_surface();
        parent.set_input_region(Some(on_parent.wl_region()));
        card.wl_surface()
            .set_input_region(Some(on_card.wl_region()));
        // An input region is double-buffered state: until each surface commits
        // it, the compositor keeps hit-testing against the one it already has.
        // The frame this update belongs to would carry it, but only if there is
        // one — a card whose height changed without anything else changing
        // still has to be re-shaped.
        parent.commit();
        card.wl_surface().commit();
        self.input_region = Some(rect);
    }

    /// Tell the compositor how much of the card's buffer is card. The frost,
    /// the rounding and the shadow follow this rectangle, so a list that grew
    /// or shrank has to say so or the material keeps the old shape.
    fn resize_card(&mut self, (width, height): (f32, f32)) {
        let (Some(card), Some(palette)) = (self.card.as_ref(), self.palette.as_ref()) else {
            return;
        };
        let Some(style) = card.base_surface().surface_style() else {
            return;
        };
        // Surface-style geometry is in physical pixels.
        let scale = AppContext::fractional_scale();
        let (x, y) = palette.card_origin();
        let place = || {
            // The anchor is the top centre, and position is measured from it.
            style.set_position((x + width / 2.0) as f64 * scale, y as f64 * scale);
            style.set_size(width as f64 * scale, height as f64 * scale);
        };
        // The agents opening under the field, and the log above it, grow the
        // card with the spring it opens with. The material clips what is drawn
        // past its edge, so rows and log are uncovered as it grows rather than
        // drawn outside it.
        if self
            .spring_until
            .is_some_and(|until| Instant::now() < until)
        {
            animate(SCALE_IN, Curve::Spring(BOUNCE), place);
        } else {
            place();
        }
        // And the subsurface itself follows, in logical points. The style moves
        // where the card is *drawn*; this is where the compositor looks for it
        // when the pointer is over it, and pointer positions arrive relative to
        // it. Left behind at the parent's origin, the card would be hovered
        // three rows away from the cursor.
        card.set_position(x as i32, y as i32);
        self.card_placed = ((x as i32) as f32, (y as i32) as f32);
    }

    /// Drag the card's corner to `(x, y)` on the output, with where the
    /// compositor draws it and takes input over it following.
    fn move_card_to(&mut self, x: f32, y: f32) {
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.move_card_to(x, y);
        let size = palette.card_size();
        self.resize_card(size);
        self.update_input_region();
        self.dirty = true;
    }

    /// Let the card in, once there is something drawn on it.
    ///
    /// Not before: the entrance animates a surface, and a surface with no
    /// buffer yet would fade in as an empty rectangle and then fill.
    fn open(&mut self) {
        if self.opened {
            return;
        }
        let Some(style) = self
            .card
            .as_ref()
            .and_then(|card| card.base_surface().surface_style())
        else {
            return;
        };
        self.opened = true;
        animate(FADE_IN, Curve::Preset(Preset::EaseOutQuad), || {
            style.set_opacity(1.0);
        });
        animate(SCALE_IN, Curve::Spring(BOUNCE), || {
            style.set_scale(1.0, 1.0);
        });
    }

    /// Start closing, and stop the launcher once the card has gone.
    ///
    /// Whatever was chosen has already happened by this point — the
    /// application is starting, the window is being focused — so the animation
    /// costs nothing but the launcher's own last hundred milliseconds.
    fn close(&mut self) {
        if self.closing_at.is_some() {
            return;
        }
        self.closing_at = Some(Instant::now() + CLOSE);

        let Some(style) = self
            .card
            .as_ref()
            .and_then(|card| card.base_surface().surface_style())
        else {
            AppContext::request_exit();
            return;
        };
        animate(FADE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_opacity(0.0);
        });
        animate(SCALE_OUT, Curve::Preset(Preset::EaseInQuad), || {
            style.set_scale(CLOSE_SCALE, CLOSE_SCALE);
        });
        // The transaction is a request like any other, and the launcher is
        // about to stop doing anything else.
        AppContext::flush();
    }

    fn frame_in_flight(&self) -> bool {
        self.painted_at
            .is_some_and(|at| at.elapsed() < FRAME_TIMEOUT)
            && self
                .surface
                .as_ref()
                .is_some_and(|surface| surface.base_surface().frame_in_flight())
    }

    fn paint(&mut self) {
        let (Some(surface), true) = (self.surface.as_ref(), self.sized) else {
            return;
        };
        let first_paint = self.painted_at.is_none();
        self.painted_at = Some(Instant::now());
        tracing::trace!(selected = self.selected, rows = self.rows.len(), "painting");
        // otto-kit hands over a canvas with the buffer scale already applied,
        // so both scenes are drawn in logical points.
        // The parent surface draws nothing: it is there to take the keyboard
        // and to catch the click that lands beside the card. Painted once per
        // configure, not once per frame: it covers the whole output, and
        // every commit of it — a caret blink, a keystroke — told the
        // compositor the whole screen had changed, which had the dock and
        // the bar re-blurred under a card that never touches them.
        if !self.parent_painted {
            surface.draw(|canvas| {
                canvas.clear(skia_safe::Color::TRANSPARENT);
            });
            self.parent_painted = true;
        }

        if let Some(card) = self.card.as_ref() {
            let base = card.base_surface();
            // The card's scene is the only thing drawn on it, so the engine
            // knows exactly which part of the buffer this paint changes —
            // the field for a blink, a row for a highlight — and that is all
            // the compositor is told to recomposite. Taken and cleared before
            // drawing, so a change landing during the draw is still owed to
            // the next paint. The card's buffer is as tall as the card ever
            // gets, and reporting all of it reached down to the dock. And
            // nothing changed is nothing to draw: a pass that asked for a
            // paint while the scene stood still — the settle after a resize,
            // a source reporting in — costs no frame at all. The first paint
            // is the whole buffer regardless: the card has to arrive complete,
            // whatever the engine has or has not been asked to lay out yet.
            // What was pushed into the scene just before this paint is applied
            // first, rather than on the engine thread's next tick: otherwise a
            // keystroke painted the scene from before it, and waited for the
            // caret to blink to be seen.
            if !first_paint {
                let damage = AppContext::take_layers_damage();
                if damage.is_empty() {
                    return;
                }
                base.add_frame_damage(&[damage]);
            }
            let drew_at = Instant::now();
            card.draw(|canvas| {
                // Transparent, not the card's colour: what shows through is
                // the frosted material the compositor put underneath.
                canvas.clear(skia_safe::Color::TRANSPARENT);
                base.render_layer_node(canvas);
            });
            let took = drew_at.elapsed();
            if took > Duration::from_millis(20) {
                tracing::debug!(ms = took.as_millis(), "card draw blocked");
            }
        }
    }
}

/// What the list pane shows: the rows, as the palette paints them.
struct Rows<'a> {
    palette: &'a Palette,
    items: &'a [&'a Item],
    labels: &'a [&'static str],
    /// The source whose rows are drawn small: the attached files, in ask mode.
    compact_source: Option<usize>,
    revision: u64,
}

impl ScrollContent for Rows<'_> {
    fn length(&self, _cross: f32) -> f32 {
        RowLayout::new(ROW_H, self.items.len()).length()
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn paint(&self, canvas: &skia_safe::Canvas, band: Rect) {
        self.palette
            .paint_rows(canvas, band, self.items, self.labels, self.compact_source);
    }
}

/// What the list pane shows once a request is made: the ask log.
struct LogRows<'a> {
    palette: &'a Palette,
    lines: &'a [LogLine],
    /// What is selected, as the boxes to paint behind the words.
    selection: &'a [Rect],
    revision: u64,
}

impl ScrollContent for LogRows<'_> {
    fn length(&self, _cross: f32) -> f32 {
        ask_log::length(self.lines)
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn paint(&self, canvas: &skia_safe::Canvas, band: Rect) {
        self.palette
            .paint_log(canvas, band, self.lines, self.selection);
    }
}

/// Put `text` on the clipboard, so that it is still there afterwards.
///
/// The offer is made here first, which is what makes a paste work while the
/// launcher is still up. But a Wayland selection dies with the client that
/// made it, and the launcher is one keystroke from closing — so `wl-copy`,
/// which forks and stays to serve the offer, is handed the same text and
/// takes the selection over. Without it the copy still works until the
/// launcher goes, which is better than refusing to copy at all.
fn copy_to_clipboard(text: &str, serial: u32) {
    clipboard::set_text(text, serial);
    if let Err(err) = std::process::Command::new("wl-copy").arg(text).spawn() {
        tracing::debug!(%err, "wl-copy is not available: the copy lasts as long as the launcher");
    }
}

/// Run `changes` inside an animated transaction of `duration`.
///
/// Every animated property the closure sets joins that one transaction, so
/// properties that should move together do. Properties that should *not* move
/// together — the card's fade and its scale — go in transactions of their own.
fn animate(duration: Duration, curve: Curve, changes: impl FnOnce()) {
    let Some(manager) = AppContext::surface_style_manager() else {
        changes();
        return;
    };
    let qh = AppContext::queue_handle();

    let timing = manager.create_timing_function(qh, ());
    match curve {
        Curve::Preset(preset) => timing.set_preset(preset),
        // The spring is tuned to settle inside the transaction's duration, so
        // the bounce is a shape rather than a length.
        Curve::Spring(bounce) => timing.set_spring(bounce, 0.0),
    }
    let transaction = manager.begin_transaction(qh, ());
    transaction.set_duration(duration.as_secs_f64());
    transaction.set_timing_function(&timing);

    changes();

    transaction.commit();
}

enum Curve {
    Preset(Preset),
    /// Overshoots and settles back. The argument is how far.
    Spring(f64),
}

/// How solid the card runs, whatever the theme's popup material says.
///
/// There is a ceiling worth staying under: past roughly this the frost stops
/// reading as frost and the card may as well be opaque, which throws away the
/// blur the compositor is doing anyway.
const CARD_MIN_ALPHA: u8 = 0xD8;

/// The card without its frost. Denser than the toolkit's general
/// unfrosted floor: the card is large and full of small text, and with no
/// blur behind it even a faint desktop showing through fights the text.
const CARD_UNFROSTED_MIN_ALPHA: u8 = 0xF6;

fn at_least_opaque(colour: skia_safe::Color, min_alpha: u8) -> skia_safe::Color {
    skia_safe::Color::from_argb(
        colour.a().max(min_alpha),
        colour.r(),
        colour.g(),
        colour.b(),
    )
}

/// The card's frost colour, which is the one part of the material that follows
/// the colour scheme. Split out of `apply_card_material` so a theme that lands
/// after the card exists can be applied without also rewinding the entrance
/// state that function sets.
fn apply_card_colour(card: &SubsurfaceSurface) {
    let Some(style) = card.base_surface().surface_style() else {
        tracing::warn!("no otto-surface-style; the card will not be frosted");
        return;
    };
    // `material_popup` is the token the bar's menus use — the launcher is the
    // same kind of thing, floating over whatever happens to be behind it. It is
    // taken up to at least `CARD_MIN_ALPHA` on top of that: the card is large
    // and full of small text, and a busy desktop showing through it costs more
    // legibility than the frost gives back.
    // Nearly opaque while the desktop's frosting is off.
    let min_alpha = if otto_kit::frosting::enabled() {
        CARD_MIN_ALPHA
    } else {
        CARD_UNFROSTED_MIN_ALPHA
    };
    let colour = skia_safe::Color4f::from(at_least_opaque(
        AppContext::current_theme().material_popup,
        min_alpha,
    ));
    style.set_background_color(
        colour.r as f64,
        colour.g as f64,
        colour.b as f64,
        colour.a as f64,
    );
}

/// Ask the compositor for the card's material: the frost, and the shape it is
/// cut to.
///
/// None of this can be drawn client-side. A blur needs the pixels behind the
/// surface, and the only process that has them is the compositor — so the card
/// declares what it wants to look like and paints its text on top of the
/// result. `material_medium` is the same token the bar's menus use, so the
/// launcher reads as the same kind of surface as the rest of the desktop.
fn apply_card_material(card: &SubsurfaceSurface) {
    apply_card_colour(card);
    let Some(style) = card.base_surface().surface_style() else {
        return;
    };
    style.set_blend_mode(if otto_kit::frosting::enabled() {
        BlendMode::BackgroundBlur
    } else {
        BlendMode::Normal
    });
    style.set_corner_radius(
        otto_kit::corners::radius(RADIUS) as f64 * AppContext::fractional_scale(),
    );
    style.set_masks_to_bounds(ClipMode::Enabled);
    // The rows and the log are panes of their own, subsurfaces of the card:
    // without this they spill past its edge while it is shorter than they are.
    style.set_clip_children(ClipMode::Enabled);
    style.set_shadow(0.32, 32.0, 0.0, 12.0, 0.0, 0.0, 0.0);
    // The buffer is the card at its tallest; a shorter card shows the top of
    // it, so the field stays where it is as rows come and go.
    style.set_contents_gravity(ContentsGravity::TopLeft);

    // Transforms are taken about the top centre. Centre horizontally so the
    // card swells outwards evenly; top vertically so the field stays put while
    // the card arrives and when the list grows under it.
    style.set_anchor_point(0.5, 0.0);

    // Where the card starts: just short of full size, and invisible. The
    // entrance animates out of this once there is something on it to see.
    style.set_scale(OPEN_SCALE, OPEN_SCALE);
    style.set_opacity(0.0);
}

/// Whether to draw dark. The colour scheme comes from the desktop portal, the
/// same source otto-kit's other components read.
fn dark() -> bool {
    matches!(
        otto_kit::color_scheme::current_color_scheme(),
        otto_kit::theme::ColorScheme::Dark
    )
}

impl App for Launcher {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(ms = since_start(), "wayland connected");
        // The engine has to exist before the surface does: a surface builds its
        // own root layer node, and that node is what the scene hangs off.
        AppContext::enable_layer_engine(1920.0, 1080.0);
        tracing::debug!(ms = since_start(), "layer engine ready");

        // Anchored to all four edges with a zero size, so the compositor gives
        // us the whole output — the scrim needs it, and so does "click outside
        // the card to dismiss".
        let surface = LayerShellSurface::with_anchor(
            Layer::Overlay,
            "otto-launcher",
            0,
            0,
            Some(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right),
            Some(0),
        )?;
        // Visible to assistive technologies from the moment it exists: the
        // launcher is modal and takes every key, so a screen reader that
        // cannot read it cannot tell the user what has taken over the session.
        AppContext::enable_accessibility(&surface.wl_surface().id());

        // Exclusive: the launcher is modal while it is up, and every keystroke
        // belongs to it — including the ones the focused window would want.
        surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        tracing::debug!(ms = since_start(), "layer surface created");

        // The card is a child surface, sized once to the tallest it can be.
        // Shrinking it is the compositor clipping the same buffer, which costs
        // nothing and keeps the text from being re-laid out as rows come and
        // go.
        let card = SubsurfaceSurface::new(
            surface.base_surface().wl_surface(),
            0,
            0,
            CARD_W as i32,
            MAX_CARD_H as i32,
        )?;
        apply_card_material(&card);

        tracing::debug!(ms = since_start(), "card surface created");
        let Some(engine) = AppContext::layers_renderer(|renderer| renderer.engine().clone()) else {
            return Err("the layers engine is unavailable".into());
        };
        let mut palette = Palette::new(engine, card.base_surface().layer_node(), dark());
        // A conversation, or a list of sessions, sits centred on the output
        // and grows both ways from there.
        palette.set_centered(self.ask.is_some());
        // The rows, over the card's own drawing and inside its frost. A row's
        // height to start with; the pane follows the list from the first
        // update.
        let list = ScrollPane::new(
            card.wl_surface(),
            Rect::from_xywh(0.0, LIST_TOP, CARD_W, ROW_H),
            Axis::Vertical,
        )?;
        // The ask log, above the field. Hidden until there is a log.
        if self.ask.is_some() {
            let log = ScrollPane::new(
                card.wl_surface(),
                Rect::from_xywh(0.0, 0.0, CARD_W, LOG_LINE_H),
                Axis::Vertical,
            )?;
            self.log_pane = Some(log);
        }

        self.list = Some(list);
        self.palette = Some(palette);
        self.card = Some(card);
        self.surface = Some(surface);
        tracing::debug!(ms = since_start(), "surfaces created");
        self.reload();
        // Attached files, or an opened session, have a log from the start.
        self.relayout_log();
        tracing::debug!(ms = since_start(), "sources loaded");
        Ok(())
    }

    fn on_configure_layer(&mut self, _ctx: &AppContext, width: i32, height: i32, _serial: u32) {
        tracing::debug!(width, height, ms = since_start(), "configured");
        if let Some(palette) = self.palette.as_mut() {
            palette.set_size(width as f32, height as f32);
        }
        self.sized = true;
        self.dirty = true;
        // A configure can bring a new size, and a buffer of the old one would
        // be the wrong shape: the next paint draws the parent again.
        self.parent_painted = false;
        // The card is centred on the output, so a different output size is a
        // different rectangle to hit-test.
        self.update_input_region();
    }

    /// The portal answers the colour scheme asynchronously, so the launcher is
    /// usually already up — and drawn in the default light — by the time the
    /// answer arrives. Recolour everything that was built from it.
    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        let dark = dark();
        self.input.style = field_style(dark);
        if let Some(palette) = self.palette.as_mut() {
            palette.set_dark(dark);
        }
        if let Some(card) = self.card.as_ref() {
            apply_card_colour(card);
        }
        // The rows' text is coloured from the scheme too.
        self.list_revision = self.list_revision.wrapping_add(1);
        self.dirty = true;
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        // Shift is tracked from the key itself: the runner reports keys, not
        // modifier state, and selection with Shift+arrows needs to know.
        match event.keysym {
            Keysym::Shift_L | Keysym::Shift_R => {
                self.shift = state == wl_keyboard::KeyState::Pressed;
                return;
            }
            _ => {}
        }
        if state != wl_keyboard::KeyState::Pressed || self.closing_at.is_some() {
            return;
        }
        self.engaged = true;

        // Ctrl combinations arrive as control characters rather than as a
        // modifier flag, which is enough to recognise them by.
        let control = event
            .utf8
            .as_deref()
            .and_then(|text| text.chars().next())
            .filter(|c| (*c as u32) < 0x20 && *c != '\r' && *c != '\n' && *c != '\t')
            .map(|c| char::from(c as u8 + 0x60));
        // Cmd+C stops an agent as Ctrl+C does.
        let modifiers = AppContext::current_modifiers();
        let stop_key = control == Some('c')
            || (modifiers.logo
                && !modifiers.ctrl
                && !modifiers.alt
                && matches!(event.keysym, Keysym::c | Keysym::C));

        // Left with nothing typed goes back a question, while the agent asks
        // several and one is behind.
        if event.keysym == Keysym::Left
            && self.input.value().is_empty()
            && self.ask.as_mut().is_some_and(Ask::input_back)
        {
            self.refilter();
            self.relayout_log();
            return;
        }

        // Left with nothing typed goes back from a session to the list of
        // sessions. Not while a request is still on its way to the service,
        // which leaving would lose.
        if event.keysym == Keysym::Left
            && self.input.value().is_empty()
            && self
                .ask
                .as_ref()
                .is_some_and(|ask| ask.running() && !ask.handing_off())
        {
            self.back_to_sessions();
            return;
        }

        // Ctrl+O takes the session up in a terminal, in the agent's own
        // interface, and the launcher goes: the terminal has the keyboard now.
        // The session being followed, or the one highlighted in the list.
        if control == Some('o') {
            if let Some(terminal) = self.selected_terminal() {
                match terminal.open() {
                    Ok(()) => self.close(),
                    Err(err) => tracing::warn!(%err, "could not open the session in a terminal"),
                }
                return;
            }
        }

        // Text picked out of the log is what Ctrl+C or Cmd+C copies, before
        // the key means anything else: the log is read far more often than a
        // turn is stopped, and a selection on screen says which was meant.
        if stop_key {
            if let Some(text) = self.selected_log_text() {
                copy_to_clipboard(&text, serial);
                return;
            }
        }

        // In the list of sessions, Ctrl+C or Cmd+C with nothing selected to
        // copy stops the highlighted session's turn.
        if stop_key && self.picking && !self.input.state.has_selection() {
            let index = self
                .selected_origin()
                .filter(|origin| origin.source == ASK_ROWS)
                .map(|origin| origin.index);
            if let (Some(index), Some(ask)) = (index, self.ask.as_mut()) {
                ask.stop_at(index);
            }
            return;
        }

        // Once a request is made the log sits above the field, and the field
        // takes the next request. When the agent asks something, Up and Down
        // pick an answer from the rows under the field; otherwise they scroll
        // the log, as the page keys always do. Ctrl+C or Cmd+C with nothing
        // selected to copy stops the agent's turn; Escape still closes, leaving
        // the agent to it.
        if self.ask_running() {
            let page = self
                .palette
                .as_ref()
                .map_or(0.0, |palette| palette.log_rect().height());
            let answering = self.asked_question() || self.asking_input();
            let scroll = match (event.keysym, control) {
                (Keysym::Up, _) if !answering => Some(-LOG_LINE_H * 3.0),
                (Keysym::Down, _) if !answering => Some(LOG_LINE_H * 3.0),
                (Keysym::Page_Up, _) => Some(-page),
                (Keysym::Page_Down, _) => Some(page),
                _ => None,
            };
            if let Some(delta) = scroll {
                self.scroll_log(delta);
                return;
            }
            if stop_key && !self.input.state.has_selection() {
                if let Some(ask) = self.ask.as_mut() {
                    ask.cancel();
                }
                return;
            }
        }

        match (event.keysym, control) {
            (Keysym::Escape, _) => {
                // Escape lets go of what was picked out of the log first, so
                // a selection made by accident is not also a reason to lose
                // the conversation.
                if self.log_selection.is_some() {
                    self.set_log_selection(None);
                    return;
                }
                self.close();
                return;
            }
            (Keysym::Return | Keysym::KP_Enter, _) => {
                self.activate();
                return;
            }
            // Before the first request, the agents stay out of the way: the
            // request goes to the default agent unless Down lists them, and
            // Up from the first one puts them away again.
            (Keysym::Down, _) | (_, Some('n'))
                if self.can_choose_agent() && !self.choosing_agent =>
            {
                self.choosing_agent = true;
                self.spring();
                self.selected = 0;
                self.refilter();
                return;
            }
            (Keysym::Up, _) | (_, Some('p')) if self.choosing_agent && self.selected == 0 => {
                self.choosing_agent = false;
                self.spring();
                self.refilter();
                return;
            }
            (Keysym::Down, _) | (_, Some('n')) => {
                self.move_selection(1);
                return;
            }
            (Keysym::Up, _) | (_, Some('p')) => {
                self.move_selection(-1);
                return;
            }
            (Keysym::Tab, _) => {
                // A completion on offer is what Tab is for; with none, it goes
                // back to walking the rows.
                if !self.shift && self.accept_completion() {
                    return;
                }
                self.move_selection(if self.shift { -1 } else { 1 });
                return;
            }
            (Keysym::ISO_Left_Tab, _) => {
                self.move_selection(-1);
                return;
            }
            (Keysym::Page_Down, _) => {
                self.move_selection(MAX_ROWS as isize);
                return;
            }
            (Keysym::Page_Up, _) => {
                self.move_selection(-(MAX_ROWS as isize));
                return;
            }
            // Clear the query without reaching for backspace — the fastest way
            // to start a different search.
            (_, Some('u')) => {
                self.input.set_value("");
                self.refilter();
                return;
            }
            (_, Some('a')) => {
                // With nothing typed, there is nothing in the field to select
                // all of, and what is on screen is the conversation: Ctrl+A
                // takes the whole log, ready to be copied.
                if self.input.value().is_empty() && !self.log_spans.is_empty() {
                    self.set_log_selection(selection::everything(&self.log_spans));
                    return;
                }
                self.input
                    .on_key(TextInputKey::SelectAll, KeyMods::default());
                self.dirty = true;
                return;
            }
            // Cut and copy hand the selection to the system clipboard; paste
            // reads it back, since the field holds no clipboard of its own.
            (_, Some('c')) | (_, Some('x')) => {
                let cut = control == Some('x');
                let key = if cut {
                    TextInputKey::Cut
                } else {
                    TextInputKey::Copy
                };
                if let TextInputResponse::Clipboard(text) =
                    self.input.on_key(key, KeyMods::default())
                {
                    copy_to_clipboard(&text, serial);
                }
                if cut {
                    self.refilter();
                } else {
                    self.dirty = true;
                }
                return;
            }
            (_, Some('v')) => {
                if let Some(text) = clipboard::text() {
                    self.input
                        .on_key(TextInputKey::Paste(text), KeyMods::default());
                    self.refilter();
                }
                return;
            }
            (_, Some('w')) => {
                self.input.on_key(
                    TextInputKey::Backspace,
                    KeyMods {
                        shift: false,
                        ctrl: true,
                    },
                );
                self.refilter();
                return;
            }
            _ => {}
        }

        let mods = KeyMods {
            shift: self.shift,
            ctrl: false,
        };
        let key = match event.keysym {
            Keysym::Left => TextInputKey::Left,
            Keysym::Right => TextInputKey::Right,
            Keysym::Home => TextInputKey::Home,
            Keysym::End => TextInputKey::End,
            Keysym::BackSpace => TextInputKey::Backspace,
            Keysym::Delete => TextInputKey::Delete,
            _ => {
                let text: String = event
                    .utf8
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if text.is_empty() {
                    return;
                }
                TextInputKey::Text(text)
            }
        };

        match self.input.on_key(key, mods) {
            TextInputResponse::Changed => self.refilter(),
            TextInputResponse::Moved => self.dirty = true,
            TextInputResponse::Commit => self.activate(),
            TextInputResponse::Cancel => self.close(),
            _ => {}
        }
    }

    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        // Something else has taken the keyboard. A modal that has lost its
        // input is only in the way — but not before it has ever had it, which
        // is what `engaged` guards against at startup.
        //
        // This is also how a click outside the card closes the launcher now
        // that the card is the only thing it takes input over: the press lands
        // on the window or the dock icon under it, and the keyboard follows.
        if self.engaged {
            self.close();
        }
    }

    fn on_pointer_event(&mut self, _ctx: &AppContext, events: &[PointerEvent]) {
        if self.closing_at.is_some() {
            return;
        }
        if self.palette.is_none() {
            return;
        }
        let card_surface = self.card.as_ref().map(|card| card.wl_surface().clone());
        let (card_w, card_h) = self.palette.as_ref().map_or((0.0, 0.0), Palette::card_size);
        for event in events {
            // The pointer arrives on the parent, in the output's points (see
            // `update_input_region`) — or on the card itself while its empty
            // input region is still a commit away, relative to where the card
            // was placed. Both are brought to the output's points, and from
            // there to the card's.
            let (left, top) = self.card_placed;
            let (screen_x, screen_y) = if card_surface.as_ref() == Some(&event.surface) {
                (
                    event.position.0 as f32 + left,
                    event.position.1 as f32 + top,
                )
            } else {
                (event.position.0 as f32, event.position.1 as f32)
            };
            let (x, y) = (screen_x - left, screen_y - top);
            let on_card = (0.0..card_w).contains(&x) && (0.0..card_h).contains(&y);
            if let Some((grab_x, grab_y)) = self.dragging {
                match event.kind {
                    PointerEventKind::Motion { .. } => {
                        self.move_card_to(screen_x - grab_x, screen_y - grab_y);
                        continue;
                    }
                    PointerEventKind::Release { .. } => {
                        self.dragging = None;
                        continue;
                    }
                    _ => {}
                }
            }
            match event.kind {
                // A selection being dragged out follows the pointer wherever
                // it goes, on the card or off it.
                PointerEventKind::Motion { .. } if self.selecting.is_some() => {
                    let (Some(anchor), Some(point)) =
                        (self.selecting, self.log_point_clamped(x, y))
                    else {
                        continue;
                    };
                    if let Some(focus) = selection::nearest_caret(&self.log_spans, point) {
                        self.set_log_selection(Some(Selection { anchor, focus }));
                    }
                }
                // The highlight is the list pane's, so following the pointer
                // repaints nothing.
                PointerEventKind::Motion { .. } if on_card => {
                    self.follow_pointer = true;
                    if let Some(list) = self.list.as_mut() {
                        list.pointer_motion(skia_safe::Point::new(x, y));
                    }
                    if let Some(log) = self.log_pane.as_mut() {
                        log.pointer_motion(skia_safe::Point::new(x, y));
                    }
                    if let Some(row) = self.list_row_at(x, y) {
                        self.selected = row;
                    }
                    // Over the log's words the pointer says so, because
                    // nothing else about painted text does. Only when it
                    // changes: motion arrives far too often to ask the
                    // compositor for the same cursor every time.
                    let over_text = self
                        .log_point(x, y)
                        .and_then(|point| selection::caret_at(&self.log_spans, point))
                        .is_some();
                    if over_text != self.over_log_text {
                        self.over_log_text = over_text;
                        AppContext::set_cursor_shape(if over_text {
                            CursorShape::Text
                        } else {
                            CursorShape::Default
                        });
                    }
                }
                // A wheel or a touchpad over the card scrolls the list, and
                // the row that comes under the pointer is the one selected.
                PointerEventKind::Axis { vertical, .. } if on_card => {
                    self.engaged = true;
                    let point = skia_safe::Point::new(x, y);
                    let delta = vertical.absolute as f32;
                    let over_log = self
                        .log_pane
                        .as_ref()
                        .is_some_and(|log| log.contains(point));
                    if over_log {
                        if let Some(log) = self.log_pane.as_mut() {
                            log.wheel_at(point, delta, vertical.discrete != 0, vertical.stop);
                        }
                        // Scrolling up takes the log off its end; reaching the
                        // end again puts it back on, in `sync_log`.
                        if delta < 0.0 {
                            self.log_following = false;
                        }
                        continue;
                    }
                    self.follow_pointer = true;
                    if let Some(list) = self.list.as_mut() {
                        list.wheel_at(point, delta, vertical.discrete != 0, vertical.stop);
                    }
                    if let Some(row) = self.list_row_at(x, y) {
                        self.selected = row;
                    }
                }
                PointerEventKind::Press { .. } => {
                    self.engaged = true;
                    // The input region should have kept these away — it is
                    // the card as drawn, not the card's full-height buffer —
                    // but a region is applied a commit late, and a press that
                    // arrives against the old one is still a press beside the
                    // card, which is the other way of saying Escape.
                    let card_h = self.palette.as_ref().map_or(0.0, |p| p.card_size().1);
                    let beside = !on_card || y > card_h;
                    if beside {
                        self.close();
                        return;
                    }
                    // A press on the log's words starts a selection: the
                    // conversation is there to be read, and read means
                    // copied. A second press takes the word under it, a
                    // third the line.
                    let caret = self
                        .log_point(x, y)
                        .and_then(|point| selection::caret_at(&self.log_spans, point));
                    if let Some(caret) = caret {
                        let time = match event.kind {
                            PointerEventKind::Press { time, .. } => time,
                            _ => 0,
                        };
                        let selection = match self.press_count(time, x, y) {
                            1 => Selection::at(caret),
                            2 => selection::word_at(&self.log_spans, caret),
                            _ => selection::line_at(&self.log_spans, caret),
                        };
                        self.selecting = Some(selection.anchor);
                        self.set_log_selection(Some(selection));
                        continue;
                    }
                    // Anywhere else puts the selection down again.
                    self.set_log_selection(None);
                    // The field and the log's background are the card's
                    // handle.
                    if self.palette.as_ref().is_some_and(|p| p.drags_at(y)) {
                        self.dragging = Some((x, y));
                    }
                }
                PointerEventKind::Release { .. } if self.selecting.is_some() => {
                    self.selecting = None;
                }
                PointerEventKind::Release { .. } if on_card => {
                    if let Some(row) = self.list_row_at(x, y) {
                        // An attached file is only shown; clicking it does
                        // nothing.
                        if self.attachment_row(row) {
                            return;
                        }
                        self.selected = row;
                        // A click on an answer answers, whatever is typed.
                        if self.asked_question() {
                            self.answer_selected();
                        } else if self.asking_input() {
                            self.answer_input(Some(row));
                        } else {
                            self.activate();
                        }
                        return;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.selecting = None;
                    self.over_log_text = false;
                    if let Some(list) = self.list.as_mut() {
                        list.pointer_leave();
                    }
                    if let Some(log) = self.log_pane.as_mut() {
                        log.pointer_leave();
                    }
                }
                _ => {}
            }
        }
    }

    /// What a screen reader reads: the field, and the results under it.
    ///
    /// The launcher already moves its own selection with the arrows, so there
    /// is no traversal ring here — the highlighted row *is* the focus, and
    /// saying so is what makes a screen reader read each result as the user
    /// arrows through them.
    fn accessibility(&mut self, _ctx: &AppContext, _surface: &ObjectId) -> Option<A11yTree> {
        let palette = self.palette.as_ref()?;
        let (card_x, card_y) = palette.card_origin();
        let (card_w, card_h) = palette.card_size();

        let title = self.input.state.placeholder.clone();
        let mut tree = A11yTree::new(title.clone());

        let field = Rect::from_xywh(card_x, card_y + palette.field_top(), card_w, FIELD_H);
        tree.control(FIELD, field, Role::SearchInput, true, |node| {
            node.set_label(title.clone());
            node.set_value(self.input.value().to_owned());
            node.add_action(Action::SetValue);
        });

        // Once a request is made, the log above the field is read out as it
        // changes; the rows under the field, when there are any, are the
        // answers to the agent's question.
        if !self.log.is_empty() {
            let log = palette.log_rect();
            let bounds = Rect::from_xywh(card_x, card_y + log.top, card_w, log.height());
            tree.status(LOG, bounds, self.log_text.clone());
        }

        // Only the rows on screen: the list scrolls, and a row that has been
        // scrolled past is not something to point at.
        let viewport = palette.list_rect();
        let offset = self.list.as_ref().map_or(0.0, ScrollPane::offset);
        let shown =
            RowLayout::new(ROW_H, self.row_count()).range(offset, offset + viewport.height());
        let below_field = palette.field_top() + FIELD_H;
        let list = Rect::from_xywh(card_x, card_y + below_field, card_w, card_h - below_field);

        let rows: Vec<(usize, String, Option<String>)> = shown
            .filter_map(|index| {
                let item = self.row(index)?;
                Some((index, item.title.clone(), item.subtitle.clone()))
            })
            .collect();
        let selected = self.selected;

        tree.region(
            RESULTS,
            list,
            Role::ListBox,
            otto_kit::t!("a11y-results"),
            |tree| {
                for (index, title, subtitle) in rows {
                    let bounds = Rect::from_xywh(
                        card_x,
                        card_y + viewport.top + index as f32 * ROW_H - offset,
                        card_w,
                        ROW_H,
                    );
                    tree.control(
                        row_focus(index),
                        bounds,
                        Role::ListBoxOption,
                        true,
                        |node| {
                            node.set_label(title);
                            if let Some(subtitle) = subtitle {
                                node.set_description(subtitle);
                            }
                            node.set_selected(index == selected);
                            node.add_action(Action::Click);
                        },
                    );
                }
            },
        );

        if self.row_count() > 0 {
            tree.set_focus(row_focus(selected));
        }

        Some(tree)
    }

    /// A screen reader picked a result: run it, exactly as Enter would.
    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &ObjectId,
        request: &ActionRequest,
    ) {
        if !matches!(request.action, Action::Click) {
            return;
        }
        let target = (0..self.row_count()).find(|index| {
            otto_kit::accessibility::node_id(row_focus(*index)) == request.target_node
        });
        let Some(index) = target else { return };

        self.selected = index;
        self.activate();
    }

    fn on_update(&mut self, _ctx: &AppContext) {
        // The card has gone; nothing is left to do but stop.
        if let Some(at) = self.closing_at {
            // A request still on its way to otto-agentsd would go with the
            // launcher, so the card goes but the process waits for it.
            if let Some(ask) = self.ask.as_mut() {
                ask.pump();
            }
            let now = Instant::now();
            let handing_off = self.ask.as_ref().is_some_and(Ask::handing_off);
            if now >= at && (!handing_off || now >= at + HAND_OFF_GRACE) {
                AppContext::request_exit();
            }
            return;
        }

        // The keyboard arriving is what "arrived" means. Noticing it here
        // rather than waiting for the first keystroke is what lets a click
        // outside close the launcher: that click never reaches us, and the
        // only thing it leaves behind is the keyboard moving on.
        if !self.engaged && AppContext::keyboard_focus().is_some() {
            self.engaged = true;
        }

        match self.ask.as_mut().map(|ask| (ask.pump(), ask.running())) {
            Some((true, true)) => {
                // The rows follow the agent's question as much as the log does.
                self.refilter();
                self.relayout_log();
            }
            Some((true, false)) => self.reload(),
            _ => {}
        }

        let mut changed = false;
        for source in self.sources.iter_mut() {
            source.pump();
            changed |= source.changed();
        }
        if changed {
            self.reload();
        }

        // The caret blinks on its own clock; the field asks to be redrawn when
        // the phase flips.
        let now = Instant::now();
        let was_visible = self.input.caret_visible();
        self.input
            .tick(now.duration_since(self.last_tick).as_secs_f32());
        self.last_tick = now;
        if self.input.caret_visible() != was_visible {
            self.dirty = true;
        }

        if self.dirty {
            self.dirty = false;
            self.push();
            self.paint();
            // The first frame is on the card, so it has something to arrive
            // with.
            let first = !self.opened;
            self.open();
            self.sync_list();
            self.sync_log();
            if first {
                tracing::debug!(ms = since_start(), "first frame");
            }
            return;
        }

        // Every pass: the list's scroll, fling and highlight move on the
        // pane's own surfaces, whatever the card is doing.
        self.sync_list();
        self.sync_log();

        if self.frame_in_flight() {
            return;
        }
        // Keep painting while a transition is still running.
        if self.settle_until.is_some_and(|until| now < until) {
            self.paint();
        } else {
            self.settle_until = None;
        }
    }

    fn idle_timeout(&self) -> Option<Duration> {
        if self.closing_at.is_some() {
            return Some(Duration::from_millis(8));
        }
        Some(
            if self.settle_until.is_some() || self.list_busy || self.log_busy {
                Duration::from_millis(8)
            } else {
                // Half a blink period: the slowest the loop may sleep and still
                // turn the caret on and off on time.
                Duration::from_secs_f32(CARET_BLINK_PERIOD / 2.0)
            },
        )
    }

    fn poll_fds(&self) -> Vec<RawFd> {
        self.sources
            .iter()
            .filter_map(|s| s.poll_fd())
            .chain(self.ask.as_ref().map(Ask::poll_fd))
            .collect()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    STARTED.get_or_init(Instant::now);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Before the first string is looked up, and before the window is drawn: a
    // launcher is judged on how fast it appears, and this is one round trip
    // that has to finish first either way.
    otto_kit::i18n::init_from_desktop();

    // `--apps` / `--windows` narrow what is offered; anything else on the
    // command line is the initial query, so a binding can open the launcher
    // already filtered.
    // Apps unless asked otherwise: the two bindings are "launch something" and
    // "switch to a window", and a mode that quietly does both is neither.
    let mut args = std::env::args();
    let mut scope = args
        .next()
        .as_deref()
        .and_then(scope_for_program)
        .unwrap_or(Scope::Apps);
    let mut words: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut session: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apps" | "-a" => scope = Scope::Apps,
            "--windows" | "-w" => scope = Scope::Windows,
            "--all" => scope = Scope::Everything,
            "--ask" => scope = Scope::Ask,
            "--agents" => scope = Scope::Agents,
            // A file attached to the first request, and a session to open in
            // place of starting one. Either means asking.
            "--file" => files.extend(args.next().map(PathBuf::from)),
            "--session" => session = args.next(),
            "--help" | "-h" => {
                println!(
                    "usage: otto-launcher [--apps|--windows|--all|--ask|--agents] \
                     [--file PATH]... [--session ID] [query]\n\
                     otto-ask and otto-agents open in --ask and --agents mode"
                );
                return Ok(());
            }
            _ => words.push(arg),
        }
    }
    if (!files.is_empty() || session.is_some()) && scope != Scope::Agents {
        scope = Scope::Ask;
    }
    let mut launcher = Launcher::new(&words.join(" "), scope);
    launcher.prepare_ask(files, session.as_deref());
    AppRunner::new(launcher).run()?;
    Ok(())
}

/// The mode an alias starts in: `otto-ask` and `otto-agents` are symlinks to
/// the launcher, standing for `--ask` and `--agents`.
fn scope_for_program(program: &str) -> Option<Scope> {
    match std::path::Path::new(program).file_name()?.to_str()? {
        "otto-ask" => Some(Scope::Ask),
        "otto-agents" => Some(Scope::Agents),
        _ => None,
    }
}

#[cfg(test)]
mod program_name_tests {
    use super::*;

    #[test]
    fn aliases_pick_their_mode() {
        assert!(scope_for_program("/usr/bin/otto-ask") == Some(Scope::Ask));
        assert!(scope_for_program("otto-agents") == Some(Scope::Agents));
        assert!(scope_for_program("/usr/bin/otto-launcher").is_none());
    }
}
