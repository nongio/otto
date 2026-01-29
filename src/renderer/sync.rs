//! EGL fence and synchronization primitives for GPU rendering.
//!
//! This module handles GPU synchronization using EGL sync objects and Skia's
//! flush callbacks. The sync logic ensures proper ordering of GPU commands
//! between the compositor and clients.
//!
//! # Safety
//!
//! This module contains unsafe FFI calls to EGL. The safety invariants are:
//! - EGLSync handles must be created from a valid EGL display
//! - EGLSync handles are destroyed exactly once when the last Arc reference drops
//! - Display handle must remain valid for the lifetime of the sync object

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use smithay::backend::{
    egl::{
        self, display::EGLDisplayHandle, ffi::egl::types::EGLSync, wrap_egl_call,
        wrap_egl_call_ptr, EGLDisplay,
    },
    renderer::sync::{Fence, Interrupted},
};

// This is a "hack" to expose finished_proc and submitted_proc
// until a PR is made to skia-bindings
use layers::sb;

/// FFI structure for Skia flush callbacks.
///
/// This structure mirrors Skia's internal GrFlushInfo but with explicit
/// callback pointers exposed. Used to hook into Skia's rendering pipeline
/// for synchronization.
///
/// # Safety
///
/// This is an FFI type that must match Skia's C++ layout exactly.
#[repr(C)]
#[allow(dead_code, non_snake_case)]
#[derive(Debug)]
pub struct FlushInfo2 {
    pub fNumSemaphores: usize,
    pub fGpuStatsFlags: sb::skgpu_GpuStatsFlags,
    pub fSignalSemaphores: *mut sb::GrBackendSemaphore,
    pub fFinishedProc: sb::GrGpuFinishedProc,
    pub fFinishedWithStatsProc: sb::GrGpuFinishedWithStatsProc,
    pub fFinishedContext: sb::GrGpuFinishedContext,
    pub fSubmittedProc: sb::GrGpuSubmittedProc,
    pub fSubmittedContext: sb::GrGpuSubmittedContext,
}

/// Global state tracking whether Skia has finished processing.
///
/// This is set by the `finished_proc` callback when Skia completes GPU work.
pub static FINISHED_PROC_STATE: AtomicBool = AtomicBool::new(false);

/// Callback invoked by Skia when GPU work completes.
///
/// # Safety
///
/// Called from Skia's internal GPU thread. Must be thread-safe and not panic.
pub unsafe extern "C" fn finished_proc(_: *mut ::core::ffi::c_void) {
    FINISHED_PROC_STATE.store(true, Ordering::SeqCst);
}

/// Inner fence data containing the EGL sync handle.
///
/// This is wrapped in an Arc to allow cheap cloning while ensuring
/// proper cleanup when the last reference is dropped.
#[derive(Debug, Clone)]
struct InnerSkiaFence {
    display_handle: std::sync::Arc<EGLDisplayHandle>,
    handle: EGLSync,
}

unsafe impl Send for InnerSkiaFence {}
unsafe impl Sync for InnerSkiaFence {}

/// EGL-based GPU synchronization fence.
///
/// Wraps an EGL sync object to provide GPU-CPU synchronization.
/// The fence can be waited on to ensure GPU commands have completed.
#[derive(Debug, Clone)]
pub struct SkiaSync(std::sync::Arc<InnerSkiaFence>);

impl SkiaSync {
    /// Creates a new EGL fence sync object.
    ///
    /// # Safety
    ///
    /// The display must be valid and current on the calling thread's context.
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
        // Best-effort destroy of the EGLSync fence to avoid leaking kernel objects/FDs.
        // Safe if called once at last Arc drop; ignored by drivers if already destroyed.
        unsafe {
            // Use the raw egl ffi to destroy the sync; ignore errors.
            let _ =
                smithay::backend::egl::ffi::egl::DestroySync(**self.display_handle, self.handle);
        }
    }
}

impl Fence for SkiaSync {
    fn export(&self) -> Option<std::os::unix::prelude::OwnedFd> {
        None
    }

    fn is_exportable(&self) -> bool {
        false
    }

    fn is_signaled(&self) -> bool {
        FINISHED_PROC_STATE.load(Ordering::SeqCst)
    }

    fn wait(&self) -> Result<(), Interrupted> {
        use smithay::backend::egl::ffi;

        let timeout = Some(Duration::from_millis(2))
            .map(|t| t.as_nanos() as ffi::egl::types::EGLuint64KHR)
            .unwrap_or(ffi::egl::FOREVER);

        let flush = false;
        let flags = if flush {
            ffi::egl::SYNC_FLUSH_COMMANDS_BIT as ffi::egl::types::EGLint
        } else {
            0
        };
        let _status = wrap_egl_call(
            || unsafe {
                ffi::egl::ClientWaitSync(**self.0.display_handle, self.0.handle, flags, timeout)
            },
            ffi::egl::FALSE as ffi::egl::types::EGLint,
        )
        .map_err(|err| {
            tracing::warn!(?err, "Waiting for fence was interrupted");
            Interrupted
        })?;

        // FIXME do we need to destroy the sync?
        // wrap_egl_call(
        //     || unsafe {
        //         ffi::egl::DestroySync(**self.0.display_handle, self.0.handle) as i32
        //     },
        //     ffi::egl::FALSE as ffi::egl::types::EGLint,
        // )
        // .map_err(|err| {
        //     tracing::warn!(?err, "Waiting for fence was interrupted");
        //     Interrupted
        // })?;
        Ok(())
        // while !self.is_signaled() {
        //     if start.elapsed() >=  {
        //         break;
        //     }
        // }
        // while !self.is_signaled() {}
    }
}
