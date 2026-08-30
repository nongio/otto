//! Accessibility: what Otto offers assistive technologies.
//!
//! Two separate things live here, both spoken over D-Bus:
//!
//! - [`keyboard_monitor`] — `org.freedesktop.a11y.KeyboardMonitor`, which lets a
//!   screen reader receive and grab keys. Without it Orca runs but every one of
//!   its keybindings is dead, since a Wayland client cannot read the keyboard.
//! - [`pointer_locator`] — `org.freedesktop.a11y.PointerLocator`, its sibling on
//!   the same object, which says where the pointer is.
//! - The compositor's own chrome as an AT-SPI application, so the dock, the app
//!   switcher and the workspace selector can be announced. That part is served
//!   through AccessKit and lives in `chrome`.
//!
//! Exposure is deliberately conditional. A nested Otto (`--winit`, `--x11`) is a
//! window inside somebody else's session, and must never take
//! `org.freedesktop.a11y.Manager` away from the compositor that owns the
//! screen, so only the udev backend ever calls
//! [`A11yState::take_dbus_parts`] — and only when `accessibility.enabled` is
//! set.

pub mod chrome;
pub mod keyboard_monitor;
pub mod pointer_locator;

use keyboard_monitor::{KeyboardMonitorHandle, PendingKeyEvent};
use pointer_locator::PointerLocatorHandle;
use tokio::sync::mpsc::UnboundedReceiver;

/// Everything the D-Bus thread needs to serve the a11y manager. Handed over
/// once, when that thread starts.
pub struct A11yDbusParts {
    pub keyboard: KeyboardMonitorHandle,
    pub key_events: UnboundedReceiver<PendingKeyEvent>,
    pub pointer: PointerLocatorHandle,
    pub pointer_moves: UnboundedReceiver<()>,
}

/// The compositor's accessibility state.
pub struct A11yState {
    /// Asked on every key press — see [`KeyboardMonitorHandle::process_key`].
    /// Inert, and nearly free, until an assistive technology registers.
    pub keyboard: KeyboardMonitorHandle,
    /// Written on every pointer move, read when an assistive technology asks.
    /// See [`pointer_locator`].
    pub pointer: PointerLocatorHandle,
    /// Taken by the D-Bus thread on startup; `None` afterwards, or from the
    /// start when accessibility is not exposed at all.
    dbus_parts: Option<A11yDbusParts>,
    /// The shell published as an AT-SPI application. Held so it outlives the
    /// weak reference the workspaces' observer list keeps.
    pub chrome: Option<std::sync::Arc<chrome::ShellAccessibility>>,
}

impl Default for A11yState {
    fn default() -> Self {
        Self::new()
    }
}

impl A11yState {
    pub fn new() -> Self {
        let (keyboard, key_events) = KeyboardMonitorHandle::new();
        let (pointer, pointer_moves) = PointerLocatorHandle::new();
        Self {
            keyboard: keyboard.clone(),
            pointer: pointer.clone(),
            dbus_parts: Some(A11yDbusParts {
                keyboard,
                key_events,
                pointer,
                pointer_moves,
            }),
            chrome: None,
        }
    }

    /// Hands the D-Bus half over to the thread that will serve it, once.
    ///
    /// A backend that must not expose accessibility simply never calls this:
    /// the monitor then has no way to acquire a client, so the input path stays
    /// on its no-clients fast path with no second condition to check.
    pub fn take_dbus_parts(&mut self) -> Option<A11yDbusParts> {
        self.dbus_parts.take()
    }
}
