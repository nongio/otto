use crate::{model, pane_surfaces, perf, picker, quickview, scene, thumbcache, thumbnails, view};

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use otto_kit::clipboard;
use otto_kit::components::context_menu::ContextMenu;
use otto_kit::components::scroll::{Axis, ScrollView};
use otto_kit::components::titlebar::{WindowControl, WindowControlsState};
use otto_kit::components::window::resize;
use otto_kit::prelude::*;
use otto_kit::CursorShape;
use skia_safe::Contains;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use smithay_client_toolkit::shell::xdg::{XdgPositioner, XdgSurface};
use wayland_client::protocol::wl_keyboard;

/// `BTN_RIGHT` from `linux/input-event-codes.h` — a right-click opens the
/// context menu instead of doing whatever the same spot does on the left
/// button.
const BTN_RIGHT: u32 = 0x111;

use model::{Column, Entry, Place, SortKey};
use view::ViewMode;

/// How soon a second press on the same column divider must land to count as
/// a double-click rather than the start of a fresh drag.
const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

/// The browser's whole state. Shared with the draw and input callbacks, which
/// outlive any borrow this struct could hand out.
struct Browser {
    /// The path stack: `[root, …, deepest]`. Miller columns render all of it;
    /// the list renders the last. Navigation pushes and pops in both views, so
    /// switching between them keeps the user where they were.
    columns: Vec<Column>,
    /// Which column has the keyboard.
    active: usize,
    places: Vec<Place>,
    mode: ViewMode,
    sort: SortKey,
    ascending: bool,
    show_hidden: bool,
    /// The list view's draggable Size/Kind/Modified column widths.
    list_columns: view::ListColumnWidths,
    /// A column divider currently being dragged: which one, the pointer x it
    /// started at, and the width it started with.
    column_resize: Option<(view::ColumnBoundary, f32, f32)>,
    /// The Miller view's shared, draggable pane width.
    miller_w: f32,
    /// A Miller pane divider being dragged: its depth, the pointer x it
    /// started at, and the width it started with.
    miller_resize: Option<(usize, f32, f32)>,
    /// The last Miller divider clicked and when, so a second click shortly
    /// after reads as a double-click rather than a fresh drag.
    last_miller_click: Option<(usize, std::time::Instant)>,
    /// The last row/cell clicked (depth, index) and when — List and Grid have
    /// no Miller-style eager descent, so a directory only opens on a second
    /// click landing on the same one within the window.
    last_row_click: Option<(usize, usize, std::time::Instant)>,
    /// The last boundary clicked and when, so a second click on the same one
    /// shortly after reads as a double-click rather than a fresh drag.
    last_boundary_click: Option<(view::ColumnBoundary, std::time::Instant)>,
    /// Horizontal pan of the Miller stack, as a scroll view of its own: the
    /// stack is one continuous strip that happens to be divided into panes,
    /// so panning it is a scroll, with the same momentum, rubber banding and
    /// overlay bar the panes scroll vertically with. Its offset is how far
    /// the strip is pushed left, in points.
    ///
    /// Navigation ([`view::miller_pan_for`]) drives it too, through
    /// `scroll_to`, which lands on an exact column edge and drops whatever
    /// fling was in flight.
    pan: ScrollView,
    /// Which axis the touchpad gesture in progress belongs to, decided by its
    /// first delta and held until it ends. Deciding per event instead lets a
    /// diagonal swipe flip-flop between panning the stack and scrolling a
    /// pane, several times a second.
    gesture_axis: Option<Axis>,
    size: (f32, f32),
    /// What a cut or copy put aside. Internal to this application — see
    /// [`model::Clipboard`].
    clipboard: model::Clipboard,
    /// The last operation's outcome, shown in the header until the next action.
    status: Option<String>,
    /// The open preview, if one is up.
    quickview: Option<quickview::Session>,
    /// The docked preview column's state, for the entry currently under a
    /// single-item selection. `None` both when the column is hidden (no
    /// selection, or not enough room for it) and briefly while a fresh
    /// selection's decode is still in flight — [`PreviewPaneState::pending`]
    /// tells the two apart.
    preview: Option<PreviewPaneState>,
    /// Bumped for every preview decode started, independent of Quick View's
    /// own generation counter — the two panels can be open at once.
    preview_generation_seed: u64,
    /// Thumbnails for the entries on screen, in place of their type icons.
    ///
    /// Only the visible ones are ever fetched, and only a few at a time — see
    /// [`thumbnails::Store`]. The store is asked what it wants on every update
    /// and the host runs the work off the UI thread, the same shape the
    /// preview column's decodes take.
    thumbs: thumbnails::Store,
    /// A decode is in flight. Keeps the frame loop alive so its result is
    /// painted without waiting for the next input.
    quickview_pending: bool,
    /// A dismissed preview still on screen, shrinking back to its file.
    quickview_closing: Option<quickview::Session>,
    /// Open Quick View on the first entry as soon as one is listed, so the
    /// panel can be looked at without anyone pressing a key. Driving the real
    /// keyboard means injecting into whatever session the test runs in, which
    /// is both unreliable and rude to whoever is using that desktop.
    /// `OTTO_FILES_QV_AUTO=1`.
    quickview_auto: bool,
    /// Bumped for every decode started. A result arriving with a stale
    /// generation is dropped: arrow-keying is much faster than decoding, and a
    /// slow PDF must not land on top of a file the user moved off three keys
    /// ago.
    quickview_generation: u64,
    /// The Get Info panel, when one is open. Not modal: it is a window of its
    /// own that floats over the browser, and the browser goes on working
    /// underneath it.
    info: Option<model::FileInfo>,
    /// Why the last permission change was refused, if it was.
    info_error: Option<String>,
    /// Pointer is over the panel's close dot, so its × glyph reveals — the
    /// same hover behaviour as the window's own traffic lights.
    info_close_hovered: bool,
    /// Set when the panel's own window has something new to show — it is a
    /// separate window with a separate buffer, so the browser's own `dirty`
    /// says nothing about it.
    info_dirty: bool,
    /// Hover and press state of the traffic lights, so they reveal their
    /// glyphs under the pointer the way the compositor's own decoration does.
    controls: WindowControlsState,
    /// The Back/Forward half being held down. Like the traffic lights, the
    /// arrows arm on press and fire on release over the same half, so a press
    /// dragged off the button changes nothing.
    nav_pressed: Option<view::NavButton>,
    /// An in-place rename in progress. List view only, for now.
    rename: Option<RenameSession>,
    /// The type-ahead buffer and when it was last appended to: typing
    /// printable characters walks the cursor to the entry that starts with
    /// them, without filtering the view or showing anything. Distinct from
    /// search, which is Ctrl+F and changes what is displayed.
    typeahead: Option<(String, std::time::Instant)>,
    /// A Back/Forward step landed and its panes are still being read. The
    /// remembered cursor is an index into a list that does not exist until
    /// those reads finish, so it is re-derived — and scrolled into view —
    /// once they do.
    pending_restore: bool,
    /// Locations left behind by Back, most recent last. Forward pops them back.
    back: Vec<Location>,
    /// Locations left behind by Forward, most recent last. Back pops them back.
    forward: Vec<Location>,
    /// Set when something changed and the window needs repainting.
    dirty: bool,
    /// The portal request this window is serving, when it is a picker rather
    /// than the browser. `None` is the browser, and every difference between
    /// the two shells reads off this one field.
    picker: Option<picker::Session>,
    /// The action row's hover and press state, tracked like the traffic
    /// lights': a button arms on press and fires on release over the same
    /// button, so a press dragged off it changes nothing.
    footer_hover: Option<view::FooterButton>,
    footer_pressed: Option<view::FooterButton>,
    /// Pointer is over Quick View's close button.
    quickview_close_hovered: bool,
    /// Where Quick View's panel actually is, in window coordinates.
    ///
    /// Written by the render path, read by the pointer handler, because the
    /// two cannot otherwise agree: a panel centred on the *display* is placed
    /// from an answer only [`pane_surfaces`] has, and the pointer callback
    /// outlives any borrow of it. `None` falls back to the window's centre,
    /// which is where the panel is when it is not centred on the display.
    quickview_panel: Option<Rect>,
    /// The pointer's last position over the Quick View panel, together with
    /// the panel rect it was measured against.
    ///
    /// The two are stored as a pair because they are not always in the same
    /// space: the panel takes its own input when it is centred on the display
    /// and the toplevel takes it otherwise, and those two report positions in
    /// two different coordinate systems. Whichever handler saw the pointer
    /// records the panel *it* was hit-testing against, so a pinch can work in
    /// that space without having to know which handler it came from.
    quickview_focus: Option<(skia_safe::Point, Rect)>,
    /// The zoom a pinch in progress started from, if one is.
    ///
    /// `zwp_pointer_gesture_pinch_v1` reports its scale against the start of
    /// the gesture rather than against the last update, so the zoom it is
    /// asking for is this times that — and applying it incrementally instead
    /// would compound rounding across a gesture that can run for seconds.
    quickview_pinch: Option<f32>,
}

/// What a pointer event over the Quick View panel is, as far as the pan's
/// scrollbars are concerned. The two handlers that can deliver one — the
/// toplevel's and the panel's own surface — funnel into
/// [`Browser::quickview_pan_pointer`] through this, so a bar behaves the same
/// whichever of them the compositor picked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum QuickviewPointer {
    Press,
    Motion,
    Release,
    Leave,
}

/// A snapshot of the column stack, for the Back/Forward pair. The whole
/// breadcrumb is kept, not just the deepest path, so returning to a Miller
/// view location restores every pane that was open, not just the last one.
struct Location {
    columns: Vec<ColumnState>,
    active: usize,
}

/// One pane of a remembered [`Location`]: where it was pointed and what was
/// picked in it. Going back to a directory you were just in should put you
/// back where you were standing in it, cursor and selection included — not
/// at the top of an unselected list.
struct ColumnState {
    path: PathBuf,
    selection: std::collections::BTreeSet<String>,
    cursor: Option<usize>,
    anchor: Option<usize>,
}

/// What the docked preview column is showing, for the entry at `path`.
struct PreviewPaneState {
    path: PathBuf,
    generation: u64,
    /// A decode for `path` is in flight.
    pending: bool,
    decoded: Option<otto_kit::preview::Preview>,
}

/// An in-place rename in progress: which row it belongs to and the text
/// field editing its name.
struct RenameSession {
    depth: usize,
    index: usize,
    original: PathBuf,
    input: TextInput,
}

/// The byte range an in-place rename should start with selected: the stem,
/// so typing replaces the base name and leaves the extension alone — the way
/// Finder and Explorer both do it. A directory has no extension to protect,
/// and a leading dot (`.bashrc`) is not an extension separator, so both keep
/// the whole name selected.
fn rename_selection(name: &str, is_dir: bool) -> std::ops::Range<usize> {
    if is_dir {
        return 0..name.len();
    }
    match name.rfind('.') {
        Some(0) | None => 0..name.len(),
        Some(dot) => 0..dot,
    }
}

impl Browser {
    fn new(start: PathBuf) -> Self {
        Self {
            columns: vec![Column::new(start)],
            active: 0,
            places: model::places(),
            mode: ViewMode::Columns,
            sort: SortKey::Name,
            ascending: true,
            show_hidden: false,
            list_columns: view::ListColumnWidths::default(),
            column_resize: None,
            miller_w: view::MILLER_W,
            miller_resize: None,
            last_miller_click: None,
            last_row_click: None,
            last_boundary_click: None,
            rename: None,
            typeahead: None,
            pending_restore: false,
            back: Vec::new(),
            forward: Vec::new(),
            nav_pressed: None,
            pan: ScrollView::horizontal(view::content_viewport(
                view::WINDOW_W,
                view::WINDOW_H,
                ViewMode::Columns,
            )),
            gesture_axis: None,
            size: (view::WINDOW_W, view::WINDOW_H),
            clipboard: model::Clipboard::default(),
            quickview: None,
            preview: None,
            preview_generation_seed: 0,
            thumbs: thumbnails::Store::new(),
            quickview_pending: false,
            quickview_closing: None,
            quickview_auto: std::env::var_os("OTTO_FILES_QV_AUTO").is_some(),
            quickview_generation: 0,
            status: None,
            info: None,
            info_error: None,
            info_close_hovered: false,
            info_dirty: false,
            controls: WindowControlsState::new(),
            dirty: true,
            picker: None,
            footer_hover: None,
            footer_pressed: None,
            quickview_close_hovered: false,
            quickview_panel: None,
            quickview_focus: None,
            quickview_pinch: None,
        }
    }

    /// A picker window serving `session`, opened at the directory the request
    /// asks for.
    fn for_picker(session: picker::Session, start: PathBuf) -> Self {
        let mut browser = Self::new(start);
        // The picker is a dialog, not a document window: one directory at a
        // time reads as a file dialog, where the Miller stack reads as the
        // browser. The user can still switch views.
        browser.mode = ViewMode::List;
        browser.picker = Some(session);
        browser.pan = ScrollView::horizontal(view::content_viewport(
            browser.size.0,
            browser.size.1 - view::FOOTER_H,
            ViewMode::List,
        ));
        browser
    }

    /// How much of the window height the action row takes — zero in the
    /// browser, which has none.
    fn footer_h(&self) -> f32 {
        if self.picker.is_some() {
            view::FOOTER_H
        } else {
            0.0
        }
    }

    /// The bottom of the *file area*, which is the window bottom less the
    /// action row. Every piece of geometry in [`view`] takes this as its
    /// `height`, so none of it has to know the row exists.
    fn content_h(&self) -> f32 {
        self.size.1 - self.footer_h()
    }

    /// The entries of column `depth`, filtered and sorted for display.
    ///
    /// Rebuilt per frame rather than cached: at the sizes a window shows this
    /// is cheap, and a cache is one more thing to invalidate when the directory
    /// changes underneath. The moment it stops being cheap it moves to the
    /// model thread, which is where the spec puts it.
    fn visible(&self, depth: usize) -> Vec<&Entry> {
        let _t = perf::now();
        let entries = self.visible_uncounted(depth);
        perf::mark(perf::Stage::Visible, _t);
        entries
    }

    /// How many entries the column shows, without materialising the list.
    ///
    /// `counts()` and the subtitle only ever wanted the length, and building
    /// a `Vec` of twenty-five thousand references to throw away is the kind
    /// of per-frame cost that is invisible until the directory is big.
    fn visible_len(&self, depth: usize) -> usize {
        self.ensure_sorted(depth);
        self.columns[depth].sorted.borrow().order.len()
    }

    /// Bring the column's cached order in line with the listing and the
    /// current sort settings, recomputing only if one of them moved.
    fn ensure_sorted(&self, depth: usize) {
        let column = &self.columns[depth];
        // The picker's filter is part of the key: picking a different one
        // re-filters in place, with no filesystem access, and picking the
        // same one re-uses the order already computed.
        let filter = self.picker.as_ref().map(|p| p.current_filter).unwrap_or(0);
        let key = (
            column.epoch,
            self.sort,
            self.ascending,
            self.show_hidden,
            filter,
        );
        if column.sorted.borrow().key == Some(key) {
            return;
        }
        let entries = &column.snapshot.entries;
        let mut order: Vec<usize> = (0..entries.len())
            .filter(|&i| self.show_hidden || !entries[i].hidden)
            .filter(|&i| match &self.picker {
                Some(session) => session.shows(&entries[i].name, entries[i].is_dir),
                None => true,
            })
            .collect();
        order.sort_by(|&a, &b| self.compare(&entries[a], &entries[b]));
        *column.sorted.borrow_mut() = model::SortCache {
            key: Some(key),
            order,
        };
    }

    fn visible_uncounted(&self, depth: usize) -> Vec<&Entry> {
        self.ensure_sorted(depth);
        let column = &self.columns[depth];
        let entries = &column.snapshot.entries;
        column
            .sorted
            .borrow()
            .order
            .iter()
            .map(|&i| &entries[i])
            .collect()
    }

    /// The order two entries sort in, under the current settings.
    fn compare(&self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
        let dirs_first = b.is_dir.cmp(&a.is_dir);
        if dirs_first != std::cmp::Ordering::Equal {
            return dirs_first;
        }
        let ord = match self.sort {
            SortKey::Name => model::natural_cmp(&a.name, &b.name),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Kind => a
                .kind_label()
                .cmp(b.kind_label())
                .then_with(|| model::natural_cmp(&a.name, &b.name)),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        if self.ascending {
            ord
        } else {
            ord.reverse()
        }
    }

    fn counts(&self) -> Vec<usize> {
        (0..self.columns.len())
            .map(|d| self.visible_len(d))
            .collect()
    }

    /// Whether the preview pane has something to show: exactly one *file*
    /// selected in the active column, in Miller view — a folder is where you
    /// already are one click away from browsing, so previewing it as if it
    /// were a document's content does not earn the pane, and List/Grid show
    /// one directory at a time with nothing for a trailing pane to sit beside.
    ///
    /// No width check: the pane is a member of the horizontally-panned Miller
    /// stack (see [`Self::preview_width`], [`Self::reveal_preview`]), not
    /// something carved out of the listing's own space, so there is nothing
    /// for a narrow window to run short of — it is simply off-screen until
    /// panned into view, the same as any column would be.
    fn preview_visible(&self) -> bool {
        if self.mode != ViewMode::Columns {
            return false;
        }
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let single_selected = self
            .columns
            .get(depth)
            .is_some_and(|c| c.selection.len() == 1);
        single_selected && self.selected_entry().is_some_and(|e| !e.is_dir)
    }

    fn preview_width(&self) -> f32 {
        if self.preview_visible() {
            view::PREVIEW_W
        } else {
            0.0
        }
    }

    /// Bring the preview pane's state in line with the current selection.
    /// Returns the path and generation to decode when the target changed and
    /// nothing is already in flight for it — the caller runs the decode off
    /// the UI thread and reports back through [`Self::finish_preview`].
    fn sync_preview_target(&mut self) -> Option<(PathBuf, u64)> {
        if !self.preview_visible() {
            if self.preview.take().is_some() {
                self.dirty = true;
            }
            return None;
        }
        let entry = self.selected_entry()?;
        if let Some(pane) = &self.preview {
            if pane.path == entry.path {
                return None; // Already showing (or decoding) this one.
            }
        }
        self.preview_generation_seed += 1;
        let generation = self.preview_generation_seed;
        self.preview = Some(PreviewPaneState {
            path: entry.path.clone(),
            generation,
            pending: true,
            decoded: None,
        });
        self.reveal_preview();
        self.dirty = true;
        Some((entry.path, generation))
    }

    /// Pan the stack so the preview pane — sitting right after the last real
    /// column, the same trailing position a freshly opened directory column
    /// would occupy — is fully in view. Called whenever the preview's target
    /// changes, the same way selecting a directory reveals its own column.
    fn reveal_preview(&mut self) {
        if !self.preview_visible() {
            return;
        }
        self.sync_scroll_metrics();
        let target = view::preview_pan_for(
            self.columns.len(),
            self.size.0,
            self.pan.offset(),
            self.miller_w,
        );
        if self.pan.scroll_to(target) {
            self.dirty = true;
        }
    }

    /// Ask the thumbnail store what the visible entries still need.
    ///
    /// Called once per update, the same place the preview column's target is
    /// synced. Returns the jobs the host is to run off the UI thread; an empty
    /// vector — the usual answer — means everything on screen is already
    /// settled.
    ///
    /// Only the pane the user is looking at is considered. In Miller view the
    /// parent columns are 16-pixel rows of icons where a thumbnail buys almost
    /// nothing, and fetching for every column at once would spend the whole
    /// in-flight budget on panes the eye is not on.
    fn sync_thumbnails(&mut self) -> Vec<thumbnails::Job> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let entries = self.visible(depth);
        if entries.is_empty() {
            return Vec::new();
        }

        let area = view::content_viewport(self.size.0, self.content_h(), self.mode);
        let scroll = self.columns[depth].scroll.offset();
        let range = match self.mode {
            view::ViewMode::Grid => view::grid_visible_range(area, entries.len(), scroll, area),
            view::ViewMode::List => {
                view::RowStrip::list(self.size.0, entries.len(), scroll).visible(area)
            }
            // A Miller pane's rows are the same height as a list's, so the
            // same strip describes them; the pane's own width is what differs,
            // and the vertical extent is all this needs.
            view::ViewMode::Columns => {
                view::RowStrip::list(self.size.0, entries.len(), scroll).visible(area)
            }
        };

        // The box a thumbnail will be drawn in decides how much detail to ask
        // for: a grid cell is worth a real picture, a list row is 16 points of
        // it.
        let box_edge = match self.mode {
            view::ViewMode::Grid => view::GRID_ICON,
            view::ViewMode::List | view::ViewMode::Columns => view::ICON_SIZE,
        };
        let scale = AppContext::scale_factor().max(1) as f32;
        let size = thumbcache::Size::for_box(box_edge, scale);

        let requests = entries
            .get(range.clone())
            .unwrap_or_default()
            .iter()
            // A directory has no picture of its own, and asking for one means
            // a sandboxed worker per folder in a folder of folders.
            .filter(|entry| !entry.is_dir)
            .map(|entry| thumbnails::Request {
                path: entry.path.clone(),
                modified: entry.modified,
                may_generate: entry.kind.thumbnailable(),
            })
            .collect::<Vec<_>>();
        self.thumbs.wanted(requests, size)
    }

    /// Show a preview decode that arrived, unless the selection has moved on
    /// since — the same staleness guard Quick View uses.
    fn finish_preview(&mut self, generation: u64, preview: otto_kit::preview::Preview) {
        let Some(pane) = &mut self.preview else {
            return;
        };
        if pane.generation != generation {
            return;
        }
        pane.pending = false;
        pane.decoded = Some(preview);
        self.dirty = true;
    }

    /// Give every pane's scroll view its viewport and content height.
    ///
    /// Re-measured rather than cached, and before both drawing and scrolling:
    /// the viewport moves with the window and the pan, and the content height
    /// changes whenever a directory finishes loading, is re-sorted, or has
    /// hidden files toggled. A scroll view with stale metrics clamps to the
    /// wrong end.
    fn sync_scroll_metrics(&mut self) {
        let (width, height) = (self.size.0, self.content_h());
        let mode = self.mode;
        let miller_w = self.miller_w;
        let depth_count = self.columns.len();
        let counts = self.counts();

        // The stack's own metrics first: the panes are laid out from its
        // offset, so a stale pan would place every pane viewport wrong. The
        // preview pane, when showing, is one more thing the stack must have
        // room to pan to — it is folded into the same content length as the
        // real columns rather than carved out of the viewport.
        self.pan
            .set_viewport(view::content_viewport(width, height, mode));
        self.pan.set_content_length(view::miller_content_width(
            depth_count,
            miller_w,
            self.preview_width(),
        ));
        let pan = self.pan.offset();

        for depth in 0..depth_count {
            let viewport = view::pane_viewport(width, height, mode, depth, pan, miller_w);
            let content = view::pane_content_height(width, height, mode, counts[depth]);
            let scroll = &mut self.columns[depth].scroll;
            scroll.state.set_viewport(viewport);
            scroll.set_content_length(content);
        }
    }

    /// Pan the stack the shortest distance that brings pane `depth` fully into
    /// view — what every navigation into a new column does.
    ///
    /// A no-op outside Miller view, where there is one pane and nothing to
    /// pan. The metrics are re-synced first because the target is clamped to
    /// the stack's content width, which grows and shrinks with the path.
    fn reveal_pane(&mut self, depth: usize) {
        if self.mode != ViewMode::Columns {
            return;
        }
        self.sync_scroll_metrics();
        let target = view::miller_pan_for(depth, self.size.0, self.pan.offset(), self.miller_w);
        if self.pan.scroll_to(target) {
            self.dirty = true;
        }
    }

    /// Which pane the pointer is over — the one the wheel and the scrollbar
    /// belong to. Only Miller view has more than one.
    fn pane_under(&self, x: f32, y: f32) -> usize {
        if self.mode != ViewMode::Columns {
            return self.columns.len() - 1;
        }
        let counts = self.counts();
        view::miller_at(
            x,
            y,
            self.size.0,
            self.content_h(),
            &self.columns,
            &counts,
            self.pan.offset(),
            self.miller_w,
        )
        .map(|(depth, _)| depth)
        .unwrap_or(self.active)
    }

    /// Whether any pane still has motion to run — momentum, an overscroll
    /// bounce, or a scrollbar fading out.
    fn scroll_animating(&self) -> bool {
        self.pan.is_animating()
            || self.columns.iter().any(|c| c.scroll.is_animating())
            || self.quickview_pan_animating()
    }

    /// Advance every pane's scrolling by one tick. Returns whether anything
    /// moved and therefore needs a repaint.
    fn tick_scroll(&mut self) -> bool {
        let mut moved = false;
        if self.pan.is_animating() {
            moved |= self.pan.tick();
        }
        for column in &mut self.columns {
            if column.scroll.is_animating() {
                moved |= column.scroll.tick();
            }
        }
        // The open preview's picture pans on scroll views of its own, and
        // they fling and spring like any other.
        moved |= self.tick_quickview_pan();
        moved
    }

    /// The directory the window is "at": the deepest column, or the selected
    /// directory within it.
    fn current_path(&self) -> PathBuf {
        self.columns
            .last()
            .map(|c| c.path.clone())
            .unwrap_or_default()
    }

    /// Select `index` in column `depth`, replacing whatever was selected.
    ///
    /// If it is a directory, push a column for it; if not, truncate the stack
    /// so nothing stale hangs to the right.
    fn select(&mut self, depth: usize, index: usize) {
        if depth >= self.columns.len() {
            return;
        }
        let entry = match self.visible(depth).get(index) {
            Some(e) => (*e).clone(),
            None => return,
        };

        // A directory descent is a real navigation, worth a Back entry;
        // recorded here, before the stack changes underneath it.
        let descending = entry.is_dir && self.mode == ViewMode::Columns;
        if descending {
            self.record_location();
        }

        let column = &mut self.columns[depth];
        column.selection.clear();
        column.selection.insert(entry.name.clone());
        column.cursor = Some(index);
        column.anchor = Some(index);

        self.active = depth;
        self.columns.truncate(depth + 1);

        // Only Miller view reveals a directory's contents on a plain select —
        // that eager next pane is the point of the view. List and Grid show
        // one directory at a time, so selecting there must not also swap it
        // out from under the click; opening is `open_selection`'s job there
        // (a double-click, or Return), the same as a file.
        if entry.is_dir && self.mode == ViewMode::Columns {
            self.columns.push(Column::new(entry.path.clone()));
            self.reveal_pane(depth + 1);
        }
        self.dirty = true;
    }

    /// Add or remove one entry, leaving the rest of the selection alone —
    /// Ctrl+click. The cursor and the anchor both move to it, so a following
    /// Shift+click ranges from here.
    fn toggle_select(&mut self, depth: usize, index: usize) {
        if depth >= self.columns.len() {
            return;
        }
        let Some(name) = self.visible(depth).get(index).map(|e| e.name.clone()) else {
            return;
        };
        let column = &mut self.columns[depth];
        if !column.selection.remove(&name) {
            column.selection.insert(name);
        }
        column.cursor = Some(index);
        column.anchor = Some(index);
        self.active = depth;
        // A multi-selection has no single child, so nothing hangs to the right.
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    /// Select the contiguous run from the anchor to `index` — Shift+click and
    /// Shift+Arrow. Replaces the previous range rather than accumulating, so
    /// dragging the far end back shrinks it.
    fn extend_select(&mut self, depth: usize, index: usize) {
        if depth >= self.columns.len() {
            return;
        }
        let names: Vec<String> = self.visible(depth).iter().map(|e| e.name.clone()).collect();
        if index >= names.len() {
            return;
        }
        let anchor = self.columns[depth]
            .anchor
            .unwrap_or(index)
            .min(names.len() - 1);
        let (lo, hi) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };

        let column = &mut self.columns[depth];
        column.selection.clear();
        for name in &names[lo..=hi] {
            column.selection.insert(name.clone());
        }
        column.cursor = Some(index);
        self.active = depth;
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    fn select_all(&mut self) {
        let depth = self.active;
        let names: Vec<String> = self.visible(depth).iter().map(|e| e.name.clone()).collect();
        let column = &mut self.columns[depth];
        column.selection = names.into_iter().collect();
        self.columns.truncate(depth + 1);
        self.dirty = true;
    }

    fn clear_selection(&mut self) {
        let column = &mut self.columns[self.active];
        column.selection.clear();
        column.cursor = None;
        column.anchor = None;
        self.dirty = true;
    }

    /// Every selected entry in the active column, in view order.
    fn selected_entries(&self) -> Vec<Entry> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let selection = &self.columns[depth].selection;
        self.visible(depth)
            .into_iter()
            .filter(|e| selection.contains(&e.name))
            .cloned()
            .collect()
    }

    /// Start editing the cursor entry's name in place — Return's job, the way
    /// it is Finder's, in every view mode.
    fn start_rename(&mut self) {
        if self.rename.is_some() {
            return;
        }
        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };
        let theme = AppContext::current_theme();
        let selection = rename_selection(&entry.name, entry.is_dir);
        let mut input = TextInput::editing(entry.name, view::rename_field_style(theme));
        input.state.select_range(selection);
        self.rename = Some(RenameSession {
            depth,
            index,
            original: entry.path,
            input,
        });
        self.dirty = true;
    }

    /// Apply the field's text as the new name, if it actually changed.
    fn commit_rename(&mut self) {
        let Some(session) = self.rename.take() else {
            return;
        };
        let new_name = session.input.value().trim().to_string();
        let old_name = session
            .original
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if new_name.is_empty() || new_name == old_name {
            self.dirty = true;
            return;
        }
        let target = session.original.with_file_name(&new_name);
        match std::fs::rename(&session.original, &target) {
            Ok(()) => {
                if let Some(column) = self.columns.get_mut(session.depth) {
                    column.selection.clear();
                    column.selection.insert(new_name.clone());
                }
                self.status = Some(format!("Renamed to \u{201c}{new_name}\u{201d}"));
                self.reload_all();
            }
            Err(err) => {
                self.status = Some(format!("Couldn\u{2019}t rename: {err}"));
            }
        }
        self.dirty = true;
    }

    /// Discard the field's text and leave the file as it was.
    fn cancel_rename(&mut self) {
        if self.rename.take().is_some() {
            self.dirty = true;
        }
    }

    /// Record a plain click on a row/cell, opening it if this is the second
    /// one to land on the same row within the double-click window — List and
    /// Grid's only mouse way to open a directory, since neither shows a
    /// child eagerly the way Miller does.
    fn note_row_click(&mut self, depth: usize, index: usize) {
        let now = std::time::Instant::now();
        let double_click = self.last_row_click.is_some_and(|(d, i, at)| {
            d == depth && i == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
        });
        if double_click {
            self.last_row_click = None;
            self.open_selection();
        } else {
            self.last_row_click = Some((depth, index, now));
        }
    }

    /// Descend into the selection: in Miller view the child column already
    /// exists, so this only moves the keyboard into it.
    fn open_selection(&mut self) {
        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };

        if entry.is_dir {
            match self.mode {
                ViewMode::Columns => {
                    if depth + 1 < self.columns.len() {
                        self.active = depth + 1;
                        if self.columns[self.active].cursor.is_none()
                            && !self.visible(self.active).is_empty()
                        {
                            // Same path a click takes: if this first entry is
                            // itself a directory, its column shows up too —
                            // every directory on screen keeps the pane to its
                            // right populated, not just the one last entered.
                            self.select(self.active, 0);
                        }
                        self.reveal_pane(self.active);
                    }
                }
                ViewMode::List | ViewMode::Grid => {
                    // These show one directory, so descending replaces it.
                    self.record_location();
                    self.columns.truncate(depth + 1);
                    self.columns.push(Column::new(entry.path.clone()));
                    self.active = self.columns.len() - 1;
                }
            }
            self.dirty = true;
            return;
        }

        // A file. In the picker, activating one *is* the accept — double-click
        // and Enter both land here, and both mean "this one". In the browser,
        // opening a file in its default application is not wired up yet.
        if self.picker.is_some() {
            self.picker_accept();
        }
    }

    // --- The picker's half of "activate" -----------------------------------

    /// What the accept button would return, or `None` if it has nothing to
    /// return and must stay disabled.
    ///
    /// Directory mode with nothing picked accepts the directory being
    /// *viewed*, which is how a user says "this folder" without having to
    /// step out of it and select it from its parent.
    fn picker_selection(&self) -> Option<Vec<PathBuf>> {
        let session = self.picker.as_ref()?;
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = self.columns.get(depth)?;

        let picked: Vec<PathBuf> = self
            .visible(depth)
            .into_iter()
            .filter(|e| column.selection.contains(&e.name))
            .filter(|e| session.selectable(&e.name, e.is_dir))
            .map(|e| e.path.clone())
            .collect();

        if picked.is_empty() {
            if session.request.directory {
                return Some(vec![column.path.clone()]);
            }
            return None;
        }
        // A single-select request returns exactly one file however many the
        // pointer managed to gather.
        if !session.request.multiple {
            return picked.into_iter().next().map(|p| vec![p]);
        }
        Some(picked)
    }

    /// Return the current selection to the application and close the window.
    fn picker_accept(&mut self) {
        let Some(paths) = self.picker_selection() else {
            return;
        };
        if let Some(session) = self.picker.as_mut() {
            session.accept(&paths);
        }
        self.dirty = true;
    }

    /// Cancel the request. The window closes and the application is told the
    /// user declined — which is a different answer from "it went wrong".
    fn picker_cancel(&mut self) {
        if let Some(session) = self.picker.as_mut() {
            session.resolve(picker::Outcome::cancelled());
        }
        self.dirty = true;
    }

    /// Switch to filter `index` and re-filter in place.
    fn set_filter(&mut self, index: usize) {
        if let Some(session) = self.picker.as_mut() {
            if index < session.filters.len() && index != session.current_filter {
                session.current_filter = index;
                // The cursor and selection are indices into an order that no
                // longer exists once the filter moves.
                for column in &mut self.columns {
                    column.selection.clear();
                    column.cursor = None;
                    column.anchor = None;
                }
            }
            session.filter_open = false;
            self.dirty = true;
        }
    }

    /// The action row's press half: arm the button under the pointer.
    fn footer_press(&mut self, button: view::FooterButton) {
        self.footer_pressed = Some(button);
        self.dirty = true;
    }

    /// The action row's release half: fire only if the pointer is still over
    /// the button that was armed.
    fn footer_release(&mut self, over: Option<view::FooterButton>) {
        let armed = self.footer_pressed.take();
        self.dirty = true;
        if armed.is_none() || armed != over {
            return;
        }
        match armed {
            Some(view::FooterButton::Accept) => self.picker_accept(),
            Some(view::FooterButton::Cancel) => self.picker_cancel(),
            Some(view::FooterButton::Filter) => {
                if let Some(session) = self.picker.as_mut() {
                    session.filter_open = !session.filter_open;
                }
            }
            Some(view::FooterButton::FilterOption(index)) => self.set_filter(index),
            None => {}
        }
    }

    /// The column stack as a [`Location`], for the Back/Forward pair.
    fn location(&self) -> Location {
        Location {
            columns: self
                .columns
                .iter()
                .map(|c| ColumnState {
                    path: c.path.clone(),
                    selection: c.selection.clone(),
                    cursor: c.cursor,
                    anchor: c.anchor,
                })
                .collect(),
            active: self.active,
        }
    }

    /// Record where the browser is now, before a navigation moves it
    /// somewhere else — Back's undo point. Any Forward history is dropped:
    /// once the user branches off by navigating anew, the old "future" no
    /// longer applies, the same rule a web browser follows.
    fn record_location(&mut self) {
        self.back.push(self.location());
        self.forward.clear();
    }

    /// Replace the column stack with a remembered one, as Back and Forward
    /// both do.
    fn restore_location(&mut self, location: Location) {
        self.columns = location
            .columns
            .into_iter()
            .map(|state| {
                let mut column = Column::new(state.path);
                column.selection = state.selection;
                column.cursor = state.cursor;
                column.anchor = state.anchor;
                column
            })
            .collect();
        if self.columns.is_empty() {
            self.columns.push(Column::new(self.current_path()));
        }
        self.active = location.active.min(self.columns.len() - 1);
        self.pan.scroll_to(0.0);
        self.reveal_pane(self.active);
        self.pending_restore = true;
        self.dirty = true;
    }

    /// Finish a Back/Forward step once its directories have been read.
    ///
    /// The selection is held by name, so it survives the reload untouched;
    /// the cursor is an index, so it is re-derived from that selection rather
    /// than trusted — a file added or removed while the user was away would
    /// otherwise leave the keyboard one row off from the highlight. Then the
    /// restored row is scrolled back into view, which needs the metrics of
    /// the listing that just landed.
    fn settle_restore(&mut self) {
        if !self.pending_restore || self.loading() {
            return;
        }
        self.pending_restore = false;
        for depth in 0..self.columns.len() {
            let Some(first) = self.columns[depth].selection.iter().next().cloned() else {
                continue;
            };
            let index = self.visible(depth).iter().position(|e| e.name == first);
            if let Some(index) = index {
                self.columns[depth].cursor = Some(index);
                self.columns[depth].anchor = Some(index);
            }
        }
        self.reveal_cursor();
        self.dirty = true;
    }

    /// The nav arrow under `(x, y)`, and only when it has somewhere to go: a
    /// half with an empty history is drawn dimmed, and a dimmed control must
    /// not light up under a press either.
    fn nav_button_at(&self, x: f32, y: f32) -> Option<view::NavButton> {
        let button = view::nav_button_at(x, y)?;
        let live = match button {
            view::NavButton::Back => !self.back.is_empty(),
            view::NavButton::Forward => !self.forward.is_empty(),
        };
        live.then_some(button)
    }

    /// Step to the previous location, if there is one.
    fn go_back(&mut self) {
        let Some(location) = self.back.pop() else {
            return;
        };
        self.forward.push(self.location());
        self.restore_location(location);
    }

    /// Step to the location Back left, if there is one.
    fn go_forward(&mut self) {
        let Some(location) = self.forward.pop() else {
            return;
        };
        self.back.push(self.location());
        self.restore_location(location);
    }

    /// Go to the parent directory.
    fn go_up(&mut self) {
        if self.columns.len() > 1 {
            self.record_location();
            self.columns.truncate(self.columns.len() - 1);
            self.active = self.columns.len() - 1;
            self.reveal_pane(self.active);
            self.dirty = true;
            return;
        }
        // At the root of the stack: re-root one level up.
        let path = self.columns[0].path.clone();
        if let Some(parent) = path.parent() {
            self.record_location();
            self.columns = vec![Column::new(parent.to_path_buf())];
            self.active = 0;
            self.pan.scroll_to(0.0);
            self.dirty = true;
        }
    }

    /// Replace the whole stack, as clicking a place does.
    fn navigate_to(&mut self, path: &Path) {
        self.record_location();
        self.columns = vec![Column::new(path.to_path_buf())];
        self.active = 0;
        self.pan.scroll_to(0.0);
        self.dirty = true;
    }

    /// How far one Up/Down press moves. In the grid that is a whole row of
    /// cells — the arrows walk the grid in two dimensions, so vertical motion
    /// crosses a row and Left/Right steps one cell — and one entry everywhere
    /// else, where the listing is a single column.
    fn row_step(&self) -> i32 {
        if self.mode != ViewMode::Grid {
            return 1;
        }
        let area = view::content_viewport(self.size.0, self.content_h(), ViewMode::Grid);
        view::grid_columns(area) as i32
    }

    /// Move the cursor within the active column. With `extend`, the selection
    /// grows from the anchor instead of being replaced.
    fn move_cursor(&mut self, delta: i32, extend: bool) {
        let count = self.visible(self.active).len();
        if count == 0 {
            return;
        }
        // With nothing selected, the first press should land the cursor on an
        // end, whatever the step: Down's obvious first stop is index 0, not
        // one grid row in.
        let next = match self.columns[self.active].cursor {
            Some(cursor) => (cursor as i32 + delta).clamp(0, count as i32 - 1) as usize,
            None if delta >= 0 => 0,
            None => count - 1,
        };
        if extend {
            self.extend_select(self.active, next);
        } else {
            self.select(self.active, next);
        }
        self.reveal_cursor();
    }

    /// Move the cursor to the first entry whose name starts with the
    /// type-ahead buffer, case-insensitively.
    ///
    /// Nothing is filtered and nothing is drawn: the only sign it happened is
    /// the selection moving, which is the whole point of the gesture —
    /// reaching a file in a long directory without leaving the keyboard.
    /// The buffer expires after a second of silence, so the next burst of
    /// typing starts a fresh name rather than extending a stale one.
    ///
    /// Repeating one character with nothing in between cycles through the
    /// entries beginning with it, rather than looking for a doubled letter
    /// that almost no name has.
    fn typeahead(&mut self, ch: char) {
        const EXPIRY: std::time::Duration = std::time::Duration::from_secs(1);

        let names: Vec<String> = self
            .visible(self.active)
            .iter()
            .map(|entry| entry.name.to_lowercase())
            .collect();
        if names.is_empty() {
            return;
        }

        let now = std::time::Instant::now();
        let live = self
            .typeahead
            .take()
            .filter(|(_, last)| now.duration_since(*last) < EXPIRY)
            .map(|(buffer, _)| buffer);
        let typed = ch.to_lowercase().to_string();
        let cycling = live.as_deref() == Some(typed.as_str());
        let buffer = match live {
            Some(buffer) if cycling => buffer,
            Some(mut buffer) => {
                buffer.push_str(&typed);
                buffer
            }
            None => typed,
        };

        // Cycling resumes just past the cursor and wraps; a buffer that grew
        // answers from the top, so the same keys always land on the same file.
        let from = match (cycling, self.columns[self.active].cursor) {
            (true, Some(cursor)) => cursor + 1,
            _ => 0,
        };
        let hit = (0..names.len())
            .map(|step| (from + step) % names.len())
            .find(|&index| names[index].starts_with(&buffer));

        // Kept even when nothing matched: the miss is part of the word being
        // typed, and dropping it would make the next character search for a
        // prefix the user never asked for.
        self.typeahead = Some((buffer, now));
        if let Some(index) = hit {
            self.select(self.active, index);
            self.reveal_cursor();
        }
    }

    /// Scroll the active pane the shortest distance that brings the cursor
    /// fully into view, so walking a long directory with the arrow keys does
    /// not leave the selection behind the edge of the viewport.
    ///
    /// Metrics come from the last [`Self::sync_scroll_metrics`], which runs at
    /// the top of every frame — a key press always follows one.
    fn reveal_cursor(&mut self) {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let (width, height) = (self.size.0, self.content_h());
        let viewport = view::pane_viewport(
            width,
            height,
            self.mode,
            depth,
            self.pan.offset(),
            self.miller_w,
        );
        if viewport.is_empty() {
            return;
        }
        let (top, item_h) = view::item_span(width, height, self.mode, index);

        let scroll = &mut self.columns[depth].scroll;
        let offset = scroll.offset();
        let target = if top < offset {
            top
        } else if top + item_h > offset + viewport.height() {
            top + item_h - viewport.height()
        } else {
            return;
        };
        // A fling still in the air would undo this on the next tick.
        scroll.stop();
        if scroll.state.set_offset(target) {
            self.dirty = true;
        }
    }

    /// Left/right in Miller view: out of a column, or into its child.
    fn move_lateral(&mut self, delta: i32) {
        if self.mode != ViewMode::Columns {
            if delta < 0 {
                self.go_up();
            } else {
                self.open_selection();
            }
            return;
        }
        if delta < 0 {
            if self.active > 0 {
                self.active -= 1;
                self.reveal_pane(self.active);
                self.dirty = true;
            }
        } else {
            self.open_selection();
        }
    }

    /// The entry the cursor is on, if any.
    fn selected_entry(&self) -> Option<Entry> {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let index = self.columns[depth].cursor?;
        self.visible(depth).get(index).map(|e| (*e).clone())
    }

    /// Open Get Info for the selection.
    ///
    /// The read is synchronous here, unlike a directory listing: it is one
    /// `stat` plus two account lookups for one file the user just asked about,
    /// and it happens on a keystroke rather than during scrolling. If the
    /// account database turns out to block in practice this moves to a worker
    /// like everything else.
    fn open_info(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        self.info = Some(model::read_info(&entry.path));
        self.info_error = None;
        self.info_dirty = true;
    }

    fn close_info(&mut self) {
        self.info = None;
        self.info_error = None;
        self.info_close_hovered = false;
        self.info_dirty = true;
    }

    /// Toggle one permission bit and apply it.
    ///
    /// Applied immediately rather than behind an OK button — there is no
    /// pending state to get out of step, and a refusal is reported in place.
    /// Only the toggled bit changes, so setuid/setgid/sticky survive.
    fn toggle_permission(&mut self, who: usize, what: usize) {
        let Some(info) = &self.info else { return };
        let path = info.path.clone();
        let next = info.mode ^ model::permission_bit(who, what);

        match model::set_mode(&path, next) {
            Ok(()) => {
                // Re-read rather than assuming: the filesystem may have applied
                // something other than what was asked (a mount's umask, an
                // acl), and the sheet must show what is true.
                self.info = Some(model::read_info(&path));
                self.info_error = None;
            }
            Err(reason) => self.info_error = Some(reason),
        }
        self.info_dirty = true;
    }

    /// Put the selection on the system clipboard.
    ///
    /// A cut only *marks*: nothing moves until the paste, so an abandoned cut
    /// costs nothing and cannot lose a file.
    ///
    /// `serial` must be from a real input event — the compositor refuses a
    /// selection claimed without one.
    fn copy_selection(&mut self, cut: bool, serial: u32) {
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();

        // Kept locally as well as offered: the local copy is what draws the
        // cut entries dimmed, and it means a paste back into this window does
        // not have to round-trip through the compositor.
        self.clipboard = model::Clipboard {
            paths: paths.clone(),
            cut,
        };

        let claimed = clipboard::set(clipboard::file_payloads(&paths, cut), serial);
        self.status = Some(if claimed {
            format!(
                "{} {count} item{} to paste",
                if cut { "Cut" } else { "Copied" },
                if count == 1 { "" } else { "s" }
            )
        } else {
            // Say so rather than pretending: the files are still pasteable
            // here, just not anywhere else.
            format!(
                "{} {count} item{} (this window only)",
                if cut { "Cut" } else { "Copied" },
                if count == 1 { "" } else { "s" }
            )
        });
        self.dirty = true;
    }

    /// Paste into the directory being viewed.
    ///
    /// Runs on the calling thread today, which is the UI thread — acceptable
    /// only because it is a deliberate keystroke on a known selection, not
    /// something that happens while scrolling. The spec puts this on the worker
    /// pool with progress and cancellation, and that is the next change; the
    /// `OpResult` it returns is already the shape that path reports.
    fn paste(&mut self) {
        // The system clipboard wins over our own copy: if another application
        // has copied since, that is what the user means by "paste", and our
        // local clipboard is stale.
        let from_system = clipboard::first_available(clipboard::file_mime_preference())
            .and_then(|mime| clipboard::read(&mime).map(|bytes| (mime, bytes)))
            .map(|(mime, bytes)| clipboard::parse_file_payload(&mime, &bytes))
            .filter(|(paths, _)| !paths.is_empty());

        let clip = match from_system {
            Some((paths, cut)) => model::Clipboard { paths, cut },
            None => self.clipboard.clone(),
        };
        if clip.is_empty() {
            return;
        }
        let dest = self.columns[self.active].path.clone();

        // Keep Both rather than Replace: without a conflict sheet to ask with,
        // the only safe default is the one that cannot destroy anything.
        let result = model::paste(&clip, &dest, model::OnConflict::KeepBoth);

        // A cut is consumed by its paste; a copy stays available to paste again.
        if clip.cut && result.errors.is_empty() {
            self.clipboard = model::Clipboard::default();
        }

        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        self.reload_all();
        self.dirty = true;
    }

    /// The entry under a point, if any — the same hit test the left-click
    /// handler uses for the current view mode. `None` means empty space: the
    /// background, a gap between rows, or (in List view) the header.
    fn entry_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let (width, height) = (self.size.0, self.content_h());
        match self.mode {
            ViewMode::Grid => {
                let depth = self.columns.len() - 1;
                let count = self.visible_len(depth);
                let scroll = self.columns[depth].scroll.offset();
                let area = view::content_viewport(width, height, ViewMode::Grid);
                view::grid_cell_at(area, x, y, count, scroll).map(|i| (depth, i))
            }
            ViewMode::List => {
                let depth = self.columns.len() - 1;
                let count = self.visible_len(depth);
                let scroll = self.columns[depth].scroll.offset();
                view::row_at(x, y, width, height, count, scroll).map(|i| (depth, i))
            }
            ViewMode::Columns => {
                let counts = self.counts();
                match view::miller_at(
                    x,
                    y,
                    width,
                    height,
                    &self.columns,
                    &counts,
                    self.pan.offset(),
                    self.miller_w,
                ) {
                    Some((depth, Some(index))) => Some((depth, index)),
                    _ => None,
                }
            }
        }
    }

    /// Point the browser at what a right-click at `(x, y)` should act on,
    /// and build the menu for it.
    ///
    /// A hit not already part of the selection replaces it, the way a plain
    /// click does; a hit that's already selected leaves a multi-selection
    /// alone, so the menu acts on the whole group. An empty-space click just
    /// moves the keyboard focus to that pane, for New Folder and Paste.
    fn context_menu_items(&mut self, x: f32, y: f32) -> Vec<MenuItem> {
        match self.entry_at(x, y) {
            Some((depth, index)) => {
                let already_selected = self
                    .visible(depth)
                    .get(index)
                    .is_some_and(|e| self.columns[depth].selection.contains(&e.name));
                if already_selected {
                    self.active = depth;
                } else {
                    self.select(depth, index);
                }
            }
            None => self.active = self.pane_under(x, y),
        }

        let entries = self.selected_entries();
        let mut items = Vec::new();

        if let [only] = entries.as_slice() {
            if only.is_dir {
                items.push(MenuItem::action("Open").with_action_id("open"));
            }
            items.push(MenuItem::action("Get Info").with_action_id("get_info"));
            items.push(MenuItem::action("Rename").with_action_id("rename"));
        }

        if entries.is_empty() {
            items.push(MenuItem::action("New Folder").with_action_id("new_folder"));
            let can_paste = !self.clipboard.is_empty()
                || clipboard::first_available(clipboard::file_mime_preference()).is_some();
            if can_paste {
                items.push(MenuItem::separator());
                items.push(MenuItem::action("Paste").with_action_id("paste"));
            }
        } else {
            if !items.is_empty() {
                items.push(MenuItem::separator());
            }
            items.push(MenuItem::action("Cut").with_action_id("cut"));
            items.push(MenuItem::action("Copy").with_action_id("copy"));
            items.push(MenuItem::separator());
            let label = if entries.len() == 1 {
                "Move to Trash".to_string()
            } else {
                format!("Move {} Items to Trash", entries.len())
            };
            items.push(MenuItem::action(label).with_action_id("trash"));
        }

        items
    }

    /// Move the current selection to Trash.
    fn move_selected_to_trash(&mut self) {
        let paths: Vec<PathBuf> = self
            .selected_entries()
            .into_iter()
            .map(|e| e.path)
            .collect();
        if paths.is_empty() {
            return;
        }
        let result = model::move_to_trash(&paths);
        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        self.reload_all();
        self.dirty = true;
    }

    /// Create "untitled folder" in the active pane and start renaming it in
    /// place, the way Finder and Explorer's New Folder both do.
    fn new_folder(&mut self) {
        let dest = self.columns[self.active].path.clone();
        match model::create_folder(&dest) {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.reload_all();
                let depth = self.active;
                if let Some(index) = self.visible(depth).iter().position(|e| e.name == name) {
                    self.select(depth, index);
                    self.start_rename();
                }
                self.status = Some(format!("New folder \u{201c}{name}\u{201d}"));
            }
            Err(err) => {
                self.status = Some(format!("Couldn\u{2019}t create folder: {err}"));
            }
        }
        self.dirty = true;
    }

    /// Re-read **every** column in the stack, keeping the user where they are.
    ///
    /// Not just the active one: a paste changes the destination *and* — for a
    /// cut — the directory the files came from, which is a different column and
    /// usually still on screen. Reloading only the destination leaves the source
    /// column listing files that are no longer there.
    ///
    /// Reloading the whole stack is the blunt version. The directory watcher the
    /// spec calls for makes this unnecessary: any directory on screen would
    /// notice its own changes, whoever made them, including changes made by
    /// another application. Until that exists this is what keeps the view
    /// truthful.
    fn reload_all(&mut self) {
        for depth in 0..self.columns.len() {
            let path = self.columns[depth].path.clone();
            let keep = self.columns[depth].selection.clone();
            let cursor = self.columns[depth].cursor;
            let offset = self.columns[depth].scroll.offset();

            let mut column = Column::new(path);
            column.selection = keep;
            column.cursor = cursor;
            column.anchor = cursor;
            // Only the position is carried over: the fresh view re-measures
            // its own content, and any momentum belonged to the old listing.
            column.scroll.state.set_offset(offset);
            self.columns[depth] = column;
        }
    }

    /// The rect Quick View grows its panel from, for the active column.
    ///
    /// Surface-local, and empty when there is nothing on screen to grow out of.
    /// Both are usable now that the panel is drawn into this same surface.
    fn quickview_anchor(&self) -> Rect {
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let entries = self.visible(depth);
        let column = &self.columns[depth];
        let pane = view::PaneData {
            selected: entries
                .iter()
                .map(|e| column.selection.contains(&e.name))
                .collect(),
            cursor: column.cursor,
            entries,
            scroll: column.scroll.offset(),
            bar: None,
            loading: column.loading(),
            error: None,
        };
        view::quickview_anchor(
            self.size.0,
            self.content_h(),
            self.mode,
            &pane,
            depth,
            self.pan.offset(),
            self.miller_w,
        )
    }

    /// Start previewing the cursor's file, replacing whatever is open.
    ///
    /// Returns the path to decode and the generation to tag it with, or `None`
    /// when there is nothing to preview. The decode itself is the caller's to
    /// run off the UI thread — this only moves the state.
    fn begin_quickview(&mut self) -> Option<(PathBuf, u64, Rect)> {
        let entry = self.selected_entry();
        if std::env::var_os("OTTO_FILES_QV_TRACE").is_some() {
            eprintln!("qv begin: selected={:?}", entry.as_ref().map(|e| &e.name));
        }
        let entry = entry?;
        // A directory previews as a listing, which the decoder handles, so
        // nothing is excluded here.
        let anchor = self.quickview_anchor();
        self.quickview_generation += 1;
        self.quickview_pending = true;
        // Clear any stale message so "Opening preview…" is not fighting the
        // last operation's summary.
        self.status = None;
        self.dirty = true;
        Some((entry.path, self.quickview_generation, anchor))
    }

    /// Show a decode that arrived, unless the user has moved on since.
    fn finish_quickview(
        &mut self,
        generation: u64,
        anchor: Rect,
        name: String,
        preview: otto_kit::preview::Preview,
    ) {
        if std::env::var_os("OTTO_FILES_QV_TRACE").is_some() {
            eprintln!(
                "qv finish: generation={generation} current={} anchor={anchor:?}",
                self.quickview_generation
            );
        }
        if generation != self.quickview_generation {
            return; // Stale: the user arrow-keyed past this file mid-decode.
        }
        self.quickview_pending = false;
        // Re-opening onto the same panel keeps its entrance rather than
        // replaying it, so arrow-keying through a folder does not pulse.
        let opened_at = match &self.quickview {
            Some(session) => session.opened_at,
            None => std::time::Instant::now(),
        };
        self.quickview = Some(quickview::Session::new(preview, name, anchor, opened_at));
        // Re-opening cancels whatever was on its way out: two panels in flight
        // at once would cross over each other.
        self.quickview_closing = None;
        self.dirty = true;
    }

    /// Remember where the pointer is over the Quick View panel, and which
    /// panel rect that position was measured against.
    ///
    /// A pinch carries no position of its own — only how far its focal point
    /// has drifted since it began — so the only way to zoom about the fingers
    /// is to have kept the last place the pointer was seen.
    fn quickview_focus(&mut self, point: skia_safe::Point, panel: Rect) {
        self.quickview_focus = Some((point, panel));
    }

    /// Zoom the open preview to `scale`, about the focal point: wherever the
    /// pointer last was, carried along by `drift` — the focal point's travel
    /// since the pinch began. Returns whether anything moved.
    fn quickview_zoom_to(&mut self, scale: f32, drift: (f32, f32)) -> bool {
        // With no remembered pointer — a pinch that began before the panel
        // ever saw one — the panel's own centre is the honest focal point.
        let (focus, panel) = match self.quickview_focus {
            Some((point, panel)) => ((point.x + drift.0, point.y + drift.1), panel),
            None => {
                let panel = self
                    .quickview_panel
                    .unwrap_or_else(|| quickview::panel_rect(self.size.0, self.size.1));
                let content = view::quickview_content_rect(panel);
                ((content.center_x(), content.center_y()), panel)
            }
        };
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let moved = session.zoom_to(scale, focus, content);
        self.dirty |= moved;
        moved
    }

    /// Feed a two-finger scroll to the open preview: a pan when there is a
    /// zoomed image to drag, and the scroll it has always been otherwise.
    ///
    /// One entry point for both handlers — the toplevel's and the panel's own
    /// surface — because the choice between panning and scrolling has to come
    /// out the same whichever of the two the compositor happened to deliver
    /// the event to.
    fn quickview_wheel(&mut self, dx: f32, dy: f32, panel: Rect, stop: bool, discrete: bool) {
        let content = view::quickview_content_rect(panel);
        let pannable = self
            .quickview
            .as_ref()
            .is_some_and(|session| session.pannable(content));
        let Some(session) = self.quickview.as_mut() else {
            return;
        };
        if pannable {
            // A picture is scrolled, not dragged: the deltas go to the pan's
            // own scroll views, which amplify them the way every other scroll
            // view in the toolkit does, keep gliding when the fingers lift,
            // and resist the ends.
            session.pan_wheel(dx, dy, content, stop, discrete);
        } else {
            // A gesture that ended moved nothing on its own.
            if stop {
                return;
            }
            // The content's box, not the card's: the rows are laid out below
            // the title strip, so scrolling has to measure against the same
            // rect the preview was drawn into.
            let rows = (dy / otto_kit::preview::ROW_HEIGHT).round() as i32;
            session.scroll_by(rows, content);
        }
        self.dirty = true;
    }

    /// Route a pointer event over the panel to the pan's scrollbars.
    ///
    /// Returns whether the bar took the press: the panel dismisses on a click
    /// outside and the close dot on a click inside, and a bar dragged over a
    /// zoomed picture must do neither.
    fn quickview_pan_pointer(
        &mut self,
        kind: QuickviewPointer,
        point: skia_safe::Point,
        panel: Rect,
    ) -> bool {
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let (handled, moved) = match kind {
            QuickviewPointer::Press => {
                let hit = session.pan_pointer_down(point.x, point.y, content);
                (hit, hit)
            }
            QuickviewPointer::Motion => {
                (false, session.pan_pointer_move(point.x, point.y, content))
            }
            QuickviewPointer::Release => {
                session.pan_pointer_up();
                (false, false)
            }
            QuickviewPointer::Leave => {
                session.pan_pointer_up();
                session.pan_pointer_leave();
                (false, false)
            }
        };
        self.dirty |= moved;
        handled
    }

    /// Advance the open preview's pan by one frame. Returns whether it moved.
    fn tick_quickview_pan(&mut self) -> bool {
        let panel = self
            .quickview_panel
            .unwrap_or_else(|| quickview::panel_rect(self.size.0, self.size.1));
        let content = view::quickview_content_rect(panel);
        let Some(session) = self.quickview.as_mut() else {
            return false;
        };
        let moved = session.tick_pan(content);
        self.dirty |= moved;
        moved
    }

    /// Whether the open preview's pan still has frames to run.
    fn quickview_pan_animating(&self) -> bool {
        self.quickview
            .as_ref()
            .is_some_and(quickview::Session::pan_animating)
    }

    /// Dismiss the preview. Returns whether one was open.
    fn close_quickview(&mut self) -> bool {
        // Bumping the generation orphans an in-flight decode, so a slow file
        // cannot re-open a panel the user has already dismissed.
        self.quickview_generation += 1;
        self.quickview_pending = false;
        // Where the file is *now*, not where it was when the panel opened: the
        // selection may have arrow-keyed on, or the list scrolled, and the
        // point of the exit is to say which file this was.
        let anchor = self.quickview_anchor();
        let Some(mut session) = self.quickview.take() else {
            return false;
        };
        if !anchor.is_empty() {
            session.anchor = anchor;
        }
        session.closing = Some(std::time::Instant::now());
        // Moved aside rather than left in `quickview`, so everything that asks
        // "is a preview open" — the key handling, the pointer routing — sees a
        // closed window from this moment, while the panel is still on screen
        // finishing its exit.
        self.quickview_closing = Some(session);
        self.dirty = true;
        true
    }

    /// The panel on screen, whichever direction it is going.
    fn quickview_visible(&self) -> Option<&quickview::Session> {
        self.quickview.as_ref().or(self.quickview_closing.as_ref())
    }

    /// Whether a panel still has frames to run — arriving or leaving.
    fn quickview_animating(&self) -> bool {
        self.quickview_visible()
            .is_some_and(quickview::Session::animating)
    }

    /// Retire a finished exit. Returns whether anything changed.
    fn tick_quickview_exit(&mut self) -> bool {
        let done = self
            .quickview_closing
            .as_ref()
            .is_some_and(|session| !session.animating());
        if done {
            self.quickview_closing = None;
        }
        done
    }

    /// Poll every column's worker. Returns whether anything landed.
    ///
    /// A freshly loaded column starts with nothing selected — no eager pick
    /// of its first entry. `move_cursor` already lands Down's first press on
    /// index 0 from an empty cursor, so there is nowhere that needs one.
    fn poll(&mut self) -> bool {
        let mut changed = false;
        for depth in 0..self.columns.len() {
            if self.columns[depth].poll() {
                changed = true;
            }
        }
        changed
    }

    fn loading(&self) -> bool {
        self.columns.iter().any(|c| c.loading())
    }

    fn title(&self) -> String {
        let path = if self.mode == ViewMode::Columns {
            self.columns[self.active].path.clone()
        } else {
            self.current_path()
        };
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }

    fn subtitle(&self) -> String {
        // A first preview pays D-Bus activation, so this can be visible for a
        // moment. Saying so beats a keystroke that appears to do nothing.
        if self.quickview_pending {
            return "Opening preview…".to_string();
        }
        if let Some(status) = &self.status {
            return status.clone();
        }
        let depth = self.active.min(self.columns.len() - 1);
        if self.columns[depth].loading() {
            return "Loading…".to_string();
        }
        let selected = self.columns[depth].selection.len();
        if selected > 1 {
            return format!("{selected} of {} selected", self.visible_len(depth));
        }
        let count = self.visible_len(depth);
        let hidden = self.columns[depth]
            .snapshot
            .entries
            .iter()
            .filter(|e| e.hidden)
            .count();
        let mut text = match count {
            0 => "No items".to_string(),
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        };
        if hidden > 0 && !self.show_hidden {
            text.push_str(&format!(", {hidden} hidden"));
        }
        text
    }

    /// Build the per-frame view data.
    fn frame<'a>(&'a self, theme: &'a Theme, title: &'a str) -> view::Frame<'a> {
        let panes = (0..self.columns.len())
            .map(|depth| {
                let entries = self.visible(depth);
                let column = &self.columns[depth];
                view::PaneData {
                    selected: entries
                        .iter()
                        .map(|e| column.selection.contains(&e.name))
                        .collect(),
                    cursor: column.cursor,
                    entries,
                    scroll: column.scroll.offset(),
                    bar: Some(&column.scroll.state),
                    loading: column.loading(),
                    error: column.snapshot.error.as_deref(),
                }
            })
            .collect();

        let preview_entry: Option<&'a Entry> = self
            .preview_visible()
            .then(|| {
                let depth = self.active.min(self.columns.len().saturating_sub(1));
                self.columns[depth]
                    .cursor
                    .and_then(|index| self.visible(depth).get(index).copied())
            })
            .flatten();
        let preview = preview_entry.map(|entry| view::PreviewData {
            name: entry.name.as_str(),
            icon_chain: entry.icon_chain(),
            decoded: self.preview.as_ref().and_then(|p| p.decoded.as_ref()),
            first_row: 0,
            info: preview_info(entry),
        });

        view::Frame {
            width: self.size.0,
            // The *file area's* bottom, not the window's — see
            // [`view::Frame::action_row`]. Every piece of geometry the frame
            // carries stops short of the picker's action row because of this
            // one line.
            height: self.content_h(),
            theme,
            title,
            subtitle: self.subtitle(),
            places: &self.places,
            selected_place: self
                .places
                .iter()
                .position(|p| p.path == self.columns[0].path),
            cut: if self.clipboard.cut {
                self.clipboard.paths.clone()
            } else {
                Default::default()
            },
            mode: self.mode,
            panes,
            active: self.active,
            pan: self.pan.offset(),
            pan_bar: (self.mode == ViewMode::Columns).then_some(&self.pan.state),
            miller_w: self.miller_w,
            sort: self.sort,
            ascending: self.ascending,
            list_columns: self.list_columns,
            renaming: self.rename.as_ref().map(|r| (r.depth, r.index)),
            controls: self.controls,
            can_go_back: !self.back.is_empty(),
            can_go_forward: !self.forward.is_empty(),
            nav_pressed: self.nav_pressed,
            preview,
            action_row: self.picker.as_ref().map(|session| view::FooterData {
                accept_label: &session.accept_label,
                accept_enabled: self.picker_selection().is_some(),
                filters: &session.filter_labels,
                current_filter: session.current_filter,
                filter_open: session.filter_open,
                hovered: self.footer_hover,
                pressed: self.footer_pressed,
            }),
            footer: self.footer_h(),
            quickview_close_hovered: self.quickview_close_hovered,
        }
    }
}

// ---------------------------------------------------------------------------
// App shell
// ---------------------------------------------------------------------------

struct FilesApp {
    window: Option<Window>,
    state: Arc<Mutex<Browser>>,
    /// Control held. Tracked from the key stream rather than inferred from the
    /// text a chord produces: Ctrl+I is historically a TAB character and Ctrl+H
    /// a backspace, so reading `utf8` to detect them is both obscure and
    /// unreliable — it depends on the keymap producing the control character at
    /// all, which it may not.
    ///
    /// Shared rather than a plain field because the pointer callback needs them
    /// too — Ctrl+click and Shift+click are the pointer half of the same
    /// selection rules — and that callback outlives any borrow of `self`.
    modifiers: Arc<Mutex<(bool, bool)>>,
    /// The right-click menu, built once — see `ContextMenu::new`'s docs for
    /// why it cannot be built lazily from inside a pointer handler. `None`
    /// until `on_app_ready` constructs it, which is the earliest point
    /// `AppContext` is set up.
    context_menu: Option<ContextMenu>,
    /// Quick View's surface and its card's rect within it, published by the
    /// render path for the pointer callback below. See
    /// [`pane_surfaces::PaneSurfaces::quickview_target`].
    quickview_target: Arc<Mutex<Option<(wayland_client::backend::ObjectId, Rect)>>>,
    /// The picker's request queue, when this process is serving
    /// `org.otto.FilePicker1`. `None` in the browser.
    picker_queue: Option<crate::dbus::SharedQueue>,
    /// Per-column subsurfaces, when `OTTO_FILES_PANE_SUBS=1`. A scroll then
    /// repaints one column's own buffer instead of the whole window.
    pane_surfaces: Option<pane_surfaces::PaneSurfaces>,
    /// The Get Info panel's window, while one is open.
    ///
    /// Shared with the pointer callback, which is registered once at startup
    /// and looks the current window up rather than being re-registered for
    /// each panel: callbacks cannot be taken off again, so registering one
    /// per opening would pile them up for the life of the process.
    info_window: Rc<RefCell<Option<Window>>>,
}

impl App for FilesApp {
    fn on_app_ready(&mut self, _ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        self.context_menu = Some(ContextMenu::new(Vec::new()));

        // Before the window: a surface builds its own root layer node at
        // construction, and that node is what the scene hangs off.
        AppContext::enable_layer_engine(view::WINDOW_W, view::WINDOW_H);

        let mut window = Window::new("Files", view::WINDOW_W as i32, view::WINDOW_H as i32)?;
        window.set_min_size(view::MIN_W as u32, view::MIN_H as u32);
        // The window as a whole has no ground of its own: the sidebar, the
        // header and the content area each carry their own material, and two
        // of the three are translucent so the compositor's blur reads through
        // them. Anything painted here would sit between that blur and them.
        window.set_background(skia_safe::Color::TRANSPARENT);

        // Name the window after its desktop entry, so the dock and the app
        // switcher find `otto-files.desktop` — and its file-manager icon —
        // directly. Without an app_id the compositor has to guess from the
        // client's pid and executable name, which lands on the same entry only
        // because the `Exec=` line happens to match.
        if let Some(surface) = window.surface() {
            surface.xdg_window().set_app_id("otto-files".to_string());
        }

        if let Some(style) = window.surface_style() {
            style.set_corner_radius(view::CORNER as f64);
            // otto-kit's materials are translucent by design — they expect a
            // blurred backdrop behind them. Without this the desktop shows
            // through the window rather than being frosted by it.
            // `OTTO_FILES_NO_BLUR=1` drops the frosted backdrop, to test what
            // it costs. The window's materials are translucent by design, so
            // without it the desktop shows through unfrosted — ugly, but it
            // isolates the compositor's blur work from everything else.
            if std::env::var_os("OTTO_FILES_NO_BLUR").is_none() {
                style.set_blend_mode(
                    otto_kit::protocols::otto_surface_style_v1::BlendMode::BackgroundBlur,
                );
            }
        }

        // The panels' scene. `None` only where the engine could not be brought
        // up, which is the case the immediate-mode chrome still covers: the
        // window then draws without its grounds rather than not at all.
        let scene = Arc::new(Mutex::new(window.layer_node().map(scene::Scene::new)));

        let state = Arc::clone(&self.state);
        window.on_draw(move |canvas| {
            let mut browser = state.lock().unwrap();

            // Drain finished directory reads. This is the UI thread by
            // construction, which is what the poll needs — see
            // `install_frame_loop` for why it cannot live on a worker.
            let t_total = perf::now();
            let t_prep = perf::now();
            browser.poll();

            let theme = AppContext::current_theme();
            let title = browser.title();
            // Panes measure themselves against the size this frame is drawn
            // at, so their scroll views are re-fitted before anything reads
            // an offset.
            browser.sync_scroll_metrics();
            // Needs those metrics, so it runs here rather than beside the poll.
            browser.settle_restore();
            perf::mark(perf::Stage::Prep, t_prep);
            let t_frame = perf::now();
            let frame = browser.frame(&theme, &title);
            perf::mark(perf::Stage::FrameBuild, t_frame);
            // The panels first, composited by the engine from cached pictures,
            // then the chrome that sits over them.
            let t0 = perf::now();
            if let Some(scene) = scene.lock().unwrap().as_mut() {
                scene.update(&frame);
                perf::mark(perf::Stage::SceneUpdate, t0);
                let t1 = perf::now();
                scene.render(canvas);
                perf::mark(perf::Stage::SceneRender, t1);
            }
            let t2 = perf::now();
            view::draw(canvas, &frame);
            perf::mark(perf::Stage::Chrome, t2);
            perf::mark(perf::Stage::Total, t_total);
            // Unless it has a surface of its own over the columns — drawn
            // here it would be under them. See [`crate::pane_surfaces`].
            if !pane_surfaces::quickview_on_surface() {
                if let Some(session) = browser.quickview_visible() {
                    // Centred on the *window*, not the file area: Quick View
                    // is a card floating over the whole picker, and the
                    // action row is behind it rather than beside it.
                    let resting = quickview::panel_rect(frame.width, frame.height + frame.footer);
                    view::draw_quickview(canvas, &frame, session, resting);
                }
            }
            drop(frame);

            if let Some(session) = browser.rename.as_ref() {
                let (depth, index) = (session.depth, session.index);
                let (width, height) = (browser.size.0, browser.content_h());
                let count = browser.visible(depth).len();
                let scroll = browser.columns[depth].scroll.offset();
                let rect = match browser.mode {
                    ViewMode::List => {
                        view::list_rename_rect(width, browser.list_columns, count, scroll, index)
                    }
                    ViewMode::Columns => {
                        let is_dir = browser.visible(depth).get(index).is_some_and(|e| e.is_dir);
                        view::miller_rename_rect(
                            height,
                            browser.pan.offset(),
                            browser.miller_w,
                            depth,
                            count,
                            scroll,
                            index,
                            is_dir,
                        )
                    }
                    ViewMode::Grid => view::grid_rename_rect(width, height, scroll, index),
                };
                let session = browser.rename.as_mut().unwrap();
                session.input.set_size(rect.width(), rect.height());
                canvas.save();
                canvas.translate((rect.left, rect.top));
                session.input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
            }
        });

        // Also when only Quick View wants a surface: the columns stay in the
        // scene, and this carries the preview alone.
        if pane_surfaces::quickview_on_surface() {
            self.pane_surfaces = Some(pane_surfaces::PaneSurfaces::new(
                AppContext::scale_factor() as f32
            ));
        }

        self.install_quickview_pointer();
        self.install_info_window_pointer();
        self.install_pointer(&window, self.context_menu.clone().unwrap());
        self.install_frame_loop(&window);
        AppContext::register_window(window.clone());
        self.window = Some(window);
        Ok(())
    }

    /// Repaint whatever changed since the last pass, from wherever it changed.
    ///
    /// The frame-callback loop can only carry work that is already on screen:
    /// it sustains itself by committing frames, so a window that is drawing
    /// nothing new stops being called. A directory read or a decode finishing
    /// on another thread wakes the loop instead — see
    /// [`AppContext::request_wakeup`] — and this is where that wakeup turns
    /// into a frame.
    fn on_update(&mut self, _ctx: &AppContext) {
        let (repaint, preview_target, scrolled_only, thumb_jobs) = {
            let mut browser = self.state.lock().unwrap();
            let changed = browser.poll();
            // Momentum, the overscroll bounce and the scrollbar's fade all
            // advance here rather than on input, since they keep running after
            // the gesture ends.
            let scrolled = browser.tick_scroll();
            let animating = browser.quickview_animating() | browser.tick_quickview_exit();
            // The docked preview column follows the selection wherever it
            // moves — a click, an arrow key, a directory finishing a load
            // that changes what "the selection" resolves to — so this is
            // checked centrally here rather than threaded through every place
            // the selection can change.
            let preview_target = browser.sync_preview_target();
            // What the entries on screen still need a picture for. Same place
            // and same reasoning as the preview target above: everything that
            // can change what is visible — a scroll, a directory landing, a
            // switch of view mode — has already happened by the time this
            // runs.
            let thumb_jobs = browser.sync_thumbnails();
            // A sideways pan moves the columns, and the hairlines between them
            // are drawn in the window, not in the column surfaces — so while
            // the stack is panning the window has to keep up or the dividers
            // are left behind. The bar's fade is a good enough stand-in for
            // "the stack is moving": it is up for exactly that long.
            let pan_bar_visible =
                browser.mode == ViewMode::Columns && browser.pan.state.scrollbar_opacity() > 0.0;
            let scrolled_only = scrolled
                && !changed
                && !browser.dirty
                && !animating
                && preview_target.is_none()
                && !pan_bar_visible;
            let repaint = changed
                || scrolled
                || std::mem::take(&mut browser.dirty)
                || animating
                || preview_target.is_some();
            (repaint, preview_target, scrolled_only, thumb_jobs)
        };
        if let Some((path, generation)) = preview_target {
            self.start_preview(path, generation);
        }
        for job in thumb_jobs {
            self.start_thumbnail(job);
        }

        // One lock, taken once. A `self.state.lock()` in an `if` condition
        // holds its guard for the whole `if`, so locking again inside the body
        // deadlocks the update loop — which is exactly what it did.
        let open_now = {
            let mut browser = self.state.lock().unwrap();
            let depth = browser.active;
            let ready = browser.quickview_auto && !browser.visible(depth).is_empty();
            if ready {
                browser.quickview_auto = false;
                browser.columns[depth].cursor = Some(0);
            }
            ready
        };
        if open_now {
            let mut browser = self.state.lock().unwrap();
            self.start_quickview(&mut browser);
        }

        // With the columns in their own surfaces, a scroll is repainted there
        // and the window is left alone — which is the whole point, so the
        // scroll must not also count towards a window repaint.
        let mut repaint = repaint;
        if self.pane_surfaces.is_some() {
            let painted = self.sync_pane_surfaces();
            if scrolled_only && painted {
                repaint = false;
            }
            // A paint the throttle turned away is only ever retried by another
            // pass, and passes stop when the content stops changing. Keeping
            // the window repainting is what keeps them coming.
            if self
                .pane_surfaces
                .as_ref()
                .is_some_and(pane_surfaces::PaneSurfaces::pending)
            {
                repaint = true;
            }
        }

        if repaint {
            self.render();
        }

        self.sync_info_window();
        self.advance_picker();
    }

    /// A hand laid on the touchpad stops whatever is gliding, the way a
    /// finger on a spinning wheel does. Nothing else in the pointer stream
    /// says so: a hold carries no motion and no button.
    fn on_pointer_hold_begin(&mut self, _ctx: &AppContext, _fingers: u32) {
        let mut browser = self.state.lock().unwrap();
        browser.pan.stop();
        browser.gesture_axis = None;
        for column in &mut browser.columns {
            column.scroll.stop();
        }
        drop(browser);
        self.render();
    }

    /// A two-finger pinch zooms the open preview's picture.
    ///
    /// Only the preview: the browser's own views have no zoom, and a pinch
    /// with no panel up is left alone rather than repurposed into something
    /// the gesture does not mean anywhere else.
    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, fingers: u32) {
        let mut browser = self.state.lock().unwrap();
        // Where this gesture's scale is measured from. Taken at the start
        // because the protocol reports scale against the start.
        browser.quickview_pinch = (fingers == 2)
            .then(|| browser.quickview.as_ref().map(|s| s.zoom.scale))
            .flatten();
    }

    fn on_pointer_pinch_update(
        &mut self,
        _ctx: &AppContext,
        dx: f64,
        dy: f64,
        scale: f64,
        _rotation: f64,
    ) {
        let mut browser = self.state.lock().unwrap();
        let Some(base) = browser.quickview_pinch else {
            return;
        };
        let moved = browser.quickview_zoom_to(base * scale as f32, (dx as f32, dy as f32));
        drop(browser);
        if moved {
            self.render();
        }
    }

    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        // Nothing to settle: every update already left the zoom clamped and
        // snapped, so the fingers lifting only ends the gesture.
        self.state.lock().unwrap().quickview_pinch = None;
    }

    /// While something is gliding the app needs a steady clock, not just the
    /// next input event.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        let browser = self.state.lock().unwrap();
        let animating = browser.scroll_animating() || browser.quickview_animating();
        animating.then(|| std::time::Duration::from_millis(8))
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        if let (Some(w), Some(h)) = (configure.new_size.0, configure.new_size.1) {
            let mut browser = self.state.lock().unwrap();
            browser.size = (w.get() as f32, h.get() as f32);
            browser.dirty = true;
        }
        self.render();
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        key_state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        use smithay_client_toolkit::seat::keyboard::Keysym;

        // Track the modifiers on both edges, before the press-only guard.
        let pressed = key_state == wl_keyboard::KeyState::Pressed;
        if matches!(event.keysym, Keysym::Control_L | Keysym::Control_R) {
            self.modifiers.lock().unwrap().0 = pressed;
            return;
        }
        if matches!(event.keysym, Keysym::Shift_L | Keysym::Shift_R) {
            self.modifiers.lock().unwrap().1 = pressed;
            return;
        }
        if !pressed {
            return;
        }
        let (ctrl, shift) = *self.modifiers.lock().unwrap();

        {
            let mut browser = self.state.lock().unwrap();

            // An in-place rename owns the keyboard outright: every key is
            // text-field input, not a browser shortcut.
            if browser.rename.is_some() {
                let key = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => Some(TextInputKey::Enter),
                    Keysym::Escape => Some(TextInputKey::Escape),
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = key {
                    let mods = KeyMods { shift, ctrl };
                    let response = browser
                        .rename
                        .as_mut()
                        .map(|session| session.input.on_key(key, mods));
                    match response {
                        Some(TextInputResponse::Commit) => browser.commit_rename(),
                        Some(TextInputResponse::Cancel) => browser.cancel_rename(),
                        Some(_) => browser.dirty = true,
                        None => {}
                    }
                }
                drop(browser);
                self.render();
                return;
            }

            // Set by the type-ahead arm below: every other key ends the
            // word being typed, the way a second of silence does.
            let mut typing = false;

            match event.keysym {
                Keysym::Down => {
                    let step = browser.row_step();
                    browser.move_cursor(step, shift)
                }
                Keysym::Up => {
                    let step = browser.row_step();
                    browser.move_cursor(-step, shift)
                }
                // The grid is two-dimensional: sideways is the next cell, not
                // a move in or out of a directory the way it is in the list
                // and Miller views.
                Keysym::Right if browser.mode == ViewMode::Grid => browser.move_cursor(1, shift),
                Keysym::Left if browser.mode == ViewMode::Grid => browser.move_cursor(-1, shift),
                Keysym::Right => browser.move_lateral(1),
                Keysym::Left => browser.move_lateral(-1),
                Keysym::Return | Keysym::KP_Enter => {
                    // In the picker, Return means "this one" — descend into a
                    // directory or accept a file. Renaming is file management,
                    // which the picker does not do.
                    if browser.picker.is_some() {
                        browser.open_selection();
                    } else {
                        browser.start_rename();
                    }
                }
                // The Linux convention, alongside Return: both rename.
                Keysym::F2 => browser.start_rename(),
                Keysym::BackSpace => browser.go_up(),
                Keysym::Home => browser.move_cursor(-100_000, shift),
                Keysym::End => browser.move_cursor(100_000, shift),
                Keysym::Page_Down => {
                    let step = browser.row_step();
                    browser.move_cursor(15 * step, shift)
                }
                Keysym::Page_Up => {
                    let step = browser.row_step();
                    browser.move_cursor(-15 * step, shift)
                }
                // Select-all only means something when the request asked for
                // more than one file.
                Keysym::a if ctrl => {
                    let multiple = browser.picker.as_ref().is_none_or(|p| p.request.multiple);
                    if multiple {
                        browser.select_all();
                    }
                }
                // Cut, copy and paste are file management: browser only.
                Keysym::c if ctrl && browser.picker.is_none() => {
                    browser.copy_selection(false, serial)
                }
                Keysym::x if ctrl && browser.picker.is_none() => {
                    browser.copy_selection(true, serial)
                }
                Keysym::v if ctrl && browser.picker.is_none() => browser.paste(),
                // Space toggles: the second press dismisses what the first
                // opened, which is the gesture people already have.
                Keysym::space => {
                    if !browser.close_quickview() {
                        self.start_quickview(&mut browser);
                    }
                }
                // Escape unwinds one layer at a time: the preview, then the
                // filter menu, then — in the picker — the request itself.
                Keysym::Escape => {
                    let menu_open = browser.picker.as_ref().is_some_and(|p| p.filter_open);
                    // The panel is not modal, so Escape does not belong to it
                    // outright — it takes its turn in the same unwinding order
                    // as everything else that is up.
                    if browser.info.is_some() {
                        browser.close_info();
                    } else if browser.close_quickview() {
                        // The preview took it.
                    } else if menu_open {
                        if let Some(session) = browser.picker.as_mut() {
                            session.filter_open = false;
                        }
                        browser.dirty = true;
                    } else if browser.picker.is_some() {
                        browser.picker_cancel();
                    } else {
                        browser.clear_selection();
                    }
                }
                // A second press closes it, the way Space does for Quick
                // View: the shortcut that opened the panel is the one already
                // under the user's fingers.
                Keysym::i if ctrl => {
                    if browser.info.is_some() {
                        browser.close_info();
                    } else {
                        browser.open_info();
                    }
                }
                Keysym::h if ctrl => {
                    browser.show_hidden = !browser.show_hidden;
                    browser.dirty = true;
                }
                Keysym::_1 if ctrl => {
                    browser.mode = ViewMode::List;
                    browser.dirty = true;
                }
                Keysym::_2 if ctrl => {
                    browser.mode = ViewMode::Grid;
                    browser.dirty = true;
                }
                Keysym::_3 if ctrl => {
                    browser.mode = ViewMode::Columns;
                    browser.dirty = true;
                }
                // Anything else printable is type-ahead. It comes last so
                // that every shortcut above keeps the key it already had.
                _ => {
                    if let Some(ch) = event
                        .utf8
                        .as_deref()
                        .filter(|_| !ctrl)
                        .and_then(|text| text.chars().next())
                        .filter(|ch| !ch.is_control() && *ch != ' ')
                    {
                        browser.typeahead(ch);
                        typing = true;
                    }
                }
            }
            if !typing {
                browser.typeahead = None;
            }

            // The preview follows the cursor: arrow-keying through a folder
            // re-decodes in place rather than dismissing. The generation on
            // each decode is what keeps a slow file from landing late.
            let moved = matches!(
                event.keysym,
                Keysym::Down
                    | Keysym::Up
                    | Keysym::Left
                    | Keysym::Right
                    | Keysym::Home
                    | Keysym::End
                    | Keysym::Page_Down
                    | Keysym::Page_Up
            );
            if moved && browser.quickview.is_some() {
                self.start_quickview(&mut browser);
            }
        }
        self.render();
    }
}

impl FilesApp {
    /// Move the picker on when its request is done with, and notice when the
    /// portal withdraws the one on screen.
    ///
    /// A no-op in the browser, and on every pass where the window is still
    /// serving a live request — which is nearly all of them.
    fn advance_picker(&mut self) {
        let Some(queue) = self.picker_queue.clone() else {
            return;
        };

        {
            let mut browser = self.state.lock().unwrap();
            let Some(session) = browser.picker.as_mut() else {
                return;
            };
            // Withdrawn by the portal — the requesting application went away,
            // or gave up. The window goes immediately; there is nobody left
            // to answer.
            if !session.answered() && queue.take_withdrawn(&session.request.handle) {
                session.resolve(picker::Outcome::ended());
            }
            if !session.answered() {
                return;
            }
        }

        // The request has been answered. Serve the next one in the same
        // window, or leave — an idle picker holds no window and no Wayland
        // connection, and the bus starts a fresh one when it is next needed.
        match queue.next_session() {
            Some(session) => {
                let start = session.request.starting_directory(None);
                let mut browser = self.state.lock().unwrap();
                let size = browser.size;
                *browser = Browser::for_picker(session, start);
                browser.size = size;
                browser.dirty = true;
                drop(browser);
                self.render();
            }
            None => AppContext::request_exit(),
        }
    }

    /// Repaint whichever column subsurfaces are out of date. Returns whether
    /// any of them actually painted.
    fn sync_pane_surfaces(&mut self) -> bool {
        let Some(window) = self.window.as_ref() else {
            return false;
        };
        let Some(surface) = window.surface() else {
            return false;
        };
        let parent = surface.wl_surface().clone();
        let mut browser = self.state.lock().unwrap();
        browser.sync_scroll_metrics();
        let theme = AppContext::current_theme();
        let title = browser.title();
        let frame = browser.frame(&theme, &title);
        let quickview = browser
            .quickview_visible()
            .map(|session| (session, browser.quickview_generation));
        let painted = match self.pane_surfaces.as_mut() {
            Some(panes) => panes.sync(&parent, &frame, quickview),
            None => false,
        };

        // Hand the pointer handler the rect the panel was actually placed at.
        // Doing it here, after the sync, is what keeps the hit test and the
        // paint from disagreeing about where the card is.
        drop(frame);
        browser.quickview_panel = self
            .pane_surfaces
            .as_ref()
            .and_then(pane_surfaces::PaneSurfaces::quickview_resting);
        *self.quickview_target.lock().unwrap() = self
            .pane_surfaces
            .as_ref()
            .and_then(pane_surfaces::PaneSurfaces::quickview_target);
        painted
    }

    fn render(&self) {
        if let Some(window) = &self.window {
            window.request_frame();
            // The runner renders dirty windows at the *top* of a loop
            // iteration, before `on_update`, so a frame requested from
            // `on_update` would sit unrendered until some other event happened
            // to wake the loop. Asking for one more turn is what makes a
            // repaint requested off the input path actually appear.
            AppContext::request_wakeup();
        }
    }

    /// Preview the cursor's file, decoding off the UI thread.
    ///
    /// [`quickview::decode`] blocks until the sandboxed worker answers or its
    /// deadline expires — inline, that would stall the frame loop for as long
    /// as the file takes.
    fn start_quickview(&self, browser: &mut Browser) {
        let Some((path, generation, anchor)) = browser.begin_quickview() else {
            return;
        };
        let panel = quickview::panel_rect(browser.size.0, browser.size.1);
        let scale = AppContext::scale_factor().max(1) as f32;
        let state = Arc::clone(&self.state);

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            let preview = quickview::decode(&path, panel, scale);
            state
                .lock()
                .unwrap()
                .finish_quickview(generation, anchor, name, preview);
            // Wake the UI thread: a window showing "Opening preview…" is not
            // committing frames, so there is no frame callback to notice the
            // decode landed.
            AppContext::request_wakeup();
        });
    }

    /// Decode the docked preview column's target, off the UI thread — the
    /// same worker path Quick View's overlay uses, just landing in
    /// [`Browser::finish_preview`] instead of [`Browser::finish_quickview`].
    fn start_preview(&self, path: PathBuf, generation: u64) {
        let panel = {
            let browser = self.state.lock().unwrap();
            Rect::from_wh(view::PREVIEW_W, browser.size.1)
        };
        let scale = AppContext::scale_factor().max(1) as f32;
        let state = Arc::clone(&self.state);

        tokio::task::spawn_blocking(move || {
            let preview = quickview::decode(&path, panel, scale);
            state.lock().unwrap().finish_preview(generation, preview);
            AppContext::request_wakeup();
        });
    }

    /// Fetch one thumbnail off the UI thread.
    ///
    /// The shared cache makes most of these a single file read; the rest end
    /// in the sandboxed decoder, which is why this is never run inline. The
    /// result is recorded whatever it is — a miss is worth remembering, or the
    /// same file is asked for again on the very next frame.
    fn start_thumbnail(&self, job: thumbnails::Job) {
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let found = thumbnails::fetch(&job);
            let mut browser = state.lock().unwrap();
            let before = browser.thumbs.epoch();
            browser.thumbs.finish(job.path, job.modified, found);
            // A picture landing changes what the panes draw; a miss changes
            // only what will be asked for next, and repainting for it would
            // render the same pixels again. The store's epoch is what tells
            // the two apart.
            browser.dirty |= browser.thumbs.epoch() != before;
            drop(browser);
            // Same reason the preview decodes wake the loop: a window that has
            // stopped committing frames has no frame callback to notice a
            // thumbnail landed.
            AppContext::request_wakeup();
        });
    }

    /// Keep repainting while a directory read is outstanding.
    ///
    /// Two constraints force this shape. A worker thread cannot ask for a
    /// repaint at all — `AppContext::request_frame` dispatches through a
    /// thread-local and is a silent no-op anywhere but the UI thread. And
    /// asking from *inside* the draw does not reliably schedule another frame:
    /// measured, a request made during a draw is honoured only when some other
    /// event also triggers a render, so a read finishing after the last input
    /// is never shown. The frame callback runs on the UI thread and after the
    /// frame is acknowledged, which is the one place both hold.
    ///
    /// The loop sustains itself only while something is loading, so an idle
    /// window costs nothing.
    fn install_frame_loop(&self, window: &Window) {
        use wayland_client::Proxy;

        let Some(surface) = window.wl_surface() else {
            return;
        };
        let state = Arc::clone(&self.state);
        let window = window.clone();

        AppContext::register_frame_callback(surface.id(), move || {
            let repaint = {
                let mut browser = state.lock().unwrap();
                // A read that lands *here* is not in the frame just drawn, and
                // clears `loading` — so without the `changed` arm the column
                // that finished would sit empty until the next input event.
                let changed = browser.poll();
                // Also while a Quick View call is outstanding, so its result
                // paints without waiting for the next keystroke.
                // …and while the preview's entrance is still running, which is
                // animated in this process now that the panel lives in this
                // window's own surface.
                let opening = browser.quickview_animating();
                let preview_pending = browser.preview.as_ref().is_some_and(|p| p.pending);
                changed
                    || browser.loading()
                    || browser.quickview_pending
                    || opening
                    || preview_pending
                    // …and while thumbnails are being fetched, so they appear
                    // as they land rather than at the next keystroke.
                    || browser.thumbs.is_busy()
            };
            if repaint {
                window.request_frame();
            }
        });
    }

    /// Quick View's panel handles its own pointer, because it is the one
    /// surface of this window that is routinely *outside* it.
    ///
    /// Centred on the display, the card hangs past the toplevel's edges, and
    /// the compositor delivers events over that part to this surface — never
    /// to the toplevel. Hit-testing the button in window coordinates
    /// therefore misses it exactly when the panel is placed correctly.
    ///
    /// The close button and the panel's own scrolling are handled here.
    /// Both are things the pointer does *over* the card, and over the card is
    /// exactly where the toplevel never hears about it. Dismissing on a click
    /// outside the panel stays with the toplevel, where the rest of the
    /// browser's hit-testing already lives — a click outside the card is a
    /// click on the window.
    fn install_quickview_pointer(&self) {
        let state = Arc::clone(&self.state);
        let target = Arc::clone(&self.quickview_target);

        AppContext::register_pointer_callback(move |events| {
            for event in events {
                use wayland_client::Proxy;
                let Some((surface, panel)) = target.lock().unwrap().clone() else {
                    continue;
                };
                if event.surface.id() != surface {
                    continue;
                }
                // Surface-local already: the compositor reports positions
                // against the surface the pointer is over. Everything derived
                // from `panel` below is in that same space.
                let point = skia_safe::Point::new(event.position.0 as f32, event.position.1 as f32);
                let over = view::quickview_close_rect(panel)
                    .with_outset((4.0, 4.0))
                    .contains(point);

                let mut browser = state.lock().unwrap();
                match event.kind {
                    PointerEventKind::Press { .. } if over => {
                        browser.close_quickview();
                    }
                    PointerEventKind::Press { .. } => {
                        // A scrollbar over a zoomed picture takes the press
                        // before anything else does.
                        browser.quickview_pan_pointer(QuickviewPointer::Press, point, panel);
                    }
                    PointerEventKind::Release { .. } => {
                        browser.quickview_pan_pointer(QuickviewPointer::Release, point, panel);
                    }
                    PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                        browser.quickview_focus(point, panel);
                        browser.quickview_pan_pointer(QuickviewPointer::Motion, point, panel);
                        if browser.quickview_close_hovered != over {
                            browser.quickview_close_hovered = over;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        browser.quickview_focus = None;
                        browser.quickview_pan_pointer(QuickviewPointer::Leave, point, panel);
                        if browser.quickview_close_hovered {
                            browser.quickview_close_hovered = false;
                            browser.dirty = true;
                        }
                    }
                    PointerEventKind::Axis {
                        vertical,
                        horizontal,
                        ..
                    } => {
                        browser.quickview_wheel(
                            horizontal.absolute as f32,
                            vertical.absolute as f32,
                            panel,
                            vertical.stop || horizontal.stop,
                            vertical.discrete != 0 || horizontal.discrete != 0,
                        );
                    }
                }
                drop(browser);
                AppContext::request_wakeup();
            }
        });
    }

    /// Bring the Get Info panel's window into line with the browser's state:
    /// open one when there is something to show, take it away when there is
    /// not, and repaint it when what it shows changes.
    ///
    /// The panel is a window of its own rather than a sheet drawn over this
    /// one. It is dragged around, it stays put while the browser goes on
    /// being used behind it, and it wants the shadow and the stacking every
    /// other window gets — all of which the compositor already does for a
    /// toplevel and none of which is worth rebuilding inside this window.
    /// It carries the browser's own `app_id`, so it lands under the same dock
    /// icon instead of adding one of its own.
    fn sync_info_window(&mut self) {
        let (wanted, dirty) = {
            let mut browser = self.state.lock().unwrap();
            (
                browser.info.is_some(),
                std::mem::take(&mut browser.info_dirty),
            )
        };
        // Never with the cell borrowed: creating a window talks to the
        // compositor, which dispatches events — the panel's own pointer
        // callback among them — and that callback reads this same cell.
        let open = self.info_window.borrow().is_some();
        match (wanted, open) {
            (true, false) => {
                let window = self.create_info_window();
                *self.info_window.borrow_mut() = window;
            }
            (false, true) => {
                let window = self.info_window.borrow_mut().take();
                if let Some(window) = window {
                    window.close();
                }
            }
            _ => {}
        }
        if dirty {
            let window = self.info_window.borrow().clone();
            if let Some(window) = window {
                window.request_frame();
                AppContext::request_wakeup();
            }
        }
    }

    fn create_info_window(&self) -> Option<Window> {
        let mut window = Window::new("Info", view::INFO_W as i32, view::INFO_H as i32).ok()?;
        // The layout inside is fixed, so the window does not resize.
        window.set_min_size(view::INFO_W as u32, view::INFO_H as u32);
        window.set_max_size(view::INFO_W as u32, view::INFO_H as u32);
        // The card paints its own ground, and the compositor rounds and
        // shadows the surface around it.
        window.set_background(skia_safe::Color::TRANSPARENT);
        if let Some(surface) = window.surface() {
            // The browser's own app_id: this is another window of the file
            // manager, not another application, and the dock and the app
            // switcher should both read it that way.
            surface.xdg_window().set_app_id("otto-files".to_string());
        }
        if let Some(style) = window.surface_style() {
            style.set_corner_radius(14.0);
        }

        let state = Arc::clone(&self.state);
        window.on_draw(move |canvas| {
            let browser = state.lock().unwrap();
            let Some(info) = browser.info.as_ref() else {
                return;
            };
            let theme = AppContext::current_theme();
            view::draw_info(
                canvas,
                &theme,
                Rect::from_wh(view::INFO_W, view::INFO_H),
                info,
                browser.info_error.as_deref(),
                browser.info_close_hovered,
                false,
            );
        });

        // A close asked for from outside — the app switcher, a keyboard
        // shortcut, the dock's Quit — closes the panel. Without this the
        // runner would take it for the application's own close request and
        // end the process, which is a surprising way for Get Info to go away.
        let state = Arc::clone(&self.state);
        window.on_close_request(move || state.lock().unwrap().close_info());

        Some(window)
    }

    /// The Get Info window's pointer. Registered once, for whichever panel is
    /// open — see [`FilesApp::info_window`].
    fn install_info_window_pointer(&self) {
        let state = Arc::clone(&self.state);
        let info_window = Rc::clone(&self.info_window);

        AppContext::register_pointer_callback(move |events| {
            use wayland_client::Proxy;
            let window = info_window.borrow().clone();
            let Some(window) = window else { return };
            let Some(surface) = window.wl_surface() else {
                return;
            };
            // The window is the card, so surface-local coordinates are the
            // panel's own and nothing has to be converted.
            let sheet = Rect::from_wh(view::INFO_W, view::INFO_H);

            for event in events {
                if event.surface.id() != surface.id() {
                    continue;
                }
                let point = (event.position.0 as f32, event.position.1 as f32);
                let drag = {
                    let mut browser = state.lock().unwrap();
                    info_pointer(&mut browser, &event.kind, sheet, point)
                };
                if drag {
                    if let PointerEventKind::Press { serial, .. } = event.kind {
                        if let Some(seat) = AppContext::seat_state().seats().next() {
                            window.start_move(&seat, serial);
                        }
                    }
                }
                AppContext::request_wakeup();
            }
        });
    }

    fn install_pointer(&self, window: &Window, context_menu: ContextMenu) {
        let state = Arc::clone(&self.state);
        let window_for_events = window.clone();
        let modifiers = Arc::clone(&self.modifiers);

        window.on_pointer_event(move |events| {
            for event in events {
                let (x, y) = (event.position.0 as f32, event.position.1 as f32);
                let (ctrl, shift) = *modifiers.lock().unwrap();
                let mut browser = state.lock().unwrap();
                // The file area's bottom, not the window's: every hit test
                // below is against the listing, which stops short of the
                // picker's action row. The row's own hit test uses the full
                // window height and runs first.
                let (width, height) = (browser.size.0, browser.content_h());

                // An in-place rename owns the pointer while it is up: a click
                // inside places the caret, a click anywhere else commits it
                // the way clicking away from a Finder rename does.
                if let Some(session) = browser.rename.as_ref() {
                    if let PointerEventKind::Press { .. } = event.kind {
                        let (depth, index) = (session.depth, session.index);
                        let count = browser.visible(depth).len();
                        let scroll = browser.columns[depth].scroll.offset();
                        let rect = match browser.mode {
                            ViewMode::List => view::list_rename_rect(
                                width,
                                browser.list_columns,
                                count,
                                scroll,
                                index,
                            ),
                            ViewMode::Columns => {
                                let is_dir =
                                    browser.visible(depth).get(index).is_some_and(|e| e.is_dir);
                                view::miller_rename_rect(
                                    height,
                                    browser.pan.offset(),
                                    browser.miller_w,
                                    depth,
                                    count,
                                    scroll,
                                    index,
                                    is_dir,
                                )
                            }
                            ViewMode::Grid => view::grid_rename_rect(width, height, scroll, index),
                        };
                        if rect.contains(skia_safe::Point::new(x, y)) {
                            if let Some(session) = browser.rename.as_mut() {
                                session.input.on_pointer_down(x - rect.left, 1, shift);
                            }
                            browser.dirty = true;
                        } else {
                            browser.commit_rename();
                        }
                    }
                    drop(browser);
                    window_for_events.request_frame();
                    continue;
                }

                // An open preview owns the pointer, the way the sheet does: a
                // click outside dismisses it, the wheel scrolls its content, and
                // nothing reaches the listing underneath.
                if browser.quickview.is_some() {
                    // Where the panel *is*, which is not where the window's
                    // centre is once the compositor has centred it on the
                    // display. The fallback is the window-centred rect, and
                    // the window height is right for it: the panel floats
                    // over the action row rather than beside it.
                    let panel = browser
                        .quickview_panel
                        .unwrap_or_else(|| quickview::panel_rect(width, browser.size.1));
                    let point = skia_safe::Point::new(x, y);
                    let over_close = view::quickview_close_rect(panel)
                        .with_outset((4.0, 4.0))
                        .contains(point);
                    match event.kind {
                        PointerEventKind::Press { .. } => {
                            // The button first: it sits inside the panel, so
                            // the "click outside dismisses" rule below would
                            // never reach it. Then the pan's scrollbars,
                            // which are inside it too.
                            if over_close || !panel.contains(point) {
                                browser.close_quickview();
                            } else {
                                browser.quickview_pan_pointer(
                                    QuickviewPointer::Press,
                                    point,
                                    panel,
                                );
                            }
                        }
                        PointerEventKind::Release { .. } => {
                            browser.quickview_pan_pointer(QuickviewPointer::Release, point, panel);
                        }
                        PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
                            browser.quickview_focus(point, panel);
                            browser.quickview_pan_pointer(QuickviewPointer::Motion, point, panel);
                            if browser.quickview_close_hovered != over_close {
                                browser.quickview_close_hovered = over_close;
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            browser.quickview_focus = None;
                            browser.quickview_pan_pointer(QuickviewPointer::Leave, point, panel);
                            if browser.quickview_close_hovered {
                                browser.quickview_close_hovered = false;
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Axis {
                            vertical,
                            horizontal,
                            ..
                        } => {
                            browser.quickview_wheel(
                                horizontal.absolute as f32,
                                vertical.absolute as f32,
                                panel,
                                vertical.stop || horizontal.stop,
                                vertical.discrete != 0 || horizontal.discrete != 0,
                            );
                        }
                    }
                    drop(browser);
                    continue;
                }

                // The picker's action row, and the filter menu it opens,
                // take the pointer before the listing does. Their geometry is
                // in *window* coordinates — `height` above is the file area's
                // bottom, which is exactly where this strip begins.
                if browser.picker.is_some() {
                    let window_h = browser.size.1;
                    let (filter_count, menu_open) = browser
                        .picker
                        .as_ref()
                        .map(|p| (p.filters.len(), p.filter_open))
                        .unwrap_or((0, false));
                    let hit = view::footer_at(x, y, width, window_h, filter_count, menu_open);
                    // A click anywhere outside the open menu closes it, the
                    // way clicking away from any menu does — including a
                    // click on the listing, which is then swallowed.
                    let dismissing_menu = menu_open
                        && !matches!(
                            hit,
                            Some(view::FooterButton::FilterOption(_))
                                | Some(view::FooterButton::Filter)
                        );

                    match event.kind {
                        PointerEventKind::Motion { .. } => {
                            if browser.footer_hover != hit {
                                browser.footer_hover = hit;
                                browser.dirty = true;
                            }
                            if hit.is_some() {
                                AppContext::set_cursor_shape(CursorShape::Default);
                                drop(browser);
                                continue;
                            }
                        }
                        PointerEventKind::Leave { .. } => {
                            if browser.footer_hover.take().is_some() {
                                browser.dirty = true;
                            }
                        }
                        PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                            if dismissing_menu {
                                if let Some(session) = browser.picker.as_mut() {
                                    session.filter_open = false;
                                }
                                browser.dirty = true;
                                drop(browser);
                                window_for_events.request_frame();
                                continue;
                            }
                            if let Some(button) = hit {
                                browser.footer_press(button);
                                drop(browser);
                                window_for_events.request_frame();
                                continue;
                            }
                        }
                        PointerEventKind::Release { button, .. } if button != BTN_RIGHT => {
                            if browser.footer_pressed.is_some() {
                                browser.footer_release(hit);
                                drop(browser);
                                window_for_events.request_frame();
                                continue;
                            }
                        }
                        _ => {}
                    }
                }

                match event.kind {
                    PointerEventKind::Motion { .. } => {
                        // A column divider being dragged owns the pointer
                        // outright — nothing else on this move should react.
                        if let Some((boundary, start_x, start_w)) = browser.column_resize {
                            let dx = x - start_x;
                            let new_w = (start_w - dx).clamp(view::COLUMN_MIN_W, 400.0);
                            match boundary {
                                view::ColumnBoundary::Size => browser.list_columns.size = new_w,
                                view::ColumnBoundary::Kind => browser.list_columns.kind = new_w,
                                view::ColumnBoundary::Modified => {
                                    browser.list_columns.modified = new_w
                                }
                            }
                            browser.dirty = true;
                            AppContext::set_cursor_shape(CursorShape::ColResize);
                            drop(browser);
                            continue;
                        }
                        if let Some((depth, start_x, start_w)) = browser.miller_resize {
                            let dx = (x - start_x) / (depth + 1) as f32;
                            browser.miller_w =
                                (start_w + dx).clamp(view::MILLER_MIN_W, view::MILLER_MAX_W);
                            browser.dirty = true;
                            AppContext::set_cursor_shape(CursorShape::ColResize);
                            drop(browser);
                            continue;
                        }

                        // Resize affordance at the window edges.
                        let edge = resize::edge_at(Rect::from_wh(width, browser.size.1), x, y);
                        let over_column_divider = (browser.mode == ViewMode::List
                            && view::column_boundary_at(x, y, width, browser.list_columns)
                                .is_some())
                            || (browser.mode == ViewMode::Columns
                                && view::miller_boundary_at(
                                    x,
                                    y,
                                    width,
                                    height,
                                    browser.pan.offset(),
                                    browser.columns.len(),
                                    browser.miller_w,
                                )
                                .is_some());
                        let shape = if over_column_divider {
                            CursorShape::ColResize
                        } else {
                            edge.map_or(CursorShape::Default, |e| e.cursor())
                        };
                        AppContext::set_cursor_shape(shape);

                        // A scrollbar drag follows the pointer wherever it
                        // goes, so the dragged pane is asked first and the
                        // hovered one only styles its bar.
                        browser.sync_scroll_metrics();
                        let hovered = browser.pane_under(x, y);
                        let mut moved = browser.pan.on_pointer_drag(x, y);
                        if browser.mode == ViewMode::Columns {
                            moved |= browser.pan.on_pointer_move(x, y);
                        } else {
                            browser.pan.on_pointer_leave();
                        }
                        for (depth, column) in browser.columns.iter_mut().enumerate() {
                            moved |= column.scroll.on_pointer_drag(x, y);
                            if depth == hovered {
                                // Hovering the scrollbar keeps it up and
                                // widens it, the way the settings app does.
                                moved |= column.scroll.on_pointer_move(x, y);
                            } else {
                                column.scroll.on_pointer_leave();
                            }
                        }
                        browser.dirty |= moved;

                        // The traffic lights reveal their glyphs while the
                        // pointer is over the group.
                        let control = view::control_at(x, y);
                        browser.dirty |= browser.controls.on_motion(control);
                    }
                    PointerEventKind::Release { .. } => {
                        browser.column_resize = None;
                        browser.miller_resize = None;
                        browser.pan.on_pointer_up();
                        for column in &mut browser.columns {
                            column.scroll.on_pointer_up();
                        }

                        // A nav arrow steps on release, and only over the half
                        // the press landed on: a press dragged off it is a
                        // cancelled click, and only clears the fill.
                        if let Some(armed) = browser.nav_pressed.take() {
                            browser.dirty = true;
                            if browser.nav_button_at(x, y) == Some(armed) {
                                match armed {
                                    view::NavButton::Back => browser.go_back(),
                                    view::NavButton::Forward => browser.go_forward(),
                                }
                            }
                        }

                        // A control fires on release, and only over the dot
                        // the press landed on.
                        let control = view::control_at(x, y);
                        browser.dirty |= browser.controls.pressed().is_some();
                        match browser.controls.on_release(control) {
                            // In the picker, closing the window *is*
                            // cancelling: exiting here would leave the
                            // requesting application waiting on a reply that
                            // no longer has a sender.
                            Some(WindowControl::Close) => {
                                if browser.picker.is_some() {
                                    browser.picker_cancel();
                                } else {
                                    std::process::exit(0)
                                }
                            }
                            Some(WindowControl::Minimize) => window_for_events.minimize(),
                            Some(WindowControl::Zoom) => window_for_events.toggle_maximized(),
                            None => {}
                        }
                    }
                    PointerEventKind::Leave { .. } => {
                        browser.column_resize = None;
                        browser.miller_resize = None;
                        browser.pan.on_pointer_leave();
                        for column in &mut browser.columns {
                            column.scroll.on_pointer_leave();
                        }
                        // Nothing in the header is hovered once the pointer is
                        // off the window, or the glyphs stay drawn on it.
                        browser.controls.on_leave();
                        // Same for a held arrow: the release will never come.
                        browser.nav_pressed = None;
                        browser.dirty = true;
                    }
                    PointerEventKind::Press { serial, button, .. } if button == BTN_RIGHT => {
                        let items = browser.context_menu_items(x, y);
                        drop(browser);

                        let Some(parent_xdg) = window_for_events
                            .surface()
                            .map(|s| s.xdg_window().xdg_surface().clone())
                        else {
                            continue;
                        };
                        let Ok(positioner) = XdgPositioner::new(AppContext::xdg_shell_state())
                        else {
                            continue;
                        };

                        context_menu.state().borrow_mut().set_items(items);
                        let theme = AppContext::current_theme();
                        context_menu
                            .clone()
                            .with_style(ContextMenuStyle::default().with_theme(theme));

                        let (menu_w, menu_h) = context_menu.get_size_at_depth(0);
                        positioner.set_size(menu_w as i32, menu_h as i32);
                        positioner.set_anchor_rect(x as i32, y as i32, 1, 1);
                        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
                        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
                        positioner.set_constraint_adjustment(
                            xdg_positioner::ConstraintAdjustment::SlideX
                                | xdg_positioner::ConstraintAdjustment::SlideY
                                | xdg_positioner::ConstraintAdjustment::FlipX
                                | xdg_positioner::ConstraintAdjustment::FlipY,
                        );

                        let click_state = Arc::clone(&state);
                        let click_window = window_for_events.clone();
                        context_menu.clone().on_item_click(move |action_id| {
                            let mut browser = click_state.lock().unwrap();
                            match action_id {
                                "open" => browser.open_selection(),
                                "get_info" => browser.open_info(),
                                "rename" => browser.start_rename(),
                                "cut" => browser.copy_selection(true, serial),
                                "copy" => browser.copy_selection(false, serial),
                                "paste" => browser.paste(),
                                "trash" => browser.move_selected_to_trash(),
                                "new_folder" => browser.new_folder(),
                                _ => {}
                            }
                            drop(browser);
                            click_window.request_frame();
                        });

                        context_menu.show(&parent_xdg, &positioner, serial);
                        continue;
                    }
                    PointerEventKind::Press { serial, .. } => {
                        if let Some(edge) = resize::edge_at(Rect::from_wh(width, height), x, y) {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                window_for_events.start_resize(&seat, serial, edge);
                            }
                            return;
                        }

                        // Arming rather than acting: the control fires on
                        // release, over the same dot.
                        if browser.controls.on_press(view::control_at(x, y)) {
                            browser.dirty = true;
                            return;
                        }

                        // Dragging the header moves the window, in every view. The
                        // sidebar's first place reaches into the header band, so a
                        // click there belongs to the place, not to the drag.
                        if view::is_drag_area(x, y, width)
                            && view::place_at(x, y, browser.places.len()).is_none()
                        {
                            if let Some(seat) = AppContext::seat_state().seats().next() {
                                window_for_events.start_move(&seat, serial);
                            }
                            return;
                        }

                        // A press on a scrollbar thumb grabs it and selects
                        // nothing: the bar sits over the rows it scrolls, so
                        // it has to win the click.
                        browser.sync_scroll_metrics();
                        // The stack's bar lies along the bottom of every
                        // pane, crossing the foot of each pane's own gutter,
                        // so it is asked first where the two overlap.
                        let depth = browser.pane_under(x, y);
                        let panning = browser.mode == ViewMode::Columns;
                        if (panning && browser.pan.on_pointer_down(x, y))
                            || browser.columns[depth].scroll.on_pointer_down(x, y)
                        {
                            browser.dirty = true;
                            drop(browser);
                            continue;
                        }

                        if let Some(button) = browser.nav_button_at(x, y) {
                            // Armed, not acted on: the step happens on
                            // release, over the same half, so the arrow can
                            // sit visibly pressed in the meantime.
                            browser.nav_pressed = Some(button);
                            browser.dirty = true;
                        } else if let Some(mode) = view::switcher_at(x, y, width) {
                            browser.mode = mode;
                            browser.dirty = true;
                        } else if let Some(index) = view::place_at(x, y, browser.places.len()) {
                            let path = browser.places[index].path.clone();
                            browser.navigate_to(&path);
                        } else if browser.mode == ViewMode::Grid {
                            let depth = browser.columns.len() - 1;
                            let count = browser.visible(depth).len();
                            let scroll = browser.columns[depth].scroll.offset();
                            let area = view::content_viewport(width, height, ViewMode::Grid);
                            if let Some(index) = view::grid_cell_at(area, x, y, count, scroll) {
                                if ctrl {
                                    browser.toggle_select(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    browser.select(depth, index);
                                    browser.note_row_click(depth, index);
                                }
                            }
                        } else if browser.mode == ViewMode::List
                            && view::column_boundary_at(x, y, width, browser.list_columns).is_some()
                        {
                            let boundary =
                                view::column_boundary_at(x, y, width, browser.list_columns)
                                    .unwrap();
                            let now = std::time::Instant::now();
                            let double_click =
                                browser.last_boundary_click.is_some_and(|(last, at)| {
                                    last == boundary && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                                });
                            if double_click && boundary == view::ColumnBoundary::Size {
                                let depth = browser.columns.len() - 1;
                                let longest = view::widest_name(
                                    browser.visible(depth).iter().map(|e| e.name.as_str()),
                                );
                                browser.list_columns.size =
                                    view::fit_size_column(width, browser.list_columns, longest);
                                browser.last_boundary_click = None;
                                browser.dirty = true;
                            } else {
                                let start = match boundary {
                                    view::ColumnBoundary::Size => browser.list_columns.size,
                                    view::ColumnBoundary::Kind => browser.list_columns.kind,
                                    view::ColumnBoundary::Modified => browser.list_columns.modified,
                                };
                                browser.column_resize = Some((boundary, x, start));
                                browser.last_boundary_click = Some((boundary, now));
                            }
                        } else if browser.mode == ViewMode::List {
                            if let Some(key) = view::column_at(x, y, width, browser.list_columns) {
                                if browser.sort == key {
                                    browser.ascending = !browser.ascending;
                                } else {
                                    browser.sort = key;
                                    browser.ascending = true;
                                }
                                browser.dirty = true;
                            } else {
                                let depth = browser.columns.len() - 1;
                                let count = browser.visible(depth).len();
                                let scroll = browser.columns[depth].scroll.offset();
                                if let Some(index) =
                                    view::row_at(x, y, width, height, count, scroll)
                                {
                                    if ctrl {
                                        browser.toggle_select(depth, index);
                                    } else if shift {
                                        browser.extend_select(depth, index);
                                    } else {
                                        browser.select(depth, index);
                                        browser.note_row_click(depth, index);
                                    }
                                }
                            }
                        } else if browser.mode == ViewMode::Columns
                            && view::miller_boundary_at(
                                x,
                                y,
                                width,
                                height,
                                browser.pan.offset(),
                                browser.columns.len(),
                                browser.miller_w,
                            )
                            .is_some()
                        {
                            let depth = view::miller_boundary_at(
                                x,
                                y,
                                width,
                                height,
                                browser.pan.offset(),
                                browser.columns.len(),
                                browser.miller_w,
                            )
                            .unwrap();
                            let now = std::time::Instant::now();
                            let double_click =
                                browser.last_miller_click.is_some_and(|(last, at)| {
                                    last == depth && now.duration_since(at) < DOUBLE_CLICK_WINDOW
                                });
                            if double_click {
                                let entries = browser.visible(depth);
                                let longest =
                                    view::widest_name(entries.iter().map(|e| e.name.as_str()));
                                let has_dirs = entries.iter().any(|e| e.is_dir);
                                browser.miller_w = view::fit_miller_width(longest, has_dirs);
                                browser.last_miller_click = None;
                                browser.dirty = true;
                            } else {
                                browser.miller_resize = Some((depth, x, browser.miller_w));
                                browser.last_miller_click = Some((depth, now));
                            }
                        } else {
                            let counts = browser.counts();
                            let hit = view::miller_at(
                                x,
                                y,
                                width,
                                height,
                                &browser.columns,
                                &counts,
                                browser.pan.offset(),
                                browser.miller_w,
                            );
                            if let Some((depth, Some(index))) = hit {
                                if ctrl {
                                    browser.toggle_select(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    browser.select(depth, index);
                                }
                            } else if let Some((depth, None)) = hit {
                                browser.active = depth;
                                browser.dirty = true;
                            }
                        }
                    }
                    PointerEventKind::Axis {
                        vertical,
                        horizontal,
                        ..
                    } => {
                        let dy = vertical.absolute as f32;
                        let dx = horizontal.absolute as f32;
                        // Both axes clamp, band and fling themselves, so the
                        // metrics have to be current before either is fed.
                        browser.sync_scroll_metrics();

                        let stop = vertical.stop || horizontal.stop;
                        let discrete = vertical.discrete != 0 || horizontal.discrete != 0;
                        // One gesture belongs to one axis, chosen by its first
                        // delta and kept until it lifts.
                        let leading = if browser.mode == ViewMode::Columns && dx.abs() > dy.abs() {
                            Axis::Horizontal
                        } else {
                            Axis::Vertical
                        };
                        let axis = *browser.gesture_axis.get_or_insert(leading);
                        let (delta, scroll) = match axis {
                            // The stack pans as a whole …
                            Axis::Horizontal => (dx, &mut browser.pan),
                            // … while a vertical scroll belongs to the pane
                            // under the pointer.
                            Axis::Vertical => {
                                let depth = browser.pane_under(x, y);
                                (dy, &mut browser.columns[depth].scroll)
                            }
                        };

                        // A notched wheel reports discrete steps and moves
                        // exactly one step per click; a touchpad reports a
                        // continuous stream, which is what momentum and
                        // rubber banding are for.
                        let moved = if stop {
                            // Fingers off the touchpad: what the gesture
                            // was carrying becomes a fling, and anything
                            // pulled past an end springs back.
                            scroll.on_wheel_end();
                            true
                        } else if discrete {
                            scroll.on_wheel_discrete(delta)
                        } else {
                            scroll.on_wheel(delta)
                        };
                        if stop || discrete {
                            // Nothing is in flight to keep an axis for: the
                            // next delta picks afresh.
                            browser.gesture_axis = None;
                        }
                        browser.dirty |= moved;
                    }
                    _ => {}
                }

                drop(browser);
            }
            window_for_events.request_frame();
        });
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// What the preview column says about a file, under its name.
///
/// The same three facts the list view's columns show, in the same words: a
/// preview that described the file differently from the row it grew out of
/// would be describing a different file as far as the reader is concerned. A
/// directory has no size worth showing — the listing does not count children
/// either — so it gets two lines rather than three.
fn preview_info(entry: &Entry) -> Vec<String> {
    let mut info = vec![entry.kind_label().to_string()];
    if let Some(size) = entry.size.filter(|_| !entry.is_dir) {
        info.push(model::format_size(size));
    }
    if let Some(modified) = entry.modified {
        info.push(model::format_time(modified));
    }
    info
}

/// Open a browser window at `start` and run until it is closed.
pub fn run_browser(start: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    run_app(Browser::new(start), None)
}

/// Serve `org.otto.FilePicker1` until there is nothing left to serve.
///
/// No window exists until a request arrives: a bus-activated picker that
/// nobody has asked anything of should not be showing a dialog. The first
/// request builds the window; later ones re-use it as it is answered and
/// freed.
///
/// **One request at a time in v1.** Two applications asking at once are
/// served in turn rather than in two windows — the app shell is built around
/// a single toplevel, and queueing is honest where a second, half-supported
/// window would not be. Neither request is dropped and neither hangs.
pub async fn run_picker() -> Result<(), Box<dyn std::error::Error>> {
    let queue: crate::dbus::SharedQueue = Default::default();

    let service_queue = Arc::clone(&queue);
    tokio::spawn(async move {
        if let Err(err) = crate::dbus::serve(service_queue).await {
            tracing::error!(?err, "file picker D-Bus service failed");
            // Nothing can reach us, and a picker nobody can call is a dialog
            // that never opens. Better to die and be re-activated.
            std::process::exit(1);
        }
    });

    // Park until the bus hands us something to do. This is the whole of the
    // idle picker: no Wayland connection, no window, no watchers.
    let session = queue.next_session_async().await;

    let start = session.request.starting_directory(None);
    run_app(Browser::for_picker(session, start), Some(queue))
}

/// Run the app shell around an already-built [`Browser`].
/// One pointer event inside the Get Info window.
///
/// `sheet` is the panel's rect in that window's own coordinates, which is the
/// whole of it: the window *is* the card. Positions arrive in the same space,
/// so nothing here converts anything.
///
/// Returns a drag request: the strip was pressed, and the compositor should
/// take over and move the window. Moving is the compositor's job — it is the
/// only party that knows where the window is on the display, and an
/// interactive move it drives keeps the pointer and the window in step even
/// when the client is busy.
fn info_pointer(
    browser: &mut Browser,
    kind: &PointerEventKind,
    sheet: Rect,
    point: (f32, f32),
) -> bool {
    let (x, y) = point;
    let point = skia_safe::Point::new(x, y);
    let over_close = view::info_close_rect(sheet)
        .with_outset((6.0, 6.0))
        .contains(point);

    match kind {
        PointerEventKind::Press { .. } => {
            if over_close {
                browser.close_info();
            } else if let Some((who, what)) = view::perm_box_at(sheet, x, y) {
                browser.toggle_permission(who, what);
            } else if view::info_titlebar_rect(sheet).contains(point) {
                return true;
            }
            false
        }
        PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } => {
            if browser.info_close_hovered != over_close {
                browser.info_close_hovered = over_close;
                browser.info_dirty = true;
            }
            false
        }
        PointerEventKind::Leave { .. } => {
            if browser.info_close_hovered {
                browser.info_close_hovered = false;
                browser.info_dirty = true;
            }
            false
        }
        _ => false,
    }
}

fn run_app(
    browser: Browser,
    picker_queue: Option<crate::dbus::SharedQueue>,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(Mutex::new(browser));

    let app = FilesApp {
        pane_surfaces: None,
        window: None,
        info_window: Rc::new(RefCell::new(None)),
        state: Arc::clone(&state),
        modifiers: Arc::new(Mutex::new((false, false))),
        context_menu: None,
        quickview_target: Arc::new(Mutex::new(None)),
        picker_queue,
    };

    AppRunner::new(app).run()
}

#[cfg(test)]
mod rename_tests {
    use super::rename_selection;

    #[test]
    fn a_file_selects_the_stem_only() {
        assert_eq!(rename_selection("photo.png", false), 0..5);
    }

    #[test]
    fn a_multi_dot_name_splits_on_the_last_dot() {
        assert_eq!(rename_selection("archive.tar.gz", false), 0..11);
    }

    #[test]
    fn a_directory_selects_the_whole_name() {
        assert_eq!(rename_selection("Documents", true), 0..9);
        assert_eq!(rename_selection("my.folder", true), 0..9);
    }

    #[test]
    fn a_dotfile_selects_the_whole_name() {
        assert_eq!(rename_selection(".bashrc", false), 0..7);
    }

    #[test]
    fn an_extensionless_file_selects_the_whole_name() {
        assert_eq!(rename_selection("README", false), 0..6);
    }
}

#[cfg(test)]
mod typeahead_tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A real directory of empty files, swept up when the test ends.
    ///
    /// The listing comes off a worker thread reading the filesystem, so
    /// type-ahead can only be exercised against something actually on disk.
    struct TempDir(PathBuf);

    impl TempDir {
        fn holding(names: &[&str]) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let path = std::env::temp_dir().join(format!(
                "otto-files-typeahead-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("temp dir");
            for name in names {
                std::fs::write(path.join(name), b"").expect("temp file");
            }
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A browser over `names`, with the first listing already in.
    fn browser_over(names: &[&str]) -> (Browser, TempDir) {
        let dir = TempDir::holding(names);
        let mut browser = Browser::new(dir.0.clone());
        // The frame loop is what normally polls the loader; a test has to.
        for _ in 0..500 {
            if browser.columns[0].poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!browser.columns[0].loading(), "listing never arrived");
        (browser, dir)
    }

    fn at_cursor(browser: &Browser) -> Option<String> {
        let index = browser.columns[browser.active].cursor?;
        Some(browser.visible(browser.active)[index].name.clone())
    }

    /// Names chosen so that a prefix, a second character and a repeat all
    /// have somewhere different to land.
    const NAMES: &[&str] = &[
        "Alpha.txt",
        "apple.txt",
        "Banana.txt",
        "beta.txt",
        "Photo.png",
    ];

    #[test]
    fn a_character_selects_the_first_entry_starting_with_it() {
        let (mut browser, _dir) = browser_over(NAMES);
        browser.typeahead('b');
        // Matching ignores case, so a lowercase key reaches a capitalised name.
        assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
    }

    #[test]
    fn a_second_character_narrows_the_same_word() {
        let (mut browser, _dir) = browser_over(NAMES);
        browser.typeahead('a');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Alpha.txt"));
        browser.typeahead('p');
        assert_eq!(at_cursor(&browser).as_deref(), Some("apple.txt"));
    }

    #[test]
    fn repeating_one_character_cycles_and_wraps() {
        let (mut browser, _dir) = browser_over(NAMES);
        browser.typeahead('b');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
        browser.typeahead('b');
        assert_eq!(at_cursor(&browser).as_deref(), Some("beta.txt"));
        // Past the last match it comes back round rather than stopping dead.
        browser.typeahead('b');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
    }

    #[test]
    fn a_second_of_silence_starts_a_new_word() {
        let (mut browser, _dir) = browser_over(NAMES);
        browser.typeahead('a');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Alpha.txt"));
        // Backdated rather than slept through: the expiry is a second, and no
        // test should take one.
        let (buffer, _) = browser.typeahead.take().expect("buffer");
        browser.typeahead = Some((buffer, std::time::Instant::now() - Duration::from_secs(2)));
        browser.typeahead('p');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Photo.png"));
    }

    #[test]
    fn a_name_that_matches_nothing_leaves_the_cursor_alone() {
        let (mut browser, _dir) = browser_over(NAMES);
        browser.typeahead('b');
        browser.typeahead('z');
        assert_eq!(at_cursor(&browser).as_deref(), Some("Banana.txt"));
        // The miss stays in the buffer: it is part of the word being typed.
        assert_eq!(
            browser.typeahead.as_ref().map(|(b, _)| b.as_str()),
            Some("bz")
        );
    }

    #[test]
    fn an_empty_directory_is_not_a_panic() {
        let (mut browser, _dir) = browser_over(&[]);
        browser.typeahead('a');
        assert_eq!(at_cursor(&browser), None);
    }
}
