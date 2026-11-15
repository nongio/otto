//! Client buffers held until the GPU has read them.
//!
//! smithay hands a client its buffer back — `wl_buffer.release`, or the
//! explicit-sync release point — the moment the compositor drops its last
//! reference, which happens when the next commit replaces the buffer. A frame
//! that sampled the old buffer may still be running on the GPU then, and a
//! client that repaints its buffer as soon as it is released (a browser does)
//! draws into pixels the GPU is still reading: parts of the window show half of
//! one frame and half of the next.
//!
//! So the compositor keeps the buffer each surface shows here, updated on every
//! commit, and every frame takes a snapshot that lives until the frame's GPU
//! fence signals (see `watch_gpu_time` in the udev backend). A buffer replaced
//! meanwhile is released when the last frame that could have read it is done.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use smithay::{
    backend::renderer::utils::{Buffer, RendererSurfaceStateUserData},
    reexports::wayland_server::{
        backend::ObjectId, protocol::wl_surface::WlSurface, Resource, Weak,
    },
    wayland::compositor::with_states,
};

type Shown = HashMap<ObjectId, (Weak<WlSurface>, Buffer)>;

fn store() -> &'static Mutex<Shown> {
    static SHOWN: OnceLock<Mutex<Shown>> = OnceLock::new();
    SHOWN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Records the buffer `surface` shows after the commit just applied.
pub(crate) fn committed(surface: &WlSurface) {
    let buffer = with_states(surface, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|state| {
                state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .buffer()
                    .cloned()
            })
    });
    let mut shown = store().lock().unwrap_or_else(|e| e.into_inner());
    match buffer {
        Some(buffer) => {
            shown.insert(surface.id(), (surface.downgrade(), buffer));
        }
        None => {
            shown.remove(&surface.id());
        }
    }
}

/// The buffers the surfaces show, for a frame to hold until it is done.
/// Surfaces that are gone are let go of here.
#[cfg_attr(not(feature = "renderer_sync"), allow(dead_code))]
pub(crate) fn snapshot() -> Vec<Buffer> {
    let mut shown = store().lock().unwrap_or_else(|e| e.into_inner());
    shown.retain(|_, (surface, _)| surface.upgrade().is_ok());
    shown.values().map(|(_, buffer)| buffer.clone()).collect()
}
