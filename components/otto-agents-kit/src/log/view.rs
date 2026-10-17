//! The chat's log as a host shows it: laid out from an [`Ask`], painted into
//! a scroll pane, and answering the pointer and the keys that are the log's.
//!
//! Every host of the chat would otherwise write the same glue again: lay the
//! conversation out whenever it changes, keep what can be selected in step
//! with it, follow the pointer over code blocks, tool calls, attachments and
//! links, and turn presses into selections, copies and clicks. [`ChatView`]
//! is that glue. It is a [`ScrollContent`], so the host hands it to its own
//! [`ScrollPane`](otto_kit::components::scroll::ScrollPane), and it works in
//! the log's content coordinates: the host says where a point on its surface
//! falls in the log, because only the host knows where the pane sits and how
//! far it has scrolled.
//!
//! What stays with the host is what differs between hosts: the field, where
//! the pane sits, Return and Escape, and what a pending attachment's click
//! does to the stash it may belong to.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use otto_kit::components::scroll::ScrollContent;
use otto_kit::preview::document;
use otto_kit::CursorShape;
use skia_safe::{Canvas, Rect};
use smithay_client_toolkit::seat::keyboard::Keysym;

use crate::chat::{input, Ask, Note, Status, Step};
use crate::keys::copy_to_clipboard;
use crate::log::selection::{self, Caret, Selection, Span};
use crate::log::{lay_out, length, AttachmentHit, Block, Footer, Kind, Line, LogPainter, Style};

/// How far the pointer may move between a press and its release for the two
/// to be a click, in points.
const SLOP: f32 = 4.0;

/// How close together, in milliseconds, presses run into a double or triple
/// press.
const DOUBLE_PRESS_MS: u32 = 400;

/// What a press on the log came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressed {
    /// The log took it: a copy, a selection begun, an attachment held.
    Taken,
    /// A group of tool calls opened or closed: the log wants laying out again.
    Toggled,
    /// Nothing of the log's is there. Any selection has been put down, and
    /// the press is the host's: a handle to drag, say.
    Missed,
}

/// What a release on the log came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    /// The log took it: a selection finished, a link followed, a sent
    /// attachment opened.
    Taken,
    /// A click on an attachment going with the next request: the host strikes
    /// it out, or takes it off when `remove` says so, and lays the log out
    /// again.
    Attachment(AttachmentHit),
    /// The log had nothing in hand; the release is the host's.
    Missed,
}

/// A key the log answers while a conversation is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keyed {
    /// Scroll the log by this many points.
    Scroll(f32),
    /// The key did what it does: the turn was stopped, or the mode changed.
    Done,
}

/// A key, and what the host knows about the moment it came in.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub keysym: Keysym,
    /// Whether Shift is held.
    pub shift: bool,
    /// Whether this is the key that copies or stops: Ctrl+C, or Cmd+C.
    pub stop: bool,
    /// Whether the rows under the field are answers to walk through, which
    /// is what Up, Down and Shift+Tab are for while they are.
    pub answering: bool,
    /// Whether the field has text selected, which Ctrl+C copies instead.
    pub field_selected: bool,
    /// How tall the log's pane is: what Page Up and Page Down move.
    pub page: f32,
}

/// The log's state between the host's events.
pub struct ChatView {
    painter: LogPainter,
    /// The width the log was last laid out at.
    width: f32,
    /// The log, laid out.
    lines: Vec<Line>,
    /// The answer and its status as plain text, for assistive technologies.
    text: String,
    /// Moves whenever the log would paint differently.
    revision: u64,
    /// Every piece of text in the log, as painted, so it can be selected.
    spans: Vec<Span>,
    /// What is selected in the log, when anything is.
    selection: Option<Selection>,
    /// The selection as boxes to paint, kept beside it so a scroll or a
    /// repaint does not measure the text again.
    selection_rects: Vec<Rect>,
    /// A selection being dragged out: where the drag took hold of the text.
    selecting: Option<Caret>,
    /// The code block the pointer is over, as the log line holding its answer
    /// and the block within it, so its copy button is shown.
    code_hover: Option<(usize, document::CodeHit)>,
    /// The code block last copied, until the pointer leaves it, so its button
    /// can say so.
    code_copied: Option<(usize, usize)>,
    /// The group of tool calls the pointer is over, as its log line, so it
    /// can say it is something to click.
    steps_hover: Option<usize>,
    /// The requests whose tool calls have been opened, by their place in the
    /// transcript. Everything else shows its last call and an ellipsis.
    steps_open: HashSet<usize>,
    /// The attachment going with the next request that the pointer is over,
    /// highlighted as something to click.
    attachment_hover: Option<AttachmentHit>,
    /// One pressed, and where: a release in the same spot strikes it out,
    /// or takes it off when on its remove button.
    attachment_press: Option<(AttachmentHit, f32, f32)>,
    /// A link pressed in the log, and where it was pressed, so a release that
    /// did not turn into a drag opens it.
    link_press: Option<(Arc<str>, f32, f32)>,
    /// The last press in the log — when, where, and how many presses have run
    /// together — so a second picks a word and a third the line.
    last_press: Option<(u32, f32, f32, u32)>,
}

impl ChatView {
    pub fn new(dark: bool) -> Self {
        Self {
            painter: LogPainter::new(dark),
            width: 0.0,
            lines: Vec::new(),
            text: String::new(),
            revision: 0,
            spans: Vec::new(),
            selection: None,
            selection_rects: Vec::new(),
            selecting: None,
            code_hover: None,
            code_copied: None,
            steps_hover: None,
            steps_open: HashSet::new(),
            attachment_hover: None,
            attachment_press: None,
            link_press: None,
            last_press: None,
        }
    }

    /// Switch the colour scheme.
    pub fn set_dark(&mut self, dark: bool) {
        self.painter.set_dark(dark);
    }

    /// The painter, for the thumbnails its attachments want.
    pub fn painter(&self) -> &LogPainter {
        &self.painter
    }

    /// The log, laid out.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// How tall the log is.
    pub fn length(&self) -> f32 {
        length(&self.lines)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The log as plain text, for a screen reader.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Moves whenever the log would paint differently; see
    /// [`ScrollContent::revision`].
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Lay the log out again from `ask`'s conversation, `width` wide. Says
    /// whether there was a conversation to lay out; with none, the log is
    /// emptied. Either way [`Self::revision`] moves if the log changed.
    pub fn lay_out(&mut self, ask: &Ask, width: f32) -> bool {
        // What goes with the next request shows before anything is asked.
        let pending = ask.pending();
        let transcript = match ask.transcript() {
            Some(transcript) => transcript,
            None if !pending.is_empty() => Default::default(),
            // Nothing asked and nothing left to send: the log goes, rather
            // than keep showing what was just taken off.
            None => {
                if !self.lines.is_empty() {
                    self.lines.clear();
                    self.text.clear();
                    self.spans.clear();
                    self.selection = None;
                    self.selection_rects.clear();
                    self.touch();
                }
                return false;
            }
        };
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
        let inputs: Vec<Vec<Vec<(String, Style)>>> = transcript
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
            .enumerate()
            .zip(&notes)
            .zip(&steps)
            .zip(&inputs)
            .map(|((((index, entry), note), steps), inputs)| Block {
                prompt: &entry.prompt,
                attachments: &entry.attachments,
                answer: &entry.answer,
                steps,
                steps_expanded: self.steps_open.contains(&index),
                inputs,
                question: entry
                    .question
                    .as_ref()
                    .map(|question| (question.title.as_str(), question.detail.as_str())),
                action: entry
                    .question
                    .as_ref()
                    .map(|question| question.action.as_slice())
                    .unwrap_or(&[]),
                note: note.as_deref(),
            })
            .collect();
        // The status closes the log; the agent and its mode sit under it.
        let status = transcript.status.as_ref().map(Status::text);
        let mode = ask.mode_line();
        let footer = mode.as_ref().map(|mode| Footer {
            agent: &mode.agent,
            mode: &mode.mode,
            hint: mode.hint.as_deref(),
        });
        let painter = &self.painter;
        self.lines = lay_out(
            &blocks,
            status.as_deref(),
            &pending,
            footer,
            width,
            |text, style| painter.measure(text, style),
            |path| painter.picture_size(path),
            |items, pending| painter.attachments_height(items, pending, width),
        );
        self.width = width;
        self.text = self
            .lines
            .iter()
            .map(Line::text)
            .collect::<Vec<_>>()
            .join("\n");
        // What can be selected, rebuilt with the lines it belongs to. A
        // selection whose text has since been laid out differently — the log
        // was cleared, or a request withdrawn — is dropped rather than left
        // highlighting whatever now sits at those coordinates.
        self.spans = self.painter.spans(&self.lines, width);
        match self.selection {
            Some(selection) if selection.fits(&self.spans) => {
                // The words may have been laid out somewhere else — the log
                // is a different width, or a line above re-wrapped — so the
                // highlight is measured again against where they are now.
                self.selection_rects = selection::rects(&self.spans, selection);
            }
            Some(_) => {
                self.selection = None;
                self.selection_rects.clear();
            }
            None => {}
        }
        self.touch();
        true
    }

    /// Let go of the conversation shown, for another one: its lines, and the
    /// tool calls opened in it, which are numbered by its requests.
    pub fn forget(&mut self) {
        self.lines.clear();
        self.text.clear();
        self.steps_open.clear();
        self.steps_hover = None;
        self.touch();
    }

    // === Selection ===

    /// Select `selection` in the log — or nothing, with `None`. Says whether
    /// that changed anything.
    pub fn set_selection(&mut self, selection: Option<Selection>) -> bool {
        let selection = selection.filter(|selection| !selection.is_empty());
        let same = match (self.selection, selection) {
            (Some(before), Some(now)) => before.range() == now.range(),
            (None, None) => true,
            _ => false,
        };
        if same {
            return false;
        }
        self.selection_rects = selection
            .map(|selection| selection::rects(&self.spans, selection))
            .unwrap_or_default();
        self.selection = selection;
        self.touch();
        true
    }

    /// Whether anything in the log is selected.
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Select the whole log, ready to be copied. Says whether there was
    /// anything in it to select.
    pub fn select_all(&mut self) -> bool {
        if self.spans.is_empty() {
            return false;
        }
        self.set_selection(selection::everything(&self.spans));
        true
    }

    /// What is selected in the log, as text.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let text = selection::text(&self.spans, selection);
        (!text.is_empty()).then_some(text)
    }

    /// Copy what is selected to the clipboard, for the key press `serial`.
    /// Says whether there was anything to copy.
    pub fn copy(&self, serial: u32) -> bool {
        match self.selected_text() {
            Some(text) => {
                copy_to_clipboard(&text, serial);
                true
            }
            None => false,
        }
    }

    // === The pointer ===

    /// Follow the pointer over the log, at `point` in its content coordinates
    /// or off it with `None`: the code block under it shows its copy button,
    /// and the tool calls or attachment under it say they can be clicked.
    /// Says whether the log needs repainting.
    pub fn hover(&mut self, point: Option<(f32, f32)>) -> bool {
        let code = self.hover_code(point);
        let steps = self.hover_steps(point);
        let attachment = self.hover_attachment(point);
        code || steps || attachment
    }

    /// The code block under the pointer. A copied block stops saying so once
    /// the pointer leaves it.
    fn hover_code(&mut self, point: Option<(f32, f32)>) -> bool {
        let hover = point.and_then(|point| self.painter.code_at(&self.lines, point, self.width));
        if hover == self.code_hover {
            return false;
        }
        let same_block = |a: Option<(usize, document::CodeHit)>,
                          b: Option<(usize, document::CodeHit)>| {
            a.map(|(line, hit)| (line, hit.block)) == b.map(|(line, hit)| (line, hit.block))
        };
        if !same_block(hover, self.code_hover) {
            self.code_copied = None;
        }
        self.code_hover = hover;
        self.touch();
        true
    }

    /// The group of tool calls under the pointer, drawn as something that can
    /// be opened.
    fn hover_steps(&mut self, point: Option<(f32, f32)>) -> bool {
        let hover = point
            .and_then(|point| self.painter.steps_at(&self.lines, point, self.width))
            .map(|(line, _)| line);
        if hover == self.steps_hover {
            return false;
        }
        self.steps_hover = hover;
        self.touch();
        true
    }

    /// The attachment under the pointer, highlighted.
    fn hover_attachment(&mut self, point: Option<(f32, f32)>) -> bool {
        let hover =
            point.and_then(|point| self.painter.attachment_at(&self.lines, point, self.width));
        if hover.map(|hit| (hit.line, hit.item))
            == self.attachment_hover.map(|hit| (hit.line, hit.item))
        {
            self.attachment_hover = hover;
            return false;
        }
        self.attachment_hover = hover;
        self.touch();
        true
    }

    /// The cursor for the pointer at `point`, after [`Self::hover`]: a hand
    /// over a copy button, a link, tool calls or an attachment, a text
    /// cursor over the log's words, and the arrow anywhere else. Painted text
    /// says nothing about itself otherwise.
    pub fn cursor(&self, point: Option<(f32, f32)>) -> CursorShape {
        let on_button = self.code_hover.is_some_and(|(_, hit)| hit.on_button);
        let over_link = self.link_at(point).is_some();
        let over_steps = self.steps_hover.is_some() || self.attachment_hover.is_some();
        let over_text = point
            .and_then(|point| selection::caret_at(&self.spans, point))
            .is_some();
        if on_button || over_link || over_steps {
            CursorShape::Pointer
        } else if over_text {
            CursorShape::Text
        } else {
            CursorShape::Default
        }
    }

    fn link_at(&self, point: Option<(f32, f32)>) -> Option<&str> {
        self.painter.link_at(&self.lines, point?, self.width)
    }

    /// Whether a selection is being dragged out, and so follows the pointer
    /// wherever it goes.
    pub fn is_selecting(&self) -> bool {
        self.selecting.is_some()
    }

    /// Drag the selection being made to `point`, in the log's content
    /// coordinates, pulled inside the log by the host. Says whether the log
    /// needs repainting.
    pub fn drag_to(&mut self, point: (f32, f32)) -> bool {
        let Some(anchor) = self.selecting else {
            return false;
        };
        match selection::nearest_caret(&self.spans, point) {
            Some(focus) => self.set_selection(Some(Selection { anchor, focus })),
            None => false,
        }
    }

    /// A press at `at` on the host's surface, which is `point` in the log's
    /// content coordinates when it is over the log. `time` and `serial` are
    /// the press's own: the first counts presses run together, the second is
    /// what a copy is made for.
    ///
    /// A press on a code block's copy button copies the block; on an
    /// attachment, it is held until the release says whether it was a click;
    /// on tool calls it opens or closes them; on the log's words it starts a
    /// selection, a second press taking the word and a third the line. A
    /// press on a link is remembered, for the release to follow.
    pub fn press(
        &mut self,
        point: Option<(f32, f32)>,
        at: (f32, f32),
        time: u32,
        serial: u32,
    ) -> Pressed {
        let (x, y) = at;
        // A press on a code block's copy button copies the block and nothing
        // else: not a selection, since the press is over the block's words
        // too.
        if let Some((line, hit)) = self.code_hover.filter(|(_, hit)| hit.on_button) {
            if let Some(text) = LogPainter::code_text(&self.lines, line, hit.block) {
                copy_to_clipboard(&text, serial);
                self.code_copied = Some((line, hit.block));
                self.touch();
            }
            return Pressed::Taken;
        }
        // A press on an attachment is a click on it, if it is released there.
        if let Some(hit) =
            point.and_then(|point| self.painter.attachment_at(&self.lines, point, self.width))
        {
            self.attachment_press = Some((hit, x, y));
            self.set_selection(None);
            return Pressed::Taken;
        }
        // A press on a group of tool calls opens or closes it, and is not
        // also the start of a selection over the words it is on.
        if let Some((_, block)) =
            point.and_then(|point| self.painter.steps_at(&self.lines, point, self.width))
        {
            if !self.steps_open.remove(&block) {
                self.steps_open.insert(block);
            }
            self.set_selection(None);
            return Pressed::Toggled;
        }
        // A press on a link is remembered rather than followed: the words of
        // a link are words like any other until the release says whether
        // they were read or clicked.
        self.link_press = self.link_at(point).map(|href| (Arc::from(href), x, y));
        // A press on the log's words starts a selection: the conversation is
        // there to be read, and read means copied.
        let caret = point.and_then(|point| selection::caret_at(&self.spans, point));
        if let Some(caret) = caret {
            let selection = match self.press_count(time, x, y) {
                1 => Selection::at(caret),
                2 => selection::word_at(&self.spans, caret),
                _ => selection::line_at(&self.spans, caret),
            };
            self.selecting = Some(selection.anchor);
            self.set_selection(Some(selection));
            return Pressed::Taken;
        }
        // Anywhere else puts the selection down again.
        self.set_selection(None);
        Pressed::Missed
    }

    /// How many presses have run together at this spot: a second within the
    /// double-press time picks out a word, a third the whole line.
    fn press_count(&mut self, time: u32, x: f32, y: f32) -> u32 {
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

    /// A release at `at`, `point` in the log as for [`Self::press`].
    pub fn release(&mut self, point: Option<(f32, f32)>, at: (f32, f32)) -> Released {
        let (x, y) = at;
        let still =
            |(from_x, from_y): (f32, f32)| (x - from_x).abs() <= SLOP && (y - from_y).abs() <= SLOP;
        if let Some((hit, from_x, from_y)) = self.attachment_press.take() {
            let here =
                point.and_then(|point| self.painter.attachment_at(&self.lines, point, self.width));
            if still((from_x, from_y)) && here == Some(hit) {
                return self.click_attachment(hit);
            }
            return Released::Taken;
        }
        if let Some((href, from_x, from_y)) = self.link_press.take() {
            self.selecting = None;
            // A press and a release in the same spot is a click; one that
            // travelled was a selection being dragged out over a link, and
            // opening it would be the last thing wanted.
            if still((from_x, from_y)) {
                self.set_selection(None);
                if let Err(err) = input::open_link(&href) {
                    tracing::warn!(%err, "could not open the link");
                }
            }
            return Released::Taken;
        }
        if self.selecting.take().is_some() {
            return Released::Taken;
        }
        Released::Missed
    }

    /// A click on an attachment. One that went with a request opens, as it
    /// would in Files; one going with the next request is the host's.
    fn click_attachment(&mut self, hit: AttachmentHit) -> Released {
        if hit.pending {
            self.attachment_hover = None;
            return Released::Attachment(hit);
        }
        let path = match self.lines.get(hit.line).map(|line| &line.kind) {
            Some(Kind::Attachments { items, .. }) => items
                .get(hit.item)
                .and_then(|(item, _)| item.path().map(Path::to_path_buf)),
            _ => None,
        };
        if let Some(path) = path {
            let uri = otto_kit::uri::path_to_uri(&path);
            if let Err(err) = input::open_link(&uri) {
                tracing::warn!(%err, path = %path.display(), "could not open the attachment");
            }
        }
        Released::Taken
    }

    /// The pointer left: nothing is under it, and nothing it held is a click
    /// any more.
    pub fn leave(&mut self) -> bool {
        self.selecting = None;
        self.link_press = None;
        self.attachment_press = None;
        self.hover(None)
    }

    // === Keys ===

    /// A key while a conversation is on. When the agent asks something, Up
    /// and Down pick an answer from the rows under the field, which are the
    /// host's; otherwise they scroll the log, as the page keys always do.
    /// The stop key with nothing selected to copy stops the agent's turn, and
    /// Shift+Tab switches the agent's mode, as it does in the agents' own
    /// interfaces. `None` for a key that is not the log's.
    pub fn key(&mut self, ask: &mut Ask, key: Key) -> Option<Keyed> {
        let scroll = match key.keysym {
            Keysym::Up if !key.answering => Some(-crate::log::LINE_H * 3.0),
            Keysym::Down if !key.answering => Some(crate::log::LINE_H * 3.0),
            Keysym::Page_Up => Some(-key.page),
            Keysym::Page_Down => Some(key.page),
            _ => None,
        };
        if let Some(delta) = scroll {
            return Some(Keyed::Scroll(delta));
        }
        if key.stop && !key.field_selected {
            ask.cancel();
            return Some(Keyed::Done);
        }
        let back_tab =
            key.keysym == Keysym::ISO_Left_Tab || (key.keysym == Keysym::Tab && key.shift);
        (back_tab && !key.answering && ask.cycle_mode()).then_some(Keyed::Done)
    }
}

impl ScrollContent for ChatView {
    fn length(&self, _cross: f32) -> f32 {
        length(&self.lines)
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn paint(&self, canvas: &Canvas, band: Rect) {
        let copy = self.code_hover.map(|(line, hit)| {
            (
                line,
                document::CopyButton {
                    block: hit.block,
                    hovered: hit.on_button,
                    copied: self.code_copied == Some((line, hit.block)),
                },
            )
        });
        self.painter.paint(
            canvas,
            band,
            &self.lines,
            self.width,
            &self.selection_rects,
            copy,
            self.steps_hover,
            self.attachment_hover,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The width the launcher's card lays the log out at.
    const WIDTH: f32 = 580.0;

    /// An Ask with nothing to talk to: nothing listens on the discard port.
    fn offline() -> Ask {
        Ask::connect("test", "ws://127.0.0.1:9".into(), PathBuf::from("/"))
    }

    /// An Ask with a file going with the next request, and the log of it.
    fn with_attachment(dir: &Path) -> (Ask, ChatView) {
        let file = dir.join("notes.txt");
        std::fs::write(&file, "the notes").expect("written");
        let mut ask = offline();
        ask.attach([file]);
        let mut chat = ChatView::new(true);
        assert!(chat.lay_out(&ask, WIDTH), "what goes next is a log");
        (ask, chat)
    }

    /// Somewhere on the attachment going with the next request, off its
    /// remove button.
    fn on_attachment(chat: &ChatView) -> (f32, f32) {
        let line = chat
            .lines()
            .iter()
            .find(|line| matches!(line.kind, Kind::Attachments { pending: true, .. }))
            .expect("the attachment is laid out");
        let (top, bottom) = (line.top as i32, (line.top + line.height) as i32);
        (top..bottom)
            .flat_map(|y| (0..WIDTH as i32).map(move |x| (x as f32, y as f32)))
            .find(|&point| {
                chat.painter
                    .attachment_at(chat.lines(), point, WIDTH)
                    .is_some_and(|hit| !hit.remove)
            })
            .expect("a point on the attachment")
    }

    /// A click on a file going with the next request is the host's to act
    /// on — it may be otto-stash's — while the log shows it under the
    /// pointer as something to click.
    #[test]
    fn a_click_on_an_attachment_going_next_is_handed_to_the_host() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (_ask, mut chat) = with_attachment(dir.path());

        let point = on_attachment(&chat);
        assert!(chat.hover(Some(point)), "the attachment lights up");
        assert_eq!(chat.cursor(Some(point)), CursorShape::Pointer);

        assert_eq!(chat.press(Some(point), point, 0, 0), Pressed::Taken);
        let Released::Attachment(hit) = chat.release(Some(point), point) else {
            panic!("the click is the host's");
        };
        assert!(hit.pending && !hit.remove);
        assert_eq!(hit.item, 0);
        assert_eq!(
            chat.release(Some(point), point),
            Released::Missed,
            "a click is let go of once it is handed over"
        );
    }

    /// A press that travelled before it was released is not a click.
    #[test]
    fn a_press_dragged_off_an_attachment_is_not_a_click() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (_ask, mut chat) = with_attachment(dir.path());

        let point = on_attachment(&chat);
        assert_eq!(chat.press(Some(point), point, 0, 0), Pressed::Taken);
        let away = (point.0 + 40.0, point.1);
        assert_eq!(chat.release(Some(away), away), Released::Taken);
    }

    /// With nothing asked and nothing going next, there is no log, nothing
    /// to select, and a press anywhere is the host's.
    #[test]
    fn an_empty_log_leaves_everything_to_the_host() {
        let ask = offline();
        let mut chat = ChatView::new(false);
        assert!(!chat.lay_out(&ask, WIDTH));
        assert!(chat.is_empty());
        assert!(!chat.select_all());
        assert_eq!(chat.selected_text(), None);
        let point = (10.0, 10.0);
        assert_eq!(chat.press(Some(point), point, 0, 0), Pressed::Missed);
        assert_eq!(chat.release(Some(point), point), Released::Missed);
    }
}
