//! The chat beside the document: ask an agent about the file and watch the
//! answer come in next to it. Plan 0016, milestone 3.
//!
//! The conversation and its log are otto-agents-kit's, the same the launcher
//! hosts: [`Ask`] talks to otto-agents and [`ChatView`] lays out and paints
//! the log. This is only the panel around them: where the log, the answers
//! and the field sit, a scroll for the log, and which keys and presses go
//! where. The file goes with the first message as its subject, unlisted
//! since it is open beside the chat, so the session that
//! starts with it is about the file. Opening the panel connects in the
//! background; nothing reaches an agent until something is sent.

// Rust guideline compliant 2026-02-21

use std::os::fd::RawFd;
use std::path::PathBuf;

use otto_agents_kit::chat::Ask;
use otto_agents_kit::keys::{self, FieldEdit};
use otto_agents_kit::log::paint::INSET;
use otto_agents_kit::log::{ChatView, Key as ChatKey, Keyed, Pressed, Released};
use otto_kit::components::scroll::ScrollContent;
use otto_kit::components::text_input::{TextInput, TextInputStyle};
use otto_kit::prelude::*;
use otto_kit::skia::{Contains, Point, RRect};
use otto_kit::CursorShape;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

/// The panel's width, in points.
pub const WIDTH: f32 = 360.0;
/// The field's height.
const FIELD_H: f32 = 34.0;
/// Around the field and the answers.
const PAD: f32 = 10.0;
/// One answer to a question.
const ROW_H: f32 = 30.0;
/// The line saying the agents can't be reached, under the log.
const UNREACHABLE_H: f32 = 36.0;
/// What one notch of a wheel scrolls the log by.
const NOTCH: f32 = 48.0;
/// The client name otto-agents shows for sessions started here.
const CLIENT: &str = "otto-preview";

/// The chat panel's state.
pub struct Chat {
    /// The file the chat is about, sent with the first message.
    path: PathBuf,
    /// Made when the panel first opens, and kept while the window lives, so
    /// closing the panel keeps the conversation.
    ask: Option<Ask>,
    view: ChatView,
    field: TextInput,
    /// Whether keys go to the field rather than the document.
    pub focused: bool,
    /// How far down the log is scrolled, in points.
    scroll: f32,
    /// Whether the log keeps its last line in view as the answer grows.
    follow: bool,
    /// The answer highlighted while a question waits.
    selected: usize,
    /// The answer pressed, until the release says whether it was a click.
    pressed_answer: Option<usize>,
    /// The log's height when last drawn, for paging and following.
    log_h: f32,
}

/// Where the panel's parts sit, for one panel box.
struct Layout {
    log: Rect,
    answers: Rect,
    field: Rect,
}

impl Layout {
    fn new(panel: Rect, answers: usize) -> Self {
        let field = Rect::from_ltrb(
            panel.left + PAD,
            panel.bottom - PAD - FIELD_H,
            panel.right - PAD,
            panel.bottom - PAD,
        );
        let rows_h = answers as f32 * ROW_H;
        let gap = if answers > 0 { PAD } else { 0.0 };
        let answers = Rect::from_ltrb(
            panel.left + PAD,
            field.top - PAD - rows_h - gap,
            panel.right - PAD,
            field.top - PAD,
        );
        let log = Rect::from_ltrb(panel.left + 1.0, panel.top, panel.right, answers.top);
        Self {
            log,
            answers,
            field,
        }
    }

    fn answer_at(&self, at: Point, count: usize) -> Option<usize> {
        if !self.answers.contains(at) {
            return None;
        }
        let index = ((at.y - self.answers.top - PAD) / ROW_H).floor();
        (index >= 0.0 && (index as usize) < count).then_some(index as usize)
    }
}

impl Chat {
    pub fn new(path: PathBuf, dark: bool) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut field = TextInput::editing("", field_style(dark));
        field.state.placeholder = otto_kit::t_owned!("preview-chat-placeholder", name = name);
        Self {
            path,
            ask: None,
            view: ChatView::new(dark),
            field,
            focused: false,
            scroll: 0.0,
            follow: true,
            selected: 0,
            pressed_answer: None,
            log_h: 0.0,
        }
    }

    /// The panel is opening: connect, the first time, with the file ready
    /// to go with the first message.
    pub fn opened(&mut self) {
        self.focused = true;
        // Connect the first time, and again when the service couldn't be
        // reached before anything was asked: it may be up now.
        let stale = self
            .ask
            .as_ref()
            .is_some_and(|ask| ask.unreachable().is_some() && !ask.running());
        if self.ask.is_none() || stale {
            let mut ask = Ask::open(CLIENT);
            ask.set_subject([self.path.clone()]);
            self.ask = Some(ask);
            self.relayout();
        }
    }

    pub fn set_dark(&mut self, dark: bool) {
        let placeholder = std::mem::take(&mut self.field.state.placeholder);
        self.field.style = field_style(dark);
        self.field.state.placeholder = placeholder;
        self.view.set_dark(dark);
        self.relayout();
    }

    /// The descriptor that wakes the loop when the service has news.
    pub fn poll_fd(&self) -> Option<RawFd> {
        self.ask.as_ref().map(Ask::poll_fd)
    }

    /// Take in what the service sent. Returns whether to repaint.
    pub fn pump(&mut self) -> bool {
        let changed = self.ask.as_mut().is_some_and(Ask::pump);
        if changed {
            self.relayout();
            self.selected = self.selected.min(self.answers().len().saturating_sub(1));
        }
        changed
    }

    /// Keep the caret blinking. Returns whether it flipped.
    pub fn tick(&mut self, delta: f32) -> bool {
        if !self.focused {
            return false;
        }
        let shown = self.field.caret_visible();
        self.field.tick(delta);
        shown != self.field.caret_visible()
    }

    fn relayout(&mut self) {
        if let Some(ask) = &self.ask {
            self.view.lay_out(ask, WIDTH - 2.0 * INSET);
        }
        if self.follow {
            self.scroll = self.max_scroll();
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.view.length() - self.log_h).max(0.0)
    }

    fn scroll_by(&mut self, dy: f32) -> bool {
        let before = self.scroll;
        self.scroll = (self.scroll + dy).clamp(0.0, self.max_scroll());
        self.follow = self.scroll >= self.max_scroll() - 1.0;
        before != self.scroll
    }

    /// The answers a waiting question or input request offers, in order.
    fn answers(&self) -> Vec<String> {
        let Some(ask) = &self.ask else {
            return Vec::new();
        };
        if let Some(question) = ask.question() {
            return question.choices.iter().map(|c| c.label.clone()).collect();
        }
        if ask.input().is_some() {
            return ask.input_rows(0).into_iter().map(|row| row.title).collect();
        }
        Vec::new()
    }

    /// Give answer `index` to whatever is waiting.
    fn answer(&mut self, index: usize) {
        let Some(ask) = &mut self.ask else {
            return;
        };
        if ask.question().is_some() {
            ask.answer(index);
        } else if ask.input().is_some() {
            let typed = self.field.value().to_string();
            if ask.choose_input(index, &typed).took_text {
                self.field.set_value("");
            }
        }
        self.selected = 0;
        self.follow = true;
        self.relayout();
    }

    /// Send what is in the field.
    fn send(&mut self) {
        let prompt = self.field.value().trim().to_string();
        let Some(ask) = &mut self.ask else {
            return;
        };
        if prompt.is_empty() {
            return;
        }
        if ask.input().is_some() && ask.input_takes_text() {
            if ask.answer_input(&prompt).took_text {
                self.field.set_value("");
            }
        } else {
            ask.send(&prompt, None);
            self.field.set_value("");
        }
        self.follow = true;
        self.relayout();
    }

    /// A key while the panel has the keyboard. Returns whether to repaint;
    /// Escape gives the keyboard back to the document.
    pub fn key(&mut self, event: &KeyEvent, modifiers: Modifiers, serial: u32) -> bool {
        let answers = self.answers();
        let field_selected = self.field.state.has_selection();
        if let Some(ask) = &mut self.ask {
            if ask.running() {
                let key = ChatKey {
                    keysym: event.keysym,
                    shift: modifiers.shift,
                    stop: modifiers.ctrl && event.keysym == Keysym::c,
                    answering: !answers.is_empty(),
                    field_selected,
                    page: self.log_h,
                };
                match self.view.key(ask, key) {
                    Some(Keyed::Scroll(dy)) => return self.scroll_by(dy),
                    Some(Keyed::Done) => {
                        self.relayout();
                        return true;
                    }
                    None => {}
                }
            }
        }
        match event.keysym {
            Keysym::Up if !answers.is_empty() => {
                self.selected = self.selected.saturating_sub(1);
                return true;
            }
            Keysym::Down if !answers.is_empty() => {
                self.selected = (self.selected + 1).min(answers.len() - 1);
                return true;
            }
            Keysym::Return | Keysym::KP_Enter
                if !answers.is_empty()
                    && !self.ask.as_ref().is_some_and(|ask| {
                        ask.input_takes_text() && !self.field.value().is_empty()
                    }) =>
            {
                self.answer(self.selected);
                return true;
            }
            _ => {}
        }
        let control = keys::control_char(event);
        match keys::edit_field(&mut self.field, event, control, modifiers.shift, serial) {
            FieldEdit::Commit => {
                self.send();
                true
            }
            FieldEdit::Cancel => {
                self.focused = false;
                true
            }
            FieldEdit::Changed | FieldEdit::Moved => true,
            FieldEdit::None => false,
        }
    }

    /// The pointer moved over the panel, or left it (`None`).
    pub fn motion(&mut self, panel: Rect, at: Option<Point>) -> (bool, CursorShape) {
        let layout = Layout::new(panel, self.answers().len());
        if self.view.is_selecting() {
            if let Some(at) = at {
                let point = self.log_point_clamped(&layout, at);
                return (self.view.drag_to(point), CursorShape::Text);
            }
        }
        let point = at.and_then(|at| self.log_point(&layout, at));
        let repaint = self.view.hover(point);
        let cursor = match at {
            Some(at) if layout.field.contains(at) => CursorShape::Text,
            Some(_) if point.is_some() => self.view.cursor(point),
            _ => CursorShape::Default,
        };
        (repaint, cursor)
    }

    pub fn leave(&mut self) -> bool {
        self.view.leave()
    }

    /// A press on the panel: it takes the keyboard.
    pub fn press(&mut self, panel: Rect, at: Point, time: u32, serial: u32) -> bool {
        self.focused = true;
        let answers = self.answers().len();
        let layout = Layout::new(panel, answers);
        if let Some(index) = layout.answer_at(at, answers) {
            self.selected = index;
            self.pressed_answer = Some(index);
            return true;
        }
        let point = self.log_point(&layout, at);
        if (point.is_some() || self.view.has_selection())
            && self.view.press(point, (at.x, at.y), time, serial) == Pressed::Toggled
        {
            self.relayout();
        }
        true
    }

    pub fn release(&mut self, panel: Rect, at: Point) -> bool {
        let answers = self.answers().len();
        let layout = Layout::new(panel, answers);
        if let Some(index) = self.pressed_answer.take() {
            if layout.answer_at(at, answers) == Some(index) {
                self.answer(index);
            }
            return true;
        }
        let point = self.log_point(&layout, at);
        match self.view.release(point, (at.x, at.y)) {
            Released::Attachment(hit) => {
                if let Some(ask) = &mut self.ask {
                    if hit.remove {
                        ask.remove_attachment(hit.item);
                    } else {
                        ask.toggle_attachment(hit.item);
                    }
                }
                self.relayout();
                true
            }
            Released::Taken => true,
            Released::Missed => false,
        }
    }

    /// A wheel or two-finger scroll over the panel.
    pub fn wheel(&mut self, dy: f32, discrete: bool) -> bool {
        self.scroll_by(if discrete { dy.signum() * NOTCH } else { dy })
    }

    /// Copy the log's selection, for Ctrl+C with the document focused.
    pub fn copy(&self, serial: u32) -> bool {
        self.view.copy(serial)
    }

    fn log_point(&self, layout: &Layout, at: Point) -> Option<(f32, f32)> {
        layout
            .log
            .contains(at)
            .then_some((at.x - layout.log.left, at.y - layout.log.top + self.scroll))
    }

    fn log_point_clamped(&self, layout: &Layout, at: Point) -> (f32, f32) {
        let x = at.x.clamp(layout.log.left, layout.log.right);
        let y = at.y.clamp(layout.log.top, layout.log.bottom);
        (x - layout.log.left, y - layout.log.top + self.scroll)
    }

    /// Paint the panel into `panel`.
    pub fn draw(&mut self, canvas: &Canvas, panel: Rect, theme: &Theme) {
        let answers = self.answers();
        let layout = Layout::new(panel, answers.len());

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(theme.fill_tertiary);
        canvas.draw_rect(
            Rect::from_ltrb(panel.left, panel.top, panel.left + 1.0, panel.bottom),
            &paint,
        );

        // The log, scrolled, inside its box.
        self.log_h = layout.log.height();
        if self.follow {
            self.scroll = self.max_scroll();
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        let unreachable = self.ask.as_ref().and_then(Ask::unreachable);
        if self.view.is_empty() {
            self.draw_empty(canvas, layout.log, theme);
        } else {
            // Why nothing will be answered, under the log and over the field,
            // when the attachments waiting for a message already fill the log.
            if let Some(reason) = unreachable {
                Label::new(reason.to_string())
                    .with_style(styles::FOOTNOTE)
                    .with_color(theme.text_secondary)
                    .with_width(layout.log.width() - 2.0 * INSET)
                    .centered_on(
                        layout.log.left + INSET,
                        layout.log.bottom - UNREACHABLE_H / 2.0,
                    )
                    .render(canvas);
            }
            let mut log = layout.log;
            if unreachable.is_some() {
                log.bottom -= UNREACHABLE_H;
            }
            canvas.save();
            canvas.clip_rect(log, None, Some(true));
            canvas.translate((layout.log.left, layout.log.top - self.scroll));
            let band = Rect::from_xywh(0.0, self.scroll, layout.log.width(), layout.log.height());
            ScrollContent::paint(&self.view, canvas, band);
            canvas.restore();
        }

        for (index, label) in answers.iter().enumerate() {
            let row = Rect::from_xywh(
                layout.answers.left,
                layout.answers.top + PAD + index as f32 * ROW_H,
                layout.answers.width(),
                ROW_H,
            );
            if index == self.selected {
                paint.set_color(theme.fill_secondary);
                canvas.draw_rrect(RRect::new_rect_xy(row, 6.0, 6.0), &paint);
            }
            Label::new(label.clone())
                .with_style(styles::BODY)
                .with_color(theme.text_primary)
                .with_width(row.width() - 20.0)
                .centered_on(row.left + 10.0, row.center_y())
                .render(canvas);
        }

        paint.set_color(theme.fill_quaternary);
        canvas.draw_rrect(RRect::new_rect_xy(layout.field, 8.0, 8.0), &paint);
        self.field.state.set_focused(self.focused);
        canvas.save();
        canvas.translate((layout.field.left, layout.field.top));
        self.field
            .render_at(canvas, layout.field.width(), layout.field.height());
        canvas.restore();
    }

    /// What the log shows before anything is asked: what the panel is for,
    /// or why the agents can't be reached.
    fn draw_empty(&self, canvas: &Canvas, log: Rect, theme: &Theme) {
        let text = match self.ask.as_ref().and_then(Ask::unreachable) {
            Some(reason) => reason.to_string(),
            None => otto_kit::t_owned!("preview-chat-empty"),
        };
        Label::new(text)
            .with_style(styles::SUBHEADLINE)
            .with_color(theme.text_secondary)
            .with_width(log.width() - 2.0 * INSET)
            .with_align(otto_kit::TextAlign::Center)
            .centered_on(log.left + INSET, log.center_y())
            .render(canvas);
    }
}

/// The field: a plain, rounded box, smaller than the launcher's.
fn field_style(dark: bool) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(if dark { Theme::dark() } else { Theme::light() });
    style.background = Color::TRANSPARENT;
    style.focus_ring_width = 0.0;
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Rect = Rect {
        left: 600.0,
        top: 80.0,
        right: 960.0,
        bottom: 720.0,
    };

    #[test]
    fn the_field_sits_at_the_bottom_and_the_log_fills_the_rest() {
        let layout = Layout::new(PANEL, 0);
        assert_eq!(layout.field.bottom, PANEL.bottom - PAD);
        assert_eq!(layout.field.height(), FIELD_H);
        assert_eq!(layout.log.top, PANEL.top);
        assert!(layout.log.bottom <= layout.field.top);
    }

    #[test]
    fn answers_take_room_from_the_log_and_are_hit_by_row() {
        let without = Layout::new(PANEL, 0);
        let with = Layout::new(PANEL, 3);
        assert!(with.log.height() < without.log.height());
        let first = Point::new(with.answers.center_x(), with.answers.top + PAD + 1.0);
        let last = Point::new(with.answers.center_x(), with.answers.bottom - 1.0);
        assert_eq!(with.answer_at(first, 3), Some(0));
        assert_eq!(with.answer_at(last, 3), Some(2));
        assert_eq!(with.answer_at(Point::new(0.0, 0.0), 3), None);
    }
}
