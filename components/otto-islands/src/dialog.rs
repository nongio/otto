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
/// Space kept clear at the right edge of an option row for the checkmark.
/// Reserved on unselected rows too, so labels don't reflow as selection moves.
const CHECK_GUTTER: f32 = 30.0;
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

/// A single-select group of options (e.g. "output" → list of connectors).
#[derive(Clone, Debug)]
pub struct ChoiceGroup {
    pub id: String,
    pub label: String,
    pub options: Vec<ChoiceOption>,
    /// Index of the initially-selected option.
    pub default: usize,
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
    Option { group: usize, option: usize },
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
/// The tallest a panel gets. Past it the text and choices scroll under a
/// fixed row of buttons, which always stay on screen.
pub const DIALOG_MAX_H: f32 = 520.0;

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
    /// coordinates.
    pub option_rects: Vec<(usize, usize, Rect)>,
    // Internal draw anchors (content coords).
    icon_present: bool,
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

/// Compute the panel layout for the given request view.
pub fn dialog_layout(view: &DialogView) -> DialogLayout {
    let w = DIALOG_W;
    let text_max_w = w - PAD * 2.0;
    let mut y = PAD;

    let icon_present = !view.icon.is_empty();
    if icon_present {
        y += ICON + ICON_GAP;
    }

    let title_lines = wrap(
        &view.title,
        &font(TITLE_SIZE, 700),
        text_max_w,
        TITLE_MAX_LINES,
    );
    let title = TextBlock {
        y,
        lines: title_lines,
    };
    y += TITLE_LINE_H * title.lines.len().max(1) as f32 + TITLE_GAP;

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
    let label_font = font(OPTION_LABEL_SIZE, 500);
    let desc_font = font(OPTION_DESC_SIZE, 400);
    for (gi, group) in view.choices.iter().enumerate() {
        if group.options.is_empty() {
            continue;
        }
        let lines = wrap(
            &group.label,
            &font(GROUP_LABEL_SIZE, 600),
            text_max_w,
            GROUP_LABEL_MAX_LINES,
        );
        if !lines.is_empty() {
            let h = TEXT_LINE_H * lines.len() as f32;
            group_labels.push(TextBlock { y, lines });
            y += h + GROUP_LABEL_GAP;
        }
        for (oi, opt) in group.options.iter().enumerate() {
            let (label, description) = split_option_label(&opt.label);
            let text_x = option_text_x(!opt.icon.is_empty());
            let col_w = (w - PAD - CHECK_GUTTER) - (PAD + text_x);
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
    let has_grant = !view.grant_label.is_empty();
    let has_open = !view.open_label.is_empty();
    let mut footer_h = 2.0 + BTN_H + PAD;
    if has_grant && has_open {
        footer_h += OPEN_ROW_GAP + OPEN_BTN_H;
    }
    let height = (content_h + footer_h).min(DIALOG_MAX_H);
    let viewport = Rect::from_xywh(0.0, 0.0, w, height - footer_h);

    let mut y = viewport.bottom + 2.0;
    let full_w = w - PAD * 2.0;
    let half_w = (full_w - BTN_GAP) / 2.0;
    let (deny_rect, grant_rect, mut open_rect) = if has_grant || has_open {
        let deny = Rect::from_xywh(PAD, y, half_w, BTN_H);
        let right = Rect::from_xywh(PAD + half_w + BTN_GAP, y, half_w, BTN_H);
        if has_grant {
            (deny, Some(right), None)
        } else {
            (deny, None, Some(right))
        }
    } else {
        (Rect::from_xywh(PAD, y, full_w, BTN_H), None, None)
    };
    y += BTN_H;
    if has_grant && has_open {
        y += OPEN_ROW_GAP;
        open_rect = Some(Rect::from_xywh(PAD, y, full_w, OPEN_BTN_H));
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
        icon_present,
        title,
        subtitle,
        body,
        group_labels,
        option_text,
    }
}

/// Where an option's text starts, relative to its row's left edge.
fn option_text_x(has_icon: bool) -> f32 {
    if has_icon {
        10.0 + OPTION_ICON + 10.0
    } else {
        14.0
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

/// The row `delta` steps from `current` (Up is `-1`, Down `+1`), stopping at
/// the first and last rows. Rows run through every group in order.
pub fn step_row(layout: &DialogLayout, current: Option<usize>, delta: i32) -> Option<usize> {
    let last = layout.option_rects.len().checked_sub(1)?;
    let Some(current) = current else {
        return Some(0);
    };
    Some((current as i64 + delta as i64).clamp(0, last as i64) as usize)
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
/// far the content is scrolled. `focus_row`, when the panel holds the
/// keyboard, is the option row that gets the focus ring.
pub fn draw_dialog(
    canvas: &Canvas,
    view: &DialogView,
    selected: &[usize],
    layout: &DialogLayout,
    scroll: f32,
    focus_row: Option<usize>,
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
    if let Some(block) = &layout.subtitle {
        let f = font(TEXT_SIZE, 500);
        draw_lines_centered(canvas, &block.lines, cx, block.y, TEXT_LINE_H, &f, dim);
    }
    if let Some(block) = &layout.body {
        let f = font(TEXT_SIZE, 400);
        draw_lines_centered(canvas, &block.lines, cx, block.y, TEXT_LINE_H, &f, dim2);
    }

    // Group labels: the question each group answers, in full.
    let group_font = font(GROUP_LABEL_SIZE, 600);
    for block in &layout.group_labels {
        draw_lines(
            canvas,
            &block.lines,
            PAD,
            block.y,
            TEXT_LINE_H,
            &group_font,
            dim,
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
        let is_selected = selected.get(*gi).copied().unwrap_or(group.default) == *oi;

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
        if focus_row == Some(row) {
            let mut ring = Paint::default();
            ring.set_anti_alias(true);
            ring.set_color(accent);
            ring.set_style(skia_safe::paint::Style::Stroke);
            ring.set_stroke_width(2.0);
            let outer = rect.with_outset((FOCUS_RING_OUTSET, FOCUS_RING_OUTSET));
            let r = OPTION_RADIUS + FOCUS_RING_OUTSET;
            canvas.draw_rrect(RRect::new_rect_xy(outer, r, r), &ring);
        }

        if !opt.icon.is_empty() {
            draw_icon(
                canvas,
                &opt.icon,
                rect.left + 10.0,
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

        // Checkmark on the selected row.
        if is_selected {
            let mut ck = Paint::default();
            ck.set_anti_alias(true);
            ck.set_color(on_accent);
            ck.set_style(skia_safe::paint::Style::Stroke);
            ck.set_stroke_width(2.0);
            ck.set_stroke_cap(skia_safe::paint::Cap::Round);
            let mx = rect.right - 22.0;
            let my = rect.center_y();
            let mut p = skia_safe::PathBuilder::new();
            p.move_to((mx - 5.0, my));
            p.line_to((mx - 1.5, my + 4.0));
            p.line_to((mx + 5.0, my - 5.0));
            canvas.draw_path(&p.detach(), &ck);
        }
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
        draw_button(canvas, rect, &view.grant_label, accent, on_accent);
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

    canvas.restore();
}

fn draw_button(canvas: &Canvas, rect: &Rect, label: &str, bg: Color, text: Color) {
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
        rect.center_y() + 4.5,
        &font(13.0, 600),
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
        }
    }

    fn center(r: Rect) -> (f32, f32) {
        (r.center_x(), r.center_y())
    }

    #[test]
    fn access_dialog_has_grant_and_deny_only() {
        let layout = dialog_layout(&view("Allow", ""));
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
        let layout = dialog_layout(&view("", ""));
        assert!(layout.grant_rect.is_none());
        assert!(layout.open_rect.is_none());
        // Deny spans the row on its own.
        assert_eq!(layout.deny_rect.width(), DIALOG_W - PAD * 2.0);
    }

    #[test]
    fn open_without_grant_takes_the_right_slot() {
        let layout = dialog_layout(&view("", "Open in Ask"));
        assert!(layout.grant_rect.is_none());
        let open = layout.open_rect.expect("open shown");
        assert_eq!(open.top, layout.deny_rect.top);
        assert!(open.left > layout.deny_rect.right);
        let (x, y) = center(open);
        assert_eq!(hit_test(&layout, x, y, 0.0), Some(DialogHit::Open));
    }

    #[test]
    fn open_with_grant_gets_its_own_row_below() {
        let two = dialog_layout(&view("Allow", ""));
        let three = dialog_layout(&view("Allow", "Open in Ask"));
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
            default: 0,
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
        let short = dialog_layout(&view("Answer", ""));
        let mut long = view("Answer", "");
        long.subtitle = LONG.into();
        long.body = "in ~/dev/otto\nsecond line".into();
        let layout = dialog_layout(&long);

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
        let layout = dialog_layout(&v);
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
        let layout = dialog_layout(&v);
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
    fn arrow_keys_walk_every_option_row_and_stop_at_the_ends() {
        let mut v = view("Answer", "");
        v.choices = vec![
            group("a", "First?", &["One", "Two"]),
            group("b", "Second?", &["Three", "Four", "Five"]),
        ];
        let layout = dialog_layout(&v);
        // Starts on the first group's selection.
        let start = keyboard_row(&layout, &[1, 0], None);
        assert_eq!(start, Some(1));
        let down = step_row(&layout, start, 1);
        assert_eq!(down, Some(2));
        assert_eq!(layout.option_rects[2].0, 1, "into the second group");
        assert_eq!(step_row(&layout, Some(4), 1), Some(4));
        assert_eq!(step_row(&layout, Some(0), -1), Some(0));
        assert_eq!(row_of(&layout, 1, 2), Some(4));

        let empty = dialog_layout(&view("Allow", ""));
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
        let layout = dialog_layout(&v);
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
}
