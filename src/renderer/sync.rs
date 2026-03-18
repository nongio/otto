//! Per-frame EGL fence synchronization for GPU rendering.
//!
//! Each frame gets its own EGL fence inserted into the GL command stream
//! immediately after Skia flushes.  The fence is returned inside a
//! `SyncPoint` that Smithay can:
//!
//!   * **export** as a native fd for DRM in-fence (zero-copy GPU-side sync), or
//!   * **wait** on from the CPU (fallback when kernel fencing isn't available).
//!
//! Delegates to Smithay's [`EGLFence`] which automatically picks
//! `EGL_SYNC_NATIVE_FENCE_ANDROID` when the driver supports it (enabling
//! zero-copy export) and falls back to `EGL_SYNC_FENCE` otherwise.

use std::time::Duration;

use smithay::backend::{
    egl::{self, fence::EGLFence, EGLDisplay},
    renderer::sync::{Fence, Interrupted},
};

/// Per-frame EGL fence sync object.
///
/// Thin wrapper around Smithay's [`EGLFence`] implementing the [`Fence`]
/// trait so it can be returned as a `SyncPoint` from the Skia renderer.
#[derive(Debug, Clone)]
pub struct SkiaSync(EGLFence);

impl SkiaSync {
    /// Insert an EGL fence into the current GL command stream.
    ///
    /// Must be called while the EGL context that performed the rendering is
    /// still current.  The returned fence will signal once every GL command
    /// submitted *before* this call has completed on the GPU.
    ///
    /// If `EGL_ANDROID_native_fence_sync` is available the fence can be
    /// exported as a native fd for DRM in-fence (GPU-to-display sync with
    /// zero CPU involvement).
    pub fn create(display: &EGLDisplay) -> Result<Self, egl::Error> {
        EGLFence::create(display).map(Self)
    }
}

impl Fence for SkiaSync {
    fn export(&self) -> Option<std::os::unix::prelude::OwnedFd> {
        self.0.export().ok()
    }

    fn is_exportable(&self) -> bool {
        self.0.is_native()
    }

    fn is_signaled(&self) -> bool {
        self.0
            .client_wait(Some(Duration::ZERO), false)
            .unwrap_or(false)
    }

    fn wait(&self) -> Result<(), Interrupted> {
        let signaled = self
            .0
            .client_wait(Some(Duration::from_millis(100)), true)
            .map_err(|err| {
                tracing::warn!(?err, "EGL fence wait failed");
                Interrupted
            })?;

        if !signaled {
            tracing::warn!("EGL fence wait timed out after 100 ms — possible GPU hang");
        }

        Ok(())
    }
}
