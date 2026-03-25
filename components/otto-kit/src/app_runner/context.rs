//! Application context - manages global state and Wayland protocol access

use super::{App, AppData};
use crate::protocols::otto_surface_style_manager_v1;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    seat::SeatState,
    shell::xdg::{window::WindowConfigure, XdgShell},
    shm::Shm,
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use wayland_client::backend::ObjectId;
use wayland_client::{protocol::wl_surface, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

// ============================================================================
// Thread-local storage (private — accessed only through AppContext methods)
// ============================================================================

// -- Core state --

thread_local! {
    static APP_CONTEXT_PTR: RefCell<Option<*const AppContextData>> = const { RefCell::new(None) };
    static TYPED_QUEUE_HANDLE: RefCell<Option<Box<dyn std::any::Any>>> = const { RefCell::new(None) };
    #[allow(clippy::type_complexity)]
    static FRAME_REQUEST_FN: RefCell<Option<Box<dyn Fn(&wl_surface::WlSurface)>>> = const { RefCell::new(None) };
    static CURRENT_CONFIGURE: RefCell<Option<(ObjectId, WindowConfigure, u32)>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<Vec<crate::components::window::Window>> = const { RefCell::new(Vec::new()) };
}

// -- Callback registries --

thread_local! {
    static CONFIGURE_HANDLERS: RefCell<Vec<Box<dyn FnMut()>>> = const { RefCell::new(Vec::new()) };
    #[allow(clippy::type_complexity)]
    static POINTER_CALLBACKS: RefCell<Vec<Box<dyn FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent])>>> = const { RefCell::new(Vec::new()) };
    static FRAME_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
    #[allow(clippy::type_complexity)]
    static POPUP_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce(u32)>>> = RefCell::new(HashMap::new());
    static POPUP_DONE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
    #[allow(clippy::type_complexity)]
    static LAYER_SHELL_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut(i32, i32, u32)>>> = RefCell::new(HashMap::new());
    static TRANSACTION_COMPLETION_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
}

// -- Rendering state --

thread_local! {
    static SHARED_SKIA_CONTEXT: RefCell<Option<crate::rendering::SkiaContext>> = const { RefCell::new(None) };
    // pub(crate) because rendering/surface.rs Drop accesses it directly via try_with
    pub(crate) static EGL_DISPLAY: RefCell<Option<khronos_egl::Display>> = const { RefCell::new(None) };
    static EGL_RESOURCES: RefCell<HashMap<ObjectId, crate::rendering::EglSurfaceResources>> = RefCell::new(HashMap::new());
}

// -- Cross-thread statics (renderer thread) --

static LAYERS_RENDERER: LazyLock<RwLock<Option<crate::rendering::LayersRenderer>>> =
    LazyLock::new(|| RwLock::new(None));
static RENDERER_THREAD: LazyLock<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));
static RENDERER_EXIT_FLAG: LazyLock<std::sync::atomic::AtomicBool> =
    LazyLock::new(|| std::sync::atomic::AtomicBool::new(false));

// -- Wakeup pipe (cross-thread) --

use std::sync::OnceLock;

/// Wakeup pipe: (read_fd, write_fd). Created once, lives for process lifetime.
static WAKEUP_PIPE: OnceLock<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> = OnceLock::new();

fn init_wakeup_pipe() -> &'static (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
    WAKEUP_PIPE.get_or_init(|| {
        use std::os::fd::FromRawFd;
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            tracing::error!("failed to create wakeup pipe: {err}");
        }
        unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(fds[0]),
                std::os::fd::OwnedFd::from_raw_fd(fds[1]),
            )
        }
    })
}

// ============================================================================
// Context data structures
// ============================================================================

/// Internal storage for app context - owns the Wayland states
/// This is owned by AppRunner and accessed via AppContext references
pub struct AppContextData {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShell,
    pub shm_state: Shm,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub surface_style_manager: Option<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1>,
    pub wlr_layer_shell: Option<ZwlrLayerShellV1>,
    pub subcompositor: Option<wayland_client::protocol::wl_subcompositor::WlSubcompositor>,
    pub otto_dock_manager: Option<crate::protocols::otto_dock_manager_v1::OttoDockManagerV1>,
    pub display_ptr: *mut std::ffi::c_void,
}

// ============================================================================
// AppContext - public API
// ============================================================================

/// Application context - provides access to Wayland states
///
/// Passed to `App` trait callbacks. Also accessible via static methods
/// for use inside component implementations.
pub struct AppContext<'a> {
    data: &'a AppContextData,
}

impl<'a> AppContext<'a> {
    /// Create a new AppContext borrowing from AppContextData
    pub(crate) fn new(data: &'a AppContextData) -> Self {
        Self { data }
    }

    // ========================================================================
    // Static accessors (for component internals that lack a context reference)
    // ========================================================================

    fn with_global<R, F>(f: F) -> R
    where
        F: FnOnce(&AppContext) -> R,
    {
        APP_CONTEXT_PTR.with(|ptr| {
            let ptr_opt = ptr.borrow();
            let data_ptr = ptr_opt.expect("AppContext not initialized");
            let data = unsafe { &*data_ptr };
            let ctx = AppContext::new(data);
            f(&ctx)
        })
    }

    pub fn compositor_state() -> &'static CompositorState {
        Self::with_global(|ctx| unsafe { &*(ctx.compositor_state_ref() as *const CompositorState) })
    }

    pub fn xdg_shell_state() -> &'static XdgShell {
        Self::with_global(|ctx| unsafe { &*(ctx.xdg_shell_state_ref() as *const XdgShell) })
    }

    pub fn surface_style_manager(
    ) -> Option<&'static otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        Self::with_global(|ctx| unsafe {
            ctx.surface_style_manager_ref()
                .map(|r| &*(r as *const otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1))
        })
    }

    pub fn wlr_layer_shell() -> Option<&'static ZwlrLayerShellV1> {
        Self::with_global(|ctx| unsafe {
            ctx.wlr_layer_shell_ref()
                .map(|r| &*(r as *const ZwlrLayerShellV1))
        })
    }

    pub fn subcompositor(
    ) -> Option<&'static wayland_client::protocol::wl_subcompositor::WlSubcompositor> {
        Self::with_global(|ctx| unsafe {
            ctx.subcompositor_ref().map(|r| {
                &*(r as *const wayland_client::protocol::wl_subcompositor::WlSubcompositor)
            })
        })
    }

    pub fn otto_dock_manager(
    ) -> Option<&'static crate::protocols::otto_dock_manager_v1::OttoDockManagerV1> {
        Self::with_global(|ctx| unsafe {
            ctx.otto_dock_manager_ref()
                .map(|r| &*(r as *const crate::protocols::otto_dock_manager_v1::OttoDockManagerV1))
        })
    }

    pub fn display_ptr() -> *mut std::ffi::c_void {
        Self::with_global(|ctx| ctx.display_ptr_ref())
    }

    pub fn seat_state() -> &'static SeatState {
        Self::with_global(|ctx| unsafe { &*(ctx.seat_state_ref() as *const SeatState) })
    }

    // ========================================================================
    // Instance accessors (preferred — no unsafe lifetime extension)
    // ========================================================================

    pub fn compositor_state_ref(&self) -> &CompositorState {
        &self.data.compositor_state
    }

    pub fn xdg_shell_state_ref(&self) -> &XdgShell {
        &self.data.xdg_shell_state
    }

    pub fn surface_style_manager_ref(
        &self,
    ) -> Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        self.data.surface_style_manager.as_ref()
    }

    pub fn wlr_layer_shell_ref(&self) -> Option<&ZwlrLayerShellV1> {
        self.data.wlr_layer_shell.as_ref()
    }

    pub fn subcompositor_ref(
        &self,
    ) -> Option<&wayland_client::protocol::wl_subcompositor::WlSubcompositor> {
        self.data.subcompositor.as_ref()
    }

    pub fn otto_dock_manager_ref(
        &self,
    ) -> Option<&crate::protocols::otto_dock_manager_v1::OttoDockManagerV1> {
        self.data.otto_dock_manager.as_ref()
    }

    pub fn display_ptr_ref(&self) -> *mut std::ffi::c_void {
        self.data.display_ptr
    }

    pub fn seat_state_ref(&self) -> &SeatState {
        &self.data.seat_state
    }

    // ========================================================================
    // Rendering state
    // ========================================================================

    pub fn skia_context<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::SkiaContext) -> R,
    {
        SHARED_SKIA_CONTEXT.with(|ctx| ctx.borrow_mut().as_mut().map(f))
    }

    pub fn set_skia_context(context: crate::rendering::SkiaContext) {
        let display = context.egl_display();
        EGL_DISPLAY.with(|d| {
            *d.borrow_mut() = Some(display);
        });
        SHARED_SKIA_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(context);
        });
    }

    pub fn layers_renderer_mut<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::LayersRenderer) -> R,
    {
        LAYERS_RENDERER.write().ok()?.as_mut().map(f)
    }

    pub fn layers_renderer<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&crate::rendering::LayersRenderer) -> R,
    {
        LAYERS_RENDERER.read().ok()?.as_ref().map(f)
    }

    pub(crate) fn layers_engine() -> Option<std::sync::Arc<layers::prelude::Engine>> {
        LAYERS_RENDERER
            .read()
            .ok()?
            .as_ref()
            .map(|r| r.engine().clone())
    }

    pub fn enable_layer_engine(width: f32, height: f32) -> bool {
        use std::sync::atomic::Ordering;

        if let Ok(mut renderer) = LAYERS_RENDERER.write() {
            if renderer.is_none() {
                *renderer = Some(crate::rendering::LayersRenderer::new(width, height));

                let exit_flag = &*RENDERER_EXIT_FLAG;
                let thread = std::thread::spawn(move || {
                    while !exit_flag.load(Ordering::Relaxed) {
                        if AppContext::layers_renderer(|renderer| {
                            renderer.update();
                        })
                        .is_some()
                        {
                            std::thread::sleep(std::time::Duration::from_millis(12));
                        } else {
                            break;
                        }
                    }
                });

                if let Ok(mut handle) = RENDERER_THREAD.lock() {
                    *handle = Some(thread);
                }
            }
            true
        } else {
            false
        }
    }

    // ========================================================================
    // EGL resource management
    // ========================================================================

    pub fn insert_egl_resources(
        surface_id: ObjectId,
        resources: crate::rendering::EglSurfaceResources,
    ) {
        EGL_RESOURCES.with(|map| {
            map.borrow_mut().insert(surface_id.clone(), resources);
        });
    }

    pub fn with_egl_resources<R, F>(surface_id: &ObjectId, f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::EglSurfaceResources) -> R,
    {
        EGL_RESOURCES.with(|map| map.borrow_mut().get_mut(surface_id).map(f))
    }

    pub fn remove_egl_resources(surface_id: &ObjectId) {
        let _ = EGL_RESOURCES.try_with(|map| {
            map.borrow_mut().remove(surface_id);
        });
    }

    // ========================================================================
    // Initialization and queue handle
    // ========================================================================

    pub(super) fn init<A: App + 'static>(
        context_data: &AppContextData,
        queue_handle: &QueueHandle<AppData<A>>,
    ) {
        APP_CONTEXT_PTR.with(|ptr| {
            *ptr.borrow_mut() = Some(context_data as *const AppContextData);
        });

        let qh_clone = queue_handle.clone();
        TYPED_QUEUE_HANDLE.with(|qh| {
            *qh.borrow_mut() = Some(Box::new(qh_clone));
        });

        let qh_clone = queue_handle.clone();
        FRAME_REQUEST_FN.with(|frame_fn| {
            *frame_fn.borrow_mut() = Some(Box::new(move |surface: &wl_surface::WlSurface| {
                surface.frame(&qh_clone, surface.clone());
            }));
        });
    }

    pub fn queue_handle_typed<A: App + 'static>() -> &'static QueueHandle<AppData<A>> {
        TYPED_QUEUE_HANDLE.with(|qh| {
            let boxed_any = qh.borrow();
            let any_ref = boxed_any.as_ref().expect("AppContext not initialized");
            let qh_ref = any_ref
                .downcast_ref::<QueueHandle<AppData<A>>>()
                .expect("Queue handle type mismatch - wrong App type?");
            unsafe { &*(qh_ref as *const QueueHandle<AppData<A>>) }
        })
    }

    pub fn queue_handle() -> &'static QueueHandle<AppData<super::DefaultApp>> {
        Self::queue_handle_typed::<super::DefaultApp>()
    }

    /// Return the theme matching the current system color scheme.
    ///
    /// Reads the value maintained by the background color-scheme watcher started
    /// by `AppRunner`. Falls back to the light theme when no preference is set.
    pub fn current_theme() -> crate::theme::Theme {
        use crate::theme::Theme;
        Theme::for_scheme(crate::color_scheme::current_color_scheme())
    }

    // ========================================================================
    // Callback registration (public API for components)
    // ========================================================================

    pub fn current_surface_configure() -> Option<(ObjectId, WindowConfigure, u32)> {
        CURRENT_CONFIGURE.with(|cfg| cfg.borrow().clone())
    }

    pub fn register_configure_handler<F>(handler: F)
    where
        F: FnMut() + 'static,
    {
        CONFIGURE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().push(Box::new(handler));
        });
    }

    pub fn register_pointer_callback<F>(callback: F)
    where
        F: FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent]) + 'static,
    {
        POINTER_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().push(Box::new(callback));
        });
    }

    pub fn register_window(window: crate::components::window::Window) {
        WINDOWS.with(|windows| {
            windows.borrow_mut().push(window);
        });
    }

    pub fn register_popup_configure_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnOnce(u32) + 'static,
    {
        POPUP_CONFIGURE_CALLBACKS.with(|callbacks| {
            callbacks
                .borrow_mut()
                .insert(surface_id, Box::new(callback));
        });
    }

    pub fn register_popup_done_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnOnce() + 'static,
    {
        POPUP_DONE_CALLBACKS.with(|callbacks| {
            callbacks
                .borrow_mut()
                .insert(surface_id, Box::new(callback));
        });
    }

    pub fn register_layer_shell_configure_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnMut(i32, i32, u32) + 'static,
    {
        LAYER_SHELL_CONFIGURE_CALLBACKS.with(|callbacks| {
            callbacks
                .borrow_mut()
                .insert(surface_id, Box::new(callback));
        });
    }

    pub fn register_layer_configure_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnMut(i32, i32, u32) + 'static,
    {
        Self::register_layer_shell_configure_callback(surface_id, callback);
    }

    pub fn register_frame_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnMut() + 'static,
    {
        FRAME_CALLBACKS.with(|callbacks| {
            callbacks
                .borrow_mut()
                .insert(surface_id, Box::new(callback));
        });
    }

    pub fn register_transaction_completion_callback(
        transaction_id: ObjectId,
        callback: Box<dyn FnOnce()>,
    ) {
        TRANSACTION_COMPLETION_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().insert(transaction_id, callback);
        });
    }

    pub fn request_frame(surface: &wl_surface::WlSurface) {
        FRAME_REQUEST_FN.with(|frame_fn| {
            if let Some(f) = frame_fn.borrow().as_ref() {
                f(surface);
            }
        });
    }

    pub fn request_initial_frame(surface: &wl_surface::WlSurface) {
        Self::request_frame(surface);
    }

    /// Wake the main event loop from any thread.
    ///
    /// Background tasks (tokio, threads) should call this after updating
    /// shared state so the main loop re-enters `on_update` promptly.
    /// Safe to call multiple times — extra wakeups are harmless.
    pub fn request_wakeup() {
        use std::os::fd::AsRawFd;
        let (_, write_fd) = init_wakeup_pipe();
        // Best-effort write; EAGAIN/EPIPE are fine (pipe already has data).
        unsafe { libc::write(write_fd.as_raw_fd(), b"w".as_ptr() as *const _, 1) };
    }

    /// Return the raw read fd for the wakeup pipe (for poll integration).
    pub(crate) fn wakeup_read_fd() -> std::os::unix::io::RawFd {
        use std::os::fd::AsRawFd;
        let (read_fd, _) = init_wakeup_pipe();
        read_fd.as_raw_fd()
    }

    /// Drain all pending bytes from the wakeup pipe.
    pub(crate) fn drain_wakeup() {
        use std::os::fd::AsRawFd;
        let (read_fd, _) = init_wakeup_pipe();
        let mut buf = [0u8; 64];
        loop {
            let n =
                unsafe { libc::read(read_fd.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
        }
    }

    // ========================================================================
    // Event dispatch (called by handlers in mod.rs)
    // ========================================================================

    pub(crate) fn has_frame_callback(surface_id: &ObjectId) -> bool {
        FRAME_CALLBACKS.with(|callbacks| callbacks.borrow().contains_key(surface_id))
    }

    pub(crate) fn dispatch_frame_callback(surface_id: &ObjectId) {
        FRAME_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().get_mut(surface_id) {
                callback();
            }
        });
    }

    pub(crate) fn set_current_configure(id: ObjectId, configure: WindowConfigure, serial: u32) {
        CURRENT_CONFIGURE.with(|cfg| {
            *cfg.borrow_mut() = Some((id, configure, serial));
        });
    }

    pub(crate) fn dispatch_configure_handlers() {
        CONFIGURE_HANDLERS.with(|handlers| {
            for handler in handlers.borrow_mut().iter_mut() {
                handler();
            }
        });
    }

    pub(crate) fn clear_current_configure() {
        CURRENT_CONFIGURE.with(|cfg| {
            *cfg.borrow_mut() = None;
        });
    }

    pub(crate) fn dispatch_pointer_callbacks(
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        POINTER_CALLBACKS.with(|callbacks| {
            for callback in callbacks.borrow_mut().iter_mut() {
                callback(events);
            }
        });
    }

    pub(crate) fn dispatch_popup_configure(surface_id: &ObjectId, serial: u32) {
        POPUP_CONFIGURE_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().remove(surface_id) {
                callback(serial);
            }
        });
    }

    pub(crate) fn dispatch_popup_done(surface_id: &ObjectId) {
        POPUP_DONE_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().remove(surface_id) {
                callback();
            }
        });
    }

    pub(crate) fn dispatch_layer_configure(
        surface_id: &ObjectId,
        width: i32,
        height: i32,
        serial: u32,
    ) {
        LAYER_SHELL_CONFIGURE_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().get_mut(surface_id) {
                callback(width, height, serial);
            }
        });
    }

    pub(crate) fn dispatch_transaction_completed(transaction_id: &ObjectId) {
        TRANSACTION_COMPLETION_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().remove(transaction_id) {
                callback();
            }
        });
    }

    // ========================================================================
    // Window update loop
    // ========================================================================

    pub fn update_windows() {
        WINDOWS.with(|windows| {
            for window in windows.borrow_mut().iter_mut() {
                window.update();
            }
        });
    }

    // ========================================================================
    // Shutdown
    // ========================================================================

    pub fn clear() {
        use std::sync::atomic::Ordering;

        // Stop renderer thread
        RENDERER_EXIT_FLAG.store(true, Ordering::Relaxed);
        if let Ok(mut handle) = RENDERER_THREAD.lock() {
            if let Some(thread) = handle.take() {
                let _ = thread.join();
            }
        }

        // Clean up rendering state
        SHARED_SKIA_CONTEXT.with(|ctx| *ctx.borrow_mut() = None);

        // Clean up core state
        APP_CONTEXT_PTR.with(|ptr| *ptr.borrow_mut() = None);
        TYPED_QUEUE_HANDLE.with(|qh| *qh.borrow_mut() = None);
        FRAME_REQUEST_FN.with(|f| *f.borrow_mut() = None);
        CURRENT_CONFIGURE.with(|cfg| *cfg.borrow_mut() = None);
        WINDOWS.with(|w| w.borrow_mut().clear());

        // Clean up callback registries
        CONFIGURE_HANDLERS.with(|h| h.borrow_mut().clear());
        POINTER_CALLBACKS.with(|c| c.borrow_mut().clear());
        FRAME_CALLBACKS.with(|c| c.borrow_mut().clear());
        POPUP_CONFIGURE_CALLBACKS.with(|c| c.borrow_mut().clear());
        POPUP_DONE_CALLBACKS.with(|c| c.borrow_mut().clear());
        LAYER_SHELL_CONFIGURE_CALLBACKS.with(|c| c.borrow_mut().clear());
        TRANSACTION_COMPLETION_CALLBACKS.with(|c| c.borrow_mut().clear());

        // Clean up EGL state
        let _ = EGL_DISPLAY.try_with(|d| *d.borrow_mut() = None);
        let _ = EGL_RESOURCES.try_with(|m| m.borrow_mut().clear());

        // Clean up renderer
        if let Ok(mut renderer) = LAYERS_RENDERER.write() {
            *renderer = None;
        }

        RENDERER_EXIT_FLAG.store(false, Ordering::Relaxed);
    }
}
