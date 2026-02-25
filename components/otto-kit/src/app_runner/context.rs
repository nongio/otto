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
use std::sync::RwLock;
use wayland_client::backend::ObjectId;
use wayland_client::{protocol::wl_surface, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

// ============================================================================
// Thread-local storage
// ============================================================================

// Store pointer to AppContextData (single pointer instead of RawAppContext struct)
// This pointer is only valid during the event loop while AppData exists
thread_local! {
    static APP_CONTEXT_PTR: RefCell<Option<*const AppContextData>> = RefCell::new(None);
}

// Store configure handlers registered by surface components
thread_local! {
    pub(super) static CONFIGURE_HANDLERS: RefCell<Vec<Box<dyn FnMut()>>> = RefCell::new(Vec::new());
}

// Store the current surface configure event being processed
thread_local! {
    pub(super) static CURRENT_CONFIGURE: RefCell<Option<(ObjectId, WindowConfigure, u32)>> = RefCell::new(None);
}

// Store the shared SkiaContext
thread_local! {
    pub(super) static SHARED_SKIA_CONTEXT: RefCell<Option<crate::rendering::SkiaContext>> = RefCell::new(None);
}

// Store the shared LayersRenderer (globally shared with RwLock for multi-threaded access)
lazy_static::lazy_static! {
    pub(super) static ref LAYERS_RENDERER: RwLock<Option<crate::rendering::LayersRenderer>> = RwLock::new(None);
}

// Store renderer thread handle and exit flag
lazy_static::lazy_static! {
    pub(super) static ref RENDERER_THREAD: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(None);
    pub(super) static ref RENDERER_EXIT_FLAG: std::sync::Arc<std::sync::atomic::AtomicBool> = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
}

// Store EGL display for cleanup
thread_local! {
    pub(crate) static EGL_DISPLAY: RefCell<Option<khronos_egl::Display>> = RefCell::new(None);
}

// Store EGL resources for all surfaces (cold path storage)
thread_local! {
    pub(super) static EGL_RESOURCES: RefCell<HashMap<ObjectId, crate::rendering::EglSurfaceResources>> = RefCell::new(HashMap::new());
}

// Store pointer event callbacks
thread_local! {
    pub(super) static POINTER_CALLBACKS: RefCell<Vec<Box<dyn FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent])>>> = RefCell::new(Vec::new());
}

// Store pending pointer callbacks to be registered after event dispatch
thread_local! {
    pub(super) static PENDING_POINTER_CALLBACKS: RefCell<Vec<Box<dyn FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent])>>> = RefCell::new(Vec::new());
}

// Store windows for rendering loop
thread_local! {
    pub(super) static WINDOWS: RefCell<Vec<crate::components::window::Window>> = RefCell::new(Vec::new());
}

// Store popup configure callbacks - map from popup wl_surface ObjectId to callback
thread_local! {
    pub(super) static POPUP_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce(u32)>>> = RefCell::new(HashMap::new());
}

// Store popup done callbacks - map from popup wl_surface ObjectId to callback
thread_local! {
    pub(super) static POPUP_DONE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
}

// Store layer shell configure callbacks - map from layer_surface ObjectId to callback
thread_local! {
    pub(super) static LAYER_SHELL_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut(i32, i32, u32)>>> = RefCell::new(HashMap::new());
}

// Store frame callbacks - map from surface ObjectId to callback
thread_local! {
    pub(super) static FRAME_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
}

// Store transaction completion callbacks - map from transaction ObjectId to callback
thread_local! {
    pub(crate) static TRANSACTION_COMPLETION_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
}

// Store frame request function (set during AppRunner initialization)
thread_local! {
    pub(super) static FRAME_REQUEST_FN: RefCell<Option<Box<dyn Fn(&wl_surface::WlSurface)>>> = RefCell::new(None);
}

// Store the actual typed queue handle in a separate thread-local
// We store it type-erased and reconstruct the type when retrieving
// QueueHandle is Clone so we can store it by value
thread_local! {
    pub(super) static TYPED_QUEUE_HANDLE: RefCell<Option<Box<dyn std::any::Any>>> = RefCell::new(None);
}

// ============================================================================
// Context data structures
// ============================================================================

/// Internal storage for app context - owns the Wayland states
/// This is owned by AppRunner and accessed via AppContext<'a> references
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
/// This is passed to your App methods as a parameter.
pub struct AppContext<'a> {
    data: &'a AppContextData,
}

impl<'a> AppContext<'a> {
    /// Create a new AppContext borrowing from AppContextData
    pub(super) fn new(data: &'a AppContextData) -> Self {
        Self { data }
    }

    /// Get an AppContext from the global pointer (for compatibility with existing code)
    /// This creates a temporary AppContext that borrows from the global AppContextData
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

    /// Get compositor state (static accessor for compatibility)
    pub fn compositor_state() -> &'static CompositorState {
        Self::with_global(|ctx| unsafe {
            // SAFETY: We extend the lifetime to 'static for compatibility
            // The pointer remains valid during the event loop
            &*(ctx.compositor_state_ref() as *const CompositorState)
        })
    }

    /// Get XDG shell state (static accessor for compatibility)
    pub fn xdg_shell_state() -> &'static XdgShell {
        Self::with_global(|ctx| unsafe { &*(ctx.xdg_shell_state_ref() as *const XdgShell) })
    }

    /// Get SC layer shell if available (static accessor for compatibility)
    pub fn surface_style_manager(
    ) -> Option<&'static otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        Self::with_global(|ctx| unsafe {
            ctx.surface_style_manager_ref()
                .map(|r| &*(r as *const otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1))
        })
    }

    /// Get wlr-layer-shell if available (static accessor for compatibility)
    pub fn wlr_layer_shell() -> Option<&'static ZwlrLayerShellV1> {
        Self::with_global(|ctx| unsafe {
            ctx.wlr_layer_shell_ref()
                .map(|r| &*(r as *const ZwlrLayerShellV1))
        })
    }

    /// Get subcompositor if available (static accessor for compatibility)
    pub fn subcompositor(
    ) -> Option<&'static wayland_client::protocol::wl_subcompositor::WlSubcompositor> {
        Self::with_global(|ctx| unsafe {
            ctx.subcompositor_ref().map(|r| {
                &*(r as *const wayland_client::protocol::wl_subcompositor::WlSubcompositor)
            })
        })
    }

    /// Get otto_dock_manager if available (static accessor for compatibility)
    pub fn otto_dock_manager(
    ) -> Option<&'static crate::protocols::otto_dock_manager_v1::OttoDockManagerV1> {
        Self::with_global(|ctx| unsafe {
            ctx.otto_dock_manager_ref()
                .map(|r| &*(r as *const crate::protocols::otto_dock_manager_v1::OttoDockManagerV1))
        })
    }

    /// Get display pointer (static accessor for compatibility)
    pub fn display_ptr() -> *mut std::ffi::c_void {
        Self::with_global(|ctx| ctx.display_ptr_ref())
    }

    /// Get seat state (static accessor for compatibility)
    pub fn seat_state() -> &'static SeatState {
        Self::with_global(|ctx| unsafe { &*(ctx.seat_state_ref() as *const SeatState) })
    }

    /// Instance method to get compositor state
    pub fn compositor_state_ref(&self) -> &CompositorState {
        &self.data.compositor_state
    }

    /// Instance method to get XDG shell state
    pub fn xdg_shell_state_ref(&self) -> &XdgShell {
        &self.data.xdg_shell_state
    }

    /// Instance method to get SC layer shell if available
    pub fn surface_style_manager_ref(
        &self,
    ) -> Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        self.data.surface_style_manager.as_ref()
    }

    /// Instance method to get wlr-layer-shell if available
    pub fn wlr_layer_shell_ref(&self) -> Option<&ZwlrLayerShellV1> {
        self.data.wlr_layer_shell.as_ref()
    }

    /// Instance method to get subcompositor if available
    pub fn subcompositor_ref(
        &self,
    ) -> Option<&wayland_client::protocol::wl_subcompositor::WlSubcompositor> {
        self.data.subcompositor.as_ref()
    }

    /// Instance method to get otto_dock_manager if available
    pub fn otto_dock_manager_ref(
        &self,
    ) -> Option<&crate::protocols::otto_dock_manager_v1::OttoDockManagerV1> {
        self.data.otto_dock_manager.as_ref()
    }

    /// Instance method to get display pointer
    pub fn display_ptr_ref(&self) -> *mut std::ffi::c_void {
        self.data.display_ptr
    }

    /// Instance method to get seat state
    pub fn seat_state_ref(&self) -> &SeatState {
        &self.data.seat_state
    }

    /// Get mutable reference to the shared SkiaContext
    /// Returns None if not yet initialized
    pub fn skia_context<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::SkiaContext) -> R,
    {
        SHARED_SKIA_CONTEXT.with(|ctx| ctx.borrow_mut().as_mut().map(f))
    }

    /// Initialize or replace the shared Skia context
    pub fn set_skia_context(context: crate::rendering::SkiaContext) {
        // Store EGL display for cleanup
        let display = context.egl_display();
        EGL_DISPLAY.with(|d| {
            *d.borrow_mut() = Some(display);
        });

        SHARED_SKIA_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(context);
        });
    }

    /// Get or initialize the shared layers renderer
    pub fn layers_renderer_mut<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::LayersRenderer) -> R,
    {
        LAYERS_RENDERER.write().ok()?.as_mut().map(f)
    }

    /// Get or initialize the shared layers renderer
    pub fn layers_renderer<R, F>(f: F) -> Option<R>
    where
        F: FnOnce(&crate::rendering::LayersRenderer) -> R,
    {
        LAYERS_RENDERER.read().ok()?.as_ref().map(f)
    }

    /// Get the layers engine (internal use by layer components)
    pub(crate) fn layers_engine() -> Option<std::sync::Arc<layers::prelude::Engine>> {
        LAYERS_RENDERER
            .read()
            .ok()?
            .as_ref()
            .map(|r| r.engine().clone())
    }

    /// Enable/initialize the layers engine
    ///
    /// This initializes the shared layers renderer and spawns the renderer update thread.
    /// If already initialized, this is a no-op.
    ///
    /// # Arguments
    /// * `width` - Optional width for the renderer (default: 1920.0)
    /// * `height` - Optional height for the renderer (default: 1080.0)
    ///
    /// # Returns
    /// * `true` if the layers engine was enabled/already enabled
    /// * `false` if initialization failed
    pub fn enable_layer_engine(width: f32, height: f32) -> bool {
        use std::sync::atomic::Ordering;

        // Initialize renderer if not already done
        if let Ok(mut renderer) = LAYERS_RENDERER.write() {
            if renderer.is_none() {
                *renderer = Some(crate::rendering::LayersRenderer::new(width, height));

                // Spawn renderer thread
                let exit_flag = RENDERER_EXIT_FLAG.clone();
                let thread = std::thread::spawn(move || {
                    while !exit_flag.load(Ordering::Relaxed) {
                        // Update the layers renderer
                        if AppContext::layers_renderer(|renderer| {
                            // println!("Updating layers renderer...");
                            renderer.update();
                        })
                        .is_some()
                        {
                            // Small sleep for ~60fps
                            std::thread::sleep(std::time::Duration::from_millis(12));
                        } else {
                            // Engine was disabled, exit thread
                            break;
                        }
                    }
                });

                // Store the thread handle
                if let Ok(mut handle) = RENDERER_THREAD.lock() {
                    *handle = Some(thread);
                }
            }
            true
        } else {
            false
        }
    }

    /// Store EGL resources for a surface (cold path)
    pub fn insert_egl_resources(
        surface_id: ObjectId,
        resources: crate::rendering::EglSurfaceResources,
    ) {
        EGL_RESOURCES.with(|map| {
            map.borrow_mut().insert(surface_id.clone(), resources);
        });
    }

    /// Access EGL resources for a surface (cold path - only for commit, resize, etc.)
    pub fn with_egl_resources<R, F>(surface_id: &ObjectId, f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::rendering::EglSurfaceResources) -> R,
    {
        EGL_RESOURCES.with(|map| map.borrow_mut().get_mut(surface_id).map(f))
    }

    /// Remove EGL resources when surface is destroyed
    pub fn remove_egl_resources(surface_id: &ObjectId) {
        // Use try_with to gracefully handle TLS destruction during app shutdown
        let _ = EGL_RESOURCES.try_with(|map| {
            map.borrow_mut().remove(surface_id);
        });
    }

    /// Initialize the context pointer and queue handle
    /// Called by AppRunner during initialization
    pub(super) fn init<A: App + 'static>(
        context_data: &AppContextData,
        queue_handle: &QueueHandle<AppData<A>>,
    ) {
        // Store pointer to context data
        APP_CONTEXT_PTR.with(|ptr| {
            *ptr.borrow_mut() = Some(context_data as *const AppContextData);
        });

        // Store the typed queue handle by value (QueueHandle is Clone)
        let qh_clone = queue_handle.clone();
        TYPED_QUEUE_HANDLE.with(|qh| {
            *qh.borrow_mut() = Some(Box::new(qh_clone));
        });

        // Store a closure that can request frames (clone the queue handle)
        let qh_clone = queue_handle.clone();
        FRAME_REQUEST_FN.with(|frame_fn| {
            *frame_fn.borrow_mut() = Some(Box::new(move |surface: &wl_surface::WlSurface| {
                surface.frame(&qh_clone, surface.clone());
            }));
        });
    }

    /// Get the typed queue handle (type determined by the AppRunner)
    /// Returns a reference with 'static lifetime - valid for the event loop duration
    pub fn queue_handle_typed<A: App + 'static>() -> &'static QueueHandle<AppData<A>> {
        TYPED_QUEUE_HANDLE.with(|qh| {
            let boxed_any = qh.borrow();
            let any_ref = boxed_any.as_ref().expect("AppContext not initialized");

            // Downcast to the concrete type
            let qh_ref = any_ref
                .downcast_ref::<QueueHandle<AppData<A>>>()
                .expect("Queue handle type mismatch - wrong App type?");

            // SAFETY: We extend the lifetime to 'static
            // The QueueHandle is stored in TLS for the duration of the event loop
            unsafe { &*(qh_ref as *const QueueHandle<AppData<A>>) }
        })
    }

    /// Get the queue handle for AppRunnerDefault (convenience method)
    ///
    /// This is the recommended way to get a queue handle when using AppRunnerDefault.
    /// It avoids needing to specify generic type parameters.
    pub fn queue_handle() -> &'static QueueHandle<AppData<super::DefaultApp>> {
        Self::queue_handle_typed::<super::DefaultApp>()
    }

    /// Get the current surface configure event (WindowConfigure is a SurfaceConfigure)
    /// Called by surface components during configure handling
    /// Returns (surface_id, configure, serial) so handlers can check if it's for their surface  
    pub fn current_surface_configure() -> Option<(ObjectId, WindowConfigure, u32)> {
        CURRENT_CONFIGURE.with(|cfg| cfg.borrow().clone())
    }

    /// Internal: Register a configure handler
    /// Called by surface components to automatically handle configuration
    pub fn register_configure_handler<F>(handler: F)
    where
        F: FnMut() + 'static,
    {
        CONFIGURE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().push(Box::new(handler));
        });
    }

    /// Register a callback for pointer events
    /// The callback will be called for all pointer events
    pub fn register_pointer_callback<F>(callback: F)
    where
        F: FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent]) + 'static,
    {
        // Add to pending callbacks to avoid borrowing issues during event dispatch
        PENDING_POINTER_CALLBACKS.with(|pending| {
            pending.borrow_mut().push(Box::new(callback));
        });

        // Move pending callbacks to active list after event loop iteration
        PENDING_POINTER_CALLBACKS.with(|pending| {
            let mut pending_vec = pending.borrow_mut();
            POINTER_CALLBACKS.with(|active| {
                active.borrow_mut().append(&mut pending_vec);
            });
        });
    }

    /// Register a window for automatic rendering updates
    pub fn register_window(window: crate::components::window::Window) {
        WINDOWS.with(|windows| {
            windows.borrow_mut().push(window);
        });
    }

    /// Register a popup configure callback
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

    /// Register a popup done callback
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

    /// Register a layer shell configure callback
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

    /// Register a frame callback for a surface
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

    /// Request a frame for a surface (calls the frame request function)
    pub fn request_frame(surface: &wl_surface::WlSurface) {
        FRAME_REQUEST_FN.with(|frame_fn| {
            if let Some(f) = frame_fn.borrow().as_ref() {
                f(surface);
            }
        });
    }

    /// Alias for register_layer_shell_configure_callback (for compatibility)
    pub fn register_layer_configure_callback<F>(surface_id: ObjectId, callback: F)
    where
        F: FnMut(i32, i32, u32) + 'static,
    {
        Self::register_layer_shell_configure_callback(surface_id, callback);
    }

    /// Request initial frame (alias for request_frame)
    pub fn request_initial_frame(surface: &wl_surface::WlSurface) {
        Self::request_frame(surface);
    }

    /// Update all registered windows (called by event loop)
    pub fn update_windows() {
        WINDOWS.with(|windows| {
            for window in windows.borrow_mut().iter_mut() {
                window.update();
            }
        });
    }

    /// Clear all global context state (called on shutdown)
    pub fn clear() {
        use std::sync::atomic::Ordering;

        // Signal renderer thread to exit
        RENDERER_EXIT_FLAG.store(true, Ordering::Relaxed);

        // Wait for renderer thread to finish
        if let Ok(mut handle) = RENDERER_THREAD.lock() {
            if let Some(thread) = handle.take() {
                let _ = thread.join();
            }
        }

        // Clean up Skia context before clearing other state
        SHARED_SKIA_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = None;
        });
        // TODO: Re-enable EGL display cleanup when feature is available
        // let _ = EGL_DISPLAY.try_with(|d| {
        //     if let Some(display) = d.borrow_mut().take() {
        //         let egl = khronos_egl::Instance::new(khronos_egl::Static);
        //         let _ = egl.terminate(display);
        //     }
        // });

        APP_CONTEXT_PTR.with(|ptr| {
            *ptr.borrow_mut() = None;
        });
        CONFIGURE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().clear();
        });
        CURRENT_CONFIGURE.with(|cfg| {
            *cfg.borrow_mut() = None;
        });
        WINDOWS.with(|windows| {
            windows.borrow_mut().clear();
        });
        POINTER_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().clear();
        });
        if let Ok(mut renderer) = LAYERS_RENDERER.write() {
            *renderer = None;
        }
        POPUP_CONFIGURE_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().clear();
        });
        FRAME_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().clear();
        });
        FRAME_REQUEST_FN.with(|frame_fn| {
            *frame_fn.borrow_mut() = None;
        });

        // Reset exit flag for potential reuse
        RENDERER_EXIT_FLAG.store(false, Ordering::Relaxed);
    }
}
