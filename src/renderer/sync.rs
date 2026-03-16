//! Per-frame EGL fence synchronization for GPU rendering.
//!
//! Each frame gets its own EGL fence inserted into the GL command stream
//! immediately after Skia flushes.  The fence is returned inside a
//! `SyncPoint` that Smithay can:
//!
//!   * **export** as a native fd for DRM in-fence (zero-copy GPU-side sync), or
//!   * **wait** on from the CPU (fallback when kernel fencing isn't available).
//!
//! This replaces the previous global `AtomicBool` + `finished_proc` callback
//! approach which was racy across multi-buffer rendering: a callback from
//! frame N could falsely signal frame N+1's fence because they shared a
//! single boolean.
//!
//! # Safety
//!
//! This module contains unsafe FFI calls to EGL.  The safety invariants are:
//! - EGLSync handles must be created from a valid EGL display
//! - EGLSync handles are destroyed exactly once when the last Arc reference drops
//! - Display handle must remain valid for the lifetime of the sync object

use std::{os::unix::io::FromRawFd, time::Duration};

use smithay::backend::{
    egl::{
        self, display::EGLDisplayHandle, ffi::egl::types::EGLSync, wrap_egl_call,
        wrap_egl_call_ptr, EGLDisplay,
    },
    renderer::sync::{Fence, Interrupted},
};

/// Inner fence data containing the EGL sync handle.
///
/// Wrapped in an Arc so cloning a `SkiaSync` is cheap while the underlying
/// kernel sync object is freed exactly once.
#[derive(Debug)]
struct InnerSkiaFence {
    display_handle: std::sync::Arc<EGLDisplayHandle>,
    handle: EGLSync,
}

unsafe impl Send for InnerSkiaFence {}
unsafe impl Sync for InnerSkiaFence {}

/// Per-frame EGL fence sync object.
///
/// Created immediately after Skia flushes its GL command stream so that the
/// fence captures *exactly* the GPU work for one buffer.  Each buffer in the
/// swapchain gets its own fence — no shared global state.
#[derive(Debug, Clone)]
pub struct SkiaSync(std::sync::Arc<InnerSkiaFence>);

impl SkiaSync {
    /// Insert an `EGL_SYNC_FENCE` into the current GL command stream.
    ///
    /// Must be called while the EGL context that performed the rendering is
    /// still current.  The returned fence will signal once every GL command
    /// submitted *before* this call has completed on the GPU.
    pub fn create(display: &EGLDisplay) -> Result<Self, egl::Error> {
        use smithay::backend::egl::ffi::egl::{CreateSync, SYNC_FENCE};

        let display_handle = display.get_display_handle();
        let handle = wrap_egl_call_ptr(|| unsafe {
            CreateSync(**display_handle, SYNC_FENCE, std::ptr::null())
        })
        .map_err(egl::Error::CreationFailed)?;

        Ok(Self(std::sync::Arc::new(InnerSkiaFence {
            display_handle,
            handle,
        })))
    }
}

impl Drop for InnerSkiaFence {
    fn drop(&mut self) {
        unsafe {
            let _ =
                smithay::backend::egl::ffi::egl::DestroySync(**self.display_handle, self.handle);
        }
    }
}

impl Fence for SkiaSync {
    fn export(&self) -> Option<std::os::unix::prelude::OwnedFd> {
        // Try to duplicate the EGL fence as a native sync fd.
        //
        // If the driver supports EGL_ANDROID_native_fence_sync this gives
        // Smithay an fd it can pass as a DRM in-fence — the display hardware
        // waits on the GPU without any CPU involvement.  If the extension is
        // missing the call returns -1 and we fall back to CPU wait.
        use smithay::backend::egl::ffi::egl;

        let fd = unsafe {
            egl::DupNativeFenceFDANDROID(**self.0.display_handle, self.0.handle)
        };
        if fd >= 0 {
            Some(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd) })
        } else {
            None
        }
    }

    fn is_exportable(&self) -> bool {
        // We attempt export in `export()` and let it fail gracefully.
        // Returning false here makes Smithay skip the native-fence path
        // and go straight to `wait()`, which is fine as a default.
        false
    }

    fn is_signaled(&self) -> bool {
        // Non-blocking poll of the per-frame EGL fence.
        use smithay::backend::egl::ffi;

        let status = unsafe {
            ffi::egl::ClientWaitSync(
                **self.0.display_handle,
                self.0.handle,
                0, // no flush — Skia already submitted
                0, // timeout = 0 → non-blocking
            )
        };
        status == ffi::egl::CONDITION_SATISFIED as ffi::egl::types::EGLint
    }

    fn wait(&self) -> Result<(), Interrupted> {
        use smithay::backend::egl::ffi;

        // 100 ms is generous — a single frame's GPU work rarely exceeds a
        // few milliseconds.  SYNC_FLUSH_COMMANDS_BIT ensures any commands
        // still queued in the GL pipeline are flushed before we block.
        let timeout_ns: ffi::egl::types::EGLuint64KHR =
            Duration::from_millis(100).as_nanos() as ffi::egl::types::EGLuint64KHR;

        let status = wrap_egl_call(
            || unsafe {
                ffi::egl::ClientWaitSync(
                    **self.0.display_handle,
                    self.0.handle,
                    ffi::egl::SYNC_FLUSH_COMMANDS_BIT as ffi::egl::types::EGLint,
                    timeout_ns,
                )
            },
            ffi::egl::FALSE as ffi::egl::types::EGLint,
        )
        .map_err(|err| {
            tracing::warn!(?err, "EGL fence wait failed");
            Interrupted
        })?;

        if status == ffi::egl::TIMEOUT_EXPIRED as ffi::egl::types::EGLint {
            tracing::warn!("EGL fence wait timed out after 100 ms — possible GPU hang");
        }

        Ok(())
    }
}
