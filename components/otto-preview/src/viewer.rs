//! What the window shows and how it responds, apart from the window itself.
//!
//! The preview's own state is a Peek [`Session`]: zoom, pan, the document
//! strip, the word selection, the playing video. This wraps it with what a
//! window adds around it: its size and decoration, the toolbar's pointer
//! state, and the bookkeeping for work running off the UI thread.

// Rust guideline compliant 2026-02-21

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use otto_files::peek::{PageRequest, Session, VideoPointer};
use otto_kit::components::scroll::ScrollView;
use otto_kit::components::titlebar::{DecorationVariant, WindowControlsState};
use otto_kit::prelude::*;
use otto_kit::preview::{Preview, Zoom, ROW_HEIGHT};
use otto_kit::skia::{Contains, Image, Point};
use otto_kit::theme::ColorScheme;
use otto_kit::CursorShape;

use crate::chrome::{self, Tool};
use crate::marks::{self, Frame, Marks};
use crate::sidebar::{self, SidebarLayout};

/// How much one press of a zoom button or a zoom shortcut magnifies.
const ZOOM_STEP: f32 = 1.25;
/// What an arrow key moves the content by, in points.
const KEY_STEP: f32 = 48.0;
/// How many pages are rasterised at once. Enough that one slow page does not
/// hold up its neighbours, few enough that a fast scroll leaves no queue.
const PAGES_AT_ONCE: usize = 3;
/// How many thumbnails are rasterised at once, beside the pages.
const THUMBS_AT_ONCE: usize = 2;
/// How many thumbnails either side of the sidebar's box are made ready, and
/// how many further out are let go of.
const THUMBS_AHEAD: usize = 3;
const THUMBS_KEPT: usize = 12;

/// The window's whole state, shared between the draw, the pointer handler,
/// the update pass and the workers.
pub struct Viewer {
    /// The file, as an absolute path.
    pub path: PathBuf,
    /// The file's name: the window's title.
    pub name: String,
    pub session: Session,
    /// Whether the decode has been started.
    pub started: bool,
    /// The box the file was decoded for, kept so every later request for the
    /// same file (its text layer, recognised words) measures against it.
    pub decode_panel: Rect,
    /// The window's size in points.
    pub size: (f32, f32),
    pub variant: DecorationVariant,
    pub active: bool,
    pub modifiers: Modifiers,
    pub controls: WindowControlsState,
    pub hovered_tool: Option<Tool>,
    pub pressed_tool: Option<Tool>,
    /// Pages whose rasterising is in flight.
    pub pages_pending: HashSet<u32>,
    /// Whether the document's text layer has been asked for.
    pub text_asked: bool,
    /// Where the pointer last was, for a pinch that reports no position.
    pub pointer: Option<Point>,
    /// The zoom a pinch started from; the protocol reports scale against it.
    pub pinch_base: Option<f32>,
    /// A drag moving a zoomed picture, and where the pointer was last.
    pub drag: Option<Point>,
    pub cursor: CursorShape,
    /// The video state last drawn, so a new frame asks for a repaint.
    pub video_key: u64,
    /// Something changed that the window has not repainted yet.
    pub dirty: bool,
    /// The window was asked to close; the loop closes it on its next turn.
    pub closing: bool,
    /// The sidebar shown or hidden from the toolbar; `None` until then, when
    /// it shows for a document of more than one page.
    pub sidebar_choice: Option<bool>,
    /// The thumbnail column's scroll: the same physics as every other
    /// scrolled view, fling, rubber band and scrollbar included.
    pub sidebar_scroll: ScrollView,
    /// Whether the scroll gesture in progress began over the sidebar, so it
    /// stays with the column it started in until the fingers lift.
    pub wheel_in_sidebar: Option<bool>,
    /// The page the sidebar last brought into its box, so it follows the
    /// document without fighting a scroll of its own.
    pub sidebar_followed: u32,
    /// Rasterised thumbnails, by 1-based page.
    pub thumbs: HashMap<u32, Image>,
    /// Thumbnails whose rasterising is in flight.
    pub thumbs_pending: HashSet<u32>,
    /// Counts the decodes: work started for an earlier one lands after a
    /// reload has replaced it, and is dropped.
    pub generation: u64,
    /// The file as last decoded, to tell a real change from a neighbour's.
    pub stamp: Option<Stamp>,
    /// Whether the chat shows beside the document.
    pub chat_open: bool,
    /// The marks on the document: the person's and the agent's.
    pub marks: Marks,
    /// Whether a press on the document draws a mark rather than pans or
    /// selects: the pen in the toolbar.
    pub marking: bool,
    /// The chat panel as last painted, in window coordinates.
    pub chat_picture: Option<otto_kit::skia::Picture>,
    /// Whether the window grew to make room for the chat, and so shrinks
    /// back when it hides.
    pub chat_grew: bool,
    /// Whether the window sizes itself: floating, not maximized or tiled.
    pub floating: bool,
    /// The room the compositor last said a window has, in points.
    pub room: Option<(f32, f32)>,
    /// Where the view was when the file changed, put back once the new
    /// decode lands. Set from the reload until then.
    pub reloading: Option<Place>,
}

/// What tells one version of a file from the next without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    inode: u64,
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    /// The stamp of the file at `path` now; `None` while it is missing.
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            inode: metadata.ino(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Where the view is in the content: kept across a reload, so a file edited
/// elsewhere changes under the reader without moving them.
#[derive(Debug, Clone, Copy)]
pub struct Place {
    zoom: Zoom,
    first_row: usize,
    /// The page at the top of the box and how far down it, for a document.
    page: Option<(usize, f32)>,
}

/// What a key press asks the window to do beyond changing the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOutcome {
    Ignored,
    Handled,
    Close,
    Copy(String),
}

impl Viewer {
    /// A viewer on `path`, waiting for its decode.
    pub fn new(path: PathBuf, size: (f32, f32)) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let session = Session::waiting(
            name.clone(),
            path.is_dir(),
            Rect::new_empty(),
            Instant::now(),
        );
        Self {
            path,
            name,
            session,
            started: false,
            decode_panel: Rect::new_empty(),
            size,
            variant: DecorationVariant::Floating,
            active: true,
            modifiers: Modifiers::default(),
            controls: WindowControlsState::new(),
            hovered_tool: None,
            pressed_tool: None,
            pages_pending: HashSet::new(),
            text_asked: false,
            pointer: None,
            pinch_base: None,
            drag: None,
            cursor: CursorShape::Default,
            video_key: 0,
            dirty: true,
            closing: false,
            sidebar_choice: None,
            sidebar_scroll: ScrollView::new(Rect::new_empty()),
            wheel_in_sidebar: None,
            sidebar_followed: 0,
            thumbs: HashMap::new(),
            thumbs_pending: HashSet::new(),
            chat_open: false,
            marks: Marks::default(),
            marking: false,
            chat_picture: None,
            chat_grew: false,
            floating: true,
            room: None,
            generation: 0,
            stamp: None,
            reloading: None,
        }
    }

    /// Decode the file again because it changed on disk. The old content
    /// stays on screen until the new decode replaces it.
    pub fn reload(&mut self) {
        self.reloading = Some(Place {
            zoom: self.session.zoom,
            first_row: self.session.first_row,
            page: self.place(),
        });
        self.generation += 1;
        self.started = false;
        self.text_asked = false;
        self.pages_pending.clear();
        self.thumbs_pending.clear();
        self.thumbs.clear();
        self.dirty = true;
    }

    /// Put a decode that has landed into the window, unless a later one has
    /// been started since. Returns whether it was taken.
    pub fn land(&mut self, generation: u64, session: Session) -> bool {
        if generation != self.generation {
            return false;
        }
        self.session = session;
        if let Some(place) = self.reloading.take() {
            self.session.first_row = place.first_row;
            // Through the pan, so the scroll views follow the zoom; clamped
            // there to what the new content allows.
            let content = self.content();
            self.session.zoom = Zoom {
                scale: place.zoom.scale,
                ..Zoom::FIT
            };
            self.session
                .pan_by(place.zoom.offset.0, place.zoom.offset.1, content);
            if let Some(page) = place.page {
                self.restore_place(page);
            }
        }
        self.dirty = true;
        true
    }

    pub fn dark(&self) -> bool {
        otto_kit::color_scheme::current_color_scheme() == ColorScheme::Dark
    }

    /// The box the preview is drawn in.
    pub fn content(&self) -> Rect {
        let mut content = chrome::content_rect(self.size.0, self.size.1, self.variant);
        if self.chat_open {
            content.right = (content.right - crate::chat::WIDTH).max(content.left + 1.0);
        }
        if self.sidebar_open() {
            content.left = (content.left + sidebar::WIDTH).min(content.right - 1.0);
        }
        content
    }

    /// What the window shows, for the agent: the file, what kind of thing it
    /// is and its size in the units marks use, the page and the zoom, the
    /// marks waiting to be sent, and whether the file is being read again.
    pub fn info(&self) -> serde_json::Value {
        let (kind, size, pages) = match &self.session.preview {
            Preview::Pixels { pixels, .. } => (
                "picture",
                Some([
                    pixels.intrinsic_width as f32,
                    pixels.intrinsic_height as f32,
                ]),
                None,
            ),
            Preview::Pages { pages, .. } => ("pages", None, Some(pages.len())),
            _ if self.session.loading => ("loading", None, None),
            _ => ("other", None, None),
        };
        serde_json::json!({
            "file": self.path,
            "kind": kind,
            "size": size,
            "pages": pages,
            "page": self.page_status().map(|(page, _)| page),
            "zoom_percent": self.zoom_percent(),
            "marks": self.marks.list.len(),
            "pending_marks": self.marks.pending(),
            "reloading": self.reloading.is_some(),
        })
    }

    /// Where the document's own coordinates land on screen now: the picture
    /// or each page. Empty for what can't be marked.
    pub fn frames(&self) -> Vec<Frame> {
        if self.session.loading || self.session.video.is_some() {
            return Vec::new();
        }
        let layout = otto_kit::preview::layout(
            self.content(),
            &self.session.preview,
            self.session.first_row,
            self.session.zoom,
        );
        marks::frames(&self.session.preview, &layout)
    }

    /// Start a mark at `at`, when the pen is on. Shift draws a box.
    pub fn mark_press(&mut self, at: Point) -> bool {
        if !self.marking {
            return false;
        }
        let frames = self.frames();
        let shift = self.modifiers.shift;
        self.dirty |= self.marks.begin(&frames, at, shift);
        true
    }

    /// Follow the pointer over the marks' badges; `None` when it left.
    pub fn mark_hover(&mut self, at: Option<Point>) {
        let frames = self.frames();
        self.dirty |= self.marks.hover(&frames, at);
    }

    /// Delete the mark whose badge is under `at`. Returns whether one was.
    pub fn mark_delete_at(&mut self, at: Point) -> bool {
        let frames = self.frames();
        let Some(index) = self.marks.badge_at(&frames, at) else {
            return false;
        };
        self.dirty |= self.marks.remove(index);
        true
    }

    /// Carry a mark being drawn on to `at`.
    pub fn mark_motion(&mut self, at: Point) {
        self.dirty |= self.marks.extend(at);
    }

    /// Finish a mark being drawn.
    pub fn mark_release(&mut self) {
        self.dirty |= self.marks.finish();
    }

    /// The chat panel's box, when it shows: the window's trailing edge, under
    /// the chrome.
    pub fn chat_rect(&self) -> Option<Rect> {
        if !self.chat_open {
            return None;
        }
        let full = chrome::content_rect(self.size.0, self.size.1, self.variant);
        Some(Rect::from_ltrb(
            (full.right - crate::chat::WIDTH).max(full.left),
            full.top,
            full.right,
            full.bottom,
        ))
    }

    /// Show or hide the chat. A floating window with room to spare grows by
    /// the panel's width, so the document keeps its size, and shrinks back
    /// when the chat hides; otherwise the document narrows, keeping its place.
    /// Returns the size the window should take, when it changes.
    pub fn toggle_chat(&mut self) -> Option<(f32, f32)> {
        let place = self.place();
        self.chat_open = !self.chat_open;
        let (width, height) = self.size;
        let resized = if self.chat_open {
            let fits = self
                .room
                .is_none_or(|(room, _)| width + crate::chat::WIDTH <= room);
            self.chat_grew = self.floating && fits;
            self.chat_grew
                .then_some((width + crate::chat::WIDTH, height))
        } else {
            std::mem::take(&mut self.chat_grew)
                .then_some(((width - crate::chat::WIDTH).max(crate::app::MIN_W), height))
        };
        if let Some(size) = resized {
            self.size = size;
        }
        if let Some(place) = place {
            self.restore_place(place);
        }
        self.dirty = true;
        resized
    }

    /// Whether the preview is a document the sidebar can show pages of.
    pub fn has_pages(&self) -> bool {
        !self.session.loading && !self.session.pages().is_empty()
    }

    /// Whether the pages sidebar is showing: as chosen from the toolbar, or
    /// by default for a document of more than one page.
    pub fn sidebar_open(&self) -> bool {
        self.has_pages()
            && self
                .sidebar_choice
                .unwrap_or(self.session.pages().len() > 1)
    }

    /// The sidebar's thumbnails where they are drawn now, when it is open.
    pub fn sidebar(&self) -> Option<SidebarLayout> {
        if !self.sidebar_open() {
            return None;
        }
        let full = chrome::content_rect(self.size.0, self.size.1, self.variant);
        let rect = Rect::from_ltrb(
            full.left,
            full.top,
            (full.left + sidebar::WIDTH).min(full.right),
            full.bottom,
        );
        let sizes: Vec<(f32, f32)> = self
            .session
            .pages()
            .iter()
            .map(|page| (page.width, page.height))
            .collect();
        Some(sidebar::layout(rect, &sizes, self.sidebar_scroll.offset()))
    }

    /// Tell the column's scroll what it scrolls: the sidebar's box and the
    /// column's height, which a resize or a newly opened document changes.
    fn fit_sidebar_scroll(&mut self) {
        if let Some(layout) = self.sidebar() {
            self.sidebar_scroll.set_viewport(layout.rect);
            self.sidebar_scroll.set_content_length(layout.column);
        }
    }

    /// Show or hide the sidebar, keeping the document on the same spot of
    /// the same page while the content box changes width under it.
    pub fn toggle_sidebar(&mut self) {
        if !self.has_pages() {
            return;
        }
        let place = self.place();
        self.sidebar_choice = Some(!self.sidebar_open());
        self.sidebar_followed = 0;
        if let Some(place) = place {
            self.restore_place(place);
        }
        self.dirty = true;
    }

    /// Scroll the document so the content box's top edge falls `along` the
    /// way down page `index`, as [`Self::place`] measured it.
    fn restore_place(&mut self, (index, along): (usize, f32)) {
        let content = self.content();
        let layout = otto_kit::preview::layout(
            content,
            &self.session.preview,
            self.session.first_row,
            self.session.zoom,
        );
        if let Some(rect) = layout.page_rects.get(index) {
            let target = rect.top + along * rect.height();
            self.session.pan_by(0.0, layout.inner.top - target, content);
        }
    }

    /// The page at the top of the content box and how far down it the box's
    /// top edge falls, as a share of its height.
    fn place(&self) -> Option<(usize, f32)> {
        let content = self.content();
        let layout = otto_kit::preview::layout(
            content,
            &self.session.preview,
            self.session.first_row,
            self.session.zoom,
        );
        let index = self.session.showing_page(content);
        let rect = layout.page_rects.get(index)?;
        (rect.height() > 0.0).then(|| (index, (layout.inner.top - rect.top) / rect.height()))
    }

    /// Whether a wheel or touchpad scroll at `at` is the sidebar's: the
    /// pane a gesture began over keeps it until it ends, as the pan does.
    pub fn wheel_goes_to_sidebar(&mut self, at: Point, stop: bool, discrete: bool) -> bool {
        let over = self
            .sidebar()
            .is_some_and(|layout| layout.rect.contains(at));
        if discrete {
            return over;
        }
        let sidebar = *self.wheel_in_sidebar.get_or_insert(over);
        if stop {
            self.wheel_in_sidebar = None;
        }
        sidebar && self.sidebar_open()
    }

    /// A wheel or touchpad scroll of the thumbnail column.
    pub fn sidebar_wheel(&mut self, dy: f32, stop: bool, discrete: bool) {
        self.fit_sidebar_scroll();
        let view = &mut self.sidebar_scroll;
        self.dirty |= if stop {
            // Fingers off the touchpad: the gesture's speed becomes a
            // fling, and a pull past an end springs back.
            view.on_wheel_end();
            true
        } else if discrete {
            view.on_wheel_discrete(dy)
        } else {
            view.on_wheel(dy)
        };
    }

    /// The pointer moved over the window: drag the column's scrollbar thumb,
    /// or let the bar know it is hovered.
    pub fn sidebar_motion(&mut self, at: Point) {
        if !self.sidebar_open() {
            return;
        }
        let view = &mut self.sidebar_scroll;
        self.dirty |= view.on_pointer_drag(at.x, at.y) || view.on_pointer_move(at.x, at.y);
    }

    /// The button came up: a thumb drag ends.
    pub fn sidebar_release(&mut self) {
        self.sidebar_scroll.on_pointer_up();
    }

    /// A press in the sidebar: grab the scrollbar's thumb, or go to the page
    /// under it.
    pub fn sidebar_press(&mut self, at: Point) {
        self.fit_sidebar_scroll();
        if self.sidebar_scroll.on_pointer_down(at.x, at.y) {
            self.dirty = true;
            return;
        }
        let Some(page) = self.sidebar().and_then(|layout| layout.page_at(at)) else {
            return;
        };
        let content = self.content();
        self.dirty |= self.session.scroll_to_page(page, content);
        // The pressed thumbnail is in view already; following it would only
        // nudge the column.
        self.sidebar_followed = page;
        self.dirty = true;
    }

    /// Keep the page showing in the sidebar's box as the document scrolls.
    fn follow_in_sidebar(&mut self) -> bool {
        let (Some(layout), Some((page, _))) = (self.sidebar(), self.page_status()) else {
            return false;
        };
        if page == self.sidebar_followed {
            return false;
        }
        self.sidebar_followed = page;
        let offset = self.sidebar_scroll.offset();
        let scroll = layout.scroll_to_show(page, offset);
        scroll != offset && self.sidebar_scroll.scroll_to(scroll)
    }

    /// The thumbnails the sidebar wants rasterised next, marked as in flight,
    /// with the file they come from. Thumbnails scrolled well out of the box
    /// are let go of.
    pub fn thumb_work(&mut self, scale: f32) -> Option<(Vec<PageRequest>, PathBuf)> {
        if self.reloading.is_some() {
            return None;
        }
        let layout = self.sidebar()?;
        let kept = layout.pages_in_view(THUMBS_KEPT);
        self.thumbs.retain(|page, _| kept.contains(page));
        let width = (sidebar::THUMB_W * scale).ceil() as u32;
        let wanted: Vec<PageRequest> = layout
            .pages_in_view(THUMBS_AHEAD)
            .filter(|page| !self.thumbs.contains_key(page) && !self.thumbs_pending.contains(page))
            .take(THUMBS_AT_ONCE.saturating_sub(self.thumbs_pending.len()))
            .map(|page| PageRequest { page, width })
            .collect();
        if wanted.is_empty() {
            return None;
        }
        for request in &wanted {
            self.thumbs_pending.insert(request.page);
        }
        Some((wanted, self.path.clone()))
    }

    /// Put a rasterised thumbnail in the sidebar.
    pub fn finish_thumb(&mut self, generation: u64, page: u32, image: Option<Image>) {
        if generation != self.generation {
            return;
        }
        self.thumbs_pending.remove(&page);
        if let Some(image) = image {
            self.thumbs.insert(page, image);
            self.dirty |= self.sidebar_open();
        }
    }

    /// The page showing and how many there are, for a document strip.
    ///
    /// Only a strip pages here: every page is laid out already, so a turn is
    /// a scroll. A single-page picture of a paginated file would need a new
    /// decode per page, which this window never asks for.
    pub fn page_status(&self) -> Option<(u32, u32)> {
        if self.session.pages().is_empty() {
            return None;
        }
        self.session.paged(self.content())
    }

    /// Whether the preview can be magnified: a picture or a document's pages.
    /// Text, listings, cards and a decode still in flight are laid out to fit.
    pub fn zoomable(&self) -> bool {
        !self.session.loading
            && self.session.video.is_none()
            && matches!(
                self.session.preview,
                Preview::Pixels { .. } | Preview::Pages { .. }
            )
    }

    /// How large the preview is drawn against its own size, in percent: a
    /// page at one point per point, a picture at one pixel per point, is
    /// 100. `None` for what is not zoomed: text, listings, a decode in flight.
    pub fn zoom_percent(&self) -> Option<u32> {
        if !self.zoomable() {
            return None;
        }
        let layout = otto_kit::preview::layout(
            self.content(),
            &self.session.preview,
            self.session.first_row,
            self.session.zoom,
        );
        let ratio = match &self.session.preview {
            Preview::Pages { pages, .. } => {
                let page = pages.first()?;
                layout.page_rects.first()?.width() / page.width
            }
            Preview::Pixels { pixels, .. } => {
                layout.content.width() / pixels.intrinsic_width as f32
            }
            _ => return None,
        };
        (ratio.is_finite() && ratio > 0.0).then(|| (ratio * 100.0).round() as u32)
    }

    pub fn tool_enabled(&self, tool: Tool) -> bool {
        let scale = self.session.zoom.scale;
        match tool {
            Tool::ZoomOut | Tool::ZoomFit => self.zoomable() && scale > 1.0,
            Tool::ZoomIn => self.zoomable() && scale < otto_kit::preview::Zoom::MAX,
            Tool::PreviousPage => self.page_status().is_some_and(|(page, _)| page > 1),
            Tool::NextPage => self.page_status().is_some_and(|(page, pages)| page < pages),
            Tool::Sidebar => self.has_pages(),
            Tool::Chat => true,
            Tool::Mark => self.marking || !self.frames().is_empty(),
        }
    }

    /// The toolbar's buttons where they are drawn now.
    pub fn toolbar(&self) -> chrome::ToolbarLayout {
        chrome::toolbar_layout(
            self.size.0,
            self.variant,
            self.page_status().is_some(),
            self.has_pages(),
        )
    }

    /// Zoom to `scale` about `focus`, a window-local point. Returns whether
    /// anything moved.
    pub fn zoom_about(&mut self, scale: f32, focus: (f32, f32)) -> bool {
        if !self.zoomable() {
            return false;
        }
        let content = self.content();
        let moved = self.session.zoom_to(scale, focus, content);
        self.dirty |= moved;
        moved
    }

    /// Zoom by `factor` about the middle of the content.
    pub fn zoom_by(&mut self, factor: f32) -> bool {
        let content = self.content();
        let scale = self.session.zoom.scale * factor;
        self.zoom_about(scale, (content.center_x(), content.center_y()))
    }

    /// Back to the whole picture, or the page's full width.
    pub fn zoom_fit(&mut self) -> bool {
        let content = self.content();
        self.zoom_about(1.0, (content.center_x(), content.center_y()))
    }

    /// Turn `delta` pages. Returns whether there was a page to turn to.
    pub fn turn_page(&mut self, delta: i32) -> bool {
        let content = self.content();
        let Some(page) = self.session.page_turn(delta, content) else {
            return false;
        };
        let moved = self.session.scroll_to_page(page, content);
        self.dirty |= moved;
        moved
    }

    /// Move the content by `dx`, `dy` points: a pan for anything that pans,
    /// and rows for a listing or plain text.
    pub fn move_by(&mut self, dx: f32, dy: f32) -> bool {
        let content = self.content();
        if self.session.pannable(content) {
            let moved = self.session.pan_by(dx, dy, content);
            self.dirty |= moved;
            return moved;
        }
        let rows = (-dy / ROW_HEIGHT).round() as i32;
        if rows == 0 {
            return false;
        }
        let before = self.session.first_row;
        self.session.scroll_by(rows, content);
        let moved = self.session.first_row != before;
        self.dirty |= moved;
        moved
    }

    /// A wheel or two-finger scroll. With Ctrl held it zooms about the
    /// pointer instead.
    pub fn wheel(&mut self, dx: f32, dy: f32, stop: bool, discrete: bool, at: Point) {
        let content = self.content();
        if self.modifiers.ctrl {
            if stop || dy == 0.0 {
                return;
            }
            let factor = if discrete {
                if dy < 0.0 {
                    ZOOM_STEP
                } else {
                    1.0 / ZOOM_STEP
                }
            } else {
                (-dy * 0.01).exp()
            };
            let scale = self.session.zoom.scale * factor;
            self.zoom_about(scale, (at.x, at.y));
            return;
        }
        if self.session.pannable(content) {
            self.session.pan_wheel(dx, dy, content, stop, discrete);
        } else if !stop {
            let rows = (dy / ROW_HEIGHT).round() as i32;
            self.session.scroll_by(rows, content);
        }
        self.dirty = true;
    }

    /// A pointer event over the content. Returns whether the content took it,
    /// and whether the viewer wants a link opened.
    pub fn content_pointer(&mut self, kind: VideoPointer, at: Point) -> Option<std::ffi::OsString> {
        let content = self.content();
        let session = &mut self.session;
        // A video's transport comes first; it and the pan never share a
        // preview.
        if let Some(handled) = session.video_pointer(kind, at.x, at.y, content) {
            self.dirty |= handled;
            if handled || kind != VideoPointer::Press {
                return None;
            }
        }
        match kind {
            VideoPointer::Press => {
                if session.link_pointer_down(at.x, at.y, content) {
                    return None;
                }
                if session.pan_pointer_down(at.x, at.y, content) {
                    self.dirty = true;
                    return None;
                }
                let selected = session.select_pointer_down(at.x, at.y, content);
                self.dirty |= selected;
                // A press on no word grabs the picture, when there is
                // somewhere to move it.
                if !session.selecting && session.pannable(content) {
                    self.drag = Some(at);
                }
                None
            }
            VideoPointer::Motion => {
                if let Some(from) = self.drag {
                    self.drag = Some(at);
                    let moved = session.pan_by(at.x - from.x, at.y - from.y, content);
                    self.dirty |= moved;
                    return None;
                }
                let panned = session.pan_pointer_move(at.x, at.y, content);
                let selected = session.select_pointer_move(at.x, at.y, content);
                self.dirty |= panned || selected;
                None
            }
            VideoPointer::Release => {
                self.drag = None;
                session.pan_pointer_up();
                session.select_pointer_up();
                session.link_pointer_up(at.x, at.y, content)
            }
            VideoPointer::Leave => {
                self.drag = None;
                session.link_pointer_leave();
                session.pan_pointer_up();
                session.pan_pointer_leave();
                session.select_pointer_up();
                None
            }
        }
    }

    /// The cursor the content wants at `at`.
    pub fn content_cursor(&self, at: Point) -> CursorShape {
        let content = self.content();
        if self.drag.is_some() {
            return CursorShape::Grabbing;
        }
        if self.marks.hovered.is_some() {
            // The bin: a click here deletes the mark (see `crate::cursors`).
            return CursorShape::NotAllowed;
        }
        if self.marking {
            return CursorShape::Crosshair;
        }
        if self.session.link_at(at.x, at.y, content).is_some() {
            CursorShape::Pointer
        } else if self.session.word_at(at.x, at.y, content).is_some() {
            CursorShape::Text
        } else {
            CursorShape::Default
        }
    }

    /// Run a toolbar button.
    pub fn run_tool(&mut self, tool: Tool) {
        if !self.tool_enabled(tool) {
            return;
        }
        match tool {
            Tool::ZoomOut => {
                self.zoom_by(1.0 / ZOOM_STEP);
            }
            Tool::ZoomFit => {
                self.zoom_fit();
            }
            Tool::ZoomIn => {
                self.zoom_by(ZOOM_STEP);
            }
            Tool::PreviousPage => {
                self.turn_page(-1);
            }
            Tool::NextPage => {
                self.turn_page(1);
            }
            Tool::Sidebar => self.toggle_sidebar(),
            Tool::Chat => {
                self.toggle_chat();
            }
            Tool::Mark => {
                self.marking = !self.marking;
                self.dirty = true;
            }
        }
    }

    /// A key press, already filtered to presses.
    pub fn key(&mut self, keysym: Keysym) -> KeyOutcome {
        let ctrl = self.modifiers.ctrl;
        let content = self.content();
        let screen = (content.height() - KEY_STEP).max(KEY_STEP);
        match keysym {
            Keysym::Escape if self.marking => {
                self.marking = false;
                self.dirty = true;
                KeyOutcome::Handled
            }
            // The person's last mark, until it goes with a message.
            Keysym::BackSpace | Keysym::Delete if !self.marks.pending().is_empty() => {
                self.dirty |= self.marks.undo();
                KeyOutcome::Handled
            }
            Keysym::w | Keysym::W | Keysym::q | Keysym::Q if ctrl => KeyOutcome::Close,
            Keysym::c | Keysym::C if ctrl => match self.session.selected_text() {
                Some(text) => KeyOutcome::Copy(text),
                None => KeyOutcome::Ignored,
            },
            Keysym::a | Keysym::A if ctrl => {
                self.dirty |= self.session.select_all_words();
                KeyOutcome::Handled
            }
            Keysym::equal | Keysym::plus | Keysym::KP_Add if ctrl => {
                self.zoom_by(ZOOM_STEP);
                KeyOutcome::Handled
            }
            Keysym::minus | Keysym::underscore | Keysym::KP_Subtract if ctrl => {
                self.zoom_by(1.0 / ZOOM_STEP);
                KeyOutcome::Handled
            }
            Keysym::_0 | Keysym::KP_0 if ctrl => {
                self.zoom_fit();
                KeyOutcome::Handled
            }
            Keysym::Escape => {
                self.dirty |= self.session.clear_selection();
                KeyOutcome::Handled
            }
            Keysym::space if self.session.video.is_some() => {
                if let Some(video) = &mut self.session.video {
                    video.player.toggle();
                }
                self.dirty = true;
                KeyOutcome::Handled
            }
            Keysym::Page_Down | Keysym::space => {
                if !self.turn_page(1) {
                    self.move_by(0.0, -screen);
                }
                KeyOutcome::Handled
            }
            Keysym::Page_Up => {
                if !self.turn_page(-1) {
                    self.move_by(0.0, screen);
                }
                KeyOutcome::Handled
            }
            Keysym::Down => {
                self.move_by(0.0, -KEY_STEP);
                KeyOutcome::Handled
            }
            Keysym::Up => {
                self.move_by(0.0, KEY_STEP);
                KeyOutcome::Handled
            }
            Keysym::Right => {
                if !self.move_by(-KEY_STEP, 0.0) {
                    self.turn_page(1);
                }
                KeyOutcome::Handled
            }
            Keysym::Left => {
                if !self.move_by(KEY_STEP, 0.0) {
                    self.turn_page(-1);
                }
                KeyOutcome::Handled
            }
            // Further than any content reaches: both stop at the end.
            Keysym::Home => {
                self.move_by(0.0, f32::MAX / 4.0);
                KeyOutcome::Handled
            }
            Keysym::End => {
                self.move_by(0.0, -f32::MAX / 4.0);
                KeyOutcome::Handled
            }
            _ => KeyOutcome::Ignored,
        }
    }

    /// The pages the document wants rasterised next, marked as in flight,
    /// and whether its text layer is still to be read. `None` when there is
    /// nothing to do.
    pub fn document_work(&mut self, scale: f32) -> Option<(Vec<PageRequest>, bool)> {
        if self.reloading.is_some() || self.session.pages().is_empty() {
            return None;
        }
        let content = self.content();
        let wanted: Vec<PageRequest> = self
            .session
            .pages_wanted(content, scale)
            .into_iter()
            .filter(|request| !self.pages_pending.contains(&request.page))
            .take(PAGES_AT_ONCE.saturating_sub(self.pages_pending.len()))
            .collect();
        let text = !self.text_asked;
        if wanted.is_empty() && !text {
            return None;
        }
        self.text_asked = true;
        for request in &wanted {
            self.pages_pending.insert(request.page);
        }
        Some((wanted, text))
    }

    /// Put a rasterised page in the strip.
    pub fn finish_page(
        &mut self,
        generation: u64,
        page: u32,
        pixels: Option<otto_kit::preview::Pixels>,
    ) {
        if generation != self.generation {
            return;
        }
        self.pages_pending.remove(&page);
        let content = self.content();
        if let Some(pixels) = pixels {
            self.dirty |= self.session.attach_page(page, pixels, content);
        }
    }

    /// Advance whatever moves on its own: a fling, an animation, a video, the
    /// recogniser's badge. Returns whether a repaint is due.
    pub fn tick(&mut self) -> bool {
        let content = self.content();
        let mut moved = self.session.tick_pan(content);
        moved |= self.session.tick_animation();
        moved |= self.session.recognising_since.is_some();
        self.fit_sidebar_scroll();
        moved |= self.follow_in_sidebar();
        if self.sidebar_open() && self.sidebar_scroll.is_animating() {
            moved |= self.sidebar_scroll.tick();
        }
        let video = self.session.video_key();
        if video != self.video_key {
            self.video_key = video;
            moved = true;
        }
        self.dirty |= moved;
        std::mem::take(&mut self.dirty)
    }

    /// Whether something keeps moving without input, so the loop needs a
    /// clock rather than waiting for an event.
    pub fn animating(&self) -> bool {
        self.session.pan_animating()
            || self.session.frames_running()
            || self.session.recognising_since.is_some()
            || (self.sidebar_open() && self.sidebar_scroll.is_animating())
    }
}

pub use smithay_client_toolkit::seat::keyboard::Keysym;

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(name: &str) -> Session {
        Session::waiting(name.to_string(), false, Rect::new_empty(), Instant::now())
    }

    #[test]
    fn a_decode_started_before_a_reload_is_dropped() {
        let mut viewer = Viewer::new(PathBuf::from("/tmp/a.png"), (800.0, 600.0));
        let first = viewer.generation;
        viewer.reload();
        assert!(!viewer.started);
        assert!(!viewer.land(first, waiting("old")));
        assert!(
            viewer.reloading.is_some(),
            "still waiting for the new decode"
        );
        assert!(viewer.land(viewer.generation, waiting("new")));
        assert!(viewer.reloading.is_none());
        assert_eq!(viewer.session.name, "new");
    }

    #[test]
    fn with_room_the_window_grows_for_the_chat_and_shrinks_back() {
        let mut viewer = Viewer::new(PathBuf::from("/tmp/a.png"), (960.0, 720.0));
        viewer.room = Some((1920.0, 1080.0));
        let document = viewer.content();
        assert_eq!(
            viewer.toggle_chat(),
            Some((960.0 + crate::chat::WIDTH, 720.0))
        );
        assert_eq!(viewer.content(), document, "the document keeps its size");
        let panel = viewer.chat_rect().unwrap();
        assert_eq!(panel.left, document.right);
        assert_eq!(panel.width(), crate::chat::WIDTH);
        assert_eq!(viewer.toggle_chat(), Some((960.0, 720.0)));
    }

    #[test]
    fn without_room_the_chat_takes_its_width_from_the_document() {
        let mut viewer = Viewer::new(PathBuf::from("/tmp/a.png"), (960.0, 720.0));
        viewer.room = Some((1100.0, 1080.0));
        let wide = viewer.content();
        assert_eq!(viewer.toggle_chat(), None);
        let panel = viewer.chat_rect().unwrap();
        assert_eq!(panel.right, 960.0);
        assert_eq!(viewer.content().right, wide.right - crate::chat::WIDTH);
        assert_eq!(panel.top, wide.top);
        assert_eq!(viewer.toggle_chat(), None, "nothing to shrink back");
    }

    #[test]
    fn no_pages_are_asked_for_while_reloading() {
        let mut viewer = Viewer::new(PathBuf::from("/tmp/a.pdf"), (800.0, 600.0));
        viewer.reload();
        assert!(viewer.document_work(1.0).is_none());
        assert!(viewer.thumb_work(1.0).is_none());
    }

    #[test]
    fn a_rewritten_file_has_a_new_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "one").unwrap();
        let before = Stamp::of(&path);
        // Written to a new file and renamed over, as most tools save.
        let next = dir.path().join(".note.md.tmp");
        std::fs::write(&next, "two, longer").unwrap();
        std::fs::rename(&next, &path).unwrap();
        assert!(before.is_some());
        assert_ne!(Stamp::of(&path), before);
        assert_eq!(Stamp::of(&dir.path().join("missing")), None);
    }
}
