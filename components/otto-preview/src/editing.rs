//! Filling a PDF's form and marking it up, saved into the file.
//!
//! Where marks (see [`crate::marks`]) point things out to the agent and stay
//! out of the document, this is the person changing the document itself:
//! typing into its form fields, ticking its boxes, picking from its lists,
//! and adding highlights, underlines, strike-outs, drawings, text and a
//! signature that any other PDF reader shows too. The engine is
//! [`otto_pdf`]; this is the window around it.
//!
//! The fields are always live on a PDF that has them: a tint shows where
//! they are, a click on a text field puts a caret in it, a click on a box
//! ticks it. The markup strip under the toolbar holds the tools that add to
//! the page, and the selection tool that moves and deletes what is there.
//!
//! Every change is made in memory, through the document's queue (see
//! [`crate::edit_queue`]), which runs them one at a time in the order they
//! were made; the pages it touched are drawn again by PDFium from memory,
//! the old drawing staying up until the new one lands. Undo and Redo step
//! through the changes not yet saved. Save writes them over the file, kept
//! as one of the file's versions (see [`crate::versions`]); a file changed
//! on disk since, or a save that fails, is asked about before anything is
//! lost.

// Rust guideline compliant 2026-02-21

use std::sync::{Arc, Mutex, Weak};

use otto_kit::components::text_input::{
    key_for, KeyMods, TextInput, TextInputKey, TextInputResponse, TextInputStyle,
};
use otto_kit::prelude::*;
use otto_kit::skia::{PaintCap, PaintJoin, PaintStyle, PathBuilder, Point, RRect};
use otto_kit::typography::TextStyle;
use otto_kit::CursorShape;
use otto_pdf::{
    AnnotId, AnnotKind, Document, Edit, Field, FieldKind, Markup, NewAnnotation, Rgb, Snapshot,
};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

use crate::confirm::{Answer, Question};
use crate::edit_queue::{Base, Failure, Job, Kind, Ledger, Outcome, Past, Queue};
use crate::marks::Frame;
use crate::viewer::{Stamp, Viewer};

/// The markup strip's height, under the toolbar.
pub const STRIP_H: f32 = 36.0;
/// A tool button in the strip.
const BUTTON: f32 = 28.0;
/// Between two buttons of one group.
const GAP: f32 = 2.0;
/// A colour swatch's diameter.
const SWATCH: f32 = 16.0;
/// How far a drawn stroke's points must be apart to be kept, in points on
/// screen.
const STEP: f32 = 1.5;
/// A drawn stroke's width, in PDF points.
const INK_WIDTH: f32 = 1.5;
/// A text box's size, in PDF points.
const TEXT_SIZE: f32 = 12.0;
/// How wide a placed signature is, in PDF points.
const SIGNATURE_W: f32 = 150.0;
/// The tint over a field that can be filled, over the accent colour.
const FIELD_ALPHA: u8 = 34;
const FIELD_HOVER_ALPHA: u8 = 60;
/// How far the pointer moves a press before it is a drag, in points.
const DRAG_SLOP: f32 = 3.0;

/// One line of marked text: its corners, as [`otto_pdf::AnnotKind::Markup`]
/// orders them.
type Quad = [(f32, f32); 4];

/// The colours markup is made in: highlighter yellow first.
pub const COLORS: [Rgb; 6] = [
    Rgb(1.0, 0.85, 0.1),
    Rgb(0.35, 0.85, 0.4),
    Rgb(0.3, 0.6, 1.0),
    Rgb(1.0, 0.45, 0.72),
    Rgb(0.9, 0.18, 0.15),
    Rgb(0.08, 0.08, 0.1),
];

/// A tool in the markup strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupTool {
    /// Select, move and delete what is on the page; fill the form.
    Select,
    Highlight,
    Underline,
    StrikeOut,
    /// Draw freehand.
    Draw,
    /// Put text on the page.
    Text,
    /// Place the person's signature.
    Signature,
}

impl MarkupTool {
    pub const ALL: [Self; 7] = [
        Self::Select,
        Self::Highlight,
        Self::Underline,
        Self::StrikeOut,
        Self::Draw,
        Self::Text,
        Self::Signature,
    ];

    fn markup(self) -> Option<Markup> {
        match self {
            Self::Highlight => Some(Markup::Highlight),
            Self::Underline => Some(Markup::Underline),
            Self::StrikeOut => Some(Markup::StrikeOut),
            _ => None,
        }
    }

    /// Whether the tool works on the text layer's words, so a drag selects
    /// text rather than draws.
    pub fn on_words(self) -> bool {
        self.markup().is_some()
    }
}

/// What a press in the strip landed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StripHit {
    Tool(MarkupTool),
    Color(usize),
    Delete,
}

/// Where the strip's controls sit for one window width.
#[derive(Debug, Clone)]
pub struct StripLayout {
    pub rect: Rect,
    pub tools: Vec<(MarkupTool, Rect)>,
    pub colors: Vec<(usize, Rect)>,
    pub delete: Rect,
}

impl StripLayout {
    pub fn hit(&self, at: Point) -> Option<StripHit> {
        use otto_kit::skia::Contains;
        if let Some((tool, _)) = self.tools.iter().find(|(_, rect)| rect.contains(at)) {
            return Some(StripHit::Tool(*tool));
        }
        if let Some((index, _)) = self.colors.iter().find(|(_, rect)| rect.contains(at)) {
            return Some(StripHit::Color(*index));
        }
        self.delete.contains(at).then_some(StripHit::Delete)
    }
}

/// Lay the strip out under the toolbar, `top` being the toolbar's bottom.
pub fn strip_layout(width: f32, top: f32) -> StripLayout {
    let rect = Rect::from_xywh(0.0, top, width, STRIP_H);
    let y = top + (STRIP_H - BUTTON) / 2.0;
    let mut x = 10.0;
    let mut tools = Vec::new();
    for (index, tool) in MarkupTool::ALL.into_iter().enumerate() {
        // The selection tool apart from the ones that add to the page.
        if index == 1 {
            x += GAP * 4.0;
        }
        tools.push((tool, Rect::from_xywh(x, y, BUTTON, BUTTON)));
        x += BUTTON + GAP;
    }
    x += GAP * 6.0;
    let mut colors = Vec::new();
    for index in 0..COLORS.len() {
        colors.push((index, Rect::from_xywh(x, y, BUTTON - 6.0, BUTTON)));
        x += BUTTON - 6.0;
    }
    let delete = Rect::from_xywh(width - 10.0 - BUTTON, y, BUTTON, BUTTON);
    StripLayout {
        rect,
        tools,
        colors,
        delete,
    }
}

/// What a text editor on the page is typing into.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorTarget {
    /// A text field, or a combo that takes typing.
    Field(AnnotId),
    /// A text box about to be added.
    NewText,
    /// A text box already on the page.
    Annotation(AnnotId),
}

/// Text being typed on the page, in a box over where it goes.
#[derive(Debug, Clone)]
pub struct Editor {
    pub target: EditorTarget,
    pub page: usize,
    /// Where it is, in the page's points.
    pub rect: otto_pdf::Rect,
    /// The text's size in the page's points.
    pub size: f32,
    pub input: TextInput,
    /// The document as it was when the editor opened, which its target is
    /// named against.
    pub base: Base,
}

/// Text just committed, drawn over its place until the page is drawn again
/// with it, so the old value never shows in between.
#[derive(Debug, Clone)]
pub struct Settling {
    pub page: usize,
    pub rect: otto_pdf::Rect,
    pub size: f32,
    pub text: String,
}

/// A press on the page being followed.
#[derive(Debug, Clone)]
enum Gesture {
    /// A freehand stroke, in the page's points.
    Stroke {
        frame: Frame,
        points: Vec<(f32, f32)>,
        last: Point,
    },
    /// An annotation being dragged, from where the press was to where the
    /// pointer is, both in the page's points.
    Move {
        id: AnnotId,
        frame: Frame,
        from: (f32, f32),
        to: (f32, f32),
        moved: bool,
        base: Base,
    },
}

/// The open document's form and annotations, and what the person is doing
/// to them.
pub struct Editing {
    pub doc: Arc<Mutex<Document>>,
    /// The document as the last job done left it.
    pub snapshot: Arc<Snapshot>,
    /// How many times the document had been renumbered then (see
    /// [`Outcome::numbering`]).
    pub numbering: u64,
    /// The document as it was under the numberings before this one.
    past: Past,
    /// Where the document's jobs wait their turn.
    queue: Queue,
    /// The decode it was opened for.
    pub generation: u64,
    pub selected: Option<AnnotId>,
    /// The field under the pointer.
    pub hovered: Option<AnnotId>,
    pub editor: Option<Editor>,
    pub settling: Vec<Settling>,
    gesture: Option<Gesture>,
    /// A field's list of choices showing, and the document it was read
    /// from.
    choosing: Option<Base>,
    /// Jobs sent and not yet finished.
    pub busy: usize,
    /// Of those, the ones that change the document: edits, undos, redos.
    changing: usize,
    /// How many edits Undo and Redo have to step through, as of the last
    /// job done.
    undo: usize,
    redo: usize,
    /// Whether the document holds changes the file doesn't, or will once
    /// the jobs in the queue are done.
    pub unsaved: bool,
    /// Close the window once the save in flight is done: Save picked when
    /// closing with changes.
    pub close_after_save: bool,
    /// The file as this window last opened or saved it, so the change it
    /// sees on disk afterwards is known to be its own.
    pub saved: Option<Stamp>,
}

impl std::fmt::Debug for Editing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editing")
            .field("generation", &self.generation)
            .field("fields", &self.snapshot.fields.len())
            .field("annotations", &self.snapshot.annotations.len())
            .field("selected", &self.selected)
            .field("busy", &self.busy)
            .finish()
    }
}

impl Editing {
    /// Edit `doc`, opened from `bytes`, the file at its path as stamped
    /// `stamp`, for the decode `generation`, in the window whose viewer is
    /// `viewer`: its queue's thread starts here.
    ///
    /// # Errors
    ///
    /// When the queue's thread can't be started.
    pub fn new(
        doc: Document,
        bytes: Vec<u8>,
        stamp: Option<Stamp>,
        generation: u64,
        viewer: Weak<Mutex<Viewer>>,
    ) -> std::io::Result<Self> {
        let snapshot = Arc::new(doc.snapshot());
        let ledger = Ledger::new(doc.path().to_path_buf(), bytes, stamp);
        let doc = Arc::new(Mutex::new(doc));
        let queue = Queue::start(Arc::clone(&doc), ledger, viewer)?;
        Ok(Self {
            doc,
            snapshot,
            numbering: 0,
            past: Past::default(),
            queue,
            generation,
            selected: None,
            hovered: None,
            editor: None,
            settling: Vec::new(),
            gesture: None,
            choosing: None,
            busy: 0,
            changing: 0,
            saved: stamp,
            undo: 0,
            redo: 0,
            unsaved: false,
            close_after_save: false,
        })
    }

    /// The document as the window knows it now, for an edit to be named
    /// against.
    pub fn base(&self) -> Base {
        Base {
            numbering: self.numbering,
            snapshot: Arc::clone(&self.snapshot),
        }
    }

    /// The document as the window knew it under `numbering`, for an edit
    /// whose ids were read then, or `None` when that is too long ago to be
    /// remembered.
    pub fn base_at(&self, numbering: u64) -> Option<Base> {
        if numbering == self.numbering {
            return Some(self.base());
        }
        self.past.at(numbering)
    }
}

/// Draw page `page` (0-based) of `doc` as it is in memory, unsaved changes
/// included, `width` pixels wide: how a PDF open for editing is drawn, so
/// a page scrolled back to or zoomed shows the changes and not the file.
pub fn draw_page(
    doc: &Mutex<Document>,
    page: usize,
    width: u32,
) -> Option<otto_kit::preview::Pixels> {
    match doc.lock().unwrap().render(page, width) {
        Ok((height, data)) => Some(otto_kit::preview::Pixels {
            width,
            height,
            intrinsic_width: width,
            intrinsic_height: height,
            data,
            frame_delays: Vec::new(),
            words: Vec::new(),
        }),
        Err(err) => {
            tracing::warn!(page, %err, "a page could not be drawn");
            None
        }
    }
}

/// Draw `touched`, pages of `doc` a job changed, and their thumbnails, from
/// the document as it is now, in place of what is on screen. Blocks: run it
/// off the UI thread.
pub fn redraw(viewer: &Mutex<Viewer>, doc: &Mutex<Document>, touched: &[usize]) {
    if touched.is_empty() {
        return;
    }
    let (pages, thumbs, generation) = {
        let viewer = viewer.lock().unwrap();
        (
            viewer.redraw_widths(touched),
            viewer.thumb_widths(touched),
            viewer.generation,
        )
    };
    tracing::debug!(?touched, ?pages, ?thumbs, "drawing edited pages");
    let draw = |page: usize, width: u32| draw_page(doc, page, width);
    for (page, width) in pages {
        let Some(pixels) = draw(page, width) else {
            continue;
        };
        let mut viewer = viewer.lock().unwrap();
        if viewer.generation == generation {
            viewer.finish_redraw(page as u32 + 1, Some(pixels));
        }
        drop(viewer);
        AppContext::request_wakeup();
    }
    // The sidebar's thumbnails of those pages, drawn the same way and put in
    // place of the old ones, so the sidebar never shows a blank page.
    for (page, width) in thumbs {
        let Some(image) = draw(page, width).and_then(|pixels| pixels.to_image()) else {
            continue;
        };
        let mut viewer = viewer.lock().unwrap();
        if viewer.generation == generation {
            viewer.thumbs.insert(page as u32 + 1, image);
            viewer.dirty = true;
        }
    }
}

/// Scale from the page's points to the screen's.
fn zoom_of(frame: &Frame) -> f32 {
    frame.screen.width() / frame.size.0.max(1.0)
}

fn to_screen_rect(frame: &Frame, rect: otto_pdf::Rect) -> Rect {
    let a = frame.to_screen((rect.left, rect.top));
    let b = frame.to_screen((rect.right, rect.bottom));
    Rect::from_ltrb(a.x, a.y, b.x, b.y)
}

fn color_of(rgb: Rgb) -> Color {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgb(channel(rgb.0), channel(rgb.1), channel(rgb.2))
}

/// The style a text editor on the page is drawn in at `scale` screen points
/// per page point: the page's own paper and ink, not the window's theme.
fn editor_style(size: f32, scale: f32, theme: &Theme) -> TextInputStyle {
    let mut style = TextInputStyle::with_theme(theme.clone());
    style.text_style = TextStyle {
        family: "Inter",
        weight: 400,
        size,
    };
    style.draw_scale = scale;
    style.horizontal_padding = 2.0;
    style.corner_radius = 2.0;
    style.focus_ring_width = 1.0 / scale.max(0.1);
    style.focus_ring_color = theme.accent;
    style.background = Color::from_argb(235, 255, 255, 255);
    style.text_color = Color::from_rgb(0x10, 0x10, 0x14);
    style.caret_color = theme.accent;
    style
}

/// The text size a field's value is shown at: what it asks for, or what
/// fits a single line of its height.
fn field_size(field: &Field) -> f32 {
    if field.font_size > 0.0 {
        field.font_size
    } else {
        (field.rect.height() * 0.68).clamp(6.0, 14.0)
    }
}

/// How big a text box is for `text` at `size`, in page points.
fn text_box(at: (f32, f32), text: &str, size: f32) -> otto_pdf::Rect {
    let font = TextStyle {
        family: "Inter",
        weight: 400,
        size,
    }
    .font();
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines
        .iter()
        .map(|line| otto_kit::typography::measure_runs(&font, line))
        .fold(size * 4.0, f32::max);
    let height = lines.len().max(1) as f32 * size * 1.25;
    otto_pdf::Rect::new(at.0, at.1, at.0 + width + 4.0, at.1 + height + 2.0)
}

impl Viewer {
    /// Whether this window can edit its document: a PDF opened for editing.
    pub fn editable(&self) -> bool {
        self.editing.is_some()
    }

    /// The markup strip, when it shows.
    pub fn strip(&self) -> Option<StripLayout> {
        self.markup?;
        let top = crate::chrome::chrome_h(self.variant);
        Some(strip_layout(self.size.0, top))
    }

    /// Show or hide the markup strip.
    pub fn toggle_markup(&mut self) {
        self.commit_editor();
        self.markup = match self.markup {
            Some(_) => None,
            None => Some(MarkupTool::Select),
        };
        if self.markup.is_some() {
            // The pen and the markup tools both want the press.
            self.marking = false;
        } else if let Some(editing) = &mut self.editing {
            editing.selected = None;
        }
        self.dirty = true;
    }

    /// Pick a tool in the strip; a selection already made is marked up at
    /// once by a tool that works on words.
    pub fn pick_tool(&mut self, tool: MarkupTool) {
        self.commit_editor();
        self.markup = Some(tool);
        self.marking = false;
        if tool.on_words() && self.session.selection.is_some() {
            self.markup_selection();
        }
        if tool == MarkupTool::Signature && crate::signature::load().is_none() {
            self.signature_pad = Some(crate::signature::Pad::default());
        }
        self.dirty = true;
    }

    /// A press in the markup strip. Returns whether it hit anything.
    pub fn strip_press(&mut self, at: Point) -> bool {
        let Some(strip) = self.strip() else {
            return false;
        };
        match strip.hit(at) {
            Some(StripHit::Tool(tool)) => self.pick_tool(tool),
            Some(StripHit::Color(index)) => {
                self.ink = index;
                let selected = self.editing.as_ref().and_then(|editing| editing.selected);
                if let Some(annotation) = selected {
                    self.submit(vec![Edit::SetColor {
                        annotation,
                        color: COLORS[index],
                    }]);
                }
                self.dirty = true;
            }
            Some(StripHit::Delete) => self.delete_selected(),
            None => {}
        }
        use otto_kit::skia::Contains;
        strip.rect.contains(at)
    }

    /// Make edits, in memory, named against the document as the window
    /// knows it now: shown once made, written to the file on
    /// [`Viewer::save`].
    pub fn submit(&mut self, edits: Vec<Edit>) {
        let Some(base) = self.editing.as_ref().map(Editing::base) else {
            return;
        };
        self.submit_on(edits, base, None);
    }

    /// Make edits named against `base`; `reply`, when given, hears how it
    /// went. Returns whether they were queued.
    pub fn submit_on(
        &mut self,
        edits: Vec<Edit>,
        base: Base,
        reply: Option<tokio::sync::oneshot::Sender<Outcome>>,
    ) -> bool {
        if edits.is_empty() {
            return false;
        }
        self.queue(Job::Apply { edits, base }, reply)
    }

    /// Queue `job` on the open document. Returns whether it was queued.
    fn queue(&mut self, job: Job, reply: Option<tokio::sync::oneshot::Sender<Outcome>>) -> bool {
        let Some(editing) = &mut self.editing else {
            return false;
        };
        let changes = job.kind().changes();
        // Counted before it is sent; the queue's thread waits on the
        // viewer's lock, held here, to count it done.
        editing.busy += 1;
        editing.changing += usize::from(changes);
        if !editing.queue.send(job, reply) {
            editing.busy -= 1;
            editing.changing -= usize::from(changes);
            tracing::warn!(path = %self.path.display(), "the document's queue has stopped");
            return false;
        }
        editing.unsaved |= changes;
        self.dirty = true;
        true
    }

    /// The document open for editing, which pages are drawn from.
    pub fn editing_doc(&self) -> Option<Arc<Mutex<Document>>> {
        self.editing
            .as_ref()
            .map(|editing| Arc::clone(&editing.doc))
    }

    /// Whether the document holds changes the file doesn't.
    pub fn unsaved(&self) -> bool {
        self.editing.as_ref().is_some_and(|editing| editing.unsaved)
    }

    /// Whether the document holds changes the file doesn't, or has jobs
    /// still to run: what reading the file again would throw away.
    pub fn edits_pending(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.unsaved || editing.busy > 0)
    }

    /// Write the document's changes over the file. Returns whether a save
    /// was queued.
    ///
    /// A close waiting on a save waits only while one is queued: with
    /// nothing queued here and no save in flight, it waits no longer.
    pub fn save(&mut self) -> bool {
        self.commit_editor();
        let queued = self.unsaved() && self.queue_save(false);
        if !queued && !self.saving() {
            if let Some(editing) = &mut self.editing {
                editing.close_after_save = false;
            }
        }
        queued
    }

    /// Whether a save or a save-as is queued and not yet done: the jobs in
    /// flight that don't change the document.
    fn saving(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.busy > editing.changing)
    }

    /// Write the document over the file, replacing it when `overwrite`
    /// even if it changed on disk. Returns whether it was queued.
    fn queue_save(&mut self, overwrite: bool) -> bool {
        // The file as it is before the first save, so there is always a
        // version to step back to.
        if self.versions.is_none() {
            self.keep_version(None);
        }
        self.queue(Job::Save { overwrite }, None)
    }

    /// Write the document to `path` and carry on with that file.
    pub fn save_as(&mut self, path: std::path::PathBuf) {
        self.commit_editor();
        self.queue(Job::SaveAs(path), None);
    }

    /// Whether Undo or Redo have an unsaved edit to take back or make again.
    pub fn can_undo_edit(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.undo > 0 || editing.changing > 0)
    }

    pub fn can_redo_edit(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.redo > 0)
    }

    /// Take back the last unsaved edit. Returns whether there was one.
    pub fn undo_edit(&mut self) -> bool {
        self.cancel_editor();
        if !self.can_undo_edit() {
            return false;
        }
        if let Some(editing) = &mut self.editing {
            editing.selected = None;
        }
        self.queue(Job::Undo, None)
    }

    /// Make the last edit taken back again. Returns whether there was one.
    pub fn redo_edit(&mut self) -> bool {
        self.cancel_editor();
        if !self.can_redo_edit() {
            return false;
        }
        if let Some(editing) = &mut self.editing {
            editing.selected = None;
        }
        self.queue(Job::Redo, None)
    }

    /// The pages of `touched` that are drawn now, 0-based, with the width
    /// each is drawn at, so it can be drawn again at the same size.
    pub fn redraw_widths(&self, touched: &[usize]) -> Vec<(usize, u32)> {
        let pages = self.session.pages();
        touched
            .iter()
            .filter_map(|&page| Some((page, pages.get(page)?.pixels.as_ref()?.width)))
            .collect()
    }

    /// The thumbnails of `touched` that the sidebar holds now, 0-based,
    /// with the width each was drawn at.
    pub fn thumb_widths(&self, touched: &[usize]) -> Vec<(usize, u32)> {
        touched
            .iter()
            .filter_map(|&page| {
                let image = self.thumbs.get(&(page as u32 + 1))?;
                Some((page, image.width().max(1) as u32))
            })
            .collect()
    }

    /// A job on `doc` has run: take the document as it really is now,
    /// whatever the result, and after a save keep the file as a version.
    /// A save that failed is asked about.
    pub fn finish_batch(&mut self, doc: &Arc<Mutex<Document>>, outcome: &Outcome) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        if !Arc::ptr_eq(&editing.doc, doc) {
            return;
        }
        editing.busy = editing.busy.saturating_sub(1);
        if outcome.kind.changes() {
            editing.changing = editing.changing.saturating_sub(1);
        }
        self.dirty = true;
        let old = std::mem::replace(&mut editing.snapshot, Arc::clone(&outcome.snapshot));
        let renumbered = editing.numbering != outcome.numbering;
        if renumbered {
            editing.past.keep(Base {
                numbering: editing.numbering,
                snapshot: Arc::clone(&old),
            });
        }
        editing.numbering = outcome.numbering;
        editing.undo = outcome.undo;
        editing.redo = outcome.redo;
        editing.unsaved = outcome.unsaved || editing.changing > 0;
        // What is selected is followed to where it is now.
        editing.selected = editing.selected.and_then(|id| {
            if renumbered {
                crate::edit_queue::follow_annotation(&old, &editing.snapshot, id)
            } else {
                editing.snapshot.annotation(id).map(|_| id)
            }
        });
        if renumbered {
            editing.hovered = None;
        }
        if let Err(err) = &outcome.result {
            editing.settling.clear();
            match outcome.kind {
                Kind::Save | Kind::SaveAs => {
                    tracing::warn!(path = %self.path.display(), %err, "the document couldn't be saved");
                    self.question = Some(match err {
                        Failure::ChangedOnDisk => Question::ChangedOnDisk,
                        other => Question::SaveFailed(other.to_string()),
                    });
                }
                Kind::Apply | Kind::Undo | Kind::Redo => {
                    tracing::warn!(path = %self.path.display(), %err, "an edit could not be made");
                }
            }
            return;
        }
        if !matches!(outcome.kind, Kind::Save | Kind::SaveAs) {
            return;
        }
        if outcome.kind == Kind::SaveAs {
            // A new file: its own name, folder and versions from now on.
            let path = outcome.path.clone();
            self.path = path.clone();
            self.name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            self.versions = crate::versions::Versions::existing(&path);
            self.renamed = Some(path);
        }
        let Some(editing) = &mut self.editing else {
            return;
        };
        // Saved: the change on disk is this window's own, not a reason to
        // read the file again.
        let close = std::mem::take(&mut editing.close_after_save);
        editing.saved = outcome.stamp;
        self.stamp = outcome.stamp;
        self.track_version();
        if close {
            self.closing = true;
            AppContext::request_wakeup();
        }
    }

    /// Close the window, or ask first when it holds unsaved changes.
    pub fn request_close(&mut self) {
        self.commit_editor();
        if self.unsaved() {
            self.question = Some(Question::Close);
            self.dirty = true;
        } else {
            self.closing = true;
            AppContext::request_wakeup();
        }
    }

    /// The answer to the question up over the window.
    pub fn answer(&mut self, answer: Answer) {
        self.question = None;
        self.dirty = true;
        match answer {
            Answer::Save => {
                if let Some(editing) = &mut self.editing {
                    editing.close_after_save = true;
                }
                self.save();
                // Nothing was left to write: close as asked.
                if !self.unsaved() && !self.saving() {
                    self.closing = true;
                    AppContext::request_wakeup();
                }
            }
            Answer::Overwrite => {
                self.queue_save(true);
            }
            Answer::SaveAs => self.wants_save_as = true,
            Answer::Discard => {
                self.closing = true;
                AppContext::request_wakeup();
            }
            Answer::Cancel => {
                if let Some(editing) = &mut self.editing {
                    editing.close_after_save = false;
                }
            }
        }
    }

    /// Save As was dismissed or couldn't ask: a close waiting on a save
    /// waits no longer.
    pub fn save_as_dismissed(&mut self) {
        if let Some(editing) = &mut self.editing {
            editing.close_after_save = false;
        }
    }

    /// A page drawn again after an edit has landed: what was settling on it
    /// is drawn by the page itself now.
    pub fn finish_redraw(&mut self, page: u32, pixels: Option<otto_kit::preview::Pixels>) {
        let content = self.content();
        if let Some(pixels) = pixels {
            self.dirty |= self.session.attach_page(page, pixels, content);
        }
        if let Some(editing) = &mut self.editing {
            let index = page as usize - 1;
            editing.settling.retain(|settling| settling.page != index);
        }
        self.dirty = true;
    }

    /// The frame of page `index`, as laid out now.
    fn frame_of(&self, index: usize) -> Option<Frame> {
        self.frames()
            .into_iter()
            .find(|frame| frame.page == Some(index))
    }

    /// The page under a window point, with the point in its points.
    fn page_at(&self, at: Point) -> Option<(Frame, (f32, f32))> {
        use otto_kit::skia::Contains;
        let frame = self
            .frames()
            .into_iter()
            .find(|frame| frame.screen.contains(at))?;
        Some((frame, frame.to_document(at)))
    }

    /// A press on the document. Returns whether editing took it.
    pub fn edit_press(&mut self, at: Point, clicks: u32) -> bool {
        if self.editing.is_none() || self.marking {
            return false;
        }
        // A press in the editor moves its caret; anywhere else commits it.
        if let Some(rect) = self.editor_screen_rect() {
            use otto_kit::skia::Contains;
            if rect.contains(at) {
                let editor = self.editing.as_mut().and_then(|e| e.editor.as_mut());
                if let Some(editor) = editor {
                    let x = (at.x - rect.left) / editor.input.style.draw_scale.max(0.01);
                    editor
                        .input
                        .on_pointer_down(x, clicks, self.modifiers.shift);
                }
                self.dirty = true;
                return true;
            }
            self.commit_editor();
        }
        let Some((frame, point)) = self.page_at(at) else {
            return false;
        };
        let Some(page) = frame.page else {
            return false;
        };
        let tool = self.markup;
        match tool {
            Some(MarkupTool::Draw) => {
                let editing = self.editing.as_mut().expect("checked above");
                editing.selected = None;
                editing.gesture = Some(Gesture::Stroke {
                    frame,
                    points: vec![point],
                    last: at,
                });
                self.dirty = true;
                return true;
            }
            Some(MarkupTool::Text) => {
                self.open_editor(EditorTarget::NewText, page, point, String::new(), TEXT_SIZE);
                return true;
            }
            Some(MarkupTool::Signature) => {
                self.place_signature(page, point);
                return true;
            }
            _ => {}
        }
        // The form, whatever the tool: it is the document's own interface.
        let field = self
            .editing
            .as_ref()
            .and_then(|editing| editing.snapshot.field_at(page, point).cloned());
        if let Some(field) = field {
            if tool.is_none_or(|tool| !tool.on_words()) {
                return self.press_field(&field);
            }
        }
        // What is on the page can be picked up with the selection tool.
        if tool == Some(MarkupTool::Select) {
            let editing = self.editing.as_mut().expect("checked above");
            let hit = editing
                .snapshot
                .annotation_at(page, point)
                .filter(|annotation| annotation.editable)
                .cloned();
            match hit {
                Some(annotation) => {
                    editing.selected = Some(annotation.id);
                    if clicks >= 2 {
                        if let AnnotKind::Text { text, size } = &annotation.kind {
                            let (text, size) = (text.clone(), *size);
                            let at = (annotation.rect.left, annotation.rect.top);
                            self.open_editor(
                                EditorTarget::Annotation(annotation.id),
                                page,
                                at,
                                text,
                                size,
                            );
                            return true;
                        }
                    }
                    editing.gesture = Some(Gesture::Move {
                        id: annotation.id,
                        frame,
                        from: point,
                        to: point,
                        moved: false,
                        base: editing.base(),
                    });
                    self.dirty = true;
                    return true;
                }
                None => {
                    if editing.selected.take().is_some() {
                        self.dirty = true;
                    }
                }
            }
        }
        false
    }

    /// A press on a field: type into it, tick it, or open its list.
    fn press_field(&mut self, field: &Field) -> bool {
        if field.read_only {
            return true;
        }
        match &field.kind {
            FieldKind::Text { value, .. } => {
                let size = field_size(field);
                self.open_field_editor(field, value.clone(), size);
            }
            FieldKind::Combo {
                options,
                selected,
                value,
                editable,
            } => {
                if *editable && options.is_empty() {
                    self.open_field_editor(field, value.clone(), field_size(field));
                } else {
                    if let Some(editing) = &mut self.editing {
                        editing.choosing = Some(editing.base());
                    }
                    self.choices = Some(Choices {
                        field: field.id,
                        rect: self.field_screen_rect(field),
                        options: options.clone(),
                        selected: *selected,
                    });
                }
            }
            FieldKind::List {
                options, selected, ..
            } => {
                if let Some(editing) = &mut self.editing {
                    editing.choosing = Some(editing.base());
                }
                self.choices = Some(Choices {
                    field: field.id,
                    rect: self.field_screen_rect(field),
                    options: options.clone(),
                    selected: selected.first().copied(),
                });
            }
            FieldKind::Checkbox { checked } => {
                self.submit(vec![Edit::SetChecked {
                    field: field.id,
                    checked: !checked,
                }]);
            }
            FieldKind::Radio { checked, .. } => {
                if !checked {
                    self.submit(vec![Edit::SetChecked {
                        field: field.id,
                        checked: true,
                    }]);
                }
            }
            FieldKind::Signature { signed: false } => {
                // The person's signature, fitted to the box.
                let rect = field.rect;
                self.place_signature_in(field.id.page, rect);
            }
            FieldKind::Signature { signed: true } | FieldKind::Button => {}
        }
        self.dirty = true;
        true
    }

    /// Where a field is on screen now.
    fn field_screen_rect(&self, field: &Field) -> Rect {
        self.frame_of(field.id.page)
            .map(|frame| to_screen_rect(&frame, field.rect))
            .unwrap_or_else(Rect::new_empty)
    }

    fn open_field_editor(&mut self, field: &Field, value: String, size: f32) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        let base = editing.base();
        let mut input = TextInput::editing(value, TextInputStyle::default());
        let max = match field.kind {
            FieldKind::Text { max_len, .. } => max_len,
            // Typed into PDFium a keystroke at a time.
            FieldKind::Combo { .. } => Some(otto_pdf::MAX_TYPED),
            _ => None,
        };
        if let Some(max) = max {
            input.state = std::mem::take(&mut input.state).with_max_chars(max);
            input.state.set_focused(true);
            input.state.select_all();
        }
        editing.editor = Some(Editor {
            target: EditorTarget::Field(field.id),
            page: field.id.page,
            rect: field.rect,
            size,
            input,
            base,
        });
        editing.selected = None;
        self.dirty = true;
    }

    fn open_editor(
        &mut self,
        target: EditorTarget,
        page: usize,
        at: (f32, f32),
        text: String,
        size: f32,
    ) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        let rect = text_box(at, if text.is_empty() { "M" } else { &text }, size);
        let mut input = TextInput::editing(text, TextInputStyle::default());
        input.state.set_focused(true);
        let base = editing.base();
        editing.editor = Some(Editor {
            target,
            page,
            rect,
            size,
            input,
            base,
        });
        self.dirty = true;
    }

    /// The editor's box on screen, sized to its text as it grows.
    pub fn editor_screen_rect(&self) -> Option<Rect> {
        let editor = self.editing.as_ref()?.editor.as_ref()?;
        let frame = self.frame_of(editor.page)?;
        let mut rect = editor.rect;
        if !matches!(editor.target, EditorTarget::Field(_)) {
            let text = editor.input.value();
            rect = text_box(
                (rect.left, rect.top),
                if text.is_empty() { "M" } else { text },
                editor.size,
            );
        }
        Some(to_screen_rect(&frame, rect))
    }

    /// Write what the editor holds and put it away.
    pub fn commit_editor(&mut self) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        let Some(editor) = editing.editor.take() else {
            return;
        };
        let text = editor.input.value().to_owned();
        let edit = match &editor.target {
            EditorTarget::Field(field) => {
                let unchanged = editor.base.snapshot.field(*field).is_some_and(|field| {
                    matches!(&field.kind, FieldKind::Text { value, .. }
                        | FieldKind::Combo { value, .. } if *value == text)
                });
                if unchanged {
                    self.dirty = true;
                    return;
                }
                editing.settling.push(Settling {
                    page: editor.page,
                    rect: editor.rect,
                    size: editor.size,
                    text: text.clone(),
                });
                Edit::SetText {
                    field: *field,
                    text,
                }
            }
            EditorTarget::NewText => {
                if text.trim().is_empty() {
                    self.dirty = true;
                    return;
                }
                editing.settling.push(Settling {
                    page: editor.page,
                    rect: editor.rect,
                    size: editor.size,
                    text: text.clone(),
                });
                Edit::Add {
                    page: editor.page,
                    annotation: NewAnnotation::Text {
                        at: (editor.rect.left, editor.rect.top),
                        text,
                        size: editor.size,
                        color: text_color(self.ink),
                    },
                }
            }
            EditorTarget::Annotation(id) => {
                if text.trim().is_empty() {
                    Edit::Delete { annotation: *id }
                } else {
                    Edit::SetContents {
                        annotation: *id,
                        text,
                    }
                }
            }
        };
        self.submit_on(vec![edit], editor.base, None);
    }

    /// Put the editor away without writing it.
    pub fn cancel_editor(&mut self) -> bool {
        let gone = self
            .editing
            .as_mut()
            .is_some_and(|editing| editing.editor.take().is_some());
        self.dirty |= gone;
        gone
    }

    /// Commit the editor and move to the next (or previous) text field.
    fn tab_to_field(&mut self, back: bool) {
        let current =
            self.editing
                .as_ref()
                .and_then(|editing| match editing.editor.as_ref()?.target {
                    EditorTarget::Field(id) => Some(id),
                    _ => None,
                });
        self.commit_editor();
        let Some(editing) = &self.editing else {
            return;
        };
        let texts: Vec<&Field> = editing
            .snapshot
            .fields
            .iter()
            .filter(|field| !field.read_only && matches!(field.kind, FieldKind::Text { .. }))
            .collect();
        if texts.is_empty() {
            return;
        }
        let at = current.and_then(|id| texts.iter().position(|field| field.id == id));
        let next = match (at, back) {
            (Some(at), false) => (at + 1) % texts.len(),
            (Some(at), true) => (at + texts.len() - 1) % texts.len(),
            (None, false) => 0,
            (None, true) => texts.len() - 1,
        };
        let field = texts[next].clone();
        let FieldKind::Text { value, .. } = &field.kind else {
            return;
        };
        let value = value.clone();
        let size = field_size(&field);
        self.open_field_editor(&field, value, size);
        self.reveal_page(field.id.page);
    }

    /// Bring page `index` into view if it is off screen.
    fn reveal_page(&mut self, index: usize) {
        use otto_kit::skia::Contains;
        let content = self.content();
        let visible = self.frame_of(index).is_some_and(|frame| {
            content.contains(Point::new(frame.screen.center_x(), frame.screen.top + 1.0))
        });
        if !visible {
            let page = self.page_status().map(|(page, _)| page as i32).unwrap_or(1);
            self.turn_page(index as i32 + 1 - page);
        }
    }

    /// The pointer moved. Returns whether a gesture of editing's took it.
    pub fn edit_motion(&mut self, at: Point) -> bool {
        let shift = self.modifiers.shift;
        let Some(editing) = &mut self.editing else {
            return false;
        };
        match &mut editing.gesture {
            Some(Gesture::Stroke {
                frame,
                points,
                last,
            }) => {
                if (at.x - last.x).hypot(at.y - last.y) >= STEP {
                    points.push(frame.to_document(at));
                    *last = at;
                    self.dirty = true;
                }
                true
            }
            Some(Gesture::Move {
                frame,
                from,
                to,
                moved,
                ..
            }) => {
                let point = frame.to_document(at);
                let zoom = zoom_of(frame);
                if !*moved && (point.0 - from.0).hypot(point.1 - from.1) * zoom >= DRAG_SLOP {
                    *moved = true;
                }
                // Shift keeps a move to one axis.
                *to = if shift {
                    if (point.0 - from.0).abs() > (point.1 - from.1).abs() {
                        (point.0, from.1)
                    } else {
                        (from.0, point.1)
                    }
                } else {
                    point
                };
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// Follow the pointer over the page: which field it is over.
    pub fn edit_hover(&mut self, at: Option<Point>) {
        let hovered = at.and_then(|at| {
            let (frame, point) = self.page_at(at)?;
            let editing = self.editing.as_ref()?;
            let field = editing.snapshot.field_at(frame.page?, point)?;
            (!field.read_only).then_some(field.id)
        });
        if let Some(editing) = &mut self.editing {
            if editing.hovered != hovered {
                editing.hovered = hovered;
                self.dirty = true;
            }
        }
    }

    /// Whether a gesture of editing's is under way.
    pub fn edit_busy(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.gesture.is_some())
    }

    /// The button went up: finish the gesture, or mark up the words a drag
    /// selected.
    pub fn edit_release(&mut self) {
        let gesture = self
            .editing
            .as_mut()
            .and_then(|editing| editing.gesture.take());
        match gesture {
            Some(Gesture::Stroke { frame, points, .. }) => {
                let Some(page) = frame.page else {
                    return;
                };
                // A click without a drag is a dot.
                let points = if points.len() == 1 {
                    let (x, y) = points[0];
                    vec![(x, y), (x + 0.5, y + 0.5)]
                } else {
                    points
                };
                self.submit(vec![Edit::Add {
                    page,
                    annotation: NewAnnotation::Ink {
                        strokes: vec![points],
                        width: INK_WIDTH,
                        color: COLORS[self.ink],
                    },
                }]);
            }
            Some(Gesture::Move {
                id,
                from,
                to,
                moved,
                base,
                ..
            }) => {
                if moved {
                    self.submit_on(
                        vec![Edit::Move {
                            annotation: id,
                            dx: to.0 - from.0,
                            dy: to.1 - from.1,
                        }],
                        base,
                        None,
                    );
                }
            }
            None => {
                if self.markup.is_some_and(MarkupTool::on_words)
                    && !self.session.selecting
                    && self.session.selection.is_some()
                {
                    self.markup_selection();
                }
            }
        }
        self.dirty = true;
    }

    /// Mark up the words selected now with the tool in hand, and let the
    /// selection go.
    fn markup_selection(&mut self) {
        let Some(markup) = self.markup.and_then(MarkupTool::markup) else {
            return;
        };
        let Some(selection) = self.session.selection else {
            return;
        };
        let layout = otto_kit::preview::layout(
            self.content(),
            &self.session.preview,
            self.session.first_row,
            self.session.zoom,
        );
        let lines = otto_kit::preview::selection_lines(&layout, &self.session.preview, selection);
        let frames = self.frames();
        let mut by_page: Vec<(usize, Vec<Quad>)> = Vec::new();
        for line in lines {
            use otto_kit::skia::Contains;
            let middle = Point::new(line.center_x(), line.center_y());
            let Some(frame) = frames.iter().find(|frame| frame.screen.contains(middle)) else {
                continue;
            };
            let Some(page) = frame.page else {
                continue;
            };
            let (left, top) = frame.to_document(Point::new(line.left, line.top));
            let (right, bottom) = frame.to_document(Point::new(line.right, line.bottom));
            let quad = [(left, top), (right, top), (left, bottom), (right, bottom)];
            match by_page.iter_mut().find(|(at, _)| *at == page) {
                Some((_, quads)) => quads.push(quad),
                None => by_page.push((page, vec![quad])),
            }
        }
        let color = COLORS[self.ink];
        let edits = by_page
            .into_iter()
            .map(|(page, quads)| Edit::Add {
                page,
                annotation: NewAnnotation::Markup {
                    markup,
                    quads,
                    color,
                },
            })
            .collect();
        self.session.clear_selection();
        self.submit(edits);
    }

    /// Place the person's signature centred on a point of page `page`.
    fn place_signature(&mut self, page: usize, at: (f32, f32)) {
        let Some(signature) = crate::signature::load() else {
            self.signature_pad = Some(crate::signature::Pad::default());
            self.dirty = true;
            return;
        };
        let height = SIGNATURE_W / signature.aspect().max(0.1);
        let rect = otto_pdf::Rect::new(
            at.0 - SIGNATURE_W / 2.0,
            at.1 - height / 2.0,
            at.0 + SIGNATURE_W / 2.0,
            at.1 + height / 2.0,
        );
        self.place_signature_in(page, rect);
    }

    /// Place the person's signature fitted into `rect` on page `page`.
    fn place_signature_in(&mut self, page: usize, rect: otto_pdf::Rect) {
        let Some(signature) = crate::signature::load() else {
            self.signature_pad = Some(crate::signature::Pad::default());
            self.dirty = true;
            return;
        };
        let strokes = signature.fitted(rect);
        if strokes.is_empty() {
            return;
        }
        self.submit(vec![Edit::Add {
            page,
            annotation: NewAnnotation::Ink {
                strokes,
                width: INK_WIDTH,
                color: Rgb(0.07, 0.12, 0.35),
            },
        }]);
    }

    /// Delete the selected annotation.
    pub fn delete_selected(&mut self) {
        let selected = self
            .editing
            .as_mut()
            .and_then(|editing| editing.selected.take());
        if let Some(annotation) = selected {
            self.submit(vec![Edit::Delete { annotation }]);
        }
    }

    /// A choice made from a field's list, in place of what was chosen.
    pub fn choose(&mut self, field: AnnotId, index: usize) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        let base = editing.choosing.take().unwrap_or_else(|| editing.base());
        self.submit_on(
            vec![Edit::Choose {
                field,
                index,
                add: false,
            }],
            base,
            None,
        );
    }

    /// A key, before the window's own keys. Returns whether editing took it.
    pub fn edit_key(&mut self, event: &KeyEvent, serial: u32) -> bool {
        let modifiers = self.modifiers;
        let has_editor = self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.editor.is_some());
        if has_editor {
            match event.keysym {
                Keysym::Tab | Keysym::ISO_Left_Tab => {
                    self.tab_to_field(modifiers.shift || event.keysym == Keysym::ISO_Left_Tab);
                    return true;
                }
                Keysym::Return | Keysym::KP_Enter if modifiers.shift => {
                    // A new line, in a text box or a field that takes them.
                    let multiline = self.editor_multiline();
                    if multiline {
                        if let Some(editor) = self.editing.as_mut().and_then(|e| e.editor.as_mut())
                        {
                            editor
                                .input
                                .on_key(TextInputKey::Char('\n'), KeyMods::default());
                        }
                        self.dirty = true;
                        return true;
                    }
                }
                _ => {}
            }
            let mods = KeyMods::from(modifiers);
            let ctrl = modifiers.ctrl || modifiers.logo;
            let key = if ctrl && matches!(event.keysym, Keysym::v | Keysym::V) {
                otto_kit::clipboard::text().map(|text| (TextInputKey::Paste(text), mods))
            } else {
                key_for(event.keysym, event.utf8.as_deref(), mods)
            };
            let Some((key, mods)) = key else {
                // Every other key stays with the editor too: nothing typed
                // here should turn the page or zoom.
                return !ctrl;
            };
            let response = self
                .editing
                .as_mut()
                .and_then(|editing| editing.editor.as_mut())
                .map(|editor| editor.input.on_key(key, mods));
            match response {
                Some(TextInputResponse::Commit) => self.commit_editor(),
                Some(TextInputResponse::Cancel) => {
                    self.cancel_editor();
                }
                Some(TextInputResponse::Clipboard(text)) => {
                    otto_kit::clipboard::set_text(&text, serial);
                }
                _ => {}
            }
            self.dirty = true;
            return true;
        }
        let selected = self.editing.as_ref().and_then(|editing| editing.selected);
        match event.keysym {
            Keysym::BackSpace | Keysym::Delete if selected.is_some() => {
                self.delete_selected();
                true
            }
            Keysym::Escape if selected.is_some() => {
                if let Some(editing) = &mut self.editing {
                    editing.selected = None;
                }
                self.dirty = true;
                true
            }
            Keysym::Escape if self.markup.is_some_and(|tool| tool != MarkupTool::Select) => {
                self.markup = Some(MarkupTool::Select);
                self.dirty = true;
                true
            }
            Keysym::Tab | Keysym::ISO_Left_Tab
                if self
                    .editing
                    .as_ref()
                    .is_some_and(|editing| !editing.snapshot.fields.is_empty()) =>
            {
                self.tab_to_field(modifiers.shift || event.keysym == Keysym::ISO_Left_Tab);
                true
            }
            _ => false,
        }
    }

    fn editor_multiline(&self) -> bool {
        let Some(editing) = &self.editing else {
            return false;
        };
        match editing.editor.as_ref().map(|editor| &editor.target) {
            Some(EditorTarget::Field(id)) => editing.snapshot.field(*id).is_some_and(|field| {
                matches!(
                    field.kind,
                    FieldKind::Text {
                        multiline: true,
                        ..
                    }
                )
            }),
            Some(_) => true,
            None => false,
        }
    }

    /// Advance the editor's caret blink. Returns whether it is showing, so
    /// the loop keeps a clock.
    pub fn edit_tick(&mut self, delta: f32) -> bool {
        let Some(editor) = self
            .editing
            .as_mut()
            .and_then(|editing| editing.editor.as_mut())
        else {
            return false;
        };
        let before = editor.input.caret_visible();
        editor.input.tick(delta);
        self.dirty |= before != editor.input.caret_visible();
        true
    }

    /// The cursor editing wants at `at`, if any.
    pub fn edit_cursor(&self, at: Point) -> Option<CursorShape> {
        let editing = self.editing.as_ref()?;
        if self.marking {
            return None;
        }
        if let Some(rect) = self.editor_screen_rect() {
            use otto_kit::skia::Contains;
            if rect.contains(at) {
                return Some(CursorShape::Text);
            }
        }
        if let Some(Gesture::Move { .. }) = editing.gesture {
            return Some(CursorShape::Grabbing);
        }
        match self.markup {
            Some(MarkupTool::Draw) | Some(MarkupTool::Signature) => {
                return Some(CursorShape::Crosshair)
            }
            Some(MarkupTool::Text) => return Some(CursorShape::Text),
            _ => {}
        }
        let (frame, point) = self.page_at(at)?;
        let page = frame.page?;
        if let Some(field) = editing.snapshot.field_at(page, point) {
            if field.read_only {
                return None;
            }
            return Some(match field.kind {
                FieldKind::Text { .. } => CursorShape::Text,
                FieldKind::Button => CursorShape::Default,
                _ => CursorShape::Pointer,
            });
        }
        if self.markup == Some(MarkupTool::Select) {
            if let Some(annotation) = editing.snapshot.annotation_at(page, point) {
                if annotation.editable {
                    return Some(CursorShape::Grab);
                }
            }
        }
        None
    }

    /// Whether the window should keep a clock for editing: the caret blinks.
    pub fn edit_animating(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.editor.is_some())
    }
}

/// The colour text is put on the page in: the swatch, except highlighter
/// yellow, which can't be read on white.
fn text_color(index: usize) -> Rgb {
    if index == 0 {
        COLORS[COLORS.len() - 1]
    } else {
        COLORS[index]
    }
}

/// A list of choices waiting for the app to show as a menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    pub field: AnnotId,
    /// The field on screen, for the menu to drop from.
    pub rect: Rect,
    pub options: Vec<String>,
    pub selected: Option<usize>,
}

/// Paint the form's tint, what is selected or being drawn, and the editor,
/// over the pages drawn into `frames`.
pub fn draw(canvas: &Canvas, viewer: &Viewer, frames: &[Frame], theme: &Theme) {
    let Some(editing) = &viewer.editing else {
        return;
    };
    let frame_of = |page: usize| frames.iter().find(|frame| frame.page == Some(page));
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // Where the form can be filled.
    let editor_field = editing
        .editor
        .as_ref()
        .and_then(|editor| match editor.target {
            EditorTarget::Field(id) => Some(id),
            _ => None,
        });
    for field in &editing.snapshot.fields {
        if field.read_only
            || matches!(field.kind, FieldKind::Button)
            || editor_field == Some(field.id)
        {
            continue;
        }
        let Some(frame) = frame_of(field.id.page) else {
            continue;
        };
        let alpha = if editing.hovered == Some(field.id) {
            FIELD_HOVER_ALPHA
        } else {
            FIELD_ALPHA
        };
        paint.set_style(PaintStyle::Fill);
        paint.set_color(theme.accent.with_a(alpha));
        let rect = to_screen_rect(frame, field.rect);
        let round = if matches!(field.kind, FieldKind::Radio { .. }) {
            rect.width().min(rect.height()) / 2.0
        } else {
            2.0
        };
        canvas.draw_rrect(RRect::new_rect_xy(rect, round, round), &paint);
        if field.required {
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_color(theme.accent.with_a(110));
            canvas.draw_rrect(RRect::new_rect_xy(rect, round, round), &paint);
        }
    }

    // Text written and not yet drawn by the page.
    for settling in &editing.settling {
        let Some(frame) = frame_of(settling.page) else {
            continue;
        };
        let zoom = zoom_of(frame);
        let rect = to_screen_rect(frame, settling.rect);
        paint.set_style(PaintStyle::Fill);
        paint.set_color(Color::WHITE);
        canvas.draw_rect(rect, &paint);
        let font = TextStyle {
            family: "Inter",
            weight: 400,
            size: settling.size * zoom,
        }
        .font();
        paint.set_color(Color::from_rgb(0x10, 0x10, 0x14));
        let mut y = rect.top + settling.size * zoom;
        for line in settling.text.split('\n') {
            canvas.draw_str(line, (rect.left + 2.0 * zoom, y), &font, &paint);
            y += settling.size * zoom * 1.25;
        }
    }

    // The annotation in hand, where it is going.
    if let Some(id) = editing.selected {
        if let (Some(annotation), Some(frame)) =
            (editing.snapshot.annotation(id), frame_of(id.page))
        {
            let mut rect = annotation.rect;
            if let Some(Gesture::Move { from, to, .. }) = &editing.gesture {
                rect = rect.translate(to.0 - from.0, to.1 - from.1);
            }
            let screen = to_screen_rect(frame, rect).with_outset((3.0, 3.0));
            paint.set_style(PaintStyle::Fill);
            paint.set_color(theme.accent.with_a(26));
            canvas.draw_rrect(RRect::new_rect_xy(screen, 3.0, 3.0), &paint);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(1.5);
            paint.set_color(theme.accent);
            canvas.draw_rrect(RRect::new_rect_xy(screen, 3.0, 3.0), &paint);
        }
    }

    // A stroke being drawn.
    if let Some(Gesture::Stroke { frame, points, .. }) = &editing.gesture {
        let mut path = PathBuilder::new();
        for (index, point) in points.iter().enumerate() {
            let at = frame.to_screen(*point);
            if index == 0 {
                path.move_to(at);
            } else {
                path.line_to(at);
            }
        }
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(INK_WIDTH * zoom_of(frame));
        paint.set_stroke_cap(PaintCap::Round);
        paint.set_stroke_join(PaintJoin::Round);
        paint.set_color(color_of(COLORS[viewer.ink]));
        canvas.draw_path(&path.detach(), &paint);
    }

    // The text being typed.
    if let (Some(editor), Some(rect)) = (&editing.editor, viewer.editor_screen_rect()) {
        if let Some(frame) = frame_of(editor.page) {
            let mut input = editor.input.clone();
            input.style = editor_style(editor.size, zoom_of(frame), theme);
            let (width, height) = (rect.width(), rect.height());
            input.set_size(
                width / input.style.draw_scale,
                height / input.style.draw_scale,
            );
            canvas.save();
            canvas.translate((rect.left, rect.top));
            input.render_at(canvas, width, height);
            canvas.restore();
        }
    }
}

/// Paint the markup strip under the toolbar.
pub fn draw_strip(canvas: &Canvas, viewer: &Viewer, theme: &Theme, ground: Color) {
    let Some(strip) = viewer.strip() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(ground);
    canvas.draw_rect(strip.rect, &paint);
    paint.set_color(theme.fill_tertiary);
    canvas.draw_rect(
        Rect::from_ltrb(
            strip.rect.left,
            strip.rect.bottom - 1.0,
            strip.rect.right,
            strip.rect.bottom,
        ),
        &paint,
    );
    let hovered = viewer.pointer.and_then(|at| strip.hit(at));
    for (tool, rect) in &strip.tools {
        let on = viewer.markup == Some(*tool);
        let over = hovered == Some(StripHit::Tool(*tool));
        if on || over {
            paint.set_color(if on {
                theme.fill_tertiary
            } else {
                theme.fill_quaternary
            });
            canvas.draw_rrect(RRect::new_rect_xy(*rect, 6.0, 6.0), &paint);
        }
        let color = if on || over {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        draw_tool_glyph(
            canvas,
            *tool,
            *rect,
            color,
            color_of(COLORS[viewer.ink]),
            on,
        );
    }
    for (index, rect) in &strip.colors {
        let center = (rect.center_x(), rect.center_y());
        if *index == viewer.ink {
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(1.5);
            paint.set_color(theme.text_secondary);
            canvas.draw_circle(center, SWATCH / 2.0 + 2.5, &paint);
        }
        paint.set_style(PaintStyle::Fill);
        paint.set_color(color_of(COLORS[*index]));
        canvas.draw_circle(center, SWATCH / 2.0, &paint);
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(0.5);
        paint.set_color(Color::from_argb(60, 0, 0, 0));
        canvas.draw_circle(center, SWATCH / 2.0, &paint);
        paint.set_style(PaintStyle::Fill);
    }
    let selected = viewer
        .editing
        .as_ref()
        .is_some_and(|editing| editing.selected.is_some());
    let color = if !selected {
        theme.text_tertiary
    } else if hovered == Some(StripHit::Delete) {
        theme.text_primary
    } else {
        theme.text_secondary
    };
    if !draw_solar_glyph(canvas, "otto-delete-symbolic", strip.delete, color, false) {
        draw_bin_glyph(canvas, strip.delete, color);
    }
    if let Some(editing) = &viewer.editing {
        if editing.busy > 0 {
            // Working: a dot by the bin while an edit is being made or the
            // file written.
            paint.set_color(theme.text_tertiary);
            canvas.draw_circle(
                (strip.delete.left - 10.0, strip.delete.center_y()),
                2.5,
                &paint,
            );
        }
    }
}

fn stroke_paint(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(1.5);
    paint.set_stroke_cap(PaintCap::Round);
    paint.set_stroke_join(PaintJoin::Round);
    paint.set_color(color);
    paint
}

/// Otto's own glyph `name`, 16 points square in the middle of `rect`, bold
/// when `bold`. Whether Otto's set had it.
fn draw_solar_glyph(canvas: &Canvas, name: &str, rect: Rect, color: Color, bold: bool) -> bool {
    let style = if bold {
        otto_kit::icons::SolarStyle::Bold
    } else {
        otto_kit::icons::SolarStyle::Linear
    };
    // Rasterised in the canvas's device pixels, so it stays sharp on a
    // scaled output.
    let scale = canvas.local_to_device_as_3x3().scale_x().abs().max(1.0);
    let Some(image) =
        otto_kit::icons::solar_icon(name, (16.0 * scale).round() as i32, style, color)
    else {
        return false;
    };
    let dst = Rect::from_xywh(rect.center_x() - 8.0, rect.center_y() - 8.0, 16.0, 16.0);
    otto_kit::icons::draw_icon(canvas, &image, dst, &Paint::default());
    true
}

/// Otto's own glyph for a markup tool.
fn tool_glyph_name(tool: MarkupTool) -> &'static str {
    match tool {
        MarkupTool::Select => "otto-mark-symbolic",
        MarkupTool::Highlight => "otto-highlight-symbolic",
        MarkupTool::Underline => "otto-underline-symbolic",
        MarkupTool::StrikeOut => "otto-strikeout-symbolic",
        MarkupTool::Draw => "otto-pen-symbolic",
        MarkupTool::Text => "otto-text-symbolic",
        MarkupTool::Signature => "otto-signature-symbolic",
    }
}

/// A tool's glyph: Otto's own, bold while the tool is chosen, or drawn when
/// the set is missing.
fn draw_tool_glyph(
    canvas: &Canvas,
    tool: MarkupTool,
    rect: Rect,
    color: Color,
    ink: Color,
    on: bool,
) {
    if draw_solar_glyph(canvas, tool_glyph_name(tool), rect, color, on) {
        return;
    }
    let r = Rect::from_xywh(rect.center_x() - 8.0, rect.center_y() - 8.0, 16.0, 16.0);
    let stroke = stroke_paint(color);
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    let mut path = PathBuilder::new();
    match tool {
        MarkupTool::Select => {
            // An arrow pointer.
            path.move_to((r.left + 3.0, r.top + 1.0));
            path.line_to((r.left + 3.0, r.bottom - 2.0));
            path.line_to((r.left + 7.0, r.bottom - 6.0));
            path.line_to((r.left + 12.0, r.bottom - 5.5));
            path.close();
            canvas.draw_path(&path.detach(), &stroke);
            return;
        }
        MarkupTool::Highlight => {
            fill.set_color(ink.with_a(150));
            canvas.draw_rect(
                Rect::from_ltrb(r.left, r.top + 4.0, r.right, r.bottom - 4.0),
                &fill,
            );
            draw_letter(canvas, r, color);
            return;
        }
        MarkupTool::Underline => {
            draw_letter(canvas, r, color);
            let mut line = stroke_paint(ink);
            line.set_stroke_width(2.0);
            canvas.draw_line(
                (r.left + 2.0, r.bottom - 1.0),
                (r.right - 2.0, r.bottom - 1.0),
                &line,
            );
            return;
        }
        MarkupTool::StrikeOut => {
            draw_letter(canvas, r, color);
            let mut line = stroke_paint(ink);
            line.set_stroke_width(2.0);
            canvas.draw_line(
                (r.left + 1.0, r.center_y()),
                (r.right - 1.0, r.center_y()),
                &line,
            );
            return;
        }
        MarkupTool::Draw => {
            // A squiggle.
            path.move_to((r.left + 1.0, r.bottom - 4.0));
            path.cubic_to(
                (r.left + 4.0, r.top),
                (r.left + 8.0, r.top),
                (r.center_x(), r.center_y()),
            );
            path.cubic_to(
                (r.left + 10.0, r.bottom),
                (r.right - 3.0, r.bottom),
                (r.right - 1.0, r.top + 3.0),
            );
        }
        MarkupTool::Text => {
            // A capital T in a box.
            canvas.draw_rrect(
                RRect::new_rect_xy(r.with_inset((0.75, 0.75)), 2.5, 2.5),
                &stroke,
            );
            path.move_to((r.left + 4.5, r.top + 4.5));
            path.line_to((r.right - 4.5, r.top + 4.5));
            path.move_to((r.center_x(), r.top + 4.5));
            path.line_to((r.center_x(), r.bottom - 4.0));
        }
        MarkupTool::Signature => {
            // A signed line.
            path.move_to((r.left + 1.0, r.bottom - 5.0));
            path.cubic_to(
                (r.left + 4.0, r.top - 1.0),
                (r.left + 7.0, r.top + 2.0),
                (r.left + 6.0, r.bottom - 6.0),
            );
            path.cubic_to(
                (r.left + 9.0, r.top + 4.0),
                (r.right - 4.0, r.top + 6.0),
                (r.right - 1.0, r.bottom - 7.0),
            );
            path.move_to((r.left + 1.0, r.bottom - 1.5));
            path.line_to((r.right - 1.0, r.bottom - 1.5));
        }
    }
    canvas.draw_path(&path.detach(), &stroke);
}

/// A letter A, the text a markup tool marks.
fn draw_letter(canvas: &Canvas, r: Rect, color: Color) {
    let stroke = stroke_paint(color);
    let mut path = PathBuilder::new();
    path.move_to((r.left + 3.5, r.bottom - 3.0));
    path.line_to((r.center_x(), r.top + 2.5));
    path.line_to((r.right - 3.5, r.bottom - 3.0));
    path.move_to((r.left + 5.5, r.bottom - 7.0));
    path.line_to((r.right - 5.5, r.bottom - 7.0));
    canvas.draw_path(&path.detach(), &stroke);
}

/// A bin, for deleting what is selected.
fn draw_bin_glyph(canvas: &Canvas, rect: Rect, color: Color) {
    let r = Rect::from_xywh(rect.center_x() - 8.0, rect.center_y() - 8.0, 16.0, 16.0);
    let stroke = stroke_paint(color);
    let mut path = PathBuilder::new();
    path.move_to((r.left + 2.0, r.top + 4.0));
    path.line_to((r.right - 2.0, r.top + 4.0));
    path.move_to((r.left + 6.0, r.top + 4.0));
    path.line_to((r.left + 6.5, r.top + 1.5));
    path.line_to((r.right - 6.5, r.top + 1.5));
    path.line_to((r.right - 6.0, r.top + 4.0));
    path.move_to((r.left + 3.5, r.top + 4.0));
    path.line_to((r.left + 4.5, r.bottom - 1.0));
    path.line_to((r.right - 4.5, r.bottom - 1.0));
    path.line_to((r.right - 3.5, r.top + 4.0));
    canvas.draw_path(&path.detach(), &stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_hits_tools_colours_and_the_bin() {
        let strip = strip_layout(800.0, 80.0);
        let (tool, rect) = strip.tools[2];
        assert_eq!(
            strip.hit(Point::new(rect.center_x(), rect.center_y())),
            Some(StripHit::Tool(tool))
        );
        let (_, swatch) = strip.colors[1];
        assert_eq!(
            strip.hit(Point::new(swatch.center_x(), swatch.center_y())),
            Some(StripHit::Color(1))
        );
        assert_eq!(
            strip.hit(Point::new(strip.delete.center_x(), strip.delete.center_y())),
            Some(StripHit::Delete)
        );
        // Everything fits the narrowest window.
        let narrow = strip_layout(crate::app::MIN_W, 80.0);
        let last = narrow.colors.last().unwrap().1;
        assert!(last.right < narrow.delete.left);
    }

    #[test]
    fn highlighter_yellow_text_is_written_dark() {
        assert_eq!(text_color(0), COLORS[COLORS.len() - 1]);
        assert_eq!(text_color(2), COLORS[2]);
    }

    /// Paint the toolbar with the markup strip under it, light and dark, and
    /// the signature pad, into `OTTO_RENDER_OUT` (default `.`), to look at.
    #[test]
    #[ignore = "writes PNGs to look at"]
    fn render_the_strip() {
        let out = std::path::PathBuf::from(
            std::env::var("OTTO_RENDER_OUT").unwrap_or_else(|_| ".".into()),
        );
        for dark in [false, true] {
            let size = (900.0, 260.0);
            let mut viewer = Viewer::new(std::path::PathBuf::from("/tmp/a.pdf"), size);
            viewer.markup = Some(MarkupTool::Highlight);
            viewer.ink = 2;
            let theme = if dark { Theme::dark() } else { Theme::light() };
            let mut surface = otto_kit::skia::surfaces::raster_n32_premul((
                (size.0 * 2.0) as i32,
                (size.1 * 2.0) as i32,
            ))
            .expect("a raster surface");
            let canvas = surface.canvas();
            canvas.scale((2.0, 2.0));
            canvas.clear(Color::from_rgb(0xD8, 0xD8, 0xDC));
            crate::chrome::draw(canvas, &viewer, &theme);
            let png = surface
                .image_snapshot()
                .encode(None, otto_kit::skia::EncodedImageFormat::PNG, None)
                .expect("a png");
            let name = if dark {
                "strip-dark.png"
            } else {
                "strip-light.png"
            };
            std::fs::write(out.join(name), png.as_bytes()).expect("written");
        }
        let size = (700.0, 420.0);
        let mut pad = crate::signature::Pad::default();
        let mut surface =
            otto_kit::skia::surfaces::raster_n32_premul((1400, 840)).expect("surface");
        let canvas = surface.canvas();
        canvas.scale((2.0, 2.0));
        canvas.clear(Color::from_rgb(0xD8, 0xD8, 0xDC));
        // A squiggle on the pad, from where the pad's area starts.
        let start = Point::new(size.0 / 2.0 - 180.0, size.1 / 2.0 - 10.0);
        pad.press(size, start);
        for step in 1..40 {
            let x = start.x + step as f32 * 8.0;
            let y = start.y + (step as f32 * 0.6).sin() * 22.0;
            pad.motion(size, Point::new(x, y));
        }
        pad.release(size, start);
        pad.draw(canvas, size, &Theme::light());
        let png = surface
            .image_snapshot()
            .encode(None, otto_kit::skia::EncodedImageFormat::PNG, None)
            .expect("a png");
        std::fs::write(out.join("pad.png"), png.as_bytes()).expect("written");
    }
}
