//! Access-style dialog: a permission/choice panel rendered as a dropdown below
//! the island bar.
//!
//! Mirrors the semantics of `org.freedesktop.impl.portal.Access`: a caller
//! presents a request with title/subtitle/body/icon and zero or more choice
//! groups; the user confirms (grant) or cancels (deny), and a
//! [`DialogResponse`] is returned over the channel held in the request.
//!
//! A question (`PresentQuestion`) is the same dialog with two extra rules: the
//! grant button is optional (hidden when its label is empty), and an optional
//! *open* button hands the question off to another app (response `3`).
//!
//! Questions (`PresentQuestions`) add multi-select groups, answered with
//! every option picked, and ask several questions one page at a time — see
//! [`QuestionStyle`].
//!
//! A modal dialog holds the keyboard until it is answered. A non-modal one
//! can be ignored: once the user moves on it shrinks into a circle in the
//! island row, still pending, and a click opens it again — see [`Presence`].
//!
//! See `specs/portal-access-dialog.md`.

use otto_kit::icons::named_icon_sized;
use otto_kit::protocols::otto_surface_style_v1::{BlendMode, ClipMode, ContentsGravity};
use otto_kit::typography::TextStyle;
use otto_kit::SubsurfaceSurface;
use skia_safe::{Canvas, Color, Paint, RRect, Rect};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

pub type DialogId = u64;

// ---------------------------------------------------------------------------
// Geometry constants (logical units — same space the canvas draws in)
// ---------------------------------------------------------------------------

pub const DIALOG_W: f32 = 320.0;
const PAD: f32 = 18.0;
const ICON: f32 = 44.0;
const ICON_GAP: f32 = 10.0;
const TITLE_GAP: f32 = 6.0;
const SUBTITLE_GAP: f32 = 4.0;
const BODY_GAP: f32 = 8.0;
const OPTION_H: f32 = 44.0;
const OPTION_GAP: f32 = 6.0;
const OPTION_RADIUS: f32 = 11.0;
/// Right padding inside an option row, matching the badge's inset on the left.
const OPTION_PAD_RIGHT: f32 = 14.0;
/// Gap between an option row and its keyboard focus ring. Kept under half of
/// `OPTION_GAP` so rings never touch the next row.
const FOCUS_RING_OUTSET: f32 = 2.5;
const BTN_H: f32 = 36.0;
const BTN_GAP: f32 = 10.0;
const BTN_RADIUS: f32 = 10.0;
/// The open button's own row, below grant/deny, when both are shown.
const OPEN_ROW_GAP: f32 = 4.0;
const OPEN_BTN_H: f32 = 32.0;
pub const PANEL_RADIUS: f32 = 20.0;

/// Subsurface buffer dimensions (logical units passed to `SubsurfaceSurface::new`).
/// Sized for the tallest panel so it never reallocates on the way there.
pub const DIALOG_BUF_W: i32 = 360;
pub const DIALOG_BUF_H: i32 = DIALOG_MAX_H as i32;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One selectable option within a choice group.
#[derive(Clone, Debug)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub icon: String,
}

/// A group of options (e.g. "output" → list of connectors). Single-select
/// unless `multi`, in which case any number of its options can be picked.
#[derive(Clone, Debug, Default)]
pub struct ChoiceGroup {
    pub id: String,
    pub label: String,
    pub options: Vec<ChoiceOption>,
    /// Index of the initially-selected option (for a multi-select group,
    /// where the keyboard starts).
    pub default: usize,
    /// Any number of options may be picked, each toggled on its own.
    pub multi: bool,
    /// For a multi-select group: the options picked to begin with.
    pub picked: Vec<usize>,
}

/// The extras a `PresentQuestions` dialog carries over `PresentQuestion`.
#[derive(Clone, Debug, Default)]
pub struct QuestionStyle {
    /// Several groups are asked one per page.
    pub paged: bool,
    /// The grant button's label on every page but the last. Empty uses the
    /// grant label throughout.
    pub next_label: String,
    /// The back button's label, from the second page on. Empty hides it
    /// (Left still goes back).
    pub back_label: String,
    /// The page counter, with `{current}` and `{total}` filled in. Empty
    /// shows none.
    pub page_label: String,
    /// A line under a multi-select group's label ("Choose any"). Empty shows
    /// none.
    pub multi_hint: String,
    /// The body reads as a left-aligned list rather than centred text.
    pub body_start: bool,
    /// The title is the agent's handle ("@claude"), not a headline: it is
    /// drawn small and muted at the top, and the question itself becomes the
    /// panel's largest text. Permission dialogs keep the headline.
    pub handle_title: bool,
}

/// What the user has picked: `selected[g]` is a single-select group's option
/// (and a multi-select group's keyboard position); `picked[g][o]` whether a
/// multi-select group's option `o` is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picks {
    pub selected: Vec<usize>,
    pub picked: Vec<Vec<bool>>,
}

impl Picks {
    /// The picks a dialog opens with: each group's defaults.
    pub fn new(choices: &[ChoiceGroup]) -> Self {
        Self {
            selected: choices.iter().map(|g| g.default).collect(),
            picked: choices
                .iter()
                .map(|g| {
                    (0..g.options.len())
                        .map(|o| g.multi && g.picked.contains(&o))
                        .collect()
                })
                .collect(),
        }
    }

    /// Whether option `option` of group `group` shows as chosen.
    pub fn is_chosen(&self, choices: &[ChoiceGroup], group: usize, option: usize) -> bool {
        match choices.get(group) {
            Some(g) if g.multi => self
                .picked
                .get(group)
                .and_then(|p| p.get(option))
                .copied()
                .unwrap_or(false),
            Some(g) => self.selected.get(group).copied().unwrap_or(g.default) == option,
            None => false,
        }
    }

    /// Choose `option` in `group`: select it in a single-select group, flip it
    /// in a multi-select one. Either way the keyboard moves onto it.
    pub fn choose(&mut self, choices: &[ChoiceGroup], group: usize, option: usize) {
        if let Some(sel) = self.selected.get_mut(group) {
            *sel = option;
        }
        if choices.get(group).is_some_and(|g| g.multi) {
            if let Some(on) = self.picked.get_mut(group).and_then(|p| p.get_mut(option)) {
                *on = !*on;
            }
        }
    }

    /// The answer: `(group_id, option_id)` for a single-select group's option,
    /// and one entry per picked option of a multi-select group (none when
    /// nothing is picked).
    pub fn results(&self, choices: &[ChoiceGroup]) -> Vec<(String, String)> {
        let mut results = Vec::new();
        for (gi, g) in choices.iter().enumerate() {
            if g.multi {
                for (oi, o) in g.options.iter().enumerate() {
                    if self.is_chosen(choices, gi, oi) {
                        results.push((g.id.clone(), o.id.clone()));
                    }
                }
            } else {
                let idx = self.selected.get(gi).copied().unwrap_or(g.default);
                if let Some(o) = g.options.get(idx) {
                    results.push((g.id.clone(), o.id.clone()));
                }
            }
        }
        results
    }
}

/// The user's decision, returned to the caller.
#[derive(Clone, Debug, Default)]
pub struct DialogResponse {
    /// `0` granted/confirmed, `1` cancelled/denied, `2` ended (withdrawn/error),
    /// `3` the open button was pressed.
    pub response: u32,
    /// `(group_id, selected_option_id)` for each choice group.
    pub results: Vec<(String, String)>,
}

/// Response codes carried in [`DialogResponse::response`].
pub const RESPONSE_GRANTED: u32 = 0;
pub const RESPONSE_DENIED: u32 = 1;
pub const RESPONSE_ENDED: u32 = 2;
pub const RESPONSE_OPEN: u32 = 3;

impl DialogResponse {
    pub fn ended() -> Self {
        Self {
            response: RESPONSE_ENDED,
            results: Vec::new(),
        }
    }
}

/// A pending dialog request. Owns the one-shot channel used to deliver the
/// decision back to the (async) D-Bus caller.
pub struct DialogRequest {
    pub id: DialogId,
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub icon: String,
    /// Empty hides the grant button.
    pub grant_label: String,
    pub deny_label: String,
    /// Empty hides the open button.
    pub open_label: String,
    pub modal: bool,
    pub choices: Vec<ChoiceGroup>,
    pub style: QuestionStyle,
    pub response_tx: Option<oneshot::Sender<DialogResponse>>,
}

impl DialogRequest {
    /// A clone-able display snapshot (without the response channel) for the UI.
    pub fn view(&self) -> DialogView {
        DialogView {
            id: self.id,
            app_id: self.app_id.clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            body: self.body.clone(),
            icon: self.icon.clone(),
            grant_label: self.grant_label.clone(),
            deny_label: self.deny_label.clone(),
            open_label: self.open_label.clone(),
            modal: self.modal,
            choices: self.choices.clone(),
            style: self.style.clone(),
        }
    }

    /// True once the caller has abandoned the request (receiver dropped).
    pub fn is_withdrawn(&self) -> bool {
        self.response_tx.as_ref().is_none_or(|tx| tx.is_closed())
    }
}

/// Display snapshot of a [`DialogRequest`] used by the render/UI layer.
#[derive(Clone)]
pub struct DialogView {
    pub id: DialogId,
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub icon: String,
    /// Empty hides the grant button.
    pub grant_label: String,
    pub deny_label: String,
    /// Empty hides the open button.
    pub open_label: String,
    pub modal: bool,
    pub choices: Vec<ChoiceGroup>,
    pub style: QuestionStyle,
}

impl DialogView {
    /// How many pages the dialog has: one per group when paged, else one.
    pub fn pages(&self) -> usize {
        if self.style.paged {
            self.choices.len().max(1)
        } else {
            1
        }
    }
}

/// How long a non-modal dialog nobody has touched stays open before it
/// shrinks into its circle. Held open while the pointer rests on it.
pub const DIALOG_READ_SECS: u64 = 12;

/// Whether a presented dialog is open as a panel or shrunk into a circle.
///
/// A modal dialog is always open. A non-modal one opens on arrival and
/// shrinks when the user moves on: when the keyboard focus it was given
/// leaves, or, if it was never touched, once [`DIALOG_READ_SECS`] pass
/// without the pointer on it. Shrinking is not an answer; the request stays
/// pending, and clicking the circle opens the panel again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presence {
    modal: bool,
    collapsed: bool,
    /// The user clicked the panel, so it holds the keyboard focus and waits
    /// for that focus to leave rather than for the read window.
    focused: bool,
    /// When the untouched panel shrinks.
    read_until: Option<Instant>,
}

impl Presence {
    pub fn new(modal: bool, now: Instant) -> Self {
        Self {
            modal,
            collapsed: false,
            focused: false,
            read_until: (!modal).then(|| now + Duration::from_secs(DIALOG_READ_SECS)),
        }
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    /// The next instant [`Presence::tick`] has something to do.
    pub fn deadline(&self) -> Option<Instant> {
        self.read_until.filter(|_| !self.collapsed)
    }

    /// The pointer is over the open panel: keep it open while it is read.
    pub fn pointer_over(&mut self, now: Instant) {
        if self.read_until.is_some() && !self.collapsed {
            self.read_until = Some(now + Duration::from_secs(DIALOG_READ_SECS));
        }
    }

    /// Which shape the dialog takes: the open panel, the circle, or — while
    /// the pointer is on the circle — the peek pill an island grows to.
    pub fn shape(&self, hovered: bool) -> Shape {
        match (self.collapsed, hovered) {
            (false, _) => Shape::Panel,
            (true, false) => Shape::Circle,
            (true, true) => Shape::Peek,
        }
    }

    /// The open panel got the keyboard: on presenting, on a click, or on
    /// opening from the circle. It now waits for that focus to leave rather
    /// than for the read window.
    pub fn focus_gained(&mut self) {
        if !self.collapsed {
            self.focused = true;
            self.read_until = None;
        }
    }

    /// The keyboard focus left the island layer. Returns whether the dialog
    /// shrank.
    pub fn focus_lost(&mut self) -> bool {
        self.focused = false;
        self.collapse()
    }

    /// Shrink an untouched panel whose read window ran out. Returns whether
    /// the dialog shrank.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.read_until {
            Some(until) if !self.focused && now >= until => self.collapse(),
            _ => false,
        }
    }

    /// The user clicked the circle: open the panel, holding the keyboard.
    pub fn expand(&mut self) {
        self.collapsed = false;
        self.focused = true;
        self.read_until = None;
    }

    fn collapse(&mut self) -> bool {
        if self.modal || self.collapsed {
            return false;
        }
        self.collapsed = true;
        self.focused = false;
        self.read_until = None;
        true
    }
}

/// The shape a presented dialog takes on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The full panel below the island bar.
    Panel,
    /// Shrunk: a Mini-sized circle at the end of the island row.
    Circle,
    /// Shrunk, with the pointer on it: grown to the Compact pill, icon and
    /// title, as a hovered island does.
    Peek,
}

/// What was hit by a pointer, in panel-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogHit {
    Grant,
    Deny,
    Open,
    Back,
    /// A progress dot: go to the question it stands for.
    Page(usize),
    Option {
        group: usize,
        option: usize,
    },
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Title: 15pt bold, wrapped over at most this many lines.
const TITLE_SIZE: f32 = 15.0;
const TITLE_LINE_H: f32 = 19.0;
const TITLE_MAX_LINES: usize = 3;
/// Subtitle and body: 12pt. Both wrap; a question's text usually lives here.
const TEXT_SIZE: f32 = 12.0;
const TEXT_LINE_H: f32 = 16.0;
const SUBTITLE_MAX_LINES: usize = 12;
const BODY_MAX_LINES: usize = 40;
/// A group label is the question a choice group answers, so it wraps too.
const GROUP_LABEL_SIZE: f32 = 12.0;
const GROUP_LABEL_MAX_LINES: usize = 12;
/// Space between a group label's last line and the group's first option.
const GROUP_LABEL_GAP: f32 = 4.0;
/// Option rows: a wrapped label, and under it an optional wrapped description.
const OPTION_LABEL_SIZE: f32 = 13.0;
const OPTION_LABEL_LINE_H: f32 = 17.0;
const OPTION_LABEL_MAX_LINES: usize = 3;
const OPTION_DESC_SIZE: f32 = 11.0;
const OPTION_DESC_LINE_H: f32 = 14.0;
const OPTION_DESC_MAX_LINES: usize = 4;
const OPTION_PAD_Y: f32 = 12.0;
const OPTION_ICON: f32 = 24.0;
/// The number badge an option's digit shortcut is shown in.
const BADGE: f32 = 20.0;
const BADGE_X: f32 = 10.0;
/// Where an option's icon (or, without one, its text) starts.
const OPTION_ICON_X: f32 = BADGE_X + BADGE + 10.0;
/// Options past this many in a group have no digit shortcut.
pub const MAX_SHORTCUTS: usize = 9;
/// The tallest a panel gets. Past it the text and choices scroll under a
/// fixed row of buttons, which always stay on screen.
pub const DIALOG_MAX_H: f32 = 620.0;

/// Computed geometry for a dialog panel, in panel-local coordinates
/// (origin at the panel top-left, logical units).
///
/// Everything above the buttons — icon, text and choices — is laid out in
/// *content* coordinates and scrolls inside [`DialogLayout::viewport`] when it
/// runs taller than the panel allows. The buttons are in panel coordinates.
pub struct DialogLayout {
    pub width: f32,
    pub height: f32,
    /// The part of the panel the content shows through: `(0, 0)` to the top
    /// of the button row.
    pub viewport: Rect,
    /// Full height of the content, which may exceed the viewport's.
    pub content_h: f32,
    /// `None` when the grant button is hidden (empty label).
    pub grant_rect: Option<Rect>,
    pub deny_rect: Rect,
    /// `None` when the open button is hidden (empty label).
    pub open_rect: Option<Rect>,
    /// `(group_idx, option_idx, row_rect)` for every option row, in content
    /// coordinates. A paged dialog lays out only its current page's group.
    pub option_rects: Vec<(usize, usize, Rect)>,
    /// The back button, in content coordinates: from a paged dialog's second
    /// page on, when it has a back label.
    pub back_rect: Option<Rect>,
    /// One dot per question, in content coordinates, when there are several.
    /// Each is a click target for the page it stands for.
    pub dots: Vec<Rect>,
    /// The page shown (`0` unless paged); `dots` says how many there are.
    pub page: usize,
    /// What the grant button says: the next label on a page before the last.
    pub grant_text: String,
    // Internal draw anchors (content coords).
    icon_present: bool,
    handle_title: bool,
    /// The page counter, drawn beside the buttons in panel coordinates.
    page_counter: Option<TextBlock>,
    group_hints: Vec<TextBlock>,
    body_start: bool,
    title: TextBlock,
    subtitle: Option<TextBlock>,
    body: Option<TextBlock>,
    group_labels: Vec<TextBlock>,
    /// Parallel to `option_rects`: the wrapped label and description lines.
    option_text: Vec<(Vec<String>, Vec<String>)>,
}

/// Wrapped lines and the top edge they start at.
struct TextBlock {
    y: f32,
    lines: Vec<String>,
}

impl DialogLayout {
    /// How far the content can scroll: zero when it fits.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.viewport.height()).max(0.0)
    }
}

/// An option's label and its description. Callers pass a description after
/// the label's first line break (`"Postgres\nRelational, battle-tested"`), so
/// the wire format stays `(id, label, icon)`.
pub fn split_option_label(label: &str) -> (&str, &str) {
    match label.split_once('\n') {
        Some((label, description)) => (label.trim_end(), description.trim()),
        None => (label, ""),
    }
}

/// Wrap `text` to `max_w`, keeping the caller's own line breaks, over at most
/// `max_lines` lines. Blank lines are kept as paragraph breaks.
fn wrap(text: &str, f: &skia_safe::Font, max_w: f32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.lines() {
        if lines.len() >= max_lines {
            break;
        }
        if paragraph.trim().is_empty() {
            // A run of blank lines is one paragraph break, never a leading one.
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        let budget = max_lines - lines.len();
        lines.extend(crate::renderer::wrap_text(paragraph, f, max_w, budget));
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    // Lines dropped past the cap: say so on the last line kept.
    let kept: usize = lines.iter().map(|l| l.split_whitespace().count()).sum();
    if kept < text.split_whitespace().count() {
        if let Some(last) = lines.last_mut() {
            if !last.ends_with('…') {
                *last = ellipsize(&format!("{last} …"), f, max_w);
            }
        }
    }
    lines
}

/// The back button's label, with the chevron that points the way back.
fn back_label(label: &str) -> String {
    format!("\u{2039} {label}")
}

/// The handle row at the top: a small icon and the agent's handle.
const HANDLE_ROW_H: f32 = 18.0;
const HANDLE_SIZE: f32 = 12.0;
const HANDLE_ICON: f32 = 14.0;
/// The back button, at the left of the handle row.
const BACK_BTN_H: f32 = 20.0;
const BACK_BTN_PAD_X: f32 = 8.0;
/// Progress dots: one per question, under the handle.
const DOT: f32 = 6.0;
const DOT_GAP: f32 = 7.0;
const DOT_ROW_H: f32 = 16.0;
const DOT_ROW_GAP: f32 = 10.0;
/// A dot's click target, centred on the dot itself.
const DOT_HIT: f32 = 18.0;
/// The page counter beside the buttons ("2 of 3"), and the column it takes.
const PAGE_TEXT_SIZE: f32 = 11.0;
const COUNTER_W: f32 = 40.0;
/// The question a page asks, the largest text on a handle-titled panel.
const QUESTION_SIZE: f32 = 15.0;
const QUESTION_LINE_H: f32 = 20.0;

/// Compute the panel layout for the given request view, showing `page` of a
/// paged dialog (ignored otherwise).
pub fn dialog_layout(view: &DialogView, page: usize) -> DialogLayout {
    let pages = view.pages();
    let page = page.min(pages - 1);
    let w = DIALOG_W;
    let text_max_w = w - PAD * 2.0;
    let mut y = PAD;

    // A handle-titled panel says who is asking in one small line, with the
    // icon beside it, and gives the room to the question itself. A headline
    // panel (a permission grant) keeps the big icon and title.
    let handle_title = view.style.handle_title;
    let icon_present = !view.icon.is_empty();
    let mut back_rect = None;
    if icon_present && !handle_title {
        y += ICON + ICON_GAP;
    }

    let (title_font, title_line_h, title_max_lines) = if handle_title {
        (font(HANDLE_SIZE, 600), HANDLE_ROW_H, 1)
    } else {
        (font(TITLE_SIZE, 700), TITLE_LINE_H, TITLE_MAX_LINES)
    };
    let title_max_w = if handle_title {
        // Room for the icon beside it, and for the back button either side.
        text_max_w - (HANDLE_ICON + 6.0) - BACK_BTN_H * 2.0
    } else {
        text_max_w
    };
    let title_lines = wrap(&view.title, &title_font, title_max_w, title_max_lines);
    let title = TextBlock {
        y,
        lines: title_lines,
    };
    if page > 0 && !view.style.back_label.is_empty() {
        let tw = font(PAGE_TEXT_SIZE + 1.0, 600)
            .measure_str(back_label(&view.style.back_label), None)
            .0;
        let bw = (tw + BACK_BTN_PAD_X * 2.0).min(text_max_w / 2.0);
        // At the top-left corner, beside the handle (or over the headline's
        // own top padding), so the page row it used to live in is free for
        // the question.
        back_rect = Some(Rect::from_xywh(
            PAD,
            PAD + (HANDLE_ROW_H - BACK_BTN_H).max(0.0) / 2.0,
            bw,
            BACK_BTN_H,
        ));
    }
    y += title_line_h * title.lines.len().max(1) as f32 + TITLE_GAP;

    // Progress: one dot per question, the current one accented. They stand in
    // for the counter at the top; the counter itself sits by the buttons.
    let mut dots = Vec::new();
    if pages > 1 {
        let row_w = pages as f32 * DOT + (pages - 1) as f32 * DOT_GAP;
        let mut x = (w - row_w) / 2.0;
        let cy = y + DOT_ROW_H / 2.0;
        for _ in 0..pages {
            dots.push(Rect::from_xywh(x, cy - DOT / 2.0, DOT, DOT));
            x += DOT + DOT_GAP;
        }
        y += DOT_ROW_H + DOT_ROW_GAP;
    }

    let text_block = |text: &str, weight: i32, max_lines: usize, gap: f32, y: &mut f32| {
        let lines = wrap(text, &font(TEXT_SIZE, weight), text_max_w, max_lines);
        if lines.is_empty() {
            return None;
        }
        let block = TextBlock { y: *y, lines };
        *y += TEXT_LINE_H * block.lines.len() as f32 + gap;
        Some(block)
    };
    let subtitle = text_block(
        &view.subtitle,
        500,
        SUBTITLE_MAX_LINES,
        SUBTITLE_GAP,
        &mut y,
    );
    let body = text_block(&view.body, 400, BODY_MAX_LINES, BODY_GAP, &mut y);

    // Choice groups.
    let mut option_rects = Vec::new();
    let mut option_text = Vec::new();
    let mut group_labels = Vec::new();
    let mut group_hints = Vec::new();
    let label_font = font(OPTION_LABEL_SIZE, 500);
    let desc_font = font(OPTION_DESC_SIZE, 400);
    for (gi, group) in view.choices.iter().enumerate() {
        if group.options.is_empty() || (pages > 1 && gi != page) {
            continue;
        }
        let lines = wrap(
            &group.label,
            &question_font(handle_title),
            text_max_w,
            GROUP_LABEL_MAX_LINES,
        );
        if !lines.is_empty() {
            let h = question_line_h(handle_title) * lines.len() as f32;
            group_labels.push(TextBlock { y, lines });
            y += h + GROUP_LABEL_GAP;
        }
        if group.multi && !view.style.multi_hint.is_empty() {
            let lines = wrap(
                &view.style.multi_hint,
                &font(OPTION_DESC_SIZE, 400),
                text_max_w,
                2,
            );
            let h = OPTION_DESC_LINE_H * lines.len() as f32;
            group_hints.push(TextBlock { y: y - 2.0, lines });
            y += h + GROUP_LABEL_GAP;
        }
        for (oi, opt) in group.options.iter().enumerate() {
            let (label, description) = split_option_label(&opt.label);
            let text_x = option_text_x(!opt.icon.is_empty());
            let col_w = (w - PAD - OPTION_PAD_RIGHT) - (PAD + text_x);
            let label_lines = wrap(label, &label_font, col_w, OPTION_LABEL_MAX_LINES);
            let desc_lines = wrap(description, &desc_font, col_w, OPTION_DESC_MAX_LINES);
            let text_h = OPTION_LABEL_LINE_H * label_lines.len().max(1) as f32
                + OPTION_DESC_LINE_H * desc_lines.len() as f32;
            let row_h = (text_h + OPTION_PAD_Y * 2.0).max(OPTION_H);
            let rect = Rect::from_xywh(PAD, y, w - PAD * 2.0, row_h);
            option_rects.push((gi, oi, rect));
            option_text.push((label_lines, desc_lines));
            y += row_h + OPTION_GAP;
        }
        y += 4.0; // extra gap after a group
    }
    let content_h = y;

    // Buttons. The grant button is the default action and always sits at the
    // right of the main row. The open button is secondary: with a grant button
    // it gets a row of its own below; without one it takes the grant's slot,
    // drawn neutral so it never reads as the default.
    let last_page = page + 1 == pages;
    let grant_text = if !last_page && !view.style.next_label.is_empty() {
        view.style.next_label.clone()
    } else {
        view.grant_label.clone()
    };
    let has_grant = !grant_text.is_empty();
    let has_open = !view.open_label.is_empty();
    let mut footer_h = 2.0 + BTN_H + PAD;
    if has_grant && has_open {
        footer_h += OPEN_ROW_GAP + OPEN_BTN_H;
    }
    let height = (content_h + footer_h).min(DIALOG_MAX_H);
    let viewport = Rect::from_xywh(0.0, 0.0, w, height - footer_h);

    let mut y = viewport.bottom + 2.0;
    // With several questions the counter takes a column at the left of the
    // button row, so it says where the page is next to the way on from it.
    let mut page_counter = None;
    let mut buttons_x = PAD;
    let mut full_w = w - PAD * 2.0;
    if pages > 1 && !view.style.page_label.is_empty() {
        let text = view
            .style
            .page_label
            .replace("{current}", &(page + 1).to_string())
            .replace("{total}", &pages.to_string());
        page_counter = Some(TextBlock {
            y: y + (BTN_H - TEXT_LINE_H) / 2.0,
            lines: vec![text],
        });
        buttons_x += COUNTER_W;
        full_w -= COUNTER_W;
    }
    let half_w = (full_w - BTN_GAP) / 2.0;
    let (deny_rect, grant_rect, mut open_rect) = if has_grant || has_open {
        let deny = Rect::from_xywh(buttons_x, y, half_w, BTN_H);
        let right = Rect::from_xywh(buttons_x + half_w + BTN_GAP, y, half_w, BTN_H);
        if has_grant {
            (deny, Some(right), None)
        } else {
            (deny, None, Some(right))
        }
    } else {
        (Rect::from_xywh(buttons_x, y, full_w, BTN_H), None, None)
    };
    y += BTN_H;
    if has_grant && has_open {
        y += OPEN_ROW_GAP;
        open_rect = Some(Rect::from_xywh(PAD, y, w - PAD * 2.0, OPEN_BTN_H));
    }

    DialogLayout {
        width: w,
        height,
        viewport,
        content_h,
        grant_rect,
        deny_rect,
        open_rect,
        option_rects,
        back_rect,
        dots,
        page,
        grant_text,
        icon_present,
        handle_title,
        page_counter,
        group_hints,
        body_start: view.style.body_start,
        title,
        subtitle,
        body,
        group_labels,
        option_text,
    }
}

/// The question a group asks is the panel's largest text when the title is
/// only a handle; under a headline title it stays a supporting label.
fn question_font(handle_title: bool) -> skia_safe::Font {
    if handle_title {
        font(QUESTION_SIZE, 600)
    } else {
        font(GROUP_LABEL_SIZE, 600)
    }
}

fn question_line_h(handle_title: bool) -> f32 {
    if handle_title {
        QUESTION_LINE_H
    } else {
        TEXT_LINE_H
    }
}

/// Where an option's text starts, relative to its row's left edge: after the
/// number badge column, and the icon when there is one.
fn option_text_x(has_icon: bool) -> f32 {
    if has_icon {
        OPTION_ICON_X + OPTION_ICON + 10.0
    } else {
        OPTION_ICON_X
    }
}

fn in_rect(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

/// Hit-test a point in panel-local coordinates, with the content scrolled
/// down by `scroll`.
pub fn hit_test(layout: &DialogLayout, lx: f32, ly: f32, scroll: f32) -> Option<DialogHit> {
    if layout.grant_rect.is_some_and(|r| in_rect(&r, lx, ly)) {
        return Some(DialogHit::Grant);
    }
    if in_rect(&layout.deny_rect, lx, ly) {
        return Some(DialogHit::Deny);
    }
    if layout.open_rect.is_some_and(|r| in_rect(&r, lx, ly)) {
        return Some(DialogHit::Open);
    }
    // A row scrolled out from under the buttons can't be picked through them.
    if !in_rect(&layout.viewport, lx, ly) {
        return None;
    }
    let cy = ly + scroll.clamp(0.0, layout.max_scroll());
    if layout.back_rect.is_some_and(|r| in_rect(&r, lx, cy)) {
        return Some(DialogHit::Back);
    }
    // A dot goes back to the question it stands for. Forward is no jump: the
    // questions in between have not been answered yet.
    for (page, dot) in layout.dots.iter().enumerate() {
        let pad = (DOT_HIT - DOT) / 2.0;
        if in_rect(&dot.with_outset((pad, pad)), lx, cy) {
            return Some(DialogHit::Page(page));
        }
    }
    for (gi, oi, rect) in &layout.option_rects {
        if in_rect(rect, lx, cy) {
            return Some(DialogHit::Option {
                group: *gi,
                option: *oi,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Keyboard navigation
// ---------------------------------------------------------------------------

/// The index into [`DialogLayout::option_rects`] of `option` in `group`.
pub fn row_of(layout: &DialogLayout, group: usize, option: usize) -> Option<usize> {
    layout
        .option_rects
        .iter()
        .position(|(g, o, _)| *g == group && *o == option)
}

/// The row the keyboard is on: `focus`, or else the selected option of the
/// first group. `None` when the dialog has no options.
pub fn keyboard_row(
    layout: &DialogLayout,
    selected: &[usize],
    focus: Option<usize>,
) -> Option<usize> {
    focus
        .filter(|row| *row < layout.option_rects.len())
        .or_else(|| {
            let (group, _, _) = layout.option_rects.first()?;
            row_of(layout, *group, selected.get(*group).copied().unwrap_or(0))
        })
}

/// The row `delta` steps from `current` (Up is `-1`, Down `+1`) within its
/// group, stopping at the group's first and last options; Tab moves between
/// groups.
pub fn step_row(layout: &DialogLayout, current: Option<usize>, delta: i32) -> Option<usize> {
    let Some(current) = current else {
        return (!layout.option_rects.is_empty()).then_some(0);
    };
    let group = layout.option_rects.get(current)?.0;
    let first = layout
        .option_rects
        .iter()
        .position(|(g, _, _)| *g == group)?;
    let last = layout
        .option_rects
        .iter()
        .rposition(|(g, _, _)| *g == group)?;
    Some((current as i64 + delta as i64).clamp(first as i64, last as i64) as usize)
}

/// A button the keyboard can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogButton {
    Back,
    Deny,
    Grant,
    Open,
}

/// Where the keyboard is: an option row (an index into `option_rects`) or a
/// button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardTarget {
    Row(usize),
    Button(DialogButton),
}

/// The buttons the layout shows, in the order Tab visits them: the main row
/// left to right, then the open button's row when it has one of its own.
pub fn buttons(layout: &DialogLayout) -> Vec<DialogButton> {
    let mut order = vec![DialogButton::Deny];
    if layout.grant_rect.is_some() {
        order.push(DialogButton::Grant);
    }
    if layout.open_rect.is_some() {
        order.push(DialogButton::Open);
    }
    if layout.back_rect.is_some() {
        order.push(DialogButton::Back);
    }
    order
}

/// Where `button` is drawn, when the layout shows it.
pub fn button_rect(layout: &DialogLayout, button: DialogButton) -> Option<Rect> {
    match button {
        // In content coordinates, unlike the rest: it scrolls with the page.
        DialogButton::Back => layout.back_rect,
        DialogButton::Deny => Some(layout.deny_rect),
        DialogButton::Grant => layout.grant_rect,
        DialogButton::Open => layout.open_rect,
    }
}

/// A place Tab stops: a whole choice group, or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabStop {
    Group(usize),
    Button(DialogButton),
}

/// The groups that have option rows, in order.
fn groups(layout: &DialogLayout) -> Vec<usize> {
    let mut groups: Vec<usize> = Vec::new();
    for (group, _, _) in &layout.option_rects {
        if groups.last() != Some(group) {
            groups.push(*group);
        }
    }
    groups
}

/// The target Tab (`backwards` for Shift+Tab) moves to from `current`: each
/// choice group as one stop (landing on its selected option), then each
/// button, wrapping around at either end.
pub fn tab_step(
    layout: &DialogLayout,
    selected: &[usize],
    current: Option<KeyboardTarget>,
    backwards: bool,
) -> KeyboardTarget {
    let order: Vec<TabStop> = groups(layout)
        .into_iter()
        .map(TabStop::Group)
        .chain(buttons(layout).into_iter().map(TabStop::Button))
        .collect();
    let len = order.len();
    let here = current.map(|c| match c {
        KeyboardTarget::Row(row) => TabStop::Group(layout.option_rects[row].0),
        KeyboardTarget::Button(button) => TabStop::Button(button),
    });
    let index = here.and_then(|h| order.iter().position(|t| *t == h));
    let next = match (index, backwards) {
        (None, false) => 0,
        (None, true) => len - 1,
        (Some(i), false) => (i + 1) % len,
        (Some(i), true) => (i + len - 1) % len,
    };
    match order[next] {
        TabStop::Group(group) => {
            let option = selected.get(group).copied().unwrap_or(0);
            KeyboardTarget::Row(row_of(layout, group, option).unwrap_or_else(|| {
                layout
                    .option_rects
                    .iter()
                    .position(|(g, _, _)| *g == group)
                    .unwrap_or(0)
            }))
        }
        TabStop::Button(button) => KeyboardTarget::Button(button),
    }
}

/// The row of the next group after `row`'s, on its selected option.
pub fn next_group_row(layout: &DialogLayout, selected: &[usize], row: usize) -> Option<usize> {
    let group = layout.option_rects.get(row)?.0;
    let next = groups(layout).into_iter().find(|g| *g > group)?;
    row_of(layout, next, selected.get(next).copied().unwrap_or(0))
}

/// Whether the dialog asks more than one question.
pub fn has_several_groups(layout: &DialogLayout) -> bool {
    groups(layout).len() > 1
}

/// The scroll that brings `row` fully into view, moving as little as
/// possible from `scroll`.
pub fn reveal(layout: &DialogLayout, row: usize, scroll: f32) -> f32 {
    let Some((_, _, rect)) = layout.option_rects.get(row) else {
        return scroll;
    };
    let view_h = layout.viewport.height();
    let mut scroll = scroll;
    if rect.bottom + OPTION_GAP > scroll + view_h {
        scroll = rect.bottom + OPTION_GAP - view_h;
    }
    if rect.top - OPTION_GAP < scroll {
        scroll = rect.top - OPTION_GAP;
    }
    scroll.clamp(0.0, layout.max_scroll())
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn font(size: f32, weight: i32) -> skia_safe::Font {
    TextStyle {
        family: "Inter",
        weight,
        size,
    }
    .font()
}

/// Truncate `text` to fit `max_w`, appending an ellipsis when it doesn't.
///
/// Button labels are arbitrary and can be longer than the button — without
/// this they run off its rounded edge. Walks back by character (not byte) so
/// multi-byte text can't be split mid-codepoint.
fn ellipsize(text: &str, f: &skia_safe::Font, max_w: f32) -> String {
    if f.measure_str(text, None).0 <= max_w {
        return text.to_string();
    }
    const ELLIPSIS: &str = "…";
    let ellipsis_w = f.measure_str(ELLIPSIS, None).0;
    // Nothing sensible fits — an ellipsis alone still beats overflowing.
    if ellipsis_w > max_w {
        return ELLIPSIS.to_string();
    }
    let budget = max_w - ellipsis_w;
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if f.measure_str(&text[..next], None).0 > budget {
            break;
        }
        end = next;
    }
    // Don't leave a dangling space before the ellipsis.
    format!("{}{ELLIPSIS}", text[..end].trim_end())
}

/// Centered variant: truncates to `max_w`, then centers whatever survived.
fn draw_text_centered_clamped(
    canvas: &Canvas,
    text: &str,
    cx: f32,
    baseline_y: f32,
    f: &skia_safe::Font,
    color: Color,
    max_w: f32,
) {
    draw_text_centered(canvas, &ellipsize(text, f, max_w), cx, baseline_y, f, color);
}

fn draw_text_centered(
    canvas: &Canvas,
    text: &str,
    cx: f32,
    baseline_y: f32,
    f: &skia_safe::Font,
    color: Color,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    let (tw, _) = f.measure_str(text, None);
    canvas.draw_str(text, (cx - tw / 2.0, baseline_y), f, &paint);
}

/// Draw wrapped lines centered on `cx`, the first line's top at `top`.
fn draw_lines_centered(
    canvas: &Canvas,
    lines: &[String],
    cx: f32,
    top: f32,
    line_h: f32,
    f: &skia_safe::Font,
    color: Color,
) {
    for (i, line) in lines.iter().enumerate() {
        let baseline = top + line_h * i as f32 + line_h - 4.0;
        draw_text_centered(canvas, line, cx, baseline, f, color);
    }
}

/// Draw wrapped lines from `x`, the first line's top at `top`.
fn draw_lines(
    canvas: &Canvas,
    lines: &[String],
    x: f32,
    top: f32,
    line_h: f32,
    f: &skia_safe::Font,
    color: Color,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    for (i, line) in lines.iter().enumerate() {
        let baseline = top + line_h * i as f32 + line_h - 4.0;
        canvas.draw_str(line, (x, baseline), f, &paint);
    }
}

/// Draw the dialog content into the (already background-styled) subsurface
/// canvas. The buffer is sized to the panel by `draw_content`, so content is
/// drawn from the origin, matching the pill/card model.
/// `selected[gi]` is the chosen option index for group `gi`; `scroll` is how
/// far the content is scrolled. `focus`, when the panel holds the keyboard,
/// is the option row or button that gets the focus ring.
pub fn draw_dialog(
    canvas: &Canvas,
    view: &DialogView,
    picks: &Picks,
    layout: &DialogLayout,
    scroll: f32,
    focus: Option<KeyboardTarget>,
) {
    canvas.save();

    let w = layout.width;
    let cx = w / 2.0;
    let theme = otto_kit::AppContext::current_theme();
    // Text is black on light, white on dark — the theme decides. The dialog
    // takes it fully opaque: `text_primary` is deliberately soft (0xD9) for
    // chrome that sits on solid fills, but this panel floats over a blurred
    // backdrop of arbitrary content, where that softness reads as washed out.
    let text = opaque(theme.text_primary);
    // Same reasoning for the supporting text, but the hierarchy is preserved:
    // subtitle and body stay lighter than the title, just not ghostly.
    let dim = at_least_opaque(theme.text_secondary, 0xC0);
    let dim2 = at_least_opaque(theme.text_tertiary, 0x99);
    let accent = theme.accent;
    // Labels drawn on top of the accent fill stay white in both schemes.
    let on_accent = Color::WHITE;

    // No panel fill here: the surface style paints the translucent material over
    // a blurred backdrop (see [`apply_dialog_style`]) and clips the rounded
    // corners, so the canvas only carries content.

    // Scrollable content: clipped to the viewport, shifted by the scroll.
    canvas.save();
    canvas.clip_rect(layout.viewport, skia_safe::ClipOp::Intersect, true);
    canvas.translate((0.0, -scroll.clamp(0.0, layout.max_scroll())));

    if layout.handle_title {
        // Who is asking, in one small line: the icon and the handle together,
        // centred, so the question below is what the panel is about.
        let handle = layout.title.lines.first().cloned().unwrap_or_default();
        let f = font(HANDLE_SIZE, 600);
        let tw = f.measure_str(&handle, None).0;
        let icon_w = if layout.icon_present {
            HANDLE_ICON + 6.0
        } else {
            0.0
        };
        let left = cx - (tw + icon_w) / 2.0;
        let mid = layout.title.y + HANDLE_ROW_H / 2.0;
        if layout.icon_present {
            draw_icon(
                canvas,
                &view.icon,
                left,
                mid - HANDLE_ICON / 2.0,
                HANDLE_ICON,
            );
        }
        draw_lines(
            canvas,
            &[handle],
            left + icon_w,
            mid - HANDLE_ROW_H / 2.0,
            HANDLE_ROW_H,
            &f,
            dim,
        );
    } else {
        // Icon.
        if layout.icon_present {
            let ix = cx - ICON / 2.0;
            draw_icon(canvas, &view.icon, ix, PAD, ICON);
        }
        draw_lines_centered(
            canvas,
            &layout.title.lines,
            cx,
            layout.title.y,
            TITLE_LINE_H,
            &font(TITLE_SIZE, 700),
            text,
        );
    }

    // Progress dots: how many questions there are, and which one this is.
    for (page, dot) in layout.dots.iter().enumerate() {
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(if page == layout.page {
            accent
        } else {
            at_least_opaque(theme.fill_secondary, 0x80)
        });
        canvas.draw_circle((dot.center_x(), dot.center_y()), dot.width() / 2.0, &paint);
    }
    if let Some(block) = &layout.subtitle {
        let f = font(TEXT_SIZE, 500);
        draw_lines_centered(canvas, &block.lines, cx, block.y, TEXT_LINE_H, &f, dim);
    }
    if let Some(block) = &layout.body {
        let f = font(TEXT_SIZE, 400);
        if layout.body_start {
            draw_lines(canvas, &block.lines, PAD, block.y, TEXT_LINE_H, &f, dim);
        } else {
            draw_lines_centered(canvas, &block.lines, cx, block.y, TEXT_LINE_H, &f, dim2);
        }
    }

    if let Some(rect) = &layout.back_rect {
        draw_button_sized(
            canvas,
            rect,
            &back_label(&view.style.back_label),
            theme.fill_secondary,
            text,
            PAGE_TEXT_SIZE + 1.0,
        );
        if focus == Some(KeyboardTarget::Button(DialogButton::Back)) {
            draw_ring(canvas, *rect, BTN_RADIUS, accent);
        }
    }
    let hint_font = font(OPTION_DESC_SIZE, 400);
    for block in &layout.group_hints {
        draw_lines(
            canvas,
            &block.lines,
            PAD,
            block.y,
            OPTION_DESC_LINE_H,
            &hint_font,
            dim2,
        );
    }

    // The question each group answers, in full. Left-aligned, like the option
    // rows under it: a wrapped question centred over a left-aligned list has
    // no edge to read down.
    let group_font = question_font(layout.handle_title);
    let question_color = if layout.handle_title { text } else { dim };
    for block in &layout.group_labels {
        draw_lines(
            canvas,
            &block.lines,
            PAD,
            block.y,
            question_line_h(layout.handle_title),
            &group_font,
            question_color,
        );
    }

    // Option rows.
    let label_font = font(OPTION_LABEL_SIZE, 500);
    let desc_font = font(OPTION_DESC_SIZE, 400);
    for (row, ((gi, oi, rect), (label_lines, desc_lines))) in layout
        .option_rects
        .iter()
        .zip(&layout.option_text)
        .enumerate()
    {
        let Some(group) = view.choices.get(*gi) else {
            continue;
        };
        let Some(opt) = group.options.get(*oi) else {
            continue;
        };
        let is_selected = picks.is_chosen(&view.choices, *gi, *oi);

        let mut row_bg = Paint::default();
        row_bg.set_anti_alias(true);
        row_bg.set_color(if is_selected {
            accent
        } else {
            theme.fill_secondary
        });
        canvas.draw_rrect(
            RRect::new_rect_xy(*rect, OPTION_RADIUS, OPTION_RADIUS),
            &row_bg,
        );
        // Focus ring: where the arrow keys are, drawn just outside the row so
        // it reads on the accent fill of a selected row too.
        if focus == Some(KeyboardTarget::Row(row)) {
            let mut ring = Paint::default();
            ring.set_anti_alias(true);
            ring.set_color(accent);
            ring.set_style(skia_safe::paint::Style::Stroke);
            ring.set_stroke_width(2.0);
            let outer = rect.with_outset((FOCUS_RING_OUTSET, FOCUS_RING_OUTSET));
            let r = OPTION_RADIUS + FOCUS_RING_OUTSET;
            canvas.draw_rrect(RRect::new_rect_xy(outer, r, r), &ring);
        }

        // The digit that picks this option from the keyboard.
        if *oi < MAX_SHORTCUTS {
            let badge = Rect::from_xywh(
                rect.left + BADGE_X,
                rect.center_y() - BADGE / 2.0,
                BADGE,
                BADGE,
            );
            let mut badge_bg = Paint::default();
            badge_bg.set_anti_alias(true);
            badge_bg.set_color(if is_selected {
                Color::from_argb(0x40, 0xFF, 0xFF, 0xFF)
            } else {
                theme.fill_secondary
            });
            canvas.draw_rrect(RRect::new_rect_xy(badge, 6.0, 6.0), &badge_bg);
            draw_text_centered_clamped(
                canvas,
                &(oi + 1).to_string(),
                badge.center_x(),
                badge.center_y() + 4.0,
                &font(11.0, 600),
                if is_selected { on_accent } else { dim },
                BADGE,
            );
        }

        if !opt.icon.is_empty() {
            draw_icon(
                canvas,
                &opt.icon,
                rect.left + OPTION_ICON_X,
                rect.center_y() - OPTION_ICON / 2.0,
                OPTION_ICON,
            );
        }
        let text_x = rect.left + option_text_x(!opt.icon.is_empty());

        // The text block sits centered in the row, however many lines it took.
        let label_h = OPTION_LABEL_LINE_H * label_lines.len().max(1) as f32;
        let text_h = label_h + OPTION_DESC_LINE_H * desc_lines.len() as f32;
        let top = rect.center_y() - text_h / 2.0;
        draw_lines(
            canvas,
            label_lines,
            text_x,
            top,
            OPTION_LABEL_LINE_H,
            &label_font,
            if is_selected { on_accent } else { text },
        );
        draw_lines(
            canvas,
            desc_lines,
            text_x,
            top + label_h,
            OPTION_DESC_LINE_H,
            &desc_font,
            if is_selected {
                Color::from_argb(0xD9, 0xFF, 0xFF, 0xFF)
            } else {
                dim
            },
        );
    }
    canvas.restore();

    // A hairline over the buttons when content scrolls beneath them.
    if layout.max_scroll() > 0.0 {
        let mut line = Paint::default();
        line.set_anti_alias(true);
        line.set_color(dim2);
        line.set_alpha(0x40);
        canvas.draw_rect(
            Rect::from_xywh(0.0, layout.viewport.bottom - 0.5, w, 1.0),
            &line,
        );
    }

    // The page counter, in its column beside the buttons.
    if let Some(block) = &layout.page_counter {
        draw_lines(
            canvas,
            &block.lines,
            PAD,
            block.y,
            TEXT_LINE_H,
            &font(PAGE_TEXT_SIZE, 600),
            dim2,
        );
    }

    // Deny button.
    draw_button(
        canvas,
        &layout.deny_rect,
        &view.deny_label,
        theme.fill_secondary,
        text,
    );
    // Grant button (accent) — the default action.
    if let Some(rect) = &layout.grant_rect {
        draw_button(canvas, rect, &layout.grant_text, accent, on_accent);
    }
    // Open button. Beside deny (no grant) it matches deny's neutral fill; on
    // its own row below grant/deny it is a borderless accent-text action, so
    // it can't be mistaken for either.
    if let Some(rect) = &layout.open_rect {
        if layout.grant_rect.is_some() {
            draw_button(canvas, rect, &view.open_label, Color::TRANSPARENT, accent);
        } else {
            draw_button(canvas, rect, &view.open_label, theme.fill_secondary, text);
        }
    }
    // Focus ring on the button the keyboard is on.
    if let Some(KeyboardTarget::Button(button)) = focus.filter(|f| {
        // The back button's ring is drawn with the page, as it scrolls.
        *f != KeyboardTarget::Button(DialogButton::Back)
    }) {
        if let Some(rect) = button_rect(layout, button) {
            let mut ring = Paint::default();
            ring.set_anti_alias(true);
            ring.set_color(accent);
            ring.set_style(skia_safe::paint::Style::Stroke);
            ring.set_stroke_width(2.0);
            let outer = rect.with_outset((FOCUS_RING_OUTSET, FOCUS_RING_OUTSET));
            let r = BTN_RADIUS + FOCUS_RING_OUTSET;
            canvas.draw_rrect(RRect::new_rect_xy(outer, r, r), &ring);
        }
    }

    canvas.restore();
}

fn draw_button(canvas: &Canvas, rect: &Rect, label: &str, bg: Color, text: Color) {
    draw_button_sized(canvas, rect, label, bg, text, 13.0);
}

/// A focus ring just outside `rect`.
fn draw_ring(canvas: &Canvas, rect: Rect, radius: f32, color: Color) {
    let mut ring = Paint::default();
    ring.set_anti_alias(true);
    ring.set_color(color);
    ring.set_style(skia_safe::paint::Style::Stroke);
    ring.set_stroke_width(2.0);
    let outer = rect.with_outset((FOCUS_RING_OUTSET, FOCUS_RING_OUTSET));
    let r = radius + FOCUS_RING_OUTSET;
    canvas.draw_rrect(RRect::new_rect_xy(outer, r, r), &ring);
}

fn draw_button_sized(canvas: &Canvas, rect: &Rect, label: &str, bg: Color, text: Color, size: f32) {
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(true);
    bg_paint.set_color(bg);
    canvas.draw_rrect(RRect::new_rect_xy(*rect, BTN_RADIUS, BTN_RADIUS), &bg_paint);
    // Caller-supplied labels ("Share", but also whatever an app passes as
    // grant_label) must not spill past the button's rounded edge.
    draw_text_centered_clamped(
        canvas,
        label,
        rect.center_x(),
        rect.center_y() + size * 0.35,
        &font(size, 600),
        text,
        rect.width() - 16.0,
    );
}

fn draw_icon(canvas: &Canvas, icon_name: &str, x: f32, y: f32, size: f32) {
    if let Some(icon) = named_icon_sized(icon_name, size as i32) {
        let dst = Rect::from_xywh(x, y, size, size);
        let r = size * 0.22;
        canvas.save();
        canvas.clip_rrect(
            RRect::new_rect_xy(dst, r, r),
            skia_safe::ClipOp::Intersect,
            true,
        );
        let src = Rect::from_xywh(0.0, 0.0, icon.width() as f32, icon.height() as f32);
        canvas.draw_image_rect(
            &icon,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
            dst,
            &Paint::default(),
        );
        canvas.restore();
    }
}

/// Apply the frosted-panel surface style to a dialog subsurface: a translucent
/// theme material over a blurred backdrop, rounded clipping, drop shadow, and
/// center anchor. [`draw_dialog`] draws only the content on top.
/// The dialog's primary text colour: black on light, white on dark.
pub fn text_color() -> Color {
    opaque(otto_kit::AppContext::current_theme().text_primary)
}

/// Drop a colour's transparency, keeping its RGB.
fn opaque(c: Color) -> Color {
    Color::from_argb(0xFF, c.r(), c.g(), c.b())
}

/// Raise a colour's alpha to at least `min_alpha`, keeping its RGB.
fn at_least_opaque(c: Color, min_alpha: u8) -> Color {
    Color::from_argb(c.a().max(min_alpha), c.r(), c.g(), c.b())
}

pub fn apply_dialog_style(surface: &SubsurfaceSurface) {
    // `material_medium` is tuned for menus, which are small and sit close to
    // what they belong to. This panel is large, modal, and carries a decision —
    // it needs the backdrop legible behind it but the content unmistakably in
    // front, so it runs a good deal more solid than the shared material.
    let c = at_least_opaque(otto_kit::AppContext::current_theme().material_medium, 0xE0);
    if let Some(ss) = surface.base_surface().surface_style() {
        ss.set_background_color(
            c.r() as f64 / 255.0,
            c.g() as f64 / 255.0,
            c.b() as f64 / 255.0,
            c.a() as f64 / 255.0,
        );
        ss.set_corner_radius(PANEL_RADIUS as f64);
        ss.set_masks_to_bounds(ClipMode::Enabled);
        ss.set_shadow(0.35, 24.0, 0.0, 8.0, 0.0, 0.0, 0.0);
        ss.set_blend_mode(if otto_kit::frosting::enabled() {
            BlendMode::BackgroundBlur
        } else {
            BlendMode::Normal
        });
        ss.set_contents_gravity(ContentsGravity::TopLeft);
        ss.set_anchor_point(0.5, 0.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(grant: &str, open: &str) -> DialogView {
        DialogView {
            id: 1,
            app_id: String::new(),
            title: "Question".into(),
            subtitle: String::new(),
            body: String::new(),
            icon: String::new(),
            grant_label: grant.into(),
            deny_label: "Deny".into(),
            open_label: open.into(),
            modal: true,
            choices: Vec::new(),
            style: QuestionStyle::default(),
        }
    }

    fn center(r: Rect) -> (f32, f32) {
        (r.center_x(), r.center_y())
    }

    #[test]
    fn access_dialog_has_grant_and_deny_only() {
        let layout = dialog_layout(&view("Allow", ""), 0);
        let grant = layout.grant_rect.expect("grant shown");
        assert!(layout.open_rect.is_none());
        assert!(grant.left > layout.deny_rect.right);
        let (x, y) = center(grant);
        assert_eq!(hit_test(&layout, x, y, 0.0), Some(DialogHit::Grant));
        let (x, y) = center(layout.deny_rect);
        assert_eq!(hit_test(&layout, x, y, 0.0), Some(DialogHit::Deny));
    }

    #[test]
    fn empty_grant_label_hides_grant() {
        let layout = dialog_layout(&view("", ""), 0);
        assert!(layout.grant_rect.is_none());
        assert!(layout.open_rect.is_none());
        // Deny spans the row on its own.
        assert_eq!(layout.deny_rect.width(), DIALOG_W - PAD * 2.0);
    }

    #[test]
    fn open_without_grant_takes_the_right_slot() {
        let layout = dialog_layout(&view("", "Open in Ask"), 0);
        assert!(layout.grant_rect.is_none());
        let open = layout.open_rect.expect("open shown");
        assert_eq!(open.top, layout.deny_rect.top);
        assert!(open.left > layout.deny_rect.right);
        let (x, y) = center(open);
        assert_eq!(hit_test(&layout, x, y, 0.0), Some(DialogHit::Open));
    }

    #[test]
    fn open_with_grant_gets_its_own_row_below() {
        let two = dialog_layout(&view("Allow", ""), 0);
        let three = dialog_layout(&view("Allow", "Open in Ask"), 0);
        let grant = three.grant_rect.expect("grant shown");
        let open = three.open_rect.expect("open shown");
        assert!(open.top >= grant.bottom);
        assert!(open.top >= three.deny_rect.bottom);
        assert!(three.height > two.height);
        let (x, y) = center(open);
        assert_eq!(hit_test(&three, x, y, 0.0), Some(DialogHit::Open));
        let (x, y) = center(grant);
        assert_eq!(hit_test(&three, x, y, 0.0), Some(DialogHit::Grant));
    }

    fn group(id: &str, label: &str, options: &[&str]) -> ChoiceGroup {
        ChoiceGroup {
            id: id.into(),
            label: label.into(),
            options: options
                .iter()
                .enumerate()
                .map(|(i, label)| ChoiceOption {
                    id: format!("{id}-{i}"),
                    label: (*label).into(),
                    icon: String::new(),
                })
                .collect(),
            ..ChoiceGroup::default()
        }
    }

    const LONG: &str = "Should the migration keep the legacy session table around \
        until every client has upgraded, or drop it in this release and accept \
        that clients older than three months will need to log in again?";

    fn words(text: &str) -> Vec<&str> {
        text.split_whitespace().collect()
    }

    #[test]
    fn long_text_wraps_instead_of_being_cut() {
        let short = dialog_layout(&view("Answer", ""), 0);
        let mut long = view("Answer", "");
        long.subtitle = LONG.into();
        long.body = "in ~/dev/otto\nsecond line".into();
        let layout = dialog_layout(&long, 0);

        let subtitle = layout.subtitle.as_ref().expect("subtitle laid out");
        assert!(subtitle.lines.len() > 1, "a long question wraps");
        assert_eq!(words(&subtitle.lines.join(" ")), words(LONG));
        let body = layout.body.as_ref().expect("body laid out");
        assert_eq!(body.lines, ["in ~/dev/otto", "second line"]);
        assert!(layout.height > short.height);
        assert_eq!(layout.max_scroll(), 0.0);
    }

    #[test]
    fn group_labels_and_option_descriptions_are_laid_out_in_full() {
        let mut v = view("Answer", "Open in Ask");
        v.choices = vec![group(
            "question_0",
            LONG,
            &[
                "Keep it\nClients keep working; the table is dropped next release",
                "Drop it",
            ],
        )];
        let layout = dialog_layout(&v, 0);
        assert_eq!(words(&layout.group_labels[0].lines.join(" ")), words(LONG));

        let (label, desc) = &layout.option_text[0];
        assert_eq!(label, &["Keep it"]);
        assert_eq!(
            words(&desc.join(" ")),
            words("Clients keep working; the table is dropped next release")
        );
        let with_desc = layout.option_rects[0].2;
        let plain = layout.option_rects[1].2;
        assert!(with_desc.height() > plain.height());
        assert_eq!(plain.height(), OPTION_H);
        assert!(plain.top >= with_desc.bottom);
        // The group's label sits above its first option.
        assert!(layout.group_labels[0].y < with_desc.top);
    }

    #[test]
    fn options_split_label_and_description_at_the_first_line_break() {
        assert_eq!(split_option_label("Postgres"), ("Postgres", ""));
        assert_eq!(
            split_option_label("Postgres\nRelational\nand old"),
            ("Postgres", "Relational\nand old")
        );
    }

    #[test]
    fn a_tall_dialog_clamps_and_scrolls_under_fixed_buttons() {
        let mut v = view("Answer", "Open in Ask");
        v.subtitle = LONG.into();
        v.choices = (0..4)
            .map(|i| {
                group(
                    &format!("question_{i}"),
                    LONG,
                    &["One\nThe first option", "Two\nThe second", "Three", "Four"],
                )
            })
            .collect();
        let layout = dialog_layout(&v, 0);
        assert_eq!(layout.height, DIALOG_MAX_H);
        assert!(layout.max_scroll() > 0.0);
        assert_eq!(layout.option_rects.len(), 16);

        // The buttons stay inside the panel, below the scrolling content.
        let grant = layout.grant_rect.expect("grant shown");
        let open = layout.open_rect.expect("open shown");
        assert!(grant.top >= layout.viewport.bottom);
        assert!(open.bottom <= layout.height);
        let (x, y) = center(grant);
        assert_eq!(
            hit_test(&layout, x, y, layout.max_scroll()),
            Some(DialogHit::Grant)
        );

        // The last option is out of view until scrolled to.
        let (_, _, last) = *layout.option_rects.last().unwrap();
        let x = last.center_x();
        let at_bottom = last.center_y() - layout.max_scroll();
        assert!(last.center_y() > layout.viewport.bottom);
        assert_eq!(
            hit_test(&layout, x, at_bottom, layout.max_scroll()),
            Some(DialogHit::Option {
                group: 3,
                option: 3
            })
        );
        // A row scrolled out of view can't be clicked.
        let (_, _, first) = layout.option_rects[0];
        let under = first.center_y() - layout.max_scroll();
        assert_ne!(
            hit_test(&layout, first.center_x(), under, layout.max_scroll()),
            Some(DialogHit::Option {
                group: 0,
                option: 0
            })
        );
    }

    #[test]
    fn a_modal_dialog_never_shrinks() {
        let now = Instant::now();
        let mut p = Presence::new(true, now);
        assert_eq!(p.deadline(), None);
        assert!(!p.tick(now + Duration::from_secs(3600)));
        assert!(!p.focus_lost());
        assert!(!p.collapsed());
    }

    #[test]
    fn an_ignored_dialog_shrinks_after_its_read_window() {
        let now = Instant::now();
        let read = Duration::from_secs(DIALOG_READ_SECS);
        let mut p = Presence::new(false, now);
        assert_eq!(p.deadline(), Some(now + read));
        assert!(!p.tick(now + read / 2));
        // The pointer resting on it holds it open.
        p.pointer_over(now + read / 2);
        assert!(!p.tick(now + read));
        assert!(p.tick(now + read + read / 2));
        assert!(p.collapsed());
        assert_eq!(p.deadline(), None);

        // Clicking the circle opens it, holding the keyboard: only a focus
        // loss shrinks it again, not time.
        p.expand();
        assert!(!p.collapsed());
        assert!(!p.tick(now + read * 10));
        assert!(p.focus_lost());
        assert!(p.collapsed());
        assert!(!p.focus_lost(), "already shrunk");
    }

    #[test]
    fn a_clicked_dialog_shrinks_when_focus_moves_away() {
        let now = Instant::now();
        let mut p = Presence::new(false, now);
        p.focus_gained();
        assert!(!p.tick(now + Duration::from_secs(DIALOG_READ_SECS * 2)));
        assert!(p.focus_lost());
        assert!(p.collapsed());
    }

    #[test]
    fn a_shrunk_dialog_peeks_while_hovered() {
        let now = Instant::now();
        let mut p = Presence::new(false, now);
        assert_eq!(p.shape(true), Shape::Panel, "the open panel never peeks");
        assert!(p.focus_lost());
        assert_eq!(p.shape(false), Shape::Circle);
        assert_eq!(p.shape(true), Shape::Peek);
        // Hovering is not opening: still shrunk, still pending.
        assert!(p.collapsed());
        p.expand();
        assert_eq!(p.shape(true), Shape::Panel);
    }

    #[test]
    fn tab_walks_rows_then_buttons_and_wraps() {
        let mut v = view("Answer", "Open in Ask");
        v.choices = vec![group("a", "First?", &["One", "Two"])];
        let layout = dialog_layout(&v, 0);
        let sel = [1];
        let row = |r| Some(KeyboardTarget::Row(r));
        let button = |b| KeyboardTarget::Button(b);
        // The options are one stop: Tab leaves them for the buttons.
        assert_eq!(
            tab_step(&layout, &sel, row(0), false),
            button(DialogButton::Deny)
        );
        assert_eq!(
            tab_step(&layout, &sel, Some(button(DialogButton::Deny)), false),
            button(DialogButton::Grant)
        );
        assert_eq!(
            tab_step(&layout, &sel, Some(button(DialogButton::Grant)), false),
            button(DialogButton::Open)
        );
        // Coming back lands on the selected option.
        assert_eq!(
            tab_step(&layout, &sel, Some(button(DialogButton::Open)), false),
            KeyboardTarget::Row(1)
        );
        assert_eq!(
            tab_step(&layout, &sel, row(1), true),
            button(DialogButton::Open)
        );

        // Two questions are two stops.
        let mut two = view("Answer", "");
        two.choices = vec![
            group("a", "First?", &["One", "Two"]),
            group("b", "Second?", &["Three", "Four"]),
        ];
        let two = dialog_layout(&two, 0);
        assert!(has_several_groups(&two));
        assert_eq!(
            tab_step(&two, &[0, 1], row(0), false),
            KeyboardTarget::Row(3)
        );

        // No grant button and no options: Tab only visits deny and open.
        let bare = dialog_layout(&view("", "Open in Ask"), 0);
        assert_eq!(buttons(&bare), vec![DialogButton::Deny, DialogButton::Open]);
        assert_eq!(
            tab_step(&bare, &[], None, false),
            button(DialogButton::Deny)
        );
    }

    #[test]
    fn arrow_keys_walk_every_option_row_and_stop_at_the_ends() {
        let mut v = view("Answer", "");
        v.choices = vec![
            group("a", "First?", &["One", "Two"]),
            group("b", "Second?", &["Three", "Four", "Five"]),
        ];
        let layout = dialog_layout(&v, 0);
        // Starts on the first group's selection.
        let start = keyboard_row(&layout, &[1, 0], None);
        assert_eq!(start, Some(1));
        // Arrows stay within a group: Tab is what moves to the next one.
        assert_eq!(step_row(&layout, start, 1), Some(1));
        assert_eq!(step_row(&layout, Some(2), 1), Some(3));
        assert_eq!(step_row(&layout, Some(2), -1), Some(2));
        assert_eq!(step_row(&layout, Some(4), 1), Some(4));
        assert_eq!(step_row(&layout, Some(0), -1), Some(0));
        assert_eq!(next_group_row(&layout, &[1, 2], 1), Some(4));
        assert_eq!(next_group_row(&layout, &[1, 2], 4), None);
        assert_eq!(row_of(&layout, 1, 2), Some(4));

        let empty = dialog_layout(&view("Allow", ""), 0);
        assert_eq!(keyboard_row(&empty, &[], None), None);
        assert_eq!(step_row(&empty, None, 1), None);
    }

    #[test]
    fn moving_the_keyboard_scrolls_its_row_into_view() {
        let mut v = view("Answer", "");
        v.subtitle = LONG.into();
        v.choices = (0..4)
            .map(|i| group(&format!("q{i}"), LONG, &["One", "Two", "Three", "Four"]))
            .collect();
        let layout = dialog_layout(&v, 0);
        assert!(layout.max_scroll() > 0.0);
        let last = layout.option_rects.len() - 1;
        let scroll = reveal(&layout, last, 0.0);
        let rect = layout.option_rects[last].2;
        assert!(rect.bottom - scroll <= layout.viewport.bottom);
        assert!(rect.top - scroll >= 0.0);
        // Back to the first row scrolls back up.
        let first = layout.option_rects[0].2;
        let up = reveal(&layout, 0, scroll);
        assert!(first.top - up >= 0.0);
        // A row already in view doesn't move anything.
        assert_eq!(reveal(&layout, 0, 0.0), 0.0);
    }

    fn multi_group(id: &str, label: &str, options: &[&str], picked: &[usize]) -> ChoiceGroup {
        ChoiceGroup {
            multi: true,
            picked: picked.to_vec(),
            ..group(id, label, options)
        }
    }

    fn questions(style: QuestionStyle, choices: Vec<ChoiceGroup>) -> DialogView {
        DialogView {
            choices,
            style,
            ..view("Answer", "Open in Ask")
        }
    }

    fn paged_style() -> QuestionStyle {
        QuestionStyle {
            paged: true,
            next_label: "Next".into(),
            back_label: "Back".into(),
            page_label: "{current} of {total}".into(),
            multi_hint: "Pick any that apply.".into(),
            ..QuestionStyle::default()
        }
    }

    #[test]
    fn several_questions_are_asked_a_page_each() {
        let v = questions(
            paged_style(),
            vec![
                group("a", "First?", &["One", "Two"]),
                group("b", "Second?", &["Three", "Four", "Five"]),
                multi_group("c", "Third?", &["Six", "Seven"], &[1]),
            ],
        );
        assert_eq!(v.pages(), 3);

        // Only the page's own question, and its own options.
        let first = dialog_layout(&v, 0);
        assert_eq!(first.group_labels[0].lines, ["First?"]);
        assert_eq!(first.option_rects.len(), 2);
        assert!(first.option_rects.iter().all(|(g, _, _)| *g == 0));
        // Before the last page the grant button moves on instead of answering.
        assert_eq!(first.grant_text, "Next");
        assert_eq!(first.page_counter.as_ref().unwrap().lines, ["1 of 3"]);
        assert!(first.back_rect.is_none(), "nothing to go back to");

        let second = dialog_layout(&v, 1);
        assert_eq!(second.option_rects.len(), 3);
        assert!(second.option_rects.iter().all(|(g, _, _)| *g == 1));
        let back = second.back_rect.expect("back button");
        assert_eq!(
            hit_test(&second, back.center_x(), back.center_y(), 0.0),
            Some(DialogHit::Back)
        );
        assert_eq!(second.page_counter.as_ref().unwrap().lines, ["2 of 3"]);

        let last = dialog_layout(&v, 2);
        assert_eq!(last.grant_text, "Answer");
        assert_eq!(last.group_hints[0].lines, ["Pick any that apply."]);
        // Past the end shows the last page rather than an empty one.
        assert_eq!(dialog_layout(&v, 9).page_counter.unwrap().lines, ["3 of 3"]);

        // Unpaged, the same questions are one long page, with no counter.
        let flat = questions(QuestionStyle::default(), v.choices.clone());
        assert_eq!(flat.pages(), 1);
        let layout = dialog_layout(&flat, 0);
        assert_eq!(layout.option_rects.len(), 7);
        assert!(layout.page_counter.is_none() && layout.back_rect.is_none());
        assert_eq!(layout.grant_text, "Answer");
    }

    #[test]
    fn multi_select_options_toggle_and_answer_together() {
        let choices = vec![
            group("a", "First?", &["One", "Two"]),
            multi_group("b", "Second?", &["Three", "Four", "Five"], &[2]),
        ];
        let mut picks = Picks::new(&choices);
        // The single-select group starts on its default, the multi-select one
        // with the options it was given picked.
        assert_eq!(picks.selected, [0, 0]);
        assert!(picks.is_chosen(&choices, 0, 0) && !picks.is_chosen(&choices, 0, 1));
        assert!(picks.is_chosen(&choices, 1, 2) && !picks.is_chosen(&choices, 1, 0));
        assert_eq!(
            picks.results(&choices),
            [("a".into(), "a-0".into()), ("b".into(), "b-2".into())]
        );

        // Choosing in a single-select group moves the pick; in a multi-select
        // one it flips that option and leaves the rest.
        picks.choose(&choices, 0, 1);
        picks.choose(&choices, 1, 0);
        assert_eq!(
            picks.results(&choices),
            [
                ("a".into(), "a-1".into()),
                ("b".into(), "b-0".into()),
                ("b".into(), "b-2".into())
            ]
        );
        // Flipping the last pick off answers "none of them" for that question.
        picks.choose(&choices, 1, 0);
        picks.choose(&choices, 1, 2);
        assert_eq!(picks.results(&choices), [("a".into(), "a-1".into())]);
        // The keyboard is still on the option it last touched.
        assert_eq!(picks.selected, [1, 2]);
    }

    #[test]
    fn progress_dots_stand_for_the_questions_and_go_back_to_them() {
        let style = QuestionStyle {
            handle_title: true,
            ..paged_style()
        };
        let v = questions(
            style,
            vec![
                group("a", "First?", &["One", "Two"]),
                group("b", "Second?", &["Three"]),
                group("c", "Third?", &["Four"]),
            ],
        );
        let second = dialog_layout(&v, 1);
        assert_eq!(second.dots.len(), 3, "one dot per question");
        assert_eq!(second.page, 1, "the second is the current one");
        // Centred as a row, in order, above the question.
        let row_mid = (second.dots[0].left + second.dots[2].right) / 2.0;
        assert!((row_mid - second.width / 2.0).abs() < 0.5);
        assert!(second.dots[0].right < second.dots[1].left);
        assert!(second.dots[2].bottom < second.group_labels[0].y);

        // Each dot is its page's click target, with room around it to aim at.
        for (page, dot) in second.dots.iter().enumerate() {
            assert_eq!(
                hit_test(&second, dot.center_x(), dot.center_y(), 0.0),
                Some(DialogHit::Page(page))
            );
            assert!(dot.width() < DOT_HIT);
        }

        // One question: nothing to show progress through.
        let one = questions(
            QuestionStyle {
                handle_title: true,
                ..QuestionStyle::default()
            },
            vec![group("a", "First?", &["One"])],
        );
        let layout = dialog_layout(&one, 0);
        assert!(layout.dots.is_empty() && layout.page_counter.is_none());
    }

    #[test]
    fn the_counter_sits_beside_the_buttons() {
        let v = questions(
            paged_style(),
            vec![
                group("a", "First?", &["One", "Two"]),
                group("b", "Second?", &["Three"]),
            ],
        );
        let layout = dialog_layout(&v, 0);
        let counter = layout.page_counter.as_ref().expect("counter");
        assert_eq!(counter.lines, ["1 of 2"]);
        // On the button row, at its left, with the buttons making room.
        assert!(counter.y >= layout.viewport.bottom);
        assert!(counter.y < layout.deny_rect.bottom);
        assert!(layout.deny_rect.left >= PAD + COUNTER_W);
        let grant = layout.grant_rect.expect("grant");
        assert!(grant.right <= layout.width - PAD + 0.01);
        // The open button keeps the full width under them.
        assert_eq!(layout.open_rect.expect("open").left, PAD);

        // Without paging the buttons span the panel as before.
        let one = questions(QuestionStyle::default(), v.choices.clone());
        let layout = dialog_layout(&one, 0);
        assert!(layout.page_counter.is_none());
        assert_eq!(layout.deny_rect.left, PAD);
    }

    #[test]
    fn a_handle_title_makes_the_question_the_biggest_text() {
        let choices = vec![group("a", "First?", &["One", "Two"])];
        let handle = questions(
            QuestionStyle {
                handle_title: true,
                ..QuestionStyle::default()
            },
            choices.clone(),
        );
        let headline = questions(QuestionStyle::default(), choices);
        let handle = dialog_layout(&handle, 0);
        let headline = dialog_layout(&headline, 0);
        // The handle is one small line; the question takes the room instead.
        assert!(handle.title.lines.len() == 1);
        let handle_question = handle.option_rects[0].2.top - handle.group_labels[0].y;
        let headline_question = headline.option_rects[0].2.top - headline.group_labels[0].y;
        assert!(
            handle_question > headline_question,
            "{handle_question} vs {headline_question}"
        );
    }

    /// Parses otto-agentsd's `dump_prompt_for_render` output into a view.
    fn view_from_dump(dump: &str) -> DialogView {
        let mut v = view("", "");
        v.modal = false;
        let unesc = |s: &str| s.replace("\\n", "\n").replace("\\\\", "\\");
        for line in dump.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["title", t] => v.title = unesc(t),
                ["subtitle", t] => v.subtitle = unesc(t),
                ["body", t] => v.body = unesc(t),
                ["icon", t] => v.icon = unesc(t),
                ["grant", t] => v.grant_label = unesc(t),
                ["handle", t] => v.style.handle_title = *t == "1",
                ["deny", t] => v.deny_label = unesc(t),
                ["open", t] => v.open_label = unesc(t),
                ["group", id, multi, label] => v.choices.push(ChoiceGroup {
                    id: unesc(id),
                    label: unesc(label),
                    multi: *multi == "1",
                    ..ChoiceGroup::default()
                }),
                ["label", key, value] => {
                    let value = unesc(value);
                    match *key {
                        "next" => v.style.next_label = value,
                        "back" => v.style.back_label = value,
                        "page" => v.style.page_label = value,
                        "multi-hint" => v.style.multi_hint = value,
                        "body-align" => v.style.body_start = value == "start",
                        _ => {}
                    }
                }
                ["option", id, label] => {
                    if let Some(group) = v.choices.last_mut() {
                        group.options.push(ChoiceOption {
                            id: unesc(id),
                            label: unesc(label),
                            icon: String::new(),
                        });
                    }
                }
                _ => {}
            }
        }
        // As `question_style` decides it on the bus: several questions are
        // asked a page each.
        v.style.paged = v.choices.len() > 1;
        v
    }

    /// Draws a dialog into a PNG at 2x over a flat panel colour.
    fn render_png(v: &DialogView, path: &str) {
        let mut picks = Picks::new(&v.choices);
        // Show a multi-select question with a pick made, as it looks in use.
        if let Some(g) = v.choices.iter().position(|g| g.multi) {
            picks.choose(&v.choices, g, 1);
        }
        let layouts: Vec<DialogLayout> = (0..v.pages()).map(|p| dialog_layout(v, p)).collect();
        let scale = 2.0;
        let gap = 16.0;
        let total_w: f32 = layouts.iter().map(|l| l.width + gap).sum();
        let max_h = layouts.iter().map(|l| l.height).fold(0.0, f32::max);
        let (w, h) = (
            (total_w * scale).ceil() as i32,
            (max_h * scale).ceil() as i32,
        );
        let mut surface = skia_safe::surfaces::raster_n32_premul((w, h)).expect("surface");
        let canvas = surface.canvas();
        canvas.clear(Color::from_rgb(0x60, 0x68, 0x70));
        canvas.scale((scale, scale));
        let material = opaque(otto_kit::AppContext::current_theme().material_medium);
        for layout in &layouts {
            let mut bg = Paint::default();
            bg.set_color(material);
            bg.set_anti_alias(true);
            canvas.draw_rrect(
                RRect::new_rect_xy(
                    Rect::from_wh(layout.width, layout.height),
                    PANEL_RADIUS,
                    PANEL_RADIUS,
                ),
                &bg,
            );
            draw_dialog(canvas, v, &picks, layout, 0.0, None);
            canvas.translate((layout.width + gap, 0.0));
        }
        let image = surface.image_snapshot();
        let data = image
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .expect("png");
        std::fs::write(path, data.as_bytes()).expect("write png");
    }

    /// Renders `/tmp/claude-1000/shots/prompt.txt` (see otto-agentsd's
    /// `dump_prompt_for_render`) to PNGs next to it, for looking at.
    #[test]
    #[ignore = "offscreen render for inspection"]
    fn render_prompt_dump() {
        let dir = std::env::var("OTTO_SHOTS").unwrap_or_else(|_| "/tmp/claude-1000/shots".into());
        let tag = std::env::var("OTTO_SHOT_TAG").unwrap_or_else(|_| "dialog".into());
        let dump = std::fs::read_to_string(format!("{dir}/prompt.txt")).expect("prompt dump");
        let v = view_from_dump(&dump);
        render_png(&v, &format!("{dir}/{tag}.png"));

        // The same set asking only its first question: no dots, no counter.
        let mut one = v.clone();
        one.choices.truncate(1);
        one.style.paged = false;
        render_png(&one, &format!("{dir}/{tag}-single.png"));

        // A permission dialog, which keeps its headline title and big icon.
        let access = DialogView {
            title: "Claude wants to run a command".into(),
            subtitle: "cargo test --all".into(),
            body: "in ~/dev/otto".into(),
            icon: "system-run".into(),
            grant_label: "Allow".into(),
            deny_label: "Reject".into(),
            open_label: String::new(),
            style: QuestionStyle::default(),
            choices: Vec::new(),
            ..v.clone()
        };
        render_png(&access, &format!("{dir}/{tag}-access.png"));
    }
}
