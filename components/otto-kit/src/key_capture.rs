//! Taking every key, for a control that records a key combination.
//!
//! A shortcut recorder has to see the combination the person presses, and in
//! the ordinary course most interesting combinations never reach a client:
//! the compositor answers its own shortcuts first, and the run loop answers
//! Tab (focus) and Cmd+W (close) before the application is asked. While a
//! capture is held both step aside — the compositor through
//! `zwp_keyboard_shortcuts_inhibit_manager_v1`, the run loop by checking
//! [`is_capturing`].
//!
//! A compositor without the inhibit protocol still delivers whatever it does
//! not claim, so a capture there records everything but the compositor's own
//! shortcuts.

use std::cell::RefCell;

use wayland_client::globals::GlobalList;
use wayland_client::protocol::{wl_seat::WlSeat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
};

/// The manager and seat an inhibitor is made from, bound at connection time.
struct Binding {
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    seat: WlSeat,
    queue: EventQueue<Inhibit>,
}

/// State for the private queue. The inhibitor's `active`/`inactive` events
/// carry nothing a recorder acts on, so there is nothing to keep.
struct Inhibit;

thread_local! {
    static BINDING: RefCell<Option<Binding>> = const { RefCell::new(None) };
    /// The capture in force, if any: the inhibitor when the compositor offers
    /// one, and `None` inside when it does not.
    static CAPTURE: RefCell<Option<Option<ZwpKeyboardShortcutsInhibitorV1>>> =
        const { RefCell::new(None) };
}

/// Bind the inhibit manager, on a queue of its own like
/// [`crate::backdrop`]'s, so nothing of the application's is dispatched for it.
pub(crate) fn init(conn: &Connection, globals: &GlobalList) {
    let queue = conn.new_event_queue::<Inhibit>();
    let qh = queue.handle();
    let manager = globals
        .bind::<ZwpKeyboardShortcutsInhibitManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    let seat = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()).ok();
    let binding = manager.zip(seat).map(|(manager, seat)| Binding {
        manager,
        seat,
        queue,
    });
    if binding.is_none() {
        tracing::debug!("no keyboard shortcuts inhibitor; compositor shortcuts cannot be recorded");
    }
    BINDING.with(|slot| *slot.borrow_mut() = binding);
}

/// Start taking every key while `surface` has the keyboard.
///
/// Holding a capture already moves it to `surface`.
pub fn start(surface: &WlSurface) {
    stop();
    let inhibitor = BINDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        let binding = slot.as_mut()?;
        let inhibitor =
            binding
                .manager
                .inhibit_shortcuts(surface, &binding.seat, &binding.queue.handle(), ());
        // Sent now rather than with the next frame: the key the person is
        // about to press must already find the compositor standing aside.
        let _ = binding.queue.flush();
        Some(inhibitor)
    });
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(inhibitor));
}

/// Give the keys back to the compositor and the run loop.
pub fn stop() {
    let Some(inhibitor) = CAPTURE.with(|capture| capture.borrow_mut().take()) else {
        return;
    };
    if let Some(inhibitor) = inhibitor {
        inhibitor.destroy();
        BINDING.with(|slot| {
            if let Some(binding) = slot.borrow().as_ref() {
                let _ = binding.queue.flush();
            }
        });
    }
}

/// Whether a capture is in force, for the run loop's own key shortcuts.
pub fn is_capturing() -> bool {
    CAPTURE.with(|capture| capture.borrow().is_some())
}

impl Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for Inhibit {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitManagerV1,
        _: <ZwpKeyboardShortcutsInhibitManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for Inhibit {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitorV1,
        _: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Inhibit {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
