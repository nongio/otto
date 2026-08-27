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
use otto_kit::dnd::DndAction;
use otto_kit::prelude::*;
use otto_kit::CursorShape;
use skia_safe::Contains;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::seat::pointer::PointerEventKind;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use smithay_client_toolkit::shell::xdg::{XdgPositioner, XdgSurface};
use wayland_client::protocol::{wl_keyboard, wl_surface};

/// `BTN_RIGHT` from `linux/input-event-codes.h` — a right-click opens the
/// context menu instead of doing whatever the same spot does on the left
/// button.
const BTN_RIGHT: u32 = 0x111;

use model::{Column, Entry, Place, SortKey};
use view::ViewMode;

/// What a drag carries: the entries to draw, where the grab happened, and the
/// bounding box they were gathered from.
type DragItems = (Vec<view::DragItem>, (f32, f32), (f32, f32));

/// How soon a second press on the same column divider must land to count as
/// a double-click rather than the start of a fresh drag.
const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

/// How far the pointer must travel, with the button still down, before a press
/// on a row becomes a drag rather than a click. Below this a hand that shifts
/// while clicking still selects, and a double-click still opens.
/// How many operations back Ctrl+Z can reach.
///
/// Deep enough that undo is a thing you can lean on, shallow enough that the
/// paths it holds cannot pile up: every step remembers where files went, and
/// an unbounded stack would keep the whole session's worth alive.
const UNDO_DEPTH: usize = 32;

const DRAG_THRESHOLD: f32 = 6.0;

/// Files landed somewhere — a paste, a drop, a restore out of the Trash.
///
/// The naming spec has no "paste", and theme coverage of the drag events is
/// thin, so this is a preference order rather than one name: the first the
/// installed theme actually has is the one that plays.
const SOUND_ARRIVED: [&str; 3] = ["drag-accept", "device-added", "complete"];

/// Files went away — a delete, or an undo that took a copy back.
const SOUND_REMOVED: [&str; 2] = ["trash-empty", "device-removed"];

/// One undoable operation: what to call it, and everything it did.
///
/// Only operations that *change files* go on the stack — a move, a copy, a
/// paste, a delete, a rename, a new folder. Selecting and navigating are not
/// undoable and never were: Ctrl+Z that could take back a click would make
/// the ones that take back a delete unreliable, because the user would never
/// know which of the two the next press was going to reach.
#[derive(Debug, Clone)]
struct UndoStep {
    /// Names the thing being taken back, for the status line: "Undid Move".
    label: &'static str,
    changes: Vec<model::Change>,
}

/// A rubber-band selection being dragged out over the icon grid.
///
/// Both corners are kept in the pane's *content* coordinates — the pointer's y
/// with the pane's scroll already added — so the band stays anchored over the
/// files it was drawn around rather than over the screen. Scroll the wheel
/// mid-drag and the band grows with the content, which is what makes it
/// possible to band-select past the bottom of the window.
///
/// `base` is the selection the press started from. A plain press clears it, so
/// it is empty and the band *is* the selection; Ctrl or Shift keeps it, so the
/// band adds to what was already there. Either way the selection is recomputed
/// from scratch on every motion, which is what lets the band shrink back and
/// give up entries again.
#[derive(Debug, Clone)]
struct Marquee {
    depth: usize,
    anchor: (f32, f32),
    cursor: (f32, f32),
    base: std::collections::BTreeSet<String>,
}

impl Marquee {
    /// The band in content coordinates, normalised so dragging up and left
    /// gives the same rectangle as dragging down and right.
    fn rect(&self) -> skia_safe::Rect {
        skia_safe::Rect::from_ltrb(
            self.anchor.0.min(self.cursor.0),
            self.anchor.1.min(self.cursor.1),
            self.anchor.0.max(self.cursor.0),
            self.anchor.1.max(self.cursor.1),
        )
    }
}

/// Is `(x, y)` inside a pane's own area, rather than the header, the sidebar
/// or the status strip around it? A click on nothing only means "nothing" when
/// it lands where the entries are.
fn hit_content(area: skia_safe::Rect, x: f32, y: f32) -> bool {
    x >= area.left && x <= area.right && y >= area.top && y <= area.bottom
}

/// Where a drag hovering the window would put the files, and what to outline.
///
/// Every variant resolves to a directory: dropping *onto* a file is not a
/// thing, so a hit on one is a hit on the pane behind it.
#[derive(Debug, Clone, PartialEq)]
enum DropTarget {
    /// A directory row or cell — the files go inside it.
    Entry {
        depth: usize,
        index: usize,
        path: PathBuf,
    },
    /// The pane's own directory, hit through its background.
    Pane { depth: usize, path: PathBuf },
    /// A sidebar place.
    Place { index: usize, path: PathBuf },
}

impl DropTarget {
    /// The directory the drop lands in.
    fn path(&self) -> &PathBuf {
        match self {
            Self::Entry { path, .. } | Self::Pane { path, .. } | Self::Place { path, .. } => path,
        }
    }

    fn highlight(&self) -> view::DropHighlight {
        match *self {
            Self::Entry { depth, index, .. } => view::DropHighlight::Row { depth, index },
            Self::Pane { depth, .. } => view::DropHighlight::Pane { depth },
            Self::Place { index, .. } => view::DropHighlight::Place { index },
        }
    }
}

/// Answer the drag source at one position, and light up whatever would take
/// the drop.
///
/// Called for every enter and every motion, because that is what the protocol
/// asks for: the answer is per-position, and a target that goes quiet has said
/// no. See [`otto_kit::dnd::accept`].
fn hover_drag(state: &Arc<Mutex<Browser>>, x: f32, y: f32) {
    use otto_kit::dnd;

    let mut browser = state.lock().unwrap();
    let target = browser.drop_target_at(x, y);
    let mime = dnd::first_offered(clipboard::file_mime_preference());

    match (&target, mime) {
        (Some(_), Some(mime)) => dnd::accept(
            Some(&mime),
            DndAction::Copy | DndAction::Move,
            // Move by default, copy on request — what every other file manager
            // does. The compositor still has the last word, and a source that
            // only offers a copy gets one.
            DndAction::Move,
        ),
        // Over nothing that takes files, or a drag carrying none.
        _ => dnd::accept(None, DndAction::empty(), DndAction::empty()),
    }

    if browser.drop_target != target {
        browser.drop_target = target;
        browser.dirty = true;
        drop(browser);
        AppContext::request_wakeup();
    }
}

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
    /// A press that landed on an already-selected entry and deliberately did
    /// *not* narrow the selection to it — see [`Self::press_entry`]. Resolved by
    /// the release, cancelled by a drag, and dropped by the next press.
    press_pending: Option<(usize, usize)>,
    /// The pane whose cursor is being opened, and when the open happened.
    /// Drives the pulse the icon leaves behind — see [`view::draw_open_pulse`].
    opening: Option<(usize, std::time::Instant)>,
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
    /// A press on part of the selection that has not moved far enough to be a
    /// drag yet: where it landed, and the serial that will authorise the drag
    /// if it does. Cleared on release, so a click that never moves is a click.
    drag_armed: Option<(f32, f32, u32)>,
    /// A rubber band being dragged out over the icon grid, if one is in
    /// progress. See [`Marquee`].
    marquee: Option<Marquee>,
    /// Where a drag now over the window would drop. Drawn outlined, and read
    /// again when the drop arrives.
    drop_target: Option<DropTarget>,
    /// The last operation's outcome, shown in the header until the next action.
    status: Option<String>,
    /// Operations that changed files, newest last. Ctrl+Z pops one and puts
    /// it back; see [`UndoStep`] for what does and does not go on here.
    undo: Vec<UndoStep>,
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
    /// Whether the window is the focused one. An unfocused window steps back:
    /// its title and traffic lights go gray, and the compositor stops blurring
    /// behind it.
    focused: bool,
    /// Whether the compositor can blur behind the window at all. False when it
    /// carries no surface style — running under another compositor — or when
    /// the blur has been turned off for measurement. The materials are filled
    /// in for the whole run then, not just while the window is unfocused.
    blur_available: bool,
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
    /// The save field, in `Save` mode only. It is the picker's keyboard
    /// focus: printable keys go here rather than to type-ahead, because in a
    /// Save dialog what the user is doing is naming a file.
    save_name: Option<TextInput>,
    /// The replace-confirmation sheet, while it is up. Modal over the whole
    /// window: the request is not answered until it is.
    confirm: Option<ConfirmSheet>,
    /// The last answer [`Browser::save_action`] gave, and what it was asked
    /// about.
    ///
    /// The action row asks on every repaint, and answering costs three
    /// syscalls against a directory that may be a stalled network mount —
    /// which is exactly what this window is not allowed to block on. Keyed on
    /// the question, so it is recomputed when the directory or the name
    /// changes and never merely because the window redrew.
    save_probe: RefCell<Option<(PathBuf, String, picker::SaveAction)>>,
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

/// The uncached half of [`Browser::save_action`]: three syscalls, and the
/// only place in the picker that touches the filesystem on the UI thread.
fn probe_save_action(dir: &Path, name: &str) -> picker::SaveAction {
    if !picker::is_writable_dir(dir) {
        return picker::SaveAction::Blocked("You do not have permission to save here");
    }
    picker::save_action(name, existing_kind(&dir.join(name.trim())))
}

/// What is at `path` today: `None` for nothing, `Some(true)` for a directory,
/// `Some(false)` for anything else.
///
/// `symlink_metadata`, not `metadata`: a dangling symlink is *something* in
/// the way, and a symlink to a directory is still a name the application
/// would be overwriting rather than a folder to descend into.
fn existing_kind(path: &Path) -> Option<bool> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| meta.is_dir())
}

/// The replace confirmation a save-mode accept puts up when something is
/// already at the path the user named.
///
/// It holds the paths it is about to answer with, not a promise to recompute
/// them: between the sheet appearing and the user pressing Replace the
/// directory may change underneath, and answering with what the user was
/// actually shown is the honest thing.
struct ConfirmSheet {
    /// The question, already worded for the number of files involved.
    message: String,
    detail: String,
    /// What accepting answers with.
    paths: Vec<PathBuf>,
    pressed: Option<view::ConfirmButton>,
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
            press_pending: None,
            opening: None,
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
            drag_armed: None,
            marquee: None,
            drop_target: None,
            quickview: None,
            preview: None,
            preview_generation_seed: 0,
            thumbs: thumbnails::Store::new(),
            quickview_pending: false,
            quickview_closing: None,
            quickview_auto: std::env::var_os("OTTO_FILES_QV_AUTO").is_some(),
            quickview_generation: 0,
            status: None,
            undo: Vec::new(),
            info: None,
            info_error: None,
            info_close_hovered: false,
            info_dirty: false,
            controls: WindowControlsState::new(),
            focused: true,
            blur_available: false,
            dirty: true,
            picker: None,
            save_name: None,
            confirm: None,
            save_probe: RefCell::new(None),
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
        if session.request.mode.names_a_file() {
            let name = session.request.initial_name();
            let selection = picker::name_stem_range(&name);
            let mut input =
                TextInput::editing(name, view::save_field_style(AppContext::current_theme()));
            input.state.select_range(selection);
            browser.save_name = Some(input);
        }
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
        match &self.picker {
            // Save mode stacks a name row on top of the buttons; the buttons
            // themselves stay anchored to the window bottom, so every rect
            // below the name row is unchanged by this.
            Some(session) if session.request.mode.names_a_file() => {
                view::FOOTER_H + view::FOOTER_NAME_H
            }
            Some(_) => view::FOOTER_H,
            None => 0.0,
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
        self.dirty = true;
        Some((entry.path, generation))
    }

    /// Pan the stack so the preview pane — sitting right after the last real
    /// column, the same trailing position a freshly opened directory column
    /// would occupy — is fully in view.
    ///
    /// **Not** called when the preview's target changes. Arrowing down a
    /// listing changes it on every keystroke, and a stack that panned each time
    /// would slide out from under the column being read: the preview is a thing
    /// offered at the edge of the view, not a place the browser goes. Kept for
    /// a caller that means to go there deliberately.
    #[allow(dead_code)]
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
            // A Miller pane's rows sit a little way down the pane and are
            // panned sideways with the stack, so its own strip and its own
            // viewport are what describe them — a list's strip would be off by
            // the inset and would not know the pane can be panned off screen
            // entirely.
            view::ViewMode::Columns => {
                let full = view::miller_pane_rect(
                    depth,
                    self.content_h(),
                    self.pan.offset(),
                    self.miller_w,
                );
                let band = view::pane_viewport(
                    self.size.0,
                    self.content_h(),
                    view::ViewMode::Columns,
                    depth,
                    self.pan.offset(),
                    self.miller_w,
                );
                view::RowStrip::miller(full, entries.len(), scroll).visible(band)
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

        for (depth, &count) in counts.iter().enumerate() {
            let viewport = view::pane_viewport(width, height, mode, depth, pan, miller_w);
            let content = view::pane_content_height(width, height, mode, count);
            // A column being re-read has no entries *yet*, and telling its
            // scroll view how long *that* is would clamp the offset to the top
            // — permanently, since the offset is not restored when the listing
            // lands. Anything that reloads in place (a delete, a paste, a drop,
            // a rename) would scroll the pane away from what the user was
            // looking at. The carried length stands until the real one is
            // known.
            //
            // Gated on the read being in flight, not on the measurement coming
            // out zero: an empty listing does not measure as zero in every
            // view. A grid counts its padding and a Miller pane its row inset
            // whether or not there are rows, so those two clamped to the top
            // anyway. Only List happens to measure an empty pane as nothing.
            let loading = self.columns[depth].loading();
            let scroll = &mut self.columns[depth].scroll;
            scroll.state.set_viewport(viewport);
            if !loading {
                scroll.set_content_length(content);
            }
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

        // Clicking a file in a Save dialog puts its name in the field. That
        // is how a user says "overwrite this one" without retyping it, and it
        // is why the replace confirmation exists at all. A directory is a
        // place to go, not a name to save under, so it leaves the field be.
        if !entry.is_dir {
            if let Some(input) = self.save_name.as_mut() {
                input.set_value(entry.name.clone());
            }
        }

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

    /// Clear one pane's selection — what a click on nothing means.
    ///
    /// The pane still becomes the active one: the click was in it, and the
    /// keyboard should follow. In Miller view the panes to its right go too.
    /// They are there because something in this one was selected, and now
    /// nothing is; leaving them up would show a child of no parent.
    fn clear_pane_selection(&mut self, depth: usize) {
        if depth >= self.columns.len() {
            return;
        }
        self.active = depth;
        self.clear_selection();
        if self.mode == ViewMode::Columns {
            self.columns.truncate(depth + 1);
        }
    }

    /// Start dragging a rubber band out from `(x, y)` — a press on the empty
    /// part of the icon grid.
    ///
    /// `additive` (Ctrl or Shift held) keeps what was already selected and
    /// adds to it. Without it the press has already cleared the pane, which is
    /// what makes a band that catches nothing — a plain click — mean nothing
    /// selected.
    fn begin_marquee(&mut self, depth: usize, x: f32, y: f32, additive: bool) {
        if self.mode != ViewMode::Grid || depth >= self.columns.len() {
            return;
        }
        let scroll = self.columns[depth].scroll.offset();
        let base = if additive {
            self.columns[depth].selection.clone()
        } else {
            Default::default()
        };
        self.active = depth;
        self.marquee = Some(Marquee {
            depth,
            anchor: (x, y + scroll),
            cursor: (x, y + scroll),
            base,
        });
    }

    /// Follow the pointer, and reselect everything the band now covers.
    ///
    /// Recomputed rather than accumulated: an entry the band has moved off
    /// leaves the selection again, unless it was in `base`.
    fn update_marquee(&mut self, x: f32, y: f32) -> bool {
        let Some(depth) = self.marquee.as_ref().map(|m| m.depth) else {
            return false;
        };
        let Some(scroll) = self.columns.get(depth).map(|c| c.scroll.offset()) else {
            return false;
        };

        let (band, mut selection) = {
            let marquee = self.marquee.as_mut().unwrap();
            marquee.cursor = (x, y + scroll);
            (marquee.rect(), marquee.base.clone())
        };

        // The band is in content coordinates, so the hit test is asked about
        // the unscrolled grid: `grid_cell_rect(area, i, 0.0)` is where cell `i`
        // sits in that same space.
        let area = view::content_viewport(self.size.0, self.size.1, ViewMode::Grid);
        let names: Vec<String> = self.visible(depth).iter().map(|e| e.name.clone()).collect();
        let caught = view::grid_cells_in_rect(area, names.len(), 0.0, band);

        let last = caught.last().copied();
        for index in caught {
            selection.insert(names[index].clone());
        }

        let column = &mut self.columns[depth];
        if column.selection == selection && column.cursor == last {
            return false;
        }
        column.selection = selection;
        column.cursor = last;
        column.anchor = last;
        self.dirty = true;
        true
    }

    /// The band as it should be drawn: window coordinates, or `None` when no
    /// band is out.
    fn marquee_band(&self) -> Option<skia_safe::Rect> {
        let marquee = self.marquee.as_ref()?;
        let scroll = self.columns.get(marquee.depth)?.scroll.offset();
        let mut band = marquee.rect();
        band.offset((0.0, -scroll));
        Some(band)
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
                self.record_undo(
                    "Rename",
                    vec![model::Change::Moved {
                        from: session.original.clone(),
                        to: target.clone(),
                    }],
                );
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

    /// How far through the open pulse we are, if one is running.
    fn opening_progress(&self) -> Option<(usize, f32)> {
        let (depth, started) = self.opening?;
        let t = started.elapsed().as_secs_f32() / view::OPEN_PULSE.as_secs_f32();
        (t < 1.0).then_some((depth, t))
    }

    /// Drop a pulse that has run its course, so the clock can stop.
    fn tick_open_pulse(&mut self) -> bool {
        if self.opening.is_some() && self.opening_progress().is_none() {
            self.opening = None;
            self.dirty = true;
        }
        self.opening.is_some()
    }

    /// A plain press on a row or cell.
    ///
    /// Pressing an entry that is *already* one of several selected leaves the
    /// selection alone: what usually follows is a drag of the whole group, and
    /// narrowing to the one under the pointer would throw the rest away before
    /// the drag could carry them. The narrowing is not abandoned, only deferred
    /// to the release — a press that comes back up without dragging was a click
    /// after all, and a click on one of several selected files does mean "just
    /// this one".
    ///
    /// Any pending narrow from an earlier gesture is dropped first. A deferred
    /// decision that outlives its own gesture is worse than no deferral at all:
    /// it would land on a later, unrelated click and undo it.
    fn press_entry(&mut self, depth: usize, index: usize) {
        self.press_pending = None;

        if self.is_in_multiple_selection(depth, index) {
            self.press_pending = Some((depth, index));
            return;
        }
        self.select(depth, index);
        self.note_row_click(depth, index);
    }

    /// The button came up. A press that deferred its narrowing and did not turn
    /// into a drag was a click, so it narrows now.
    fn release_entry(&mut self) {
        let Some((depth, index)) = self.press_pending.take() else {
            return;
        };
        self.select(depth, index);
        self.note_row_click(depth, index);
    }

    /// A drag has begun: the group travels whole, so the press that started it
    /// must not narrow to one of them when the button comes up.
    fn drag_started(&mut self) {
        self.press_pending = None;
    }

    /// Is `index` one of *several* entries selected in `depth`?
    ///
    /// One selected entry is not a group: pressing it again means the same
    /// thing either way, and deferring would only make the common case answer
    /// late.
    fn is_in_multiple_selection(&self, depth: usize, index: usize) -> bool {
        let Some(column) = self.columns.get(depth) else {
            return false;
        };
        if column.selection.len() < 2 {
            return false;
        }
        self.visible(depth)
            .get(index)
            .is_some_and(|entry| column.selection.contains(&entry.name))
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

    /// Record a Ctrl+click on a row/cell. The first one toggles the row into
    /// or out of the selection; a second one on the same row within the
    /// double-click window opens it in a *new window* instead of toggling it
    /// back out, so Ctrl+double-click reads as "open this one elsewhere"
    /// rather than as two selection changes.
    ///
    /// The picker never takes this path: it answers one request in one
    /// window, so there Ctrl+click only ever toggles.
    fn note_ctrl_row_click(&mut self, depth: usize, index: usize) {
        let now = std::time::Instant::now();
        let double_click = self.picker.is_none()
            && self.last_row_click.is_some_and(|(d, i, at)| {
                d == depth && i == index && now.duration_since(at) < DOUBLE_CLICK_WINDOW
            });
        if double_click {
            self.last_row_click = None;
            self.open_in_new_window(depth, index);
        } else {
            self.toggle_select(depth, index);
            self.last_row_click = Some((depth, index, now));
        }
    }

    /// Open one directory in a second browser window — Ctrl+double-click.
    ///
    /// A window is a process here: the app shell is built around a single
    /// toplevel, so the second window is a second `otto-files` handed the
    /// directory on its command line. Anything that is not a directory falls
    /// back to plain activation, which is all a new window could do with it.
    fn open_in_new_window(&mut self, depth: usize, index: usize) {
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };
        if !entry.is_dir {
            self.select(depth, index);
            self.open_selection();
            return;
        }

        self.spawn_window(&entry.path);
    }

    /// Open a second browser window on `path`.
    ///
    /// A window is a process here — the app shell is built around a single
    /// toplevel — so this re-executes this binary with the directory on its
    /// command line. Shared by Ctrl+double-click and by the New Window
    /// shortcut, which differ only in which directory they name.
    fn spawn_window(&mut self, path: &Path) {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(err) => {
                self.status = Some(format!("Couldn\u{2019}t open a new window: {err}"));
                self.dirty = true;
                return;
            }
        };
        let spawned = std::process::Command::new(exe)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            // Reap it on a thread of its own: the child outlives this call and
            // nothing else here would ever wait on it.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                self.status = Some(format!("Couldn\u{2019}t open a new window: {err}"));
                self.dirty = true;
            }
        }
    }

    /// Open a new window on the default location — Ctrl+N.
    ///
    /// The default location, not this window's directory: a new window is a
    /// fresh start, and starting somewhere arbitrary — wherever the window
    /// that happened to have focus was pointed — is what makes a new window
    /// feel like a copy of the old one rather than a new one. Ctrl+double-click
    /// is the gesture for "that directory, in another window".
    fn open_new_window(&mut self) {
        let Some(path) = self.new_window_target() else {
            return;
        };
        self.spawn_window(&path);
    }

    /// Where a new window would open, or `None` when one makes no sense.
    ///
    /// Split from the spawning so the choice can be tested without launching
    /// a process: everything interesting about Ctrl+N is which directory it
    /// names, and that is all this answers.
    fn new_window_target(&self) -> Option<PathBuf> {
        // The picker answers one request in one window: a second browser
        // window would have nothing to do with the request and no way to
        // answer it.
        if self.picker.is_some() {
            return None;
        }
        Some(Self::default_location())
    }

    /// Where a window with nowhere in particular to be starts.
    ///
    /// The home directory for now. It is a single function rather than a
    /// literal at each call site because this is the thing a preference would
    /// replace — when there is one, it is read here and everywhere that opens
    /// a fresh window follows.
    fn default_location() -> PathBuf {
        model::home_dir().unwrap_or_else(|| PathBuf::from("/"))
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

        // Say that the open was heard, before doing it: handing a file to
        // another process can take long enough that a double-click with no
        // answer reads as one that did not land. In every view — the gesture
        // is the same everywhere, so the answer to it should be too.
        //
        // A directory is the exception, and not because of the waiting: going
        // into one *shows* itself. The listing changes, or in column view a
        // child column arrives beside the one that was clicked, and a ghost of
        // the row on top of that is one answer too many.
        if !entry.is_dir {
            self.opening = Some((depth, std::time::Instant::now()));
            self.dirty = true;
        }

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
        // and Enter both land here, and both mean "this one".
        if self.picker.is_some() {
            self.picker_accept();
        } else {
            self.open_in_default_app(&entry.path);
        }
    }

    /// Hand a file to whatever the desktop opens that type with.
    ///
    /// `xdg-open` rather than resolving the association here: it is the
    /// desktop's own answer to this question, it already knows about
    /// `mimeapps.list`, the portal and the fallbacks, and a file manager that
    /// disagreed with the rest of the session about what opens a `.pdf` would
    /// be the thing that was wrong.
    ///
    /// Detached, like a new window: stdio closed and reaped on a thread of its
    /// own, so the application outlives the browser that started it.
    fn open_in_default_app(&mut self, path: &Path) {
        let spawned = std::process::Command::new("xdg-open")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(err) => {
                self.status = Some(format!("Couldn\u{2019}t open that file: {err}"));
                self.dirty = true;
            }
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

    // --- The save modes ----------------------------------------------------

    /// The directory a save-mode accept writes into.
    ///
    /// `Save` always uses the directory being *viewed*: the name field says
    /// what to call the file, the listing says where it goes, and a folder
    /// merely selected in that listing is somewhere the user is looking at,
    /// not somewhere they have gone. `SaveFiles` follows the directory-mode
    /// rule instead — the whole request is "which folder", so a selected one
    /// is the answer.
    fn save_directory(&self) -> Option<PathBuf> {
        let session = self.picker.as_ref()?;
        let depth = self.active.min(self.columns.len().saturating_sub(1));
        let column = self.columns.get(depth)?;

        if session.request.mode == picker::Mode::SaveFiles {
            let picked: Vec<&Entry> = self
                .visible(depth)
                .into_iter()
                .filter(|e| e.is_dir && column.selection.contains(&e.name))
                .collect();
            if let [only] = picked.as_slice() {
                return Some(only.path.clone());
            }
        }
        Some(column.path.clone())
    }

    /// What the accept button would do in `Save` mode, and why it is disabled
    /// when it is.
    ///
    /// The directory check comes first: a name is never the problem when the
    /// folder cannot be written to at all, and saying "Enter a name" about a
    /// read-only folder would send the user off correcting the wrong thing.
    fn save_action(&self) -> picker::SaveAction {
        let Some(dir) = self.save_directory() else {
            return picker::SaveAction::Blocked("Nowhere to save");
        };
        let name = self
            .save_name
            .as_ref()
            .map(TextInput::value)
            .unwrap_or("")
            .to_string();

        if let Some((cached_dir, cached_name, action)) = self.save_probe.borrow().as_ref() {
            if *cached_dir == dir && *cached_name == name {
                return action.clone();
            }
        }
        let action = probe_save_action(&dir, &name);
        *self.save_probe.borrow_mut() = Some((dir, name, action.clone()));
        action
    }

    /// Ask the filesystem again, ignoring the memo.
    ///
    /// Accept uses this: between the last repaint and the click, someone else
    /// may have created the file the user is about to be told is not there,
    /// and the confirmation exists precisely to catch that.
    fn save_action_now(&self) -> picker::SaveAction {
        self.save_probe.borrow_mut().take();
        self.save_action()
    }

    /// Whether the accept button is live, for the frame.
    fn picker_accept_enabled(&self) -> bool {
        match self.picker.as_ref().map(|s| s.request.mode) {
            Some(picker::Mode::Save) => {
                !matches!(self.save_action(), picker::SaveAction::Blocked(_))
            }
            Some(picker::Mode::SaveFiles) => self
                .save_directory()
                .is_some_and(|dir| picker::is_writable_dir(&dir)),
            Some(picker::Mode::Open) => self.picker_selection().is_some(),
            None => false,
        }
    }

    /// The reason the accept button is disabled, shown beside the name field.
    /// `None` while it is enabled — there is then nothing to explain.
    fn save_problem(&self) -> Option<&'static str> {
        match self.picker.as_ref()?.request.mode {
            picker::Mode::Save => match self.save_action() {
                // "Enter a name" is not a complaint about an empty field the
                // user has not filled in yet; it is the placeholder's job.
                picker::SaveAction::Blocked("Enter a name") => None,
                picker::SaveAction::Blocked(reason) => Some(reason),
                _ => None,
            },
            picker::Mode::SaveFiles => self
                .save_directory()
                .filter(|dir| !picker::is_writable_dir(dir))
                .map(|_| "You do not have permission to save here"),
            picker::Mode::Open => None,
        }
    }

    /// `Save`: resolve the name field against the directory being viewed.
    fn save_accept(&mut self) {
        match self.save_action_now() {
            picker::SaveAction::Blocked(_) => {}
            picker::SaveAction::Descend => {
                // The name names a folder that is already there. Going into it
                // is what the user meant; clearing the field is what stops the
                // next Return from bouncing straight back out of it.
                let Some(dir) = self.save_directory() else {
                    return;
                };
                let name = self
                    .save_name
                    .as_ref()
                    .map(|i| i.value().trim().to_string())
                    .unwrap_or_default();
                self.navigate_to(&dir.join(name));
                if let Some(input) = self.save_name.as_mut() {
                    input.set_value(String::new());
                }
                self.dirty = true;
            }
            picker::SaveAction::Replace => {
                let Some(target) = self.save_target() else {
                    return;
                };
                let name = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.confirm = Some(ConfirmSheet {
                    message: format!("\u{201c}{name}\u{201d} already exists. Replace it?"),
                    detail: "Replacing it overwrites its current contents.".to_string(),
                    paths: vec![target],
                    pressed: None,
                });
                self.dirty = true;
            }
            picker::SaveAction::Write => {
                let Some(target) = self.save_target() else {
                    return;
                };
                self.answer_with(vec![target]);
            }
        }
    }

    /// The single path `Save` would answer with.
    fn save_target(&self) -> Option<PathBuf> {
        let name = self.save_name.as_ref()?.value().trim();
        if name.is_empty() {
            return None;
        }
        Some(self.save_directory()?.join(name))
    }

    /// `SaveFiles`: one path per name the request carried, all in the chosen
    /// directory.
    ///
    /// Each name is reduced to its final component. The spec's "no name
    /// mangling" is about not inventing `file (1).txt` when something is in
    /// the way; it is not a licence for an application to reach out of the
    /// directory the user chose by sending `../../.bashrc`.
    fn save_files_targets(&self) -> Vec<PathBuf> {
        let Some(dir) = self.save_directory() else {
            return Vec::new();
        };
        let Some(session) = self.picker.as_ref() else {
            return Vec::new();
        };
        session
            .request
            .files
            .iter()
            .filter_map(|name| Path::new(name).file_name().map(|n| dir.join(n)))
            .collect()
    }

    fn save_files_accept(&mut self) {
        let targets = self.save_files_targets();
        if targets.is_empty() {
            return;
        }
        let clashes: Vec<&PathBuf> = targets
            .iter()
            .filter(|p| existing_kind(p).is_some())
            .collect();
        if clashes.is_empty() {
            self.answer_with(targets);
            return;
        }
        // One sheet for all of them, not one per file: the user is answering
        // a single question about a single batch.
        let message = if clashes.len() == 1 {
            let name = clashes[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("\u{201c}{name}\u{201d} already exists. Replace it?")
        } else {
            format!(
                "{} of these files already exist. Replace them?",
                clashes.len()
            )
        };
        self.confirm = Some(ConfirmSheet {
            message,
            detail: "Replacing them overwrites their current contents.".to_string(),
            paths: targets,
            pressed: None,
        });
        self.dirty = true;
    }

    /// Answer the request with `paths` and let the window go.
    fn answer_with(&mut self, paths: Vec<PathBuf>) {
        if let Some(session) = self.picker.as_mut() {
            session.accept(&paths);
        }
        self.dirty = true;
    }

    /// Replace, from the confirmation sheet: answer with what the sheet was
    /// showing. The picker still creates nothing — the application writes.
    fn confirm_replace(&mut self) {
        let Some(sheet) = self.confirm.take() else {
            return;
        };
        self.answer_with(sheet.paths);
    }

    /// Dismiss the sheet without answering. The request stays open and the
    /// user is back in the dialog, which is what "Cancel" means here — it
    /// cancels the replacement, not the save.
    fn confirm_dismiss(&mut self) {
        if self.confirm.take().is_some() {
            self.dirty = true;
        }
    }

    /// Return the current selection to the application and close the window.
    fn picker_accept(&mut self) {
        match self.picker.as_ref().map(|s| s.request.mode) {
            Some(picker::Mode::Save) => self.save_accept(),
            Some(picker::Mode::SaveFiles) => self.save_files_accept(),
            Some(picker::Mode::Open) => {
                let Some(paths) = self.picker_selection() else {
                    return;
                };
                self.answer_with(paths);
            }
            None => {}
        }
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
                self.descend_selection();
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
            self.descend_selection();
        }
    }

    /// Ctrl+O: open the entry at the cursor the way the desktop would.
    ///
    /// Everything goes to `xdg-open`, folders included — the shortcut asks the
    /// desktop to open the thing, and the desktop's answer for a directory is
    /// whatever it has registered as the file manager. That is the difference
    /// between this and a double-click: the click descends where you are, the
    /// shortcut hands the entry over. Same in every view.
    ///
    /// It pulses either way. Ctrl+O is a deliberate ask with no click to
    /// acknowledge it, and whatever answers can take a moment to appear.
    fn open_cursor_entry(&mut self) {
        // The picker answers one request in one window: a second window would
        // have nothing to do with the request, and no way to answer it.
        if self.picker.is_some() {
            self.open_selection();
            return;
        }

        let depth = self.active;
        let Some(index) = self.columns[depth].cursor else {
            return;
        };
        let Some(entry) = self.visible(depth).get(index).map(|e| (*e).clone()) else {
            return;
        };

        self.opening = Some((depth, std::time::Instant::now()));
        self.dirty = true;
        self.open_in_default_app(&entry.path);
    }

    /// Go *into* the selection, and only that: a directory is descended, and
    /// anything else is left alone.
    ///
    /// An arrow key is navigation. Opening a file hands it to another
    /// application, which is a thing to ask for deliberately — with a
    /// double-click or Ctrl+O — not something a cursor key should do on its way
    /// across a listing.
    fn descend_selection(&mut self) {
        let depth = self.active;
        let is_dir = self.columns[depth]
            .cursor
            .and_then(|index| self.visible(depth).get(index).map(|entry| entry.is_dir))
            .unwrap_or(false);
        if is_dir {
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
        Self::play_op_sound(&result);
        self.record_undo(if clip.cut { "Move" } else { "Copy" }, result.changes);
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

    /// Whether this window does drag and drop at all.
    ///
    /// The picker does not: it is a transient serving someone else's request,
    /// and file management belongs to the browser — see
    /// [`specs/file-picker.md`]. Dropping files into the directory it happens
    /// to be showing would be exactly that.
    fn dnd_enabled(&self) -> bool {
        self.picker.is_none()
    }

    /// Where a drag at `(x, y)` would put its files, if anywhere.
    ///
    /// Everything resolves to a directory. A hit on a *file* row is not a
    /// target of its own — the files go beside it, into the directory it is
    /// in — which is why a miss falls through to the pane rather than
    /// rejecting the drop.
    fn drop_target_at(&self, x: f32, y: f32) -> Option<DropTarget> {
        if !self.dnd_enabled() {
            return None;
        }
        if let Some(index) = view::place_at(x, y, self.places.len()) {
            return Some(DropTarget::Place {
                index,
                path: self.places[index].path.clone(),
            });
        }
        // The rest of the sidebar takes nothing: it is chrome, not a place.
        if x < view::SIDEBAR_W {
            return None;
        }

        if let Some((depth, index)) = self.entry_at(x, y) {
            if let Some(entry) = self.visible(depth).get(index) {
                if entry.is_dir && !Self::is_a_move_home(&entry.path) {
                    return Some(DropTarget::Entry {
                        depth,
                        index,
                        path: entry.path.clone(),
                    });
                }
            }
        }

        let content = view::content_viewport(self.size.0, self.content_h(), self.mode);
        if x < content.left || x >= content.right || y < content.top || y >= content.bottom {
            return None;
        }
        let depth = self.pane_under(x, y);
        let path = self.columns[depth].path.clone();
        if Self::is_a_move_home(&path) {
            return None;
        }
        Some(DropTarget::Pane { depth, path })
    }

    /// Would dropping our own drag on `dest` move the files exactly where they
    /// already are?
    ///
    /// Such a drop has nothing to do, and a target that lights up to say it
    /// will do nothing is worse than no target at all: the answer is a
    /// dismissal — no outline, a cursor that says the drop is refused, and the
    /// icon flying home to where it was picked up.
    ///
    /// A *copy* onto the same directory is a real request — that is how a
    /// duplicate is made — so this only speaks for a move. The test is
    /// "not a copy" rather than "is a move" so it stays put once we refuse:
    /// refusing clears the negotiated action, and an `is_move` test would then
    /// accept again on the next motion and flicker between the two.
    fn is_a_move_home(dest: &Path) -> bool {
        use otto_kit::dnd;

        // Someone else's drag: we do not know where those files live, and the
        // source is the one that would have to answer this anyway. Asked first
        // because it reads our own payload, while `selected_action` needs a
        // live `AppContext` there is no reason to require of a target test.
        let Some(paths) = dnd::own_files() else {
            return false;
        };
        if paths.is_empty() || paths.iter().any(|path| path.parent() != Some(dest)) {
            return false;
        }
        !dnd::selected_action().contains(DndAction::Copy)
    }

    /// Put `paths` into the current drop target, moving them or copying them.
    ///
    /// Runs on the UI thread, like [`Browser::paste`], and inherits the same
    /// caveat: fine for a deliberate gesture on a known set of files, and due
    /// to move to the worker pool with the rest of the file operations.
    fn apply_drop(&mut self, paths: Vec<PathBuf>, move_them: bool) {
        let Some(target) = self.drop_target.take() else {
            return;
        };
        self.dirty = true;
        let dest = target.path().clone();

        // Files dragged back into the directory they already live in: a move
        // there is a no-op, and doing it through `paste` would rename them
        // out of the way of themselves. A *copy* onto the same directory is a
        // real request — that is how a duplicate is made — so it is kept.
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !move_them || path.parent() != Some(dest.as_path()))
            .collect();
        if paths.is_empty() {
            return;
        }

        // Keep Both for the same reason the paste path does: with no conflict
        // sheet to ask with, the only safe default is the one that cannot
        // destroy anything. A directory dropped into itself is refused by
        // `paste` itself, with a message.
        let result = model::paste(
            &model::Clipboard {
                paths,
                cut: move_them,
            },
            &dest,
            model::OnConflict::KeepBoth,
        );

        let summary = result.summary();
        self.status = (!summary.is_empty()).then_some(summary);
        Self::play_op_sound(&result);
        self.record_undo(if move_them { "Move" } else { "Copy" }, result.changes);
        self.reload_all();
    }

    /// What to draw under the cursor: the first selected entry's name and icon
    /// chain, and how many entries are travelling in total.
    ///
    /// The pictures a drag from `(x, y)` carries, laid out where they are on
    /// screen right now — plus the box that holds them, and where the grab sits
    /// inside it.
    ///
    /// The drag image is one surface, not one per file, so the whole spread has
    /// to fit in a single box: its size is the bounding box of the entries at
    /// the moment the drag begins, which is what lets them start where they
    /// were and gather from there. The nearest [`view::DRAG_ITEMS_MAX`] to the
    /// grab are the ones shown; the badge still counts them all.
    fn drag_items(&self, x: f32, y: f32) -> Option<DragItems> {
        let (depth, grabbed) = self.entry_at(x, y)?;
        let names: Vec<String> = self
            .selected_entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        if names.is_empty() {
            return None;
        }

        // The selected rows, nearest the grabbed one first, capped — and then
        // put back in listing order so the pile stacks the way the eye saw them.
        let entries = self.visible(depth);
        let mut chosen: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| names.contains(&entry.name))
            .map(|(index, _)| index)
            .collect();
        chosen.sort_by_key(|index| index.abs_diff(grabbed));
        chosen.truncate(view::DRAG_ITEMS_MAX);
        chosen.sort_unstable();
        let rects: Vec<Rect> = chosen
            .iter()
            .map(|index| self.entry_rect(depth, *index))
            .collect();
        let (image_w, image_h) = view::drag_image_size(self.mode);

        // The box: every picture's start, and the grab point itself, have to be
        // inside it.
        let left = rects.iter().fold(x, |acc, r| acc.min(r.left));
        let top = rects.iter().fold(y, |acc, r| acc.min(r.top));
        let right = rects
            .iter()
            .fold(x, |acc, r| acc.max(r.left + image_w))
            .max(left + image_w);
        let bottom = rects
            .iter()
            .fold(y, |acc, r| acc.max(r.top + image_h))
            .max(top + image_h);

        let items = chosen
            .iter()
            .zip(&rects)
            .filter_map(|(index, rect)| {
                let entry = entries.get(*index)?;
                Some(view::DragItem {
                    entry: (*entry).clone(),
                    thumb: self.thumbs.image(&entry.path, entry.modified).cloned(),
                    start: (rect.left - left, rect.top - top),
                })
            })
            .collect();

        // The badge hangs off the cursor's bottom right, and anything drawn
        // past the surface's edge is simply not there.
        let right = right.max(x + 8.0 + view::drag_badge_width(names.len()));
        let bottom = bottom.max(y + 30.0);

        Some((items, (right - left, bottom - top), (x - left, y - top)))
    }

    /// Where entry `index` of pane `depth` sits in the window right now.
    fn entry_rect(&self, depth: usize, index: usize) -> Rect {
        let (width, height) = (self.size.0, self.content_h());
        let count = self.visible_len(depth);
        let scroll = self.columns[depth].scroll.offset();

        match self.mode {
            ViewMode::Grid => view::grid_cell_rect(
                view::content_viewport(width, height, ViewMode::Grid),
                index,
                scroll,
            ),
            ViewMode::List => view::list_row_rect(width, count, index, scroll),
            ViewMode::Columns => view::miller_row_rect(
                depth,
                height,
                self.pan.offset(),
                self.miller_w,
                count,
                index,
                scroll,
            ),
        }
    }

    /// The paths a drag started now would carry: the whole selection, so
    /// dragging one of several selected files takes all of them.
    fn drag_paths(&self) -> Vec<PathBuf> {
        self.selected_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect()
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

    /// Say out loud what an operation did.
    ///
    /// Chosen from the outcome rather than from the command, which is what
    /// makes undo sound right without a special case: undoing a delete is a
    /// restore, and a restore moves files back into place, so it gets the
    /// arriving sound. Undoing a copy takes files away, and gets the other
    /// one. An operation that did nothing stays quiet.
    fn play_op_sound(result: &model::OpResult) {
        if result.trashed > 0 {
            otto_kit::sound::play_first(&SOUND_REMOVED);
        } else if result.moved + result.copied > 0 {
            otto_kit::sound::play_first(&SOUND_ARRIVED);
        }
    }

    /// Put `result`'s changes on the undo stack, if it changed anything.
    ///
    /// Called with every operation's outcome rather than only the clean ones:
    /// a paste that half worked still moved real files, and those are exactly
    /// the ones a user reaches for Ctrl+Z about.
    fn record_undo(&mut self, label: &'static str, changes: Vec<model::Change>) {
        if changes.is_empty() {
            return;
        }
        self.undo.push(UndoStep { label, changes });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Ctrl+Z — take back the last operation that changed files.
    fn undo_last(&mut self) {
        let Some(step) = self.undo.pop() else {
            self.status = Some("Nothing to undo".to_string());
            self.dirty = true;
            return;
        };

        let result = model::undo(&step.changes);
        Self::play_op_sound(&result);
        self.status = Some(if result.errors.is_empty() {
            format!("Undid {}", step.label)
        } else {
            result.errors[0].clone()
        });
        // Deliberately not pushed back on as a redo: an undo of a delete is a
        // restore, and re-deleting it would be a second trip to the Trash
        // rather than the inverse of anything. Redo is its own feature.
        self.reload_all();
        self.dirty = true;
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
        Self::play_op_sound(&result);
        self.record_undo("Delete", result.changes);
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
                self.record_undo(
                    "New Folder",
                    vec![model::Change::Created { path: path.clone() }],
                );
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
    /// Re-reading is in place: a column keeps its selection, cursor and scroll
    /// while only its listing is replaced. The watcher would get here on its
    /// own within a debounce, but an operation the user just performed should
    /// not visibly lag behind their own hand.
    fn reload_all(&mut self) {
        for column in &mut self.columns {
            column.reload();
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
        let mut refreshed = false;
        let mut vanished = None;
        for depth in 0..self.columns.len() {
            if self.columns[depth].poll() {
                changed = true;
            }
            if std::mem::take(&mut self.columns[depth].refreshed) {
                refreshed = true;
            }
            // The shallowest one wins: everything below it is inside it, so
            // it is gone too.
            if self.columns[depth].gone && vanished.is_none() {
                vanished = Some(depth);
            }
        }
        if refreshed {
            self.resync_cursors();
        }
        if let Some(depth) = vanished {
            self.follow_vanished(depth);
            changed = true;
        }
        changed
    }

    /// Put every cursor back on what is selected, after a listing was replaced
    /// underneath it.
    ///
    /// The selection is by name and survives; the cursor is an index into the
    /// visible order and does not, so a file appearing above the selection
    /// would otherwise leave the keyboard one row off from the highlight.
    /// Nothing is scrolled: a change somebody else made must not move the
    /// view out from under the person reading it.
    fn resync_cursors(&mut self) {
        for depth in 0..self.columns.len() {
            let Some(first) = self.columns[depth].selection.iter().next().cloned() else {
                continue;
            };
            let index = self.visible(depth).iter().position(|e| e.name == first);
            self.columns[depth].cursor = index;
            self.columns[depth].anchor = index;
        }
    }

    /// A displayed directory was deleted or moved away: go to the nearest
    /// ancestor that still exists, and say why.
    fn follow_vanished(&mut self, depth: usize) {
        let path = self.columns[depth].path.clone();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        if depth > 0 {
            // Its parent is on screen already — drop it and everything under
            // it, and leave the parent showing.
            self.columns.truncate(depth);
            self.active = self.columns.len() - 1;
            self.reveal_pane(self.active);
        } else {
            let home = model::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let surviving = path
                .ancestors()
                .skip(1)
                .find(|p| p.is_dir())
                .map(|p| p.to_path_buf())
                .unwrap_or(home);
            self.columns = vec![Column::new(surviving)];
            self.active = 0;
            self.pan.scroll_to(0.0);
        }
        self.status = Some(format!("\u{201c}{name}\u{201d} is no longer there"));
        self.dirty = true;
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
            opening: self.opening_progress(),
            renaming: self.rename.as_ref().map(|r| (r.depth, r.index)),
            controls: self.controls,
            focused: self.focused,
            // A translucent material needs something blurred behind it to be
            // translucent over. See [`view::opaque`].
            blurred: self.focused && self.blur_available,
            can_go_back: !self.back.is_empty(),
            can_go_forward: !self.forward.is_empty(),
            nav_pressed: self.nav_pressed,
            preview,
            action_row: self.picker.as_ref().map(|session| view::FooterData {
                accept_label: &session.accept_label,
                accept_enabled: self.picker_accept_enabled(),
                save_name: session.request.mode.names_a_file(),
                save_problem: self.save_problem(),
                filters: &session.filter_labels,
                current_filter: session.current_filter,
                filter_open: session.filter_open,
                hovered: self.footer_hover,
                pressed: self.footer_pressed,
            }),
            footer: self.footer_h(),
            quickview_close_hovered: self.quickview_close_hovered,
            thumbs: Some(&self.thumbs),
            drop_target: self.drop_target.as_ref().map(DropTarget::highlight),
            marquee: self.marquee_band(),
        }
    }
}

// ---------------------------------------------------------------------------
// App shell
// ---------------------------------------------------------------------------

struct FilesApp {
    window: Option<Window>,
    state: Arc<Mutex<Browser>>,
    /// The panel materials' fade, which the scene runs and this drains: the
    /// blur it wants switched, and whether it is still running. Held here
    /// because the window is — see `scene::FrostState`.
    frost: Option<Arc<scene::FrostState>>,
    /// The modifier state, as the compositor reports it in
    /// `wl_keyboard.modifiers` — not inferred from the text a chord produces
    /// (Ctrl+I is historically a TAB character and Ctrl+H a backspace, so
    /// reading `utf8` to detect them is both obscure and unreliable), and not
    /// tracked from `Control_L`/`Control_R` presses either: those miss a
    /// modifier that was already held when the window took focus, and any
    /// chord the compositor swallowed before the key reached us.
    ///
    /// Shared rather than a plain field because the pointer callback needs it
    /// too — Ctrl+click and Shift+click are the pointer half of the same
    /// selection rules — and that callback outlives any borrow of `self`.
    modifiers: Arc<Mutex<Modifiers>>,
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
        }

        // otto-kit's materials are translucent by design — they expect a
        // blurred backdrop behind them. Without one the desktop shows through
        // the window rather than being frosted by it.
        //
        // Asked of the window rather than of the style directly: the window
        // drops the blur while it is unfocused and puts it back on the next
        // activate, so the compositor is not running a full-window gaussian
        // for a window nobody is looking at.
        //
        // `OTTO_FILES_NO_BLUR=1` drops the frosted backdrop entirely, to test
        // what it costs — as does running under a compositor that carries no
        // surface style at all. The panels are filled in rather than left
        // translucent in both cases, so what that isolates is the compositor's
        // blur work and not the window's legibility.
        let blur =
            window.surface_style().is_some() && std::env::var_os("OTTO_FILES_NO_BLUR").is_none();
        window.set_background_blur(blur);
        self.state.lock().unwrap().blur_available = blur;

        // The panels' scene. `None` only where the engine could not be brought
        // up, which is the case the immediate-mode chrome still covers: the
        // window then draws without its grounds rather than not at all.
        let scene = Arc::new(Mutex::new(window.layer_node().map(scene::Scene::new)));

        // The panels are this client's own layers, so the fade between their
        // translucent and their filled-in forms is the scene's to run — and
        // with it the only moment the blur can be switched without being seen
        // doing it. The window stops following focus and waits to be told; see
        // `scene::FrostState`, which `on_update` drains.
        if let Some(scene) = scene.lock().unwrap().as_ref() {
            window.set_fades_own_material(true);
            self.frost = Some(scene.frost_state());
        }

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

            // The save field's value, over the box the action row drew for
            // it — the same two-step the in-place rename takes, and for the
            // same reason: the text input owns its caret and selection and
            // paints them itself.
            if browser.save_name.is_some() {
                let (width, window_h) = (browser.size.0, browser.size.1);
                let rect = view::footer_name_rect(width, window_h);
                let input = browser.save_name.as_mut().unwrap();
                input.set_size(rect.width(), rect.height());
                canvas.save();
                canvas.translate((rect.left, rect.top));
                input.render_at(canvas, rect.width(), rect.height());
                canvas.restore();
            }

            // Last of all, because it is modal and dims everything above.
            if let Some(sheet) = browser.confirm.as_ref() {
                let (width, window_h) = (browser.size.0, browser.size.1);
                view::draw_confirm(
                    canvas,
                    &theme,
                    width,
                    window_h,
                    &view::ConfirmData {
                        message: &sheet.message,
                        detail: &sheet.detail,
                        pressed: sheet.pressed,
                    },
                );
            }
        });

        // Also when only Quick View wants a surface: the columns stay in the
        // scene, and this carries the preview alone.
        if pane_surfaces::quickview_on_surface() {
            self.pane_surfaces = Some(pane_surfaces::PaneSurfaces::new(
                AppContext::scale_factor() as f32
            ));
        }

        self.install_dnd(&window);
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
        // The scene decides when the compositor's backdrop blur may be
        // switched — it owns the fade the switch has to hide under — but the
        // window is what carries the request, so it is applied here.
        if let (Some(frost), Some(window)) = (self.frost.as_ref(), self.window.as_ref()) {
            let fading = frost.is_fading();
            if let Some(frosted) = frost.take_pending() {
                window.set_frost(frosted);
            }
            if fading {
                window.request_frame();
            }
        }

        let (repaint, preview_target, scrolled_only, thumb_jobs) = {
            let mut browser = self.state.lock().unwrap();
            let changed = browser.poll();
            // Momentum, the overscroll bounce and the scrollbar's fade all
            // advance here rather than on input, since they keep running after
            // the gesture ends.
            let scrolled = browser.tick_scroll();
            let animating = browser.quickview_animating()
                | browser.tick_quickview_exit()
                | browser.tick_open_pulse();
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
        let animating = browser.scroll_animating()
            || browser.quickview_animating()
            || browser.opening.is_some()
            // The panel materials' fade runs on this client's own engine, and
            // an engine only advances when it is ticked.
            || self.frost.as_ref().is_some_and(|frost| frost.is_fading());
        animating.then(|| std::time::Duration::from_millis(8))
    }

    fn on_configure(&mut self, _ctx: &AppContext, configure: WindowConfigure, _serial: u32) {
        let mut browser = self.state.lock().unwrap();
        if let (Some(w), Some(h)) = (configure.new_size.0, configure.new_size.1) {
            browser.size = (w.get() as f32, h.get() as f32);
            browser.dirty = true;
        }
        // Focus arrives here, and the chrome is drawn dimmer without it.
        if browser.focused != configure.is_activated() {
            browser.focused = configure.is_activated();
            browser.dirty = true;
        }
        drop(browser);
        self.render();
    }

    /// The authoritative modifier state, sent before the key event it
    /// belongs to and again whenever the window takes focus.
    fn on_modifiers(&mut self, _ctx: &AppContext, modifiers: Modifiers) {
        *self.modifiers.lock().unwrap() = modifiers;
    }

    /// Quick View is a preview of what the window has selected, so it belongs
    /// to the window's focus: once the keyboard goes somewhere else the panel
    /// is a card floating over a background window with nothing to preview.
    ///
    /// This is also how expose reaches us. The panel is a subsurface, not a
    /// popup, so the compositor's `dismiss_all_popups` on the way into Show All
    /// cannot take it down — but Otto drops keyboard focus entering expose, and
    /// that lands here.
    ///
    /// Only the browser's own toplevel counts. A leave on the Get Info panel is
    /// focus moving between two of our windows, not away from the browser.
    fn on_keyboard_leave(&mut self, _ctx: &AppContext, surface: &wl_surface::WlSurface) {
        use wayland_client::Proxy;
        let ours = self
            .window
            .as_ref()
            .and_then(|window| window.wl_surface())
            .is_some_and(|main| main.id() == surface.id());
        if !ours {
            return;
        }
        if self.state.lock().unwrap().close_quickview() {
            self.render();
        }
    }

    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        event: &KeyEvent,
        key_state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        use smithay_client_toolkit::seat::keyboard::Keysym;

        // A modifier key on its own is not a shortcut and not type-ahead:
        // the state it changed already arrived in `on_modifiers`.
        if matches!(
            event.keysym,
            Keysym::Control_L
                | Keysym::Control_R
                | Keysym::Shift_L
                | Keysym::Shift_R
                | Keysym::Alt_L
                | Keysym::Alt_R
                | Keysym::Super_L
                | Keysym::Super_R
        ) {
            return;
        }
        if key_state != wl_keyboard::KeyState::Pressed {
            return;
        }
        let mods = *self.modifiers.lock().unwrap();
        let (ctrl, shift) = (mods.ctrl, mods.shift);

        {
            let mut browser = self.state.lock().unwrap();

            // The confirmation sheet is modal, so it takes the keyboard
            // whole: Return is the answer it is asking for and Escape backs
            // out of it. Nothing else reaches the window behind it.
            if browser.confirm.is_some() {
                match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => browser.confirm_replace(),
                    Keysym::Escape => browser.confirm_dismiss(),
                    _ => {}
                }
                drop(browser);
                self.render();
                return;
            }

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

            // In Save mode the name field holds the keyboard focus: what the
            // user is doing is naming a file, so printable keys are the name
            // rather than type-ahead, and Backspace edits it rather than
            // walking up a directory.
            //
            // What the field does *not* take is vertical motion. Up, Down and
            // the page keys still drive the listing, so a user can pick the
            // file they mean to overwrite without ever leaving the field —
            // which is the whole point of it holding focus.
            if browser.save_name.is_some() {
                let editing = match event.keysym {
                    Keysym::Return | Keysym::KP_Enter => {
                        browser.picker_accept();
                        drop(browser);
                        self.render();
                        return;
                    }
                    Keysym::Escape => {
                        browser.picker_cancel();
                        drop(browser);
                        self.render();
                        return;
                    }
                    Keysym::Up
                    | Keysym::Down
                    | Keysym::Page_Up
                    | Keysym::Page_Down
                    | Keysym::Tab => None,
                    Keysym::Left => Some(TextInputKey::Left),
                    Keysym::Right => Some(TextInputKey::Right),
                    Keysym::Home => Some(TextInputKey::Home),
                    Keysym::End => Some(TextInputKey::End),
                    Keysym::BackSpace => Some(TextInputKey::Backspace),
                    Keysym::Delete => Some(TextInputKey::Delete),
                    Keysym::a if ctrl => Some(TextInputKey::SelectAll),
                    // A chord that is not the field's own is the window's:
                    // Ctrl+W and friends still reach the shortcuts below.
                    _ if ctrl => None,
                    _ => event
                        .utf8
                        .as_ref()
                        .and_then(|s| s.chars().next())
                        .map(TextInputKey::Char),
                };
                if let Some(key) = editing {
                    let mods = KeyMods { shift, ctrl };
                    if let Some(input) = browser.save_name.as_mut() {
                        input.on_key(key, mods);
                    }
                    browser.dirty = true;
                    drop(browser);
                    self.render();
                    return;
                }
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
                // Move to Trash. Plain Delete is the spec's binding; the
                // modified forms are there because the chord people reach for
                // is Cmd+Delete, and on a keyboard whose big key is Backspace
                // that arrives as Ctrl+BackSpace. Plain Backspace is *not*
                // one of them — it goes up a directory, and always has.
                //
                // Shift is excluded deliberately: Shift+Delete is spelled for
                // permanent deletion, which is not built. Trashing instead
                // would be the wrong answer to a keystroke that means "and I
                // mean it", so the chord does nothing until it does the right
                // thing.
                Keysym::Delete | Keysym::KP_Delete if !shift && browser.picker.is_none() => {
                    browser.move_selected_to_trash()
                }
                Keysym::BackSpace if ctrl && !shift && browser.picker.is_none() => {
                    browser.move_selected_to_trash()
                }
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
                // Undo is file management too: a picker is a chooser, and has
                // nothing of its own to take back.
                Keysym::z if ctrl && browser.picker.is_none() => browser.undo_last(),
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
                Keysym::n if ctrl => browser.open_new_window(),
                // What a double-click does, from the keyboard: descend into a
                // directory, or activate a file. Return is not free for this —
                // it renames, the way it does on the desktop this follows — so
                // opening needs a chord of its own.
                Keysym::o if ctrl => browser.open_cursor_entry(),
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
                    // Only an unmodified key is a letter of a name. A chord
                    // this app does not bind is still a chord — it belongs to
                    // whoever does bind it, not to the buffer.
                    let chord = ctrl || mods.alt || mods.logo;
                    if let Some(ch) = event
                        .utf8
                        .as_deref()
                        .filter(|_| !chord)
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

    /// Take drops: from another application, and from this window's own drags.
    ///
    /// The drag conversation is per-position — see [`otto_kit::dnd`] — so every
    /// enter and every motion answers, even to say no. Answering nothing is how
    /// a target refuses, and a refused drag never delivers.
    fn install_dnd(&self, window: &Window) {
        use otto_kit::dnd::{self, DragEvent};
        use wayland_client::Proxy;

        let state = Arc::clone(&self.state);
        let window = window.clone();
        // Enter names the surface; motion and drop do not. A drag over some
        // other surface of ours — Quick View, the info sheet — is not a drop
        // target, so what the enter decided has to be remembered.
        let on_toplevel = std::cell::Cell::new(false);

        dnd::register(move |event| {
            let is_ours = |id: &wayland_client::backend::ObjectId| {
                window.wl_surface().is_some_and(|s| s.id() == *id)
            };

            match event {
                DragEvent::Enter { surface, x, y } => {
                    on_toplevel.set(is_ours(surface));
                    if !on_toplevel.get() {
                        return;
                    }
                    hover_drag(&state, *x as f32, *y as f32);
                }
                DragEvent::Motion { x, y } => {
                    if !on_toplevel.get() {
                        return;
                    }
                    hover_drag(&state, *x as f32, *y as f32);
                }
                DragEvent::Leave => {
                    if !on_toplevel.replace(false) {
                        return;
                    }
                    let mut browser = state.lock().unwrap();
                    browser.dirty |= browser.drop_target.take().is_some();
                    drop(browser);
                    AppContext::request_wakeup();
                }
                DragEvent::Drop { x, y } => {
                    if !on_toplevel.replace(false) {
                        return;
                    }
                    // The negotiated action, not the one we asked for: the
                    // compositor picks, and a source that only offered a copy
                    // must not have its files moved.
                    let move_them = dnd::selected_action().contains(dnd::DndAction::Move);

                    // Our own drag is served from our own payload; see
                    // `dnd::own_files` for why it cannot go over the pipe.
                    let paths = if dnd::dragging() {
                        let paths = dnd::own_files();
                        dnd::finish();
                        paths
                    } else {
                        dnd::receive_files().map(|(paths, _)| paths)
                    };

                    let mut browser = state.lock().unwrap();
                    // Resolved again at the drop's own position rather than
                    // trusting the last motion: the release may land somewhere
                    // no motion reported.
                    browser.drop_target = browser.drop_target_at(*x as f32, *y as f32);
                    match paths {
                        Some(paths) => browser.apply_drop(paths, move_them),
                        None => {
                            browser.drop_target = None;
                            browser.dirty = true;
                        }
                    }
                    drop(browser);
                    AppContext::request_wakeup();
                }
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
                let mods = *modifiers.lock().unwrap();
                let (ctrl, shift) = (mods.ctrl, mods.shift);
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

                // The confirmation sheet is modal: it answers its own two
                // buttons and swallows everything else, so a click meant for
                // it can never land on the listing behind it.
                if browser.confirm.is_some() {
                    let window_h = browser.size.1;
                    let hit = view::confirm_at(x, y, width, window_h);
                    match event.kind {
                        PointerEventKind::Press { button, .. } if button != BTN_RIGHT => {
                            if let Some(sheet) = browser.confirm.as_mut() {
                                sheet.pressed = hit;
                            }
                            browser.dirty = true;
                        }
                        PointerEventKind::Release { button, .. } if button != BTN_RIGHT => {
                            let armed = browser
                                .confirm
                                .as_mut()
                                .and_then(|sheet| sheet.pressed.take());
                            if armed.is_some() && armed == hit {
                                match armed {
                                    Some(view::ConfirmButton::Replace) => {
                                        browser.confirm_replace()
                                    }
                                    Some(view::ConfirmButton::Cancel) => browser.confirm_dismiss(),
                                    None => {}
                                }
                            }
                            browser.dirty = true;
                        }
                        PointerEventKind::Motion { .. } => {
                            AppContext::set_cursor_shape(CursorShape::Default);
                        }
                        _ => {}
                    }
                    drop(browser);
                    window_for_events.request_frame();
                    continue;
                }

                // A click in the name field places the caret in it. The field
                // never loses focus to the listing, so there is no focus to
                // take here — only a caret to move.
                if browser.save_name.is_some()
                    && matches!(event.kind, PointerEventKind::Press { button, .. } if button != BTN_RIGHT)
                {
                    let field = view::footer_name_rect(width, browser.size.1);
                    if field.contains(skia_safe::Point::new(x, y)) {
                        if let Some(input) = browser.save_name.as_mut() {
                            input.on_pointer_down(x - field.left, 1, shift);
                        }
                        browser.dirty = true;
                        drop(browser);
                        window_for_events.request_frame();
                        continue;
                    }
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
                        PointerEventKind::Release { button, .. }
                            if button != BTN_RIGHT && browser.footer_pressed.is_some() =>
                        {
                            browser.footer_release(hit);
                            drop(browser);
                            window_for_events.request_frame();
                            continue;
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

                        // A rubber band owns the gesture while it is out: no
                        // scrollbar, hover or resize affordance should answer
                        // a pointer that is busy drawing a selection.
                        if browser.marquee.is_some() {
                            browser.update_marquee(x, y);
                            drop(browser);
                            continue;
                        }

                        // Far enough from the press to be a drag rather than an
                        // unsteady click. The whole selection goes, and the
                        // compositor owns the pointer from here — nothing else
                        // in this handler will see the rest of the gesture.
                        if let Some((start_x, start_y, serial)) = browser.drag_armed {
                            if (x - start_x).hypot(y - start_y) >= DRAG_THRESHOLD {
                                browser.drag_armed = None;
                                browser.drag_started();
                                let paths = browser.drag_paths();
                                // The picture the cursor carries: the first
                                // file of the selection, and how many are
                                // coming with it.
                                // Every travelling file, where it is on screen
                                // now: the picture starts as the listing and
                                // gathers from there.
                                let items = browser.drag_items(start_x, start_y);
                                // The picture is shaped like the view it came
                                // from: a cell in the grid, a row card in the
                                // list and column views.
                                let mode = browser.mode;
                                let count = paths.len();
                                drop(browser);
                                if let (Some(surface), Some((items, size, anchor))) =
                                    (window_for_events.wl_surface(), items)
                                {
                                    let theme = AppContext::current_theme();
                                    otto_kit::dnd::start_file_drag_with_icon(
                                        &paths,
                                        DndAction::Copy | DndAction::Move,
                                        &surface,
                                        serial,
                                        (size.0 as i32, size.1 as i32),
                                        anchor,
                                        move |canvas, _w, _h| {
                                            view::draw_drag_image(
                                                canvas, &theme, mode, &items, anchor, count,
                                            );
                                        },
                                    );
                                }
                                continue;
                            }
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
                        // A press that came up without travelling was a click —
                        // including one that left its narrowing until now.
                        browser.drag_armed = None;
                        // The band goes away with the button that drew it; what
                        // it caught stays selected.
                        browser.dirty |= browser.marquee.take().is_some();
                        browser.release_entry();
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
                                // A double click on the header zooms the
                                // window instead of moving it.
                                window_for_events.titlebar_press(&seat, serial, x, y);
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

                        // A press on a row might be the start of a drag. Armed
                        // here and decided on motion: the selection below still
                        // happens, so a press that never travels is an ordinary
                        // click and a second one still opens the directory.
                        if browser.dnd_enabled() && browser.entry_at(x, y).is_some() {
                            browser.drag_armed = Some((x, y, serial));
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
                                    browser.note_ctrl_row_click(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    browser.press_entry(depth, index);
                                }
                            } else if hit_content(area, x, y) {
                                // Nothing under the press: it is the corner of
                                // a rubber band. A band that never travels is
                                // an empty one, which is how a plain click on
                                // nothing comes to mean nothing selected.
                                if !ctrl && !shift {
                                    browser.clear_pane_selection(depth);
                                }
                                browser.begin_marquee(depth, x, y, ctrl || shift);
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
                                        browser.note_ctrl_row_click(depth, index);
                                    } else if shift {
                                        browser.extend_select(depth, index);
                                    } else {
                                        browser.press_entry(depth, index);
                                    }
                                } else if !ctrl && !shift {
                                    let area =
                                        view::content_viewport(width, height, ViewMode::List);
                                    if hit_content(area, x, y) {
                                        browser.clear_pane_selection(depth);
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
                                    browser.note_ctrl_row_click(depth, index);
                                } else if shift {
                                    browser.extend_select(depth, index);
                                } else {
                                    // A directory here already opened on the
                                    // single click — Miller shows its child
                                    // eagerly. A *file* did not, and a double
                                    // click is how one is opened in the other
                                    // two views, so it is how one is opened
                                    // here too.
                                    browser.press_entry(depth, index);
                                }
                            } else if let Some((depth, None)) = hit {
                                // Inside a pane, below its last row. The pane
                                // takes the keyboard either way; without a
                                // modifier the click also means "nothing".
                                if ctrl || shift {
                                    browser.active = depth;
                                    browser.dirty = true;
                                } else {
                                    browser.clear_pane_selection(depth);
                                }
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
        frost: None,
        modifiers: Arc::new(Mutex::new(Modifiers::default())),
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

    /// Ctrl+O is bound to the same call a double-click makes, so on a folder
    /// it descends. Worth pinning because Return is *not* this — it renames —
    /// and it would be an easy mistake to give opening back to Return and
    /// leave the chord doing nothing.
    #[test]
    fn opening_a_folder_descends_into_it() {
        let dir = TempDir::holding(&["a.txt"]);
        let sub = dir.0.join("sub");
        std::fs::create_dir_all(sub.join("inner")).expect("child dir");

        let mut browser = Browser::new(dir.0.clone());
        for _ in 0..500 {
            if browser.columns[0].poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // List view: descending replaces the column, so the deepest path is
        // the plainest statement of where the window ended up.
        browser.mode = ViewMode::List;
        let index = browser
            .visible(0)
            .iter()
            .position(|e| e.name == "sub")
            .expect("the folder is listed");
        browser.select(0, index);

        assert_eq!(browser.current_path(), dir.0);
        browser.open_selection();
        assert_eq!(browser.current_path(), sub);
    }

    /// Ctrl+N opens a new window at the default location, not wherever this
    /// window happens to be pointed — a new window is a fresh start. Where the
    /// window is browsing must not move the target.
    #[test]
    fn a_new_window_opens_at_the_default_location() {
        let (mut browser, dir) = browser_over(&["one.txt", "two.txt"]);
        let default = Browser::default_location();
        assert_eq!(browser.new_window_target(), Some(default.clone()));

        // Navigating somewhere else leaves it alone. `dir` is a temporary
        // directory, so it is never the default location, and a target that
        // followed the window would show up here.
        let child = dir.0.join("sub");
        std::fs::create_dir_all(&child).expect("child dir");
        browser.navigate_to(&child);
        for _ in 0..500 {
            if browser.columns.last_mut().is_some_and(|c| c.poll()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(browser.new_window_target(), Some(default));
        assert_ne!(
            browser.new_window_target().as_deref(),
            Some(child.as_path())
        );
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

#[cfg(test)]
mod dnd_tests {
    use super::*;

    /// A real directory holding files and subdirectories, swept up on drop.
    ///
    /// Drop targets turn on `is_dir`, which comes off the filesystem, so these
    /// tests need something actually on disk the way the type-ahead ones do.
    struct TempDir(PathBuf);

    impl TempDir {
        fn holding(files: &[&str], dirs: &[&str]) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let path = std::env::temp_dir().join(format!(
                "otto-files-dnd-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("temp dir");
            for name in files {
                std::fs::write(path.join(name), b"contents").expect("temp file");
            }
            for name in dirs {
                std::fs::create_dir_all(path.join(name)).expect("temp subdir");
            }
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A list-view browser over a loaded directory. List view because its row
    /// geometry is a single strip under the header, which a test can point at
    /// without reproducing the Miller stack's pan.
    fn browser_over(files: &[&str], dirs: &[&str]) -> (Browser, TempDir) {
        let dir = TempDir::holding(files, dirs);
        let mut browser = Browser::new(dir.0.clone());
        browser.mode = ViewMode::List;
        for _ in 0..500 {
            if browser.columns[0].poll() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!browser.columns[0].loading(), "listing never arrived");
        (browser, dir)
    }

    /// The middle of row `index` in list view.
    fn row_point(index: usize) -> (f32, f32) {
        (
            view::SIDEBAR_W + 100.0,
            view::HEADER_H + view::COLUMNS_H + view::ROW_H * index as f32 + view::ROW_H / 2.0,
        )
    }

    /// Where `name` sits in the sorted listing.
    fn row_of(browser: &Browser, name: &str) -> usize {
        browser
            .visible(browser.active)
            .iter()
            .position(|entry| entry.name == name)
            .expect("entry is in the listing")
    }

    #[test]
    fn a_directory_row_takes_the_drop_itself() {
        let (browser, dir) = browser_over(&["a.txt"], &["target"]);
        let (x, y) = row_point(row_of(&browser, "target"));

        let hit = browser.drop_target_at(x, y).expect("a target");
        assert_eq!(hit.path(), &dir.join("target"));
        assert!(matches!(hit, DropTarget::Entry { .. }));
    }

    /// A press on an entry that is not selected acts at once: the selection
    /// follows the pointer down, the way it always has.
    #[test]
    fn a_press_on_an_unselected_entry_selects_it_immediately() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
        let b = row_of(&browser, "b.txt");

        browser.press_entry(0, b);

        assert_eq!(browser.columns[0].selection.len(), 1);
        assert!(browser.columns[0].selection.contains("b.txt"));
    }

    /// A press on one of several selected entries leaves the group alone, so
    /// the drag that usually follows can carry all of it.
    #[test]
    fn a_press_inside_a_group_keeps_the_whole_selection() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
        let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
        browser.select(0, a);
        browser.extend_select(0, c);
        assert_eq!(browser.columns[0].selection.len(), 3, "three are selected");

        browser.press_entry(0, row_of(&browser, "b.txt"));

        assert_eq!(
            browser.columns[0].selection.len(),
            3,
            "the group survives the press"
        );
    }

    /// …and if the press comes back up without dragging, it was a click after
    /// all: the selection narrows to the one under the pointer.
    #[test]
    fn a_click_inside_a_group_narrows_to_it_on_release() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
        let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
        browser.select(0, a);
        browser.extend_select(0, c);
        let b = row_of(&browser, "b.txt");

        browser.press_entry(0, b);
        browser.release_entry();

        assert_eq!(browser.columns[0].selection.len(), 1);
        assert!(browser.columns[0].selection.contains("b.txt"));
    }

    /// A drag cancels the deferred narrowing outright: the group is what
    /// travels, and the button coming up at the end must not collapse it.
    #[test]
    fn a_drag_out_of_a_group_leaves_the_group_whole() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
        let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
        browser.select(0, a);
        browser.extend_select(0, c);

        browser.press_entry(0, row_of(&browser, "b.txt"));
        browser.drag_started();
        browser.release_entry();

        assert_eq!(
            browser.columns[0].selection.len(),
            3,
            "all three are still selected after the drag"
        );
    }

    /// A deferred narrowing must never outlive its own gesture. If a release
    /// goes missing — consumed by something else on its way through — the next
    /// press drops the stale decision instead of letting it land on this click.
    #[test]
    fn a_pending_narrow_never_lands_on_a_later_click() {
        let (mut browser, _dir) = browser_over(&["a.txt", "b.txt", "c.txt"], &[]);
        let (a, c) = (row_of(&browser, "a.txt"), row_of(&browser, "c.txt"));
        browser.select(0, a);
        browser.extend_select(0, c);

        // A press inside the group whose release never arrives.
        browser.press_entry(0, row_of(&browser, "b.txt"));

        // A later, unrelated click elsewhere: it selects its own entry, and the
        // release resolves nothing left over from before.
        let a_row = row_of(&browser, "a.txt");
        browser.press_entry(0, a_row);
        browser.release_entry();

        assert_eq!(browser.columns[0].selection.len(), 1);
        assert!(
            browser.columns[0].selection.contains("a.txt"),
            "the click that happened is the one that counts"
        );
    }

    #[test]
    fn a_file_row_drops_into_the_directory_it_is_in() {
        // Dropping "onto" a file means beside it, not inside it.
        let (browser, dir) = browser_over(&["a.txt"], &["target"]);
        let (x, y) = row_point(row_of(&browser, "a.txt"));

        let hit = browser.drop_target_at(x, y).expect("a target");
        assert_eq!(hit.path(), &dir.0);
        assert!(matches!(hit, DropTarget::Pane { .. }));
    }

    #[test]
    fn empty_space_below_the_rows_drops_into_the_pane() {
        let (browser, dir) = browser_over(&["a.txt"], &[]);
        // Past the one entry, but still inside the window: a point below the
        // viewport is off the pane altogether, which is a different miss.
        let (x, y) = row_point(10);

        let hit = browser.drop_target_at(x, y).expect("a target");
        assert_eq!(hit.path(), &dir.0);
    }

    #[test]
    fn the_sidebar_takes_a_drop_only_on_a_place() {
        let (browser, _dir) = browser_over(&["a.txt"], &[]);
        assert!(!browser.places.is_empty(), "the sidebar has places");

        let place = view::place_rect(0);
        let hit = browser
            .drop_target_at(place.center_x(), place.center_y())
            .expect("a place takes a drop");
        assert_eq!(hit.path(), &browser.places[0].path);

        // The header band above the first place is chrome, and takes nothing.
        assert_eq!(browser.drop_target_at(20.0, 4.0), None);
    }

    #[test]
    fn the_picker_refuses_every_drop() {
        let (mut browser, _dir) = browser_over(&["a.txt"], &["target"]);
        let (x, y) = row_point(row_of(&browser, "target"));
        assert!(
            browser.drop_target_at(x, y).is_some(),
            "the browser takes it"
        );

        let (responder, _receiver) = tokio::sync::oneshot::channel();
        browser.picker = Some(picker::Session::new(
            picker::Request {
                mode: picker::Mode::Open,
                handle: String::new(),
                app_id: String::new(),
                parent_window: String::new(),
                title: String::new(),
                accept_label: String::new(),
                multiple: false,
                directory: false,
                modal: false,
                current_name: String::new(),
                current_folder: None,
                current_file: None,
                files: Vec::new(),
                filters: Vec::new(),
                current_filter: 0,
                choices: Vec::new(),
            },
            responder,
        ));

        assert!(!browser.dnd_enabled());
        assert_eq!(browser.drop_target_at(x, y), None);
    }

    /// A drop is a reload, and a reload must leave the pane looking at what
    /// it was looking at — in every view. Run over all three because the two
    /// that measure an empty pane as taller than nothing (a grid counts its
    /// padding, a Miller pane its row inset) are exactly the ones that used to
    /// clamp to the top while the fresh listing was still being read.
    #[test]
    fn a_drop_keeps_the_pane_where_it_was_scrolled() {
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
            let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let (mut browser, dir) = browser_over(&refs, &["sub"]);
            browser.mode = mode;
            browser.size = (900.0, 400.0);
            browser.sync_scroll_metrics();

            browser.columns[0].scroll.state.set_offset(300.0);
            let before = browser.columns[0].scroll.offset();
            assert!(before > 0.0, "{mode:?}: the pane has to be scrolled");

            browser.drop_target = Some(DropTarget::Entry {
                depth: 0,
                index: 0,
                path: dir.join("sub"),
            });
            browser.apply_drop(vec![dir.join("f00.txt")], true);

            // The frame loop re-measures every frame, including the ones that
            // land while the re-read is still in flight. Those are the frames
            // that used to lose the position, so the test has to run them.
            for _ in 0..500 {
                browser.sync_scroll_metrics();
                if browser.columns[0].poll() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            browser.sync_scroll_metrics();

            assert_eq!(
                browser.columns[0].scroll.offset(),
                before,
                "{mode:?}: the drop scrolled the pane away from what the user was looking at"
            );
        }
    }

    #[test]
    fn undoing_a_drop_puts_the_file_back() {
        let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: 0,
            path: dir.join("sub"),
        });
        browser.apply_drop(vec![dir.join("a.txt")], true);
        assert!(dir.join("sub/a.txt").exists(), "the move happened");

        browser.undo_last();

        assert!(dir.join("a.txt").exists(), "back where it came from");
        assert!(!dir.join("sub/a.txt").exists(), "and not still there");
        assert_eq!(browser.status.as_deref(), Some("Undid Move"));
    }

    /// Undoing a copy takes the copy away — via the Trash, so an undo is never
    /// itself the thing that loses a file.
    #[test]
    fn undoing_a_copy_removes_the_copy_and_leaves_the_original() {
        // Keeps the delete out of the real Trash; see `model::test_data_home`.
        let _trash = model::test_data_home();
        let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: 0,
            path: dir.join("sub"),
        });
        browser.apply_drop(vec![dir.join("a.txt")], false);
        assert!(dir.join("sub/a.txt").exists(), "the copy happened");

        browser.undo_last();

        assert!(dir.join("a.txt").exists(), "the original is untouched");
        assert!(!dir.join("sub/a.txt").exists(), "the copy is gone");
    }

    #[test]
    fn undoing_a_delete_restores_the_file() {
        // Keeps the delete out of the real Trash; see `model::test_data_home`.
        let _trash = model::test_data_home();
        let (mut browser, dir) = browser_over(&["a.txt"], &[]);
        browser.select(0, 0);
        browser.move_selected_to_trash();
        assert!(!dir.join("a.txt").exists(), "the delete happened");

        browser.undo_last();

        assert!(dir.join("a.txt").exists(), "restored out of the Trash");
        assert_eq!(browser.status.as_deref(), Some("Undid Delete"));
    }

    /// The stack is per operation, and Ctrl+Z walks back through it.
    #[test]
    fn undo_walks_back_one_operation_at_a_time() {
        let (mut browser, dir) = browser_over(&["a.txt", "b.txt"], &["sub"]);
        for name in ["a.txt", "b.txt"] {
            browser.drop_target = Some(DropTarget::Entry {
                depth: 0,
                index: 0,
                path: dir.join("sub"),
            });
            browser.apply_drop(vec![dir.join(name)], true);
        }

        browser.undo_last();
        assert!(dir.join("b.txt").exists(), "the last one came back first");
        assert!(dir.join("sub/a.txt").exists(), "and only the last one");

        browser.undo_last();
        assert!(dir.join("a.txt").exists(), "then the one before it");

        browser.undo_last();
        assert_eq!(browser.status.as_deref(), Some("Nothing to undo"));
    }

    /// Selecting and navigating are not operations. Nothing about them may end
    /// up on the stack, or a Ctrl+Z meant for a delete would spend itself on a
    /// click instead.
    #[test]
    fn selecting_and_navigating_are_not_undoable() {
        let (mut browser, dir) = browser_over(&["a.txt"], &["sub"]);
        browser.select(0, 0);
        browser.select(0, 1);
        browser.clear_pane_selection(0);
        browser.navigate_to(&dir.join("sub"));

        assert!(browser.undo.is_empty(), "{:?}", browser.undo);
    }

    /// A click on nothing means nothing is selected — the way it does in every
    /// file manager. Run over all three views because each resolves the click
    /// through its own hit test.
    #[test]
    fn a_click_on_empty_space_clears_the_selection() {
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Columns] {
            let (mut browser, _dir) = browser_over(&["a.txt"], &[]);
            browser.mode = mode;
            browser.size = (900.0, 600.0);
            browser.sync_scroll_metrics();
            browser.select(0, 0);
            assert!(
                !browser.columns[0].selection.is_empty(),
                "{mode:?}: selected"
            );

            browser.clear_pane_selection(0);

            assert!(
                browser.columns[0].selection.is_empty(),
                "{mode:?}: the click on nothing left a selection behind"
            );
            assert_eq!(browser.columns[0].cursor, None, "{mode:?}: and no cursor");
        }
    }

    /// A browser in icon view, sized, with its scroll metrics current — what
    /// the marquee's geometry needs before it can be asked anything.
    fn grid_over(files: &[&str]) -> (Browser, TempDir) {
        let (mut browser, dir) = browser_over(files, &[]);
        browser.mode = ViewMode::Grid;
        browser.size = (900.0, 600.0);
        browser.sync_scroll_metrics();
        (browser, dir)
    }

    /// The middle of cell `index`, in window coordinates, in an unscrolled
    /// grid the size `grid_over` builds.
    fn cell_center(index: usize) -> (f32, f32) {
        let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
        let cell = view::grid_cell_rect(area, index, 0.0);
        (cell.center_x(), cell.center_y())
    }

    /// Dragging from empty space rubber-bands: everything the band touches is
    /// selected, and nothing else is.
    #[test]
    fn a_band_dragged_over_the_grid_selects_what_it_covers() {
        let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
        let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();

        let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
        browser.begin_marquee(0, area.right - 4.0, area.bottom - 4.0, false);
        let (x, y) = cell_center(1);
        browser.update_marquee(x, y);

        let selected = &browser.columns[0].selection;
        assert!(selected.contains(&names[1]), "the band caught the cell");
        assert!(
            !selected.contains(&names[0]),
            "and left the one it never reached: {selected:?}"
        );
    }

    /// The selection is recomputed from the band, not accumulated along the
    /// way, so pulling the band back off an entry deselects it again.
    #[test]
    fn a_band_pulled_back_gives_up_what_it_leaves() {
        let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
        let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();

        let (x0, y0) = cell_center(0);
        browser.begin_marquee(0, x0, y0, false);
        let (x2, y2) = cell_center(2);
        browser.update_marquee(x2, y2);
        assert_eq!(browser.columns[0].selection.len(), 3, "all three, sweeping");

        browser.update_marquee(x0 + 1.0, y0);

        assert_eq!(
            browser.columns[0].selection.iter().collect::<Vec<_>>(),
            vec![&names[0]],
            "only the one still under the band"
        );
    }

    /// Ctrl keeps what was selected and adds to it, the way Ctrl+click does.
    #[test]
    fn a_band_held_with_ctrl_adds_to_the_selection() {
        let (mut browser, _dir) = grid_over(&["a.txt", "b.txt", "c.txt"]);
        let names: Vec<String> = browser.visible(0).iter().map(|e| e.name.clone()).collect();
        browser.select(0, 0);

        let (x, y) = cell_center(2);
        browser.begin_marquee(0, x - view::CELL_W / 2.0 + 1.0, y, true);
        browser.update_marquee(x, y);

        let selected = &browser.columns[0].selection;
        assert!(selected.contains(&names[0]), "the earlier click survived");
        assert!(selected.contains(&names[2]), "and the band added to it");
    }

    /// A band the size of a point catches nothing — which is exactly what a
    /// plain click on empty space has to mean.
    #[test]
    fn a_band_that_never_travels_selects_nothing() {
        let (mut browser, _dir) = grid_over(&["a.txt"]);
        browser.select(0, 0);

        let area = view::content_viewport(900.0, 600.0, ViewMode::Grid);
        browser.clear_pane_selection(0);
        browser.begin_marquee(0, area.right - 4.0, area.bottom - 4.0, false);
        browser.update_marquee(area.right - 4.0, area.bottom - 4.0);

        assert!(browser.columns[0].selection.is_empty());
    }

    /// The band is anchored to the content, not to the screen: scrolling under
    /// it keeps it around the same files.
    #[test]
    fn a_band_stays_over_the_files_when_the_pane_scrolls() {
        // Enough cells that the pane has somewhere to scroll to: `set_offset`
        // clamps, and a short listing would silently stay at the top.
        let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (mut browser, _dir) = grid_over(&refs);
        let (x, y) = cell_center(0);
        browser.begin_marquee(0, x, y, false);

        let band = browser.marquee_band().expect("a band is out");
        browser.columns[0].scroll.state.set_offset(40.0);

        let scrolled = browser.marquee_band().expect("still out");
        assert_eq!(scrolled.top, band.top - 40.0, "the band scrolled with them");
    }

    #[test]
    fn moving_a_file_into_the_directory_it_is_already_in_does_nothing() {
        let (mut browser, dir) = browser_over(&["a.txt"], &[]);
        browser.drop_target = Some(DropTarget::Pane {
            depth: 0,
            path: dir.0.clone(),
        });

        browser.apply_drop(vec![dir.join("a.txt")], true);

        assert!(dir.join("a.txt").exists(), "the file is still there");
        assert!(
            !dir.join("a 2.txt").exists(),
            "and was not renamed out of the way of itself"
        );
        assert_eq!(browser.status, None, "a no-op reports nothing");
    }

    #[test]
    fn copying_a_file_into_the_directory_it_is_already_in_duplicates_it() {
        // The same gesture with copy asked for is how a duplicate is made, so
        // this one is not skipped.
        let (mut browser, dir) = browser_over(&["a.txt"], &[]);
        browser.drop_target = Some(DropTarget::Pane {
            depth: 0,
            path: dir.0.clone(),
        });

        browser.apply_drop(vec![dir.join("a.txt")], false);

        assert!(dir.join("a.txt").exists(), "the original is untouched");
        let copies: Vec<_> = std::fs::read_dir(&dir.0)
            .expect("listing")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "a.txt")
            .collect();
        assert_eq!(copies.len(), 1, "exactly one copy, got {copies:?}");
    }

    #[test]
    fn a_drop_moves_a_file_into_a_directory() {
        let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: row_of(&browser, "target"),
            path: dir.join("target"),
        });

        browser.apply_drop(vec![dir.join("a.txt")], true);

        assert!(dir.join("target/a.txt").exists(), "the file moved in");
        assert!(!dir.join("a.txt").exists(), "and left where it was");
    }

    #[test]
    fn a_drop_copy_leaves_the_original_alone() {
        let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: row_of(&browser, "target"),
            path: dir.join("target"),
        });

        browser.apply_drop(vec![dir.join("a.txt")], false);

        assert!(dir.join("target/a.txt").exists(), "the copy arrived");
        assert!(dir.join("a.txt").exists(), "the original stayed");
    }

    #[test]
    fn a_directory_cannot_be_dropped_into_itself() {
        let (mut browser, dir) = browser_over(&[], &["outer"]);
        std::fs::create_dir_all(dir.join("outer/inner")).expect("nested dir");
        browser.drop_target = Some(DropTarget::Pane {
            depth: 0,
            path: dir.join("outer/inner"),
        });

        browser.apply_drop(vec![dir.join("outer")], true);

        assert!(dir.join("outer").exists(), "nothing was moved");
        assert!(
            browser
                .status
                .as_deref()
                .is_some_and(|status| status.contains("itself")),
            "and it said so: {:?}",
            browser.status
        );
    }

    #[test]
    fn the_drop_target_clears_once_it_has_been_applied() {
        let (mut browser, dir) = browser_over(&["a.txt"], &["target"]);
        browser.drop_target = Some(DropTarget::Entry {
            depth: 0,
            index: row_of(&browser, "target"),
            path: dir.join("target"),
        });

        browser.apply_drop(vec![dir.join("a.txt")], true);

        assert_eq!(browser.drop_target, None, "the outline goes with the drop");
        assert!(browser.dirty, "and the window repaints");
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;

    /// A real directory, swept up on drop. Watching is about the filesystem
    /// changing, so these tests need one.
    struct TempDir(PathBuf);

    impl TempDir {
        fn holding(names: &[&str]) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let path = std::env::temp_dir().join(format!(
                "otto-files-watch-app-{}-{}",
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

    fn browser_over(names: &[&str]) -> (Browser, TempDir) {
        let dir = TempDir::holding(names);
        let mut browser = Browser::new(dir.0.clone());
        browser.mode = ViewMode::List;
        assert!(settle(&mut browser, |b| !b.loading()));
        (browser, dir)
    }

    fn at_cursor(browser: &Browser) -> Option<String> {
        let index = browser.columns[browser.active].cursor?;
        Some(browser.visible(browser.active)[index].name.clone())
    }

    /// Drive the browser's own poll until `done`, or give up. This is the
    /// frame loop's job in a running window; a test has to do it by hand, and
    /// a watch-driven refresh takes a debounce plus a worker read to land.
    fn settle(browser: &mut Browser, done: impl Fn(&Browser) -> bool) -> bool {
        for _ in 0..400 {
            browser.poll();
            if done(browser) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// The whole point of watching: a file another application creates shows
    /// up without the user asking for a refresh.
    #[test]
    fn a_file_created_underneath_appears_on_its_own() {
        let (mut browser, dir) = browser_over(&["one.txt"]);
        std::fs::write(dir.0.join("two.txt"), b"x").expect("write");

        let landed = settle(&mut browser, |b| {
            b.visible(0).iter().any(|e| e.name == "two.txt")
        });
        assert!(landed, "the new file never appeared");
    }

    /// A refresh must not disturb what the user is doing: the selection is
    /// held by name and survives, and the cursor is put back on it even though
    /// the new file sorts above it and moved every index down one.
    #[test]
    fn a_refresh_keeps_the_selection_and_the_cursor_together() {
        let (mut browser, dir) = browser_over(&["m.txt", "z.txt"]);
        browser.mode = ViewMode::List;
        let index = browser
            .visible(0)
            .iter()
            .position(|e| e.name == "z.txt")
            .expect("listed");
        browser.select(0, index);

        std::fs::write(dir.0.join("a.txt"), b"x").expect("write");
        let landed = settle(&mut browser, |b| {
            b.visible(0).iter().any(|e| e.name == "a.txt")
        });
        assert!(landed, "the new file never appeared");

        assert!(browser.columns[0].selection.contains("z.txt"));
        assert_eq!(at_cursor(&browser).as_deref(), Some("z.txt"));
    }

    /// A refresh must not move the view. The listing is replaced in place,
    /// so the scroll view — its offset and its measurements — is never rebuilt
    /// out from under whoever is reading it.
    #[test]
    fn a_refresh_leaves_the_scroll_where_it_was() {
        let names: Vec<String> = (0..200).map(|i| format!("f{i:03}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (mut browser, dir) = browser_over(&refs);

        // Measurements first: an offset means nothing against a pane that has
        // never been sized, and would be clamped straight back to zero.
        let column = &mut browser.columns[0];
        column
            .scroll
            .state
            .set_viewport(Rect::from_xywh(0.0, 0.0, 600.0, 400.0));
        column.scroll.state.set_content_length(4000.0);
        column.scroll.state.set_offset(750.0);
        let before = column.scroll.offset();
        assert!(before > 0.0);

        std::fs::write(dir.0.join("aaa.txt"), b"x").expect("write");
        let landed = settle(&mut browser, |b| {
            b.visible(0).iter().any(|e| e.name == "aaa.txt")
        });
        assert!(landed, "the new file never appeared");
        assert_eq!(browser.columns[0].scroll.offset(), before);
    }

    /// A directory that goes away takes its pane with it: the window lands on
    /// the nearest place that still exists rather than showing a listing of
    /// something that is not there.
    #[test]
    fn losing_the_directory_falls_back_to_an_ancestor() {
        let dir = TempDir::holding(&[]);
        let child = dir.0.join("sub");
        std::fs::create_dir_all(&child).expect("child dir");

        let mut browser = Browser::new(child.clone());
        assert!(settle(&mut browser, |b| !b.loading()));

        std::fs::remove_dir_all(&child).expect("remove");
        let moved = settle(&mut browser, |b| b.current_path() != child);
        assert!(moved, "the window stayed on a directory that is gone");
        assert_eq!(browser.current_path(), dir.0);
    }
}
