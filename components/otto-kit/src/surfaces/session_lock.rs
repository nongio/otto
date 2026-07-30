//! `ext-session-lock-v1` surfaces — what a screen locker draws into.
//!
//! A locker asks the compositor for a lock and then creates one surface per
//! output. Until every one of them has a buffer the compositor shows its own
//! blank, and the session stays hidden either way: "locked" is compositor
//! state, so a locker that crashes leaves the screen blank rather than
//! exposing the desktop.
//!
//! That last part is why [`SessionLock`] has no `Drop` that tidies up. Losing
//! the lock object is exactly the case the protocol is built around, and the
//! only way out of it is [`SessionLock::unlock`], which the locker calls once
//! it has authenticated the user.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::Proxy;
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ext_session_lock_v1::ExtSessionLockV1,
};

use super::common::{BaseWaylandSurface, SurfaceError};
use crate::app_runner::AppContext;

/// A held session lock.
///
/// Created by [`SessionLock::acquire`], which hides the session immediately —
/// the compositor blanks every output before it answers. The answer arrives as
/// `App::on_session_locked` (the session is hidden and nothing of it is
/// visible) or `App::on_session_lock_finished` (the request was refused, and
/// nothing was locked).
pub struct SessionLock {
    lock: ExtSessionLockV1,
    /// Whether the lock object has been given back. The protocol makes it an
    /// error to destroy a lock that is still holding the session, so nothing
    /// here disposes of it implicitly.
    released: Cell<bool>,
}

impl SessionLock {
    /// Ask the compositor to lock the session.
    pub fn acquire() -> Result<Self, SurfaceError> {
        let manager = AppContext::session_lock_manager().ok_or_else(|| {
            SurfaceError::WaylandError("ext-session-lock-v1 not available".to_string())
        })?;
        let lock = manager.lock(AppContext::queue_handle(), ());
        Ok(Self {
            lock,
            released: Cell::new(false),
        })
    }

    /// Create the lock surface for `output`.
    ///
    /// The compositor answers with a configure carrying the size to draw at;
    /// nothing may be painted before then, which is what
    /// [`SessionLockSurface::is_configured`] reports.
    pub fn surface_for(&self, output: &WlOutput) -> Result<SessionLockSurface, SurfaceError> {
        SessionLockSurface::new(&self.lock, output)
    }

    /// Give the session back. The user has authenticated; the compositor
    /// destroys the lock surfaces and restores what was on screen.
    ///
    /// The request is flushed here rather than left for the run loop. A locker
    /// unlocks in order to exit, and a request still sitting in the client's
    /// buffer when the connection closes is one the compositor never sees — it
    /// observes a locker that died instead, which by design leaves the session
    /// locked and the screen blank.
    pub fn unlock(&self) {
        if self.released.replace(true) {
            return;
        }
        self.lock.unlock_and_destroy();
        AppContext::flush();
    }

    /// Dispose of a lock the compositor refused. Only valid before `locked`:
    /// see [`SessionLock::unlock`] for the other case.
    pub fn abandon(&self) {
        if self.released.replace(true) {
            return;
        }
        self.lock.destroy();
    }

    /// Whether this lock has been unlocked or abandoned.
    pub fn is_released(&self) -> bool {
        self.released.get()
    }
}

struct SessionLockSurfaceInner {
    base_surface: BaseWaylandSurface,
    lock_surface: ExtSessionLockSurfaceV1,
    configured: bool,
}

/// One output's worth of lock screen, with a Skia canvas to draw it into.
#[derive(Clone)]
pub struct SessionLockSurface {
    inner: Rc<RefCell<SessionLockSurfaceInner>>,
    output: WlOutput,
}

impl SessionLockSurface {
    fn new(lock: &ExtSessionLockV1, output: &WlOutput) -> Result<Self, SurfaceError> {
        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();

        let wl_surface = compositor.create_surface(qh);

        // The same 2x buffer scale the rest of otto-kit draws at, so the panel
        // is laid out in logical points and scaled once.
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        let lock_surface = lock.get_lock_surface(&wl_surface, output, qh, ());

        // No commit here: a lock surface may not attach a buffer before it has
        // acked a configure, and the compositor sends that on its own.
        let base_surface = BaseWaylandSurface::new(wl_surface, 0, 0, buffer_scale);

        let surface = Self {
            inner: Rc::new(RefCell::new(SessionLockSurfaceInner {
                base_surface,
                lock_surface: lock_surface.clone(),
                configured: false,
            })),
            output: output.clone(),
        };

        let inner = surface.inner.clone();
        AppContext::register_lock_surface_configure_callback(
            lock_surface.id(),
            move |width, height, serial| {
                let mut inner = inner.borrow_mut();
                inner.lock_surface.ack_configure(serial);

                if !inner.configured {
                    inner.base_surface.width = width;
                    inner.base_surface.height = height;
                    if let Err(err) = inner.base_surface.create_skia_surface() {
                        tracing::error!(?err, "could not create the lock surface's canvas");
                        return;
                    }
                    if let Some(layer) = inner.base_surface.layer_node() {
                        layer.set_size(
                            layers::types::Size::points(width as f32, height as f32),
                            None,
                        );
                        layer.engine.update(0.0);
                    }
                    inner.configured = true;
                } else if width != inner.base_surface.width || height != inner.base_surface.height {
                    inner.base_surface.resize(width, height);
                }
            },
        );

        Ok(surface)
    }

    /// The output this surface covers.
    pub fn output(&self) -> &WlOutput {
        &self.output
    }

    /// Whether a configure has arrived and there is a canvas to draw on.
    pub fn is_configured(&self) -> bool {
        self.inner.borrow().configured
    }

    /// The `ext_session_lock_surface_v1` object.
    pub fn lock_surface(&self) -> ExtSessionLockSurfaceV1 {
        self.inner.borrow().lock_surface.clone()
    }

    /// Dimensions in logical points, as configured by the compositor.
    pub fn dimensions(&self) -> (i32, i32) {
        self.base_surface().dimensions()
    }

    pub fn base_surface(&self) -> &BaseWaylandSurface {
        unsafe {
            let ptr = self.inner.as_ptr();
            &(*ptr).base_surface
        }
    }

    /// Draw on the surface using a callback.
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        self.base_surface().draw(draw_fn);
    }

    /// Destroy the lock surface. The output falls back to the compositor's
    /// blank; the lock itself is unaffected.
    pub fn destroy(&self) {
        let inner = self.inner.borrow();
        AppContext::unregister_lock_surface_configure_callback(&inner.lock_surface.id());
        inner.lock_surface.destroy();
    }
}
