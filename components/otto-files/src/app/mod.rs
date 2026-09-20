use crate::{
    command, model, ocrcache, palette, pane_surfaces, peek, perf, picker, remembered, rename,
    scene, scripts, thumbcache, thumbnails, view,
};

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use otto_kit::accessibility::{A11yTree, Action, ActionRequest, Role};
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

use listing_pointer::{After, DragStart, MenuAt};
use model::{Column, Entry, Place, SortKey};
use view::ViewMode;

mod construct;
mod cursor;
mod drag_drop;
mod file_ops;
mod frame;
mod info;
mod input;
mod keys;
mod layout;
mod lifecycle;
mod listing;
mod listing_pointer;
mod menus;
mod navigation;
mod ocr;
mod opening;
mod palette_session;
mod peek_session;
mod picking;
mod pointer;
mod present;
mod preview;
mod refresh;
mod renaming;
mod scroll;
mod searching;
mod selection;

/// Reading text out of pictures from the command line, with no window.
pub use ocr::recognise_paths;

/// What a drag carries: the picture to draw, the size of the surface it needs,
/// and where inside that surface the grab happened.
type DragItems = (DragPicture, (f32, f32), (f32, f32));

/// A drawing handed over as a value: the preview's picture, built by the view
/// and replayed onto the drag surface.
type BoxedPainter = Box<dyn Fn(&skia_safe::Canvas, f32, f32) + Send + Sync>;

/// The picture a drag lifts off the window.
///
/// Which one it is follows what the user was looking at when they pressed:
/// rows out of a listing, the preview's own picture out of the preview column.
enum DragPicture {
    /// Entries lifted out of a listing, each drawn where it sat.
    Entries(Vec<view::DragItem>),
    /// The preview column's picture, lifted whole. Drawn by a closure the
    /// view builds, because what it paints depends on which decoder answered.
    Preview(BoxedPainter),
}

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

/// Files went away — a move to the Trash, or an undo that took a copy back.
///
/// `file-trash` is the naming spec's own name for this and comes first even
/// though no theme here ships it: a theme that does should win, and the
/// fallbacks are what cover the ones that do not. Deliberately not
/// `trash-empty`, which the spec reserves for the bin actually being emptied.
const SOUND_REMOVED: [&str; 3] = ["file-trash", "item-deleted", "device-removed"];

/// Files were destroyed — the Trash emptied, or a Delete Forever.
const SOUND_DESTROYED: [&str; 3] = ["trash-empty", "item-deleted", "device-removed"];

/// Files came back out of the Trash.
///
/// Its own sound rather than the arriving one: a put-back is the answer to a
/// delete, and hearing the delete undone is worth more than hearing it as one
/// more paste. The naming spec has nothing for it, so this is borrowed.
const SOUND_RESTORED: [&str; 2] = ["complete", "device-added"];

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
/// A file operation running on a worker thread.
///
/// The window keeps a handle to it so it can show where it has got to, stop
/// it, and take its outcome when it lands. The work itself is on the worker:
/// nothing here touches the disk.
struct Job {
    /// What the worker has said so far, drained in `poll`.
    updates: std::sync::mpsc::Receiver<JobUpdate>,
    /// Set to ask the worker to stop. It is read between items, so stopping
    /// never leaves half a file behind.
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// What the undo step this job leaves behind is called.
    undo_label: &'static str,
    /// Whether the clipboard is spent when this finishes — a cut is consumed
    /// by its paste, a copy is not.
    cut: bool,
}

/// One thing a running job has to say for itself.
enum JobUpdate {
    /// `done` of `total` items handled; `item` is the one it is on now.
    Progress {
        done: usize,
        total: usize,
        item: String,
    },
    /// It is over, and this is what it did.
    Done(model::OpResult),
}

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
    /// Whether the user has picked a sort themselves. Until they do, switching
    /// view takes that view's own default — list is a "what did I touch last"
    /// view, so it opens newest-first — and once they have, their choice
    /// follows them between views instead of being overwritten.
    sort_pinned: bool,
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
    /// The file operation running on a worker thread, if there is one. See
    /// [`Job`]: one at a time, and the window stays usable while it runs.
    job: Option<Job>,
    /// Operations that changed files, newest last. Ctrl+Z pops one and puts
    /// it back; see [`UndoStep`] for what does and does not go on here.
    undo: Vec<UndoStep>,
    /// The open preview, if one is up.
    peek: Option<peek::Session>,
    /// The docked preview column's state, for the entry currently under a
    /// single-item selection. `None` both when the column is hidden (no
    /// selection, or not enough room for it) and briefly while a fresh
    /// selection's decode is still in flight — [`PreviewPaneState::pending`]
    /// tells the two apart.
    preview: Option<PreviewPaneState>,
    /// Bumped for every preview decode started, independent of Peek's
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
    peek_pending: bool,
    /// A dismissed preview still on screen, shrinking back to its file.
    peek_closing: Option<peek::Session>,
    /// Open Peek on the first entry as soon as one is listed, so the
    /// panel can be looked at without anyone pressing a key. Driving the real
    /// keyboard means injecting into whatever session the test runs in, which
    /// is both unreliable and rude to whoever is using that desktop.
    /// `OTTO_FILES_QV_AUTO=1`.
    peek_auto: bool,
    /// Open the command palette as soon as the window has a listing, so it can
    /// be looked at without anyone pressing a key. Driving the real keyboard
    /// means injecting into whatever session the test runs in, which is both
    /// unreliable and rude to whoever is using that desktop.
    /// `OTTO_FILES_PALETTE_AUTO=1`, optionally `=<query>` to type one in too.
    palette_auto: Option<String>,
    /// Bumped for every decode started. A result arriving with a stale
    /// generation is dropped: arrow-keying is much faster than decoding, and a
    /// slow PDF must not land on top of a file the user moved off three keys
    /// ago.
    peek_generation: u64,
    /// Whether the recogniser is running on the previewed picture. Keeps
    /// the frame loop alive so the words paint when they land.
    peek_recognising: bool,
    /// The pages of the open document whose pixels have been asked for and
    /// have not arrived. A document scrolls with two pages of images held, so
    /// the rest are fetched as they come into view — and each one only once,
    /// however many frames go by before it lands.
    peek_pages_pending: std::collections::HashSet<u32>,
    /// Whether the open document's text layer has been asked for. One pass
    /// per document: it reads the whole file, so a second would be the same
    /// work for the same answer.
    peek_text_asked: bool,
    /// Whether the cursor is the text beam because the pointer is over a
    /// recognised word on the panel. Tracked so the shape is set on the
    /// crossing rather than on every motion event.
    peek_text_cursor: bool,
    /// Pictures the background recognition pass has already considered in
    /// this window, whatever it decided.
    ocr_seen: std::collections::HashSet<PathBuf>,
    /// Pictures somebody asked to have read, in the order they were asked
    /// for. Drained ahead of the background pass's own scan and read whatever
    /// is already remembered about them: the command exists because the
    /// remembered answer is the one that is wrong.
    ocr_queue: std::collections::VecDeque<PathBuf>,
    /// The pictures a recogniser is on right now. A set rather than a flag
    /// because the panel's own recognition and the background pass can
    /// overlap, and one of them finishing must not speak for the other.
    ocr_reading: std::collections::HashSet<PathBuf>,
    /// The cursor moved on its own — a delete landing its selection on the
    /// survivor — rather than through an arrow key. Peek follows the
    /// cursor, so it has to re-decode for those moves too, and the deleted
    /// file's preview must not be left up over a file that no longer exists.
    /// Drained by the host, which owns the decode; see [`FilesApp::follow_peek`].
    peek_follow: bool,
    /// This window is the Trash: one flat listing of the trash can, with Put
    /// Back and Empty Trash in place of the view switcher, and every command
    /// that would move, rename or open a file suppressed.
    ///
    /// A shell, not a place. The browser reaching the same directory through
    /// a path would still be the browser; this is set once, at startup, by
    /// the entry point that opened the window.
    trash: bool,
    /// This window is showing the Recent listing rather than a directory:
    /// what was written most recently across the user's folders, newest first,
    /// under a heading per day.
    ///
    /// Unlike [`Self::trash`] this is a *mode*, not a shell — the same window
    /// switches into it from the sidebar and back out again by clicking any
    /// other place.
    ///
    /// The listing is real — [`crate::search`] fills it — but it has no
    /// directory behind it, so every command that acts on a *place* is refused
    /// while it is set. See [`Self::is_synthetic`].
    recent: bool,
    /// The search field's text, when it has been opened with Ctrl+F. `None`
    /// is the whole of "there is no search": the filter strip is not drawn and
    /// the listing sits back up against the header.
    search: Option<TextInput>,
    /// Where the search was started from, so clearing it puts the listing back
    /// rather than leaving the window somewhere it never navigated to.
    search_origin: Option<PathBuf>,
    /// That listing's name, for the field's placeholder — "Filter Documents"
    /// says what these rows are, which "Search" never did.
    search_where: String,
    /// Which scope pill is lit.
    search_scope: model::SearchScope,
    /// This window is showing search results rather than a directory.
    searching: bool,
    /// The recent listing's day sections, rebuilt whenever the listing is.
    /// Empty — a flat grid — whenever [`Self::recent`] is false.
    recent_sections: view::GridSections,
    /// Which of the Trash header's two buttons is held down.
    trash_pressed: Option<view::TrashAction>,
    /// The Get Info panel, when one is open. Not modal: it is a window of its
    /// own that floats over the browser, and the browser goes on working
    /// underneath it.
    info: Option<model::FileInfo>,
    /// Why the last permission change was refused, if it was.
    info_error: Option<String>,
    /// What the panel says about the words in the picture it is showing.
    /// Held rather than read at draw time: the answer comes off the disk, and
    /// it only changes when a recogniser starts or finishes.
    info_text: Option<ocrcache::Status>,
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
    /// A folder just created, waiting to be selected and put into rename.
    ///
    /// The listing is read off-thread, so the new directory is not in the
    /// column's snapshot yet when `new_folder` returns — the pane and the name
    /// are held here until the re-read lands.
    pending_rename: Option<(usize, String)>,
    /// The command palette, open on Ctrl+P. While it is up it takes the
    /// keyboard whole; the browser underneath is untouched until a command
    /// actually runs. See `specs/file-command-palette.md`.
    palette: Option<palette::Palette>,
    /// What was selected when the palette opened: the pane, its selection, its
    /// cursor and its anchor.
    ///
    /// A previewed argument selects as it is typed, so there has to be
    /// something to put back when the palette is abandoned — and something to
    /// start each keystroke's answer from, since a pattern narrowed by one
    /// character is a fresh question rather than a refinement of the last one.
    palette_selection: Option<SelectionMark>,
    /// How far the palette has been dragged from where it opens, in points.
    /// Reset every time it opens: a panel that came back where it was left
    /// would be a placement the user has to undo before they can read the
    /// window under it.
    palette_offset: (f32, f32),
    /// When the caret was last advanced, so the blink runs on real time.
    /// The update loop runs as often as anything asks it to — a surface
    /// repainting, a pointer callback waking it — and a blink that counted
    /// passes rather than seconds sped up whenever the loop did, which read
    /// as a nervous caret in the palette.
    caret_clock: Option<std::time::Instant>,
    /// The palette's list, scrolling under its field: momentum, the band past
    /// either end and the fading bar, the same view the columns run on. Owned
    /// here rather than by the palette because the palette is I/O-free and
    /// clock-free, and a scroll view keeps time.
    palette_scroll: ScrollView,
    /// Where the palette was last left, as an offset from where it opens, so
    /// the next open finds it there. Kept for the session always; written to
    /// the state file as well once the host has handed one over.
    palette_memory: Option<(f32, f32)>,
    /// Whether the remembered offset is also kept on disk — set by the host,
    /// never by a test.
    palette_memory_on_disk: bool,
    /// The display the palette may be dragged around, in window points, as the
    /// compositor last answered. `None` until it has, and the drag then falls
    /// back to the window's own edges.
    palette_display: Option<Rect>,
    /// Where the palette's caret is, in window points, when the card is on a
    /// surface of its own and so is painted outside the window's draw.
    palette_caret: Option<(f32, f32, f32, f32)>,
    /// A drag of the palette in progress: where in the card the pointer took
    /// hold, so the card follows the pointer without jumping to centre on it.
    palette_drag: Option<(f32, f32)>,
    /// A palette command asked for Peek. The panel and its decode belong
    /// to the window around the browser, so the request is left here for it.
    palette_peek: bool,
    /// Where commands come from. The built-ins today, and the seam a later
    /// extension system hangs off — see [`crate::command`].
    commands: command::Registry,
    /// The path entry, open on Ctrl+L. While it is up the header's title is
    /// replaced by the field, and every key belongs to it — it is where you
    /// are, made editable, rather than a dialog asking where to go.
    path_entry: Option<TextInput>,
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
    /// Ask the Empty Trash question as soon as there is a listing to ask it
    /// about. Set by `--empty-trash`, whose window opens on a directory that
    /// has not been read yet — and the question carries the count.
    pending_empty_ask: bool,
    /// A row to land the selection on once the reload that removed the old one
    /// has landed: which pane, and the [`Entry::selection_key`] to look for.
    /// Set by a delete —
    /// the successor is chosen from the listing that is still on screen, and
    /// acted on against the one that replaces it.
    pending_pick: Option<(usize, Option<String>)>,
    /// A pane the keyboard stepped into before its listing had arrived, and
    /// which should take the cursor on its first row as soon as it does.
    ///
    /// Pressing Right on a folder makes its column the active one and puts the
    /// cursor on the first entry — but the column is read on a worker, and a
    /// folder reached from a file's selection has had no head start at all, so
    /// the press usually beats the read. Without this the pane arrives with no
    /// cursor in it, and the next arrow press has nothing to move.
    entering: Option<usize>,
    /// Locations left behind by Back, most recent last. Forward pops them back.
    back: Vec<Location>,
    /// Locations left behind by Forward, most recent last. Back pops them back.
    forward: Vec<Location>,
    /// Set when something changed and the window needs repainting.
    dirty: bool,
    /// Set when a scroll view moved under the wheel or the touchpad. Kept
    /// apart from `dirty` because a frame that only scrolled repaints — and
    /// reports — the file area alone.
    scroll_moved: bool,
    /// Something changed inside the file area and nowhere else — a thumbnail
    /// landing in the list or the grid — so a repaint for it reports that
    /// area alone, like a scroll.
    listing_dirty: bool,
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
    /// The path bar crumb under the pointer, drawn lit.
    path_crumb_hover: Option<usize>,
    /// Which sidebar row was last clicked.
    ///
    /// The highlight cannot be worked out from the path alone: two rows may
    /// name the same folder — a configured shortcut pointing at Downloads, or
    /// two of them pointing at one place — and matching on the path lights
    /// whichever comes first, so the row actually clicked stays dark and
    /// reads as not having worked. Remembering the row is the only way to
    /// light the one that was pressed.
    active_place: Option<usize>,
    footer_pressed: Option<view::FooterButton>,
    /// Pointer is over Peek's close button.
    peek_close_hovered: bool,
    /// Pointer is over Peek's expand button.
    peek_expand_hovered: bool,
    /// Where Peek's panel actually is, in window coordinates.
    ///
    /// Written by the render path, read by the pointer handler, because the
    /// two cannot otherwise agree: a panel centred on the *display* is placed
    /// from an answer only [`pane_surfaces`] has, and the pointer callback
    /// outlives any borrow of it. `None` falls back to the window's centre,
    /// which is where the panel is when it is not centred on the display.
    peek_panel: Option<Rect>,
    /// The pointer's last position over the Peek panel, together with
    /// the panel rect it was measured against.
    ///
    /// The two are stored as a pair because they are not always in the same
    /// space: the panel takes its own input when it is centred on the display
    /// and the toplevel takes it otherwise, and those two report positions in
    /// two different coordinate systems. Whichever handler saw the pointer
    /// records the panel *it* was hit-testing against, so a pinch can work in
    /// that space without having to know which handler it came from.
    peek_focus: Option<(skia_safe::Point, Rect)>,
    /// A drag of the panel by its title strip in progress: the point the
    /// press was reported at, and where the card was in the window then.
    ///
    /// Both are fixed at the press, because a pointer holding a button keeps
    /// the focus it was grabbed with: its positions go on being reported in
    /// the frame the press landed in, whichever of the two doors that was —
    /// the panel's own surface, or the toplevel. See
    /// [`Browser::drag_peek_to`].
    peek_drag: Option<((f32, f32), (f32, f32))>,
    /// When the panel's title strip was last pressed, for the double-click
    /// that fills the display.
    last_peek_title_click: Option<std::time::Instant>,
    /// The offset [`Browser::peek_panel`] was placed with, so a drag can
    /// tell where the card would rest untouched from where it actually is.
    /// Written by the render path beside the rect itself, for the same reason.
    peek_placed_offset: Option<(f32, f32)>,
    /// The display the panel may be dragged around, in window points, as the
    /// compositor last answered. `None` until it has; the window stands in
    /// for it until then, which is only wrong in being too strict.
    peek_display: Option<Rect>,
    /// The zoom a pinch in progress started from, if one is.
    ///
    /// `zwp_pointer_gesture_pinch_v1` reports its scale against the start of
    /// the gesture rather than against the last update, so the zoom it is
    /// asking for is this times that — and applying it incrementally instead
    /// would compound rounding across a gesture that can run for seconds.
    peek_pinch: Option<f32>,
}

/// A pane's selection as it stood at some moment, so it can be put back.
///
/// Held while the palette is open: a previewed argument selects as it is
/// typed, and every keystroke answers afresh from here rather than refining
/// the last answer.
#[derive(Clone)]
struct SelectionMark {
    depth: usize,
    selection: std::collections::BTreeSet<String>,
    cursor: Option<usize>,
    anchor: Option<usize>,
}

/// One of the palette's rows, with its text owned.
///
/// The view draws from borrowed strings, and a row's text is assembled from
/// several places — a command's title, its group's label, an argument's name.
/// Gathering it here keeps the draw closure from having to hold the palette
/// borrowed while it paints.
pub struct PaletteRowData {
    pub kind: view::PaletteRowKind,
    pub title: String,
    pub badge: Option<String>,
    pub subtitle: Option<String>,
    pub shortcut: Option<String>,
    pub highlighted: bool,
}

/// What the host window still has to do after a palette command ran.
///
/// Almost everything a command does is the browser's own; Peek is not —
/// the panel and its decode belong to the window around the browser, so that
/// one command is handed back rather than run in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Followup {
    Nothing,
    Peek,
}

/// How many path completions the palette is offered at once. A directory can
/// hold thousands, and a list nobody will scroll costs a syscall each to
/// stat — see [`Browser::path_completions`].
const PALETTE_COMPLETION_LIMIT: usize = 64;

/// The stable id a view mode is named by across the command seam. Not the
/// label: a label is localised, and an id is what a request carries.
fn view_id(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "list",
        ViewMode::Grid => "grid",
        ViewMode::Columns => "columns",
    }
}

fn view_from_id(id: &str) -> Option<ViewMode> {
    match id {
        "list" => Some(ViewMode::List),
        "grid" => Some(ViewMode::Grid),
        "columns" => Some(ViewMode::Columns),
        _ => None,
    }
}

fn sort_id(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::Kind => "kind",
        SortKey::Modified => "modified",
    }
}

fn sort_from_id(id: &str) -> Option<SortKey> {
    match id {
        "name" => Some(SortKey::Name),
        "size" => Some(SortKey::Size),
        "kind" => Some(SortKey::Kind),
        "modified" => Some(SortKey::Modified),
        _ => None,
    }
}

/// What a pointer event over the Peek panel is, as far as the pan's
/// scrollbars are concerned. The two handlers that can deliver one — the
/// toplevel's and the panel's own surface — funnel into
/// [`Browser::peek_pan_pointer`] through this, so a bar behaves the same
/// whichever of them the compositor picked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PeekPointer {
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
    /// What the window was showing, when that was not a folder.
    ///
    /// Recent and search results are listings with no directory behind them,
    /// and the sentinel path standing in for one is not somewhere that can be
    /// re-read. Remembering the *listing* rather than its stand-in is what
    /// lets Back and Forward pass through them: without it, stepping back into
    /// one opened `/dev/null/otto-search` and drew the error of failing to.
    synthetic: Option<Synthetic>,
}

/// A listing with no folder behind it, as the history remembers it.
enum Synthetic {
    Recent,
    /// Everything needed to ask the question again — the answer is not stored,
    /// because a search stepped back into should say what is on the disk now
    /// rather than replay what it said before.
    Search {
        query: String,
        scope: model::SearchScope,
        origin: Option<PathBuf>,
        label: String,
    },
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
    /// A player, when the decode said video. Opened paused: the column
    /// follows the selection, and a video that started playing on every
    /// arrow key would be a column that talks.
    video: Option<peek::Video>,
    /// What the caption says about the words in the picture. Held rather than
    /// read while drawing: the answer comes off the disk, and the caption is
    /// rebuilt every frame.
    text: Option<ocrcache::Status>,
}

/// An in-place rename in progress: which row it belongs to and the text
/// field editing its name.
struct RenameSession {
    depth: usize,
    index: usize,
    original: PathBuf,
    input: TextInput,
}

/// How often the window wakes itself while something is moving: a scroll
/// fling, a material fading, or a caret blinking.
const IDLE_TICK: std::time::Duration = std::time::Duration::from_millis(8);

/// A focused field's caret as `(x, y, width, height)` in window coordinates,
/// given where the field itself was drawn. `None` for a field that does not
/// hold the keyboard — its caret is not on screen and is nobody's business.
///
/// Window coordinates are surface coordinates for this application: the
/// toolkit sets a window geometry whose origin is the surface's, so there is
/// no decoration offset to take back off before handing this to
/// [`AppContext::report_text_cursor`](otto_kit::AppContext::report_text_cursor).
fn caret_in_window(input: &TextInput, origin: (f32, f32)) -> Option<(f32, f32, f32, f32)> {
    input.state.focused().then(|| {
        let caret = input.caret_rect();
        (
            origin.0 + caret.left,
            origin.1 + caret.top,
            caret.width(),
            caret.height(),
        )
    })
}

/// The uncached half of [`Browser::save_action`]: three syscalls, and the
/// only place in the picker that touches the filesystem on the UI thread.
fn probe_save_action(dir: &Path, name: &str) -> picker::SaveAction {
    if !picker::is_writable_dir(dir) {
        return picker::SaveAction::Blocked("files-save-permission-denied");
    }
    picker::save_action(name, existing_kind(&dir.join(name.trim())))
}

/// `target` broken into the crumbs the path bar draws, root first.
///
/// Split out from the browser so the naming rules — which are the whole of
/// the interesting part — can be tested without a window.
fn crumbs_for(
    target: &Path,
    leaf_icon: Vec<String>,
    leaf_is_dir: bool,
    home: Option<&Path>,
) -> Vec<view::PathCrumb> {
    let components: Vec<_> = target.components().collect();
    let mut path = PathBuf::new();
    let mut crumbs = Vec::with_capacity(components.len());

    for (index, component) in components.iter().enumerate() {
        path.push(component.as_os_str());
        let leaf = index + 1 == components.len();

        let (label, icon) = if home == Some(path.as_path()) {
            // The home directory keeps its name on disk — the header titles
            // it that way too, and a bar that reads "/ › home › Home" spends
            // a crumb saying the same word twice. What it gets instead is the
            // sidebar's icon, which is the part that makes it findable.
            (
                component.as_os_str().to_string_lossy().into_owned(),
                vec!["user-home".to_string()],
            )
        } else if matches!(component, std::path::Component::RootDir) {
            // The volume the whole trail hangs off. Named by its one
            // character rather than by a word: every other crumb is what the
            // thing is called on disk, and inventing a name here — a
            // hostname, "Computer" — would be the only guess in the row.
            ("/".to_string(), vec!["drive-harddisk".to_string()])
        } else {
            let name = component.as_os_str().to_string_lossy().into_owned();
            if leaf {
                (name, leaf_icon.clone())
            } else {
                (
                    name,
                    vec!["folder".to_string(), "inode-directory".to_string()],
                )
            }
        };

        crumbs.push(view::PathCrumb {
            label,
            icon,
            path: path.clone(),
            // Everything above the leaf is a directory by construction; the
            // leaf is one only when what is selected is a folder.
            is_dir: !leaf || leaf_is_dir,
        });
    }
    crumbs
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
    /// The affirmative button's words. Spelled for the question rather than
    /// fixed, since the sheet now asks three of them.
    accept_label: String,
    /// What accepting does.
    action: ConfirmAction,
    pressed: Option<view::ConfirmButton>,
}

/// What the sheet's affirmative button carries out.
///
/// Each holds the paths it is about to act on rather than a promise to work
/// them out again: between the sheet appearing and the button being pressed
/// the directory may change underneath, and acting on what the user was
/// actually shown is the honest thing.
enum ConfirmAction {
    /// Answer the picker's save request with these paths. Creates nothing —
    /// the application does the writing.
    Answer(Vec<PathBuf>),
    /// Destroy these trashed items outright.
    DeleteForever(Vec<PathBuf>),
    /// Destroy everything in the Trash.
    EmptyTrash,
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

// ---------------------------------------------------------------------------
// App shell
// ---------------------------------------------------------------------------

/// The listing's identity for assistive technologies.
const FILES_LIST: otto_kit::focus::FocusId = otto_kit::focus::FocusId::from_raw(0xF11E_5000);
/// The line shown in place of a listing that could not be read.
const FILES_STATUS: otto_kit::focus::FocusId = otto_kit::focus::FocusId::from_raw(0xF11E_5001);

/// The column view's preview of the selected file.
const PREVIEW_PANE: otto_kit::focus::FocusId = otto_kit::focus::FocusId::from_raw(0xF11E_5003);

/// The preview panel's, when one is open.
const PEEK: otto_kit::focus::FocusId = otto_kit::focus::FocusId::from_raw(0xF11E_5002);

/// The longest prefix `names` all share, in whole characters. Empty when
/// they diverge at the first one — which the caller reads as "nothing more
/// to add", not as an error.
fn common_prefix(names: &[String]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let mut prefix = String::new();
    for (index, ch) in first.char_indices() {
        let candidate = &first[..index + ch.len_utf8()];
        if rest.iter().all(|name| name.starts_with(candidate)) {
            prefix = candidate.to_string();
        } else {
            break;
        }
    }
    prefix
}

/// One listing row's, by its position in the visible order.
fn row_focus(index: usize) -> otto_kit::focus::FocusId {
    otto_kit::focus::FocusId::new(format!("entry-{index}"))
}

struct FilesApp {
    window: Option<Window>,
    state: Arc<Mutex<Browser>>,
    /// The panel materials' fade, which the scene runs and this drains: the
    /// blur it wants switched, and whether it is still running. Held here
    /// because the window is — see `scene::FrostState`.
    frost: Option<Arc<scene::FrostState>>,
    /// The window's opaque region as last declared, so it is only sent again
    /// when the file area actually moves.
    opaque_region: Option<Rect>,
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
    /// Peek's surface and its card's rect within it, published by the
    /// render path for the pointer callback below. See
    /// [`pane_surfaces::PaneSurfaces::peek_target`].
    peek_target: Arc<Mutex<Option<(wayland_client::backend::ObjectId, Rect)>>>,
    /// The palette's surface and where it sits in window points, for the same
    /// reason Peek has one: dragged clear of the window, the card is
    /// over pixels the toplevel is never told about.
    palette_target: Arc<Mutex<Option<(wayland_client::backend::ObjectId, Rect)>>>,
    /// The picker's request queue, when this process is serving
    /// `org.otto.FilePicker1`. `None` in the browser.
    picker_queue: Option<crate::dbus::SharedQueue>,
    /// The surfaces the window hangs over itself: each column's scroll pane,
    /// the stack's bar, Peek, the palette and the preview's player.
    /// `None` until the window exists.
    pane_surfaces: Option<pane_surfaces::PaneSurfaces>,
    /// The Get Info panel's window, while one is open.
    ///
    /// Shared with the pointer callback, which is registered once at startup
    /// and looks the current window up rather than being re-registered for
    /// each panel: callbacks cannot be taken off again, so registering one
    /// per opening would pile them up for the life of the process.
    info_window: Rc<RefCell<Option<Window>>>,
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
///
/// A picture gets one line the listing cannot give it: how large it actually
/// is, and for an animation how long a loop runs. That comes from `decoded`
/// rather than from the file's name or its bytes here, because the decoder is
/// the only thing that has read the picture — which is also why the line
/// appears when the decode lands rather than with the rest.
fn preview_info(
    entry: &Entry,
    decoded: Option<&otto_kit::preview::Preview>,
    text: Option<ocrcache::Status>,
) -> Vec<String> {
    let mut info = vec![entry.kind_label().to_string()];
    if let Some(line) = decoded.and_then(picture_info) {
        info.push(line);
    }
    if let Some(size) = entry.size.filter(|_| !entry.is_dir) {
        info.push(model::format_size(size));
    }
    if let Some(modified) = entry.modified {
        info.push(model::format_time(modified));
    }
    // Last, under the dates: a picture always has this line once there is a
    // recogniser to read it, so the caption keeps its height as the answer
    // changes and the picture above it does not jump.
    if let Some(text) = text {
        info.push(view::describe_text_status(text));
    }
    info
}

/// The picture's own line: its size in pixels, and the length of one loop
/// when it is an animation. `None` for everything that is not a picture.
///
/// The *source's* size, not the decode's: a preview is decoded at the size it
/// will be shown, and "450 × 281" would describe this column rather than the
/// file.
fn picture_info(decoded: &otto_kit::preview::Preview) -> Option<String> {
    let pixels = match decoded {
        otto_kit::preview::Preview::Pixels { pixels, .. } => pixels,
        otto_kit::preview::Preview::Card { hero, .. } => hero.as_ref()?,
        _ => return None,
    };
    if pixels.intrinsic_width == 0 || pixels.intrinsic_height == 0 {
        return None;
    }
    let (width, height) = (
        pixels.intrinsic_width as f64,
        pixels.intrinsic_height as f64,
    );
    if !pixels.is_animated() {
        return Some(otto_kit::t_owned!(
            "files-preview-dimensions",
            width = width,
            height = height
        ));
    }
    let loop_length: std::time::Duration =
        (0..pixels.frames()).map(|frame| pixels.delay(frame)).sum();
    Some(otto_kit::t_owned!(
        "files-preview-animation",
        width = width,
        height = height,
        duration = clock(loop_length)
    ))
}

/// A duration as minutes and seconds, the way the video transport writes one.
fn clock(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The desktop entry this window belongs to, which is also its `app_id`.
fn app_id() -> &'static str {
    match view::shell() {
        view::Shell::Browser => "otto-files",
        view::Shell::Trash => "otto-trash",
    }
}

/// Open a browser window at `start` and run until it is closed.
pub fn run_browser(start: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut browser = Browser::new(start);
    browser.remember(&remembered::Remembered::load());
    run_app(browser, None)
}

/// Open the Trash window and run until it is closed.
///
/// The third shell over this view layer, beside the browser and the picker —
/// its own window and its own entry in the applications list, out of the same
/// binary, because a listing of the trash is a listing.
pub fn run_trash() -> Result<(), Box<dyn std::error::Error>> {
    run_app(Browser::for_trash(), None)
}

/// The Trash window with the Empty Trash question already up.
///
/// What the dock's *Empty Trash* runs. It is the window rather than a bare
/// confirmation because the question is about a listing, and showing what is
/// being destroyed is the point of asking; the operation, the wording and the
/// count are the window's own, so the two routes cannot drift.
pub fn run_empty_trash() -> Result<(), Box<dyn std::error::Error>> {
    let mut browser = Browser::for_trash();
    browser.pending_empty_ask = true;
    run_app(browser, None)
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

    // Resolve the file-operation sounds off the main thread now, so the first
    // delete of the session does not pay for a walk of every theme directory
    // at the moment it wants to make a noise.
    let sounds: Vec<&str> = SOUND_DESTROYED
        .iter()
        .chain(SOUND_REMOVED.iter())
        .chain(SOUND_RESTORED.iter())
        .chain(SOUND_ARRIVED.iter())
        .copied()
        .collect();
    otto_kit::sound::prewarm(&sounds);

    let app = FilesApp {
        pane_surfaces: None,
        window: None,
        info_window: Rc::new(RefCell::new(None)),
        state: Arc::clone(&state),
        frost: None,
        opaque_region: None,
        modifiers: Arc::new(Mutex::new(Modifiers::default())),
        context_menu: None,
        peek_target: Arc::new(Mutex::new(None)),
        palette_target: Arc::new(Mutex::new(None)),
        picker_queue,
    };

    AppRunner::new(app).run()
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod sort_default_tests;

#[cfg(test)]
mod rename_tests;

#[cfg(test)]
mod typeahead_tests;

#[cfg(test)]
/// The command palette — see `specs/file-command-palette.md`.
mod palette_tests;

#[cfg(test)]
mod dnd_tests;

#[cfg(test)]
mod watch_tests;

/// Where the selection goes when what held it is deleted.
#[cfg(test)]
mod delete_tests;

/// The path entry: Ctrl+L, tab completion, and where Return lands.
#[cfg(test)]
mod path_entry_tests;

/// The Trash window: its own shell over the browser's view layer.
///
/// These build the listing with [`Browser::listing_the_trash`] rather than
/// `for_trash`, so the process-wide chrome switch is left alone — the trash
/// can itself is already shared with every other test in this binary, and
/// flipping the sidebar out from under the geometry tests would be one shared
/// thing too many. Nothing here empties the can for the same reason.
#[cfg(test)]
mod trash_tests;

#[cfg(test)]
mod caret_report_tests;

#[cfg(test)]
mod search_tests;

/// Dragging Peek's panel by its title strip.
#[cfg(test)]
mod peek_drag_tests;

#[cfg(test)]
mod path_bar_tests;

#[cfg(test)]
mod picture_info_tests {
    use super::*;

    use otto_kit::preview::{Pixels, Preview};

    fn picture(frame_delays: Vec<u32>) -> Preview {
        Preview::Pixels {
            pixels: Pixels {
                width: 450,
                height: 281,
                intrinsic_width: 900,
                intrinsic_height: 563,
                data: Vec::new(),
                frame_delays,
                words: Vec::new(),
            },
            pages: 1,
            page: 1,
        }
    }

    #[test]
    fn a_picture_is_described_by_the_file_not_by_the_decode() {
        otto_kit::i18n::init(&["en-GB".to_string()]);
        // The source's size, although the decode is half of it.
        assert_eq!(
            picture_info(&picture(Vec::new())).as_deref(),
            Some("900 × 563")
        );

        // A pixel count is not a quantity of anything: it is written plainly,
        // without the grouping a number in a sentence would get.
        let mut wide = picture(Vec::new());
        if let Preview::Pixels { pixels, .. } = &mut wide {
            pixels.intrinsic_width = 1920;
            pixels.intrinsic_height = 1080;
        }
        assert_eq!(picture_info(&wide).as_deref(), Some("1920 × 1080"));
    }

    #[test]
    fn an_animation_also_says_how_long_a_loop_runs() {
        otto_kit::i18n::init(&["en-GB".to_string()]);
        let looping = picture(vec![100; 25]);
        assert_eq!(picture_info(&looping).as_deref(), Some("900 × 563 · 0:02"));
    }

    #[test]
    fn what_is_not_a_picture_has_no_line() {
        let text = Preview::Text {
            lines: vec!["fn main() {}".into()],
            truncated: false,
            language: "rust".into(),
        };
        assert!(picture_info(&text).is_none());
    }
}

/// Stepping in and out of Miller columns from the keyboard.
#[cfg(test)]
mod columns_tests;

/// Where a paste puts what is on the clipboard.
#[cfg(test)]
mod paste_target_tests;
