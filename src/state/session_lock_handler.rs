//! `ext-session-lock-v1` handler.
//!
//! The protocol side is thin: accept or refuse a lock, register a surface per
//! output, unlock. Everything the compositor has to *do* about a locked session
//! lives in [`crate::lock`].

use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};
use tracing::warn;

use crate::state::{Backend, Otto};

impl<BackendData: Backend> SessionLockHandler for Otto<BackendData> {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_manager_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // `begin_lock` refuses a second lock by dropping the confirmation,
        // which sends `finished`.
        self.begin_lock(confirmation);
    }

    fn unlock(&mut self) {
        self.finish_unlock();
    }

    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(output) = Output::from_resource(&wl_output) else {
            warn!("lock surface for an output that is gone");
            return;
        };
        self.add_lock_surface(surface, output);
    }
}
