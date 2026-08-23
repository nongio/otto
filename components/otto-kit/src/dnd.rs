//! Drag and drop, over `wl_data_device`.
//!
//! The same object carries the clipboard — see [`crate::clipboard`] — and the
//! transfer machinery is identical: the source holds the bytes and writes them
//! down a pipe when someone asks. What drag adds is a *conversation* while the
//! pointer is moving, and it has two halves that must both be held up or the
//! drop never happens:
//!
//! - **The target answers every position.** On enter and on each motion it says
//!   which MIME type it would take and which action it would perform. Say
//!   nothing and the source is told the drop is refused: the cursor turns to
//!   "no", and releasing the button delivers nothing.
//! - **The action is negotiated, not chosen.** The source offers a set, the
//!   target offers a set, and the *compositor* picks one. Only the pick is
//!   binding, so a target that assumes it got the action it asked for will
//!   sometimes move files it was supposed to copy. [`selected_action`] is the
//!   answer; ask it at drop time, not before.
//!
//! Everything here concerns one drag at a time, which is all a pointer can do,
//! so the state is global the way the clipboard's is.
//!
//! ## Moving files
//!
//! A [`DndAction::Move`] on a file drag is performed **by the target**, which
//! has the paths and can move them itself. The source does not delete anything
//! when the drop finishes. That is what other file managers do, and it is the
//! safe reading: a foreign target that took `text/uri-list` for its own reasons
//! has not necessarily copied the files anywhere, and deleting on its say-so
//! would lose them.

use std::cell::RefCell;
use std::sync::Mutex;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

pub use wayland_client::protocol::wl_data_device_manager::DndAction;

use crate::app_runner::context::AppContext;

/// A drag passing over one of this application's surfaces.
///
/// Positions are surface-local, in the same coordinates a pointer event
/// carries, so an application hit-tests a drag exactly as it hit-tests a click.
#[derive(Debug, Clone)]
pub enum DragEvent {
    /// The drag came over `surface`. The offered types are already recorded —
    /// see [`offered_mime_types`].
    Enter {
        surface: ObjectId,
        x: f64,
        y: f64,
    },
    Motion {
        x: f64,
        y: f64,
    },
    /// The drag left, or was cancelled. Whatever was highlighted should stop
    /// being highlighted; no drop is coming.
    Leave,
    /// The button was released over us. The payload is read with
    /// [`receive`] — at the position of the last enter or motion, which is
    /// carried here again so the handler does not have to have remembered it.
    Drop {
        x: f64,
        y: f64,
    },
}

thread_local! {
    #[allow(clippy::type_complexity)]
    static CALLBACKS: RefCell<Vec<Box<dyn FnMut(&DragEvent)>>> = const { RefCell::new(Vec::new()) };
}

/// The MIME types the drag currently over us offers.
static OFFERED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What this application is offering while *it* is the drag source. Kept apart
/// from the clipboard's payload so starting a drag does not overwrite what was
/// last copied — both live on `wl_data_device`, and both can be asked for at
/// the same time.
static DRAG_PAYLOAD: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------
// Receiving a drag
// ---------------------------------------------------------------------------

/// Be told when a drag enters, moves over, leaves, or is dropped on this
/// application. Callbacks are never removed, matching the pointer registry.
pub fn register<F>(callback: F)
where
    F: FnMut(&DragEvent) + 'static,
{
    CALLBACKS.with(|callbacks| callbacks.borrow_mut().push(Box::new(callback)));
}

pub(crate) fn dispatch(event: DragEvent) {
    // Taken out of the RefCell first: a handler is free to register another.
    CALLBACKS.with(|callbacks| {
        let mut taken = std::mem::take(&mut *callbacks.borrow_mut());
        for callback in taken.iter_mut() {
            callback(&event);
        }
        let mut slot = callbacks.borrow_mut();
        taken.append(&mut slot);
        *slot = taken;
    });
}

pub(crate) fn set_offered_mime_types(mime_types: Vec<String>) {
    *OFFERED.lock().unwrap() = mime_types;
}

/// The MIME types offered by the drag now over this application.
pub fn offered_mime_types() -> Vec<String> {
    OFFERED.lock().unwrap().clone()
}

/// The first of `preferred` the current drag offers, so a caller can express a
/// preference order in one call.
pub fn first_offered(preferred: &[&str]) -> Option<String> {
    let have = offered_mime_types();
    preferred
        .iter()
        .find(|want| have.iter().any(|h| h == *want))
        .map(|s| (*s).to_string())
}

/// Tell the source this drag would be taken as `mime`, performing `preferred`
/// out of `actions`.
///
/// **Call this on every enter and every motion.** The answer is per-position,
/// not per-drag: a target that accepts once and then goes quiet is treated as
/// having stopped accepting, because that is how a target says the pointer has
/// moved off the part of it that wanted the drop.
///
/// Passing `None` rejects the drag at this position, which is how a file
/// manager says "not over a folder".
pub fn accept(mime: Option<&str>, actions: DndAction, preferred: DndAction) {
    AppContext::accept_drag(mime.map(str::to_string), actions, preferred);
}

/// The action the compositor settled on, out of what the source offered and
/// what the target accepted.
///
/// This is the one that binds. At drop time it is the difference between a copy
/// and a move.
pub fn selected_action() -> DndAction {
    AppContext::drag_selected_action()
}

/// Read the dropped payload as `mime`, and tell the source the drag is done.
///
/// Only call this after [`DragEvent::Drop`]. It finishes the offer, so it reads
/// once: the caller keeps the bytes.
pub fn receive(mime: &str) -> Option<Vec<u8>> {
    let pipe = AppContext::receive_drag(mime)?;
    let bytes = crate::clipboard::read_bounded(pipe);
    AppContext::finish_drag();
    bytes
}

/// The dropped payload as file paths, plus whether the source called it a cut.
///
/// The cut flag comes from the payload itself and is only meaningful for
/// `x-special/gnome-copied-files`; for a drag, [`selected_action`] is the
/// authority on copy-versus-move, and this flag is nearly always `false`.
pub fn receive_files() -> Option<(Vec<std::path::PathBuf>, bool)> {
    let mime = first_offered(crate::clipboard::file_mime_preference())?;
    let bytes = receive(&mime)?;
    let (paths, cut) = crate::clipboard::parse_file_payload(&mime, &bytes);
    (!paths.is_empty()).then_some((paths, cut))
}

/// The files *this* application is dragging, read straight out of the payload
/// it offered rather than through the compositor.
///
/// A drop on our own window has to be served this way. The source and the
/// target are the same thread: asking for the data over the pipe would block
/// that thread waiting for a write only it can make, and the read would sit
/// there until it timed out. Use this whenever [`dragging`] is true, and
/// [`receive_files`] otherwise.
pub fn own_files() -> Option<Vec<std::path::PathBuf>> {
    let mime = crate::clipboard::URI_LIST;
    let bytes = payload_for(mime)?;
    let (paths, _) = crate::clipboard::parse_file_payload(mime, &bytes);
    (!paths.is_empty()).then_some(paths)
}

/// Tell the source the drop is done with, without reading anything more.
///
/// [`receive`] already does this. A target that served itself from
/// [`own_files`] still owes the source the acknowledgement, and this is it.
pub fn finish() {
    AppContext::finish_drag();
}

// ---------------------------------------------------------------------------
// Being the drag source
// ---------------------------------------------------------------------------

/// Start a drag carrying `payloads`, one `(mime_type, bytes)` per type offered.
///
/// `origin` is the surface the drag started on and `serial` must come from the
/// press that started it — the compositor checks both, which is what stops a
/// client dragging without the user having touched it. `icon`, if given, is a
/// surface the compositor carries under the cursor; it takes the drag-icon role
/// here, so it must not already have one, and it is drawn by the caller *after*
/// this returns.
///
/// Returns whether the drag started. `false` means there is no data device: no
/// seat yet, or a compositor without `wl_data_device_manager`.
pub fn start(
    payloads: Vec<(String, Vec<u8>)>,
    actions: DndAction,
    origin: &WlSurface,
    icon: Option<&WlSurface>,
    serial: u32,
) -> bool {
    let mime_types: Vec<String> = payloads.iter().map(|(mime, _)| mime.clone()).collect();
    *DRAG_PAYLOAD.lock().unwrap() = payloads;
    let started = AppContext::start_drag(mime_types, actions, origin, icon, serial);
    if !started {
        DRAG_PAYLOAD.lock().unwrap().clear();
    }
    started
}

thread_local! {
    /// The picture under the cursor, alive for exactly as long as the drag is.
    static DRAG_ICON: RefCell<Option<DragIcon>> = const { RefCell::new(None) };
}

/// The surface the compositor carries under the pointer during a drag.
///
/// A plain `wl_surface` with no role of its own until `start_drag` gives it
/// one, which is why it is created here and handed over rather than being any
/// of the window types. Its origin sits at the pointer, so whatever is drawn
/// at `(0, 0)` is what the cursor appears to be holding.
struct DragIcon {
    surface: crate::surfaces::BaseWaylandSurface,
    /// The size it was created at, in points. Kept so a redraw can be handed
    /// the same box the first draw was given.
    size: (f32, f32),
}

impl DragIcon {
    /// Create the surface, without committing anything to it. The buffer comes
    /// after the drag has started — the role has to be assigned first.
    fn new(width: i32, height: i32) -> Option<Self> {
        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();
        let wl_surface = compositor.create_surface(qh);

        // Same 2x buffer every other otto-kit surface uses for HiDPI.
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        let mut surface =
            crate::surfaces::BaseWaylandSurface::new(wl_surface, width, height, buffer_scale);
        if let Err(err) = surface.create_skia_surface() {
            tracing::warn!(%err, "could not create the drag icon's surface");
            return None;
        }
        Some(Self {
            surface,
            size: (width as f32, height as f32),
        })
    }

    /// Put the point `(x, y)` *inside* the icon under the cursor, instead of
    /// the icon's top-left corner.
    ///
    /// The icon's origin is the cursor hotspot, so anchoring means shifting the
    /// surface up and left by the grab point. `wl_surface.offset` is the way to
    /// say that from version 5 on — `attach`'s own x and y are deprecated, and
    /// the EGL swap that put the buffer there passes zeroes for them anyway.
    /// Below version 5 the icon simply hangs from its corner.
    fn set_anchor(&self, (x, y): (f32, f32)) {
        let surface = self.surface.wl_surface();
        if surface.version() < 5 {
            return;
        }
        surface.offset(-(x.round() as i32), -(y.round() as i32));
        // A commit of its own: the buffer is already attached, and this moves
        // the icon without giving it new content.
        surface.commit();
    }

    fn destroy(&mut self) {
        // Skia and EGL first, then the Wayland object they were built on.
        drop(self.surface.take_skia_surface());
        self.surface.take_surface_style();
        self.surface.wl_surface().destroy();
    }
}

impl Drop for DragIcon {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Drag `paths` as files, carrying a picture under the cursor.
///
/// `draw` is handed the icon's canvas and its size in points, and runs once,
/// after the drag has started. Everything [`start_file_drag`] does applies;
/// the icon is the only difference, and it lives until the drag ends.
pub fn start_file_drag_with_icon<F>(
    paths: &[std::path::PathBuf],
    actions: DndAction,
    origin: &WlSurface,
    serial: u32,
    size: (i32, i32),
    anchor: (f32, f32),
    draw: F,
) -> bool
where
    F: FnOnce(&skia_safe::Canvas, f32, f32),
{
    if paths.is_empty() {
        return false;
    }

    // The previous drag's icon, now that this one is starting: see
    // `clear_payload` for why it outlived its own drag.
    DRAG_ICON.with(|slot| slot.borrow_mut().take());

    // No icon is not a reason to refuse the drag: the gesture works without
    // one, and the cursor still says copy or move.
    let icon = DragIcon::new(size.0, size.1);
    let started = start(
        crate::clipboard::file_payloads(paths, false),
        actions,
        origin,
        icon.as_ref().map(|icon| icon.surface.wl_surface()),
        serial,
    );
    if !started {
        return false;
    }

    if let Some(icon) = icon {
        // Only now: the surface has the drag-icon role, and this attaches the
        // first buffer to it.
        icon.surface
            .draw(|canvas| draw(canvas, size.0 as f32, size.1 as f32));
        icon.set_anchor(anchor);
        DRAG_ICON.with(|slot| *slot.borrow_mut() = Some(icon));
    }
    true
}

/// Drag `paths` as files, offering every payload a file manager, a text editor
/// or another Otto application would understand.
pub fn start_file_drag(
    paths: &[std::path::PathBuf],
    actions: DndAction,
    origin: &WlSurface,
    icon: Option<&WlSurface>,
    serial: u32,
) -> bool {
    if paths.is_empty() {
        return false;
    }
    // `cut` is false: a drag says what it is doing through the negotiated
    // action, and a payload that also claimed "cut" would double the message.
    start(
        crate::clipboard::file_payloads(paths, false),
        actions,
        origin,
        icon,
        serial,
    )
}

/// The payload for `mime` while this application is the drag source.
pub(crate) fn payload_for(mime: &str) -> Option<Vec<u8>> {
    DRAG_PAYLOAD
        .lock()
        .unwrap()
        .iter()
        .find(|(m, _)| m == mime)
        .map(|(_, bytes)| bytes.clone())
}

/// The drag ended — dropped, cancelled, or refused. The payload goes with it.
///
/// The icon does *not*. The compositor may still be animating it — Otto flies a
/// refused drag's icon back to where it started — and destroying the surface
/// here leaves it animating nothing. It is kept until the next drag needs one,
/// by which time whatever the compositor was doing with it is long over.
pub(crate) fn clear_payload() {
    DRAG_PAYLOAD.lock().unwrap().clear();
}

/// Draw the drag icon again, over what is already there.
///
/// The picture under the cursor is a surface of our own, so it can change while
/// the drag runs: a group of files can gather into a stack, a target's answer
/// can change what is shown. The size is fixed at the drag's start — the buffer
/// is allocated once — so this repaints the same box.
///
/// Returns whether there was an icon to draw into: a drag started without one,
/// or one already over, is not an error.
pub fn redraw_icon<F>(draw: F) -> bool
where
    F: FnOnce(&skia_safe::Canvas, f32, f32),
{
    DRAG_ICON.with(|slot| {
        let slot = slot.borrow();
        let Some(icon) = slot.as_ref() else {
            return false;
        };
        let (w, h) = icon.size;
        icon.surface.draw(|canvas| draw(canvas, w, h));
        true
    })
}

/// Is this application the source of a drag in flight?
pub fn dragging() -> bool {
    !DRAG_PAYLOAD.lock().unwrap().is_empty()
}
