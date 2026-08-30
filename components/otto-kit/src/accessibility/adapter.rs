//! One AT-SPI adapter per accessible surface.
//!
//! The adapter's three handlers are called from the adapter's own thread, and
//! a kit application's state lives on its UI thread — so nothing is built
//! there. Activation raises a flag and wakes the run loop; actions are queued
//! and drained by it. The tree itself is built where it can be: in the loop,
//! from the application, through [`crate::App::accessibility`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use accesskit_unix::Adapter;

/// What the adapter's thread hands to the UI thread.
#[derive(Default)]
pub(crate) struct Mailbox {
    /// An assistive technology has attached and wants a tree.
    wanted: AtomicBool,
    /// Actions asked of the application, oldest first.
    actions: Mutex<Vec<ActionRequest>>,
}

impl Mailbox {
    fn wake(&self) {
        // The run loop is usually blocked in `poll`; without this the tree
        // would not appear until something else happened to wake it.
        crate::app_runner::AppContext::request_wakeup();
    }

    pub(crate) fn is_wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }

    pub(crate) fn take_actions(&self) -> Vec<ActionRequest> {
        std::mem::take(&mut *self.actions.lock().unwrap())
    }
}

struct Activation(Arc<Mailbox>);

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // Deliberately `None`: the tree can only be built on the UI thread, so
        // it follows on the next pass of the run loop. The adapter stands in
        // with a placeholder until then.
        self.0.wanted.store(true, Ordering::Relaxed);
        self.0.wake();
        None
    }
}

struct Actions(Arc<Mailbox>);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        self.0.actions.lock().unwrap().push(request);
        self.0.wake();
    }
}

struct Deactivation(Arc<Mailbox>);

impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {
        self.0.wanted.store(false, Ordering::Relaxed);
    }
}

/// The accessibility of one surface.
pub(crate) struct SurfaceAdapter {
    adapter: Adapter,
    pub(crate) mailbox: Arc<Mailbox>,
    /// Whether the surface has been described at least once since an assistive
    /// technology attached. Until it has, an update must carry the whole tree —
    /// which is all a kit application ever sends anyway.
    pub(crate) described: bool,
    /// The last frame handed to the adapter, so an unmoved window does not
    /// push the same bounds on every pass of the run loop.
    desktop_frame: Option<accesskit::Rect>,
}

impl SurfaceAdapter {
    pub(crate) fn new() -> Self {
        let mailbox = Arc::new(Mailbox::default());
        let adapter = Adapter::new(
            Activation(mailbox.clone()),
            Actions(mailbox.clone()),
            Deactivation(mailbox.clone()),
        );

        Self {
            adapter,
            mailbox,
            described: false,
            desktop_frame: None,
        }
    }

    /// Pushes a tree, if anything is listening. The closure does not run
    /// otherwise, so describing a window costs nothing with no assistive
    /// technology present.
    pub(crate) fn update(&mut self, build: impl FnOnce() -> TreeUpdate) {
        self.adapter.update_if_active(build);
        self.described = true;
    }

    /// Whether this surface has the keyboard. A screen reader reads the focused
    /// window; without this it would have no reason to read any of them.
    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.adapter.update_window_focus_state(focused);
    }

    /// Where the window is on the desktop, in logical pixels.
    ///
    /// Everything the application describes is in its own coordinates, with
    /// the origin at the window's top-left corner. This is what turns those
    /// into the desktop coordinates an assistive technology asks in: without
    /// it every window claims the rectangle at the same offset from the
    /// desktop's origin, and a screen reader pointed at a control finds
    /// whatever is drawn in the corner of the screen instead.
    ///
    /// Outer and inner are the same rect. Otto's applications draw their own
    /// decorations into their own surface, so there is no frame around the
    /// client area to distinguish the two.
    pub(crate) fn set_desktop_frame(&mut self, frame: accesskit::Rect) {
        if self.desktop_frame == Some(frame) {
            return;
        }
        self.desktop_frame = Some(frame);
        self.adapter.set_root_window_bounds(frame, frame);
    }
}
