//! Which graphics API and driver the session draws with.
//!
//! Recorded once, when the first renderer comes up, and read by the
//! compositor's `org.otto.Compositor.DebugInfo` for bug reports.

// Rust guideline compliant 2026-02-21

use std::sync::OnceLock;

/// The graphics API and the driver strings of the first renderer created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererInfo {
    /// `"gl"` or `"vulkan"`.
    pub api: &'static str,
    /// `GL_VENDOR`, or the Vulkan driver name.
    pub vendor: String,
    /// `GL_RENDERER`, or the Vulkan device name.
    pub renderer: String,
    /// `GL_VERSION`, or the Vulkan API version and driver info.
    pub version: String,
}

/// The first renderer's API and driver, set by [`record`].
static RENDERER_INFO: OnceLock<RendererInfo> = OnceLock::new();

/// The API and driver of the session's first renderer, once one exists.
///
/// On a machine with several GPUs this is the first one a renderer was made
/// for, which is the primary GPU on the udev backend.
pub fn renderer_info() -> Option<&'static RendererInfo> {
    RENDERER_INFO.get()
}

/// Records `info` unless a renderer was already recorded.
pub(crate) fn record(info: RendererInfo) {
    let _ = RENDERER_INFO.set(info);
}

/// Reads `GL_VENDOR`, `GL_RENDERER` and `GL_VERSION` and records them.
///
/// # Safety
///
/// `gl` must have been loaded for the GL context current on this thread.
pub(crate) unsafe fn record_gl(gl: &smithay::backend::renderer::gles::ffi::Gles2) {
    use smithay::backend::renderer::gles::ffi;

    if RENDERER_INFO.get().is_some() {
        return;
    }
    let read = |name| {
        // SAFETY: the caller made `gl`'s context current; `GetString`
        // returns a static, NUL-terminated string or null.
        let ptr = unsafe { gl.GetString(name) };
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: non-null, NUL-terminated and owned by the GL implementation
        // for the life of the context.
        unsafe { std::ffi::CStr::from_ptr(ptr.cast()) }
            .to_string_lossy()
            .into_owned()
    };
    record(RendererInfo {
        api: "gl",
        vendor: read(ffi::VENDOR),
        renderer: read(ffi::RENDERER),
        version: read(ffi::VERSION),
    });
}
