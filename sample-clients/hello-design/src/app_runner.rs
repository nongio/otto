//! AppRunner - High-level application framework
//!
//! Hides all Wayland boilerplate and provides a simple trait-based API
//! for creating window-based applications.

use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::xdg::{
        window::{Window as StkWindow, WindowConfigure, WindowHandler},
        XdgShell,
    },
    shm::{Shm, ShmHandler},
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use wayland_client::backend::ObjectId;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, QueueHandle,
};

thread_local! {
    static APP_CONTEXT: RefCell<Option<RawAppContext>> = RefCell::new(None);
}

// Store configure handlers registered by surface components
thread_local! {
    static CONFIGURE_HANDLERS: RefCell<Vec<Box<dyn FnMut()>>> = RefCell::new(Vec::new());
}

// Store the current surface configure event being processed
thread_local! {
    static CURRENT_CONFIGURE: RefCell<Option<(ObjectId, WindowConfigure, u32)>> = RefCell::new(None);
}

// Store the shared SkiaContext
thread_local! {
    static SHARED_SKIA_CONTEXT: RefCell<Option<crate::rendering::SkiaContext>> = RefCell::new(None);
}

// Store EGL resources for all surfaces (cold path storage)
thread_local! {
    static EGL_RESOURCES: RefCell<HashMap<ObjectId, crate::rendering::EglSurfaceResources>> = RefCell::new(HashMap::new());
}

/// Raw pointers to app context data
struct RawAppContext {
    compositor_state_ptr: *const CompositorState,
    xdg_shell_state_ptr: *const XdgShell,
    shm_state_ptr: *mut Shm,
    seat_state_ptr: *mut SeatState,
    output_state_ptr: *mut OutputState,
    sc_layer_shell_ptr: *const Option<sc_layer_shell_v1::ScLayerShellV1>,
    display_ptr: *mut std::ffi::c_void,
    queue_handle_ptr: *const std::ffi::c_void, // Type-erased QueueHandle pointer
}
// Store the actual typed queue handle in a separate thread-local
// This allows us to return it with the proper type without requiring turbofish
thread_local! {
    static TYPED_QUEUE_HANDLE: RefCell<Option<*const std::ffi::c_void>> = RefCell::new(None);
}
unsafe impl Send for RawAppContext {}
unsafe impl Sync for RawAppContext {}

/// Internal storage for app context - owns the Wayland states
struct AppContextData<A: App + 'static> {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShell,
    shm_state: Shm,
    seat_state: SeatState,
    output_state: OutputState,
    sc_layer_shell: Option<sc_layer_shell_v1::ScLayerShellV1>,
    display_ptr: *mut std::ffi::c_void,
    _phantom: PhantomData<A>,
}

/// Global application context - provides access to Wayland states
///
/// This is accessible from within your App methods.
pub struct AppContext;

impl AppContext {
    /// Get compositor state
    pub fn compositor_state() -> &'static CompositorState {
        APP_CONTEXT.with(|ctx| {
            let ctx_ref = ctx.borrow();
            let raw = ctx_ref.as_ref().expect("AppContext not initialized");
            unsafe { &*raw.compositor_state_ptr }
        })
    }

    /// Get XDG shell state
    pub fn xdg_shell_state() -> &'static XdgShell {
        APP_CONTEXT.with(|ctx| {
            let ctx_ref = ctx.borrow();
            let raw = ctx_ref.as_ref().expect("AppContext not initialized");
            unsafe { &*raw.xdg_shell_state_ptr }
        })
    }

    /// Get SC layer shell if available
    pub fn sc_layer_shell() -> Option<&'static sc_layer_shell_v1::ScLayerShellV1> {
        APP_CONTEXT.with(|ctx| {
            let ctx_ref = ctx.borrow();
            let raw = ctx_ref.as_ref().expect("AppContext not initialized");
            unsafe { (*raw.sc_layer_shell_ptr).as_ref() }
        })
    }

    /// Get display pointer
    pub fn display_ptr() -> *mut std::ffi::c_void {
        APP_CONTEXT.with(|ctx| {
            let ctx_ref = ctx.borrow();
            let raw = ctx_ref.as_ref().expect("AppContext not initialized");
            raw.display_ptr
        })
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
        SHARED_SKIA_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(context);
        });
    }

    /// Store EGL resources for a surface (cold path)
    pub fn insert_egl_resources(
        surface_id: ObjectId,
        resources: crate::rendering::EglSurfaceResources,
    ) {
        EGL_RESOURCES.with(|map| {
            map.borrow_mut().insert(surface_id, resources);
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
        EGL_RESOURCES.with(|map| {
            map.borrow_mut().remove(surface_id);
        });
    }

    fn set<A: App + 'static>(context: &AppContextData<A>, queue_handle: &QueueHandle<AppData<A>>) {
        let raw = RawAppContext {
            compositor_state_ptr: &context.compositor_state as *const _,
            xdg_shell_state_ptr: &context.xdg_shell_state as *const _,
            shm_state_ptr: &context.shm_state as *const _ as *mut _,
            seat_state_ptr: &context.seat_state as *const _ as *mut _,
            output_state_ptr: &context.output_state as *const _ as *mut _,
            sc_layer_shell_ptr: &context.sc_layer_shell as *const _,
            display_ptr: context.display_ptr,
            queue_handle_ptr: queue_handle as *const _ as *const std::ffi::c_void,
        };
        APP_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(raw);
        });
        // Store the typed queue handle separately
        TYPED_QUEUE_HANDLE.with(|qh| {
            *qh.borrow_mut() = Some(queue_handle as *const _ as *const std::ffi::c_void);
        });
    }

    /// Get the typed queue handle (type determined by the AppRunner)
    pub fn queue_handle<A: App + 'static>() -> &'static QueueHandle<AppData<A>> {
        TYPED_QUEUE_HANDLE.with(|qh| {
            let ptr = qh.borrow().expect("AppContext not initialized");
            unsafe { &*(ptr as *const QueueHandle<AppData<A>>) }
        })
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

    fn clear() {
        APP_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = None;
        });
        TYPED_QUEUE_HANDLE.with(|qh| {
            *qh.borrow_mut() = None;
        });
        CONFIGURE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().clear();
        });
    }
}

/// The App trait - implement this to create a runnable application
///
/// This trait defines the lifecycle of your application:
/// - `on_app_ready()`: Called once when the app launches
/// - `on_configure()`: Called when a window configure event occurs
/// - `on_close()`: Called when the user tries to close the app
pub trait App {
    /// Called when the app is ready to run
    /// This is where you create your window and setup your UI
    fn on_app_ready(&mut self) -> Result<(), Box<dyn std::error::Error>>;

    /// Called when a window configure event occurs
    /// Override this to handle window configuration
    fn on_configure(&mut self, _configure: WindowConfigure, _serial: u32) {
        // Default: do nothing
    }

    /// Called when the user requests to close the app
    /// Return `true` to allow closing, `false` to prevent it
    fn on_close(&mut self) -> bool;
}

/// AppRunner - manages the Wayland event loop and application lifecycle
pub struct AppRunner<A: App + 'static> {
    app: A,
}

impl<A: App + 'static> AppRunner<A> {
    /// Create a new AppRunner with your App instance
    pub fn new(app: A) -> Self {
        Self { app }
    }

    /// Run the application
    ///
    /// This method:
    /// 1. Connects to Wayland
    /// 2. Initializes all required protocols (compositor, xdg-shell, etc.)
    /// 3. Calls your app's `on_app_ready()` method
    /// 4. Runs the event loop until the app exits
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        // Connect to Wayland
        let conn = Connection::connect_to_env()?;
        let (globals, mut event_queue) = registry_queue_init::<AppData<A>>(&conn)?;
        let qh = event_queue.handle();

        // Initialize Wayland protocol states
        let compositor_state = CompositorState::bind(&globals, &qh)?;
        let xdg_shell_state = XdgShell::bind(&globals, &qh)?;
        let shm_state = Shm::bind(&globals, &qh)?;
        let seat_state = SeatState::new(&globals, &qh);
        let output_state = OutputState::new(&globals, &qh);
        let registry_state = RegistryState::new(&globals);
        let sc_layer_shell = globals.bind(&qh, 1..=1, ()).ok();

        // Get display pointer for creating surfaces
        let display_ptr = conn.backend().display_ptr() as *mut std::ffi::c_void;

        // Move states into the context data structure
        let context = AppContextData {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            output_state,
            sc_layer_shell,
            display_ptr,
            _phantom: PhantomData,
        };

        // Create the internal app data (now minimal, just holds app and registry)
        let mut app_data = AppData {
            app: self.app,
            registry_state,
            context_data: context,
            exit: false,
        };

        // Set up the global context with pointers - type A is captured here
        AppContext::set::<A>(&app_data.context_data, &qh);

        // Call the app's ready callback
        app_data.app.on_app_ready()?;

        // Run the event loop
        println!("Starting application event loop...");
        while !app_data.exit {
            event_queue.blocking_dispatch(&mut app_data)?;
        }

        // Clean up global context
        AppContext::clear();

        Ok(())
    }
}

/// Internal app data that wraps the user's App and handles Wayland protocols
pub struct AppData<A: App + 'static> {
    app: A,
    registry_state: RegistryState,
    context_data: AppContextData<A>,
    exit: bool,
}

// Wayland protocol handler implementations
impl<A: App + 'static> CompositorHandler for AppData<A> {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }
    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl<A: App + 'static> OutputHandler for AppData<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.context_data.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl<A: App + 'static> WindowHandler for AppData<A> {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &StkWindow) {
        // Ask the app if it wants to close
        if self.app.on_close() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &StkWindow,
        configure: WindowConfigure,
        serial: u32,
    ) {
        // Store configure event with window's surface ID for handlers to check
        use smithay_client_toolkit::shell::WaylandSurface;
        use wayland_client::Proxy;
        let surface_id = window.wl_surface().id();
        CURRENT_CONFIGURE.with(|cfg| {
            *cfg.borrow_mut() = Some((surface_id, configure, serial));
        });

        // Call all registered configure handlers
        CONFIGURE_HANDLERS.with(|handlers| {
            for handler in handlers.borrow_mut().iter_mut() {
                handler();
            }
        });

        // Get the configure back from thread-local storage to pass to app
        let (_surface_id, configure, serial) =
            CURRENT_CONFIGURE.with(|cfg| cfg.borrow_mut().take().expect("Configure was just set"));

        // Forward to app's configure handler
        self.app.on_configure(configure, serial);
    }
}

impl<A: App + 'static> SeatHandler for AppData<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.context_data.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }
    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }
    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl<A: App + 'static> ShmHandler for AppData<A> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.context_data.shm_state
    }
}

impl<A: App + 'static> ProvidesRegistryState for AppData<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

// Delegate macros for protocol handling
wayland_client::delegate_noop!(@<A: App> AppData<A>: ignore wl_keyboard::WlKeyboard);
wayland_client::delegate_noop!(@<A: App> AppData<A>: ignore wl_pointer::WlPointer);
wayland_client::delegate_noop!(@<A: App> AppData<A>: ignore sc_layer_shell_v1::ScLayerShellV1);
wayland_client::delegate_noop!(@<A: App> AppData<A>: ignore sc_layer_v1::ScLayerV1);

smithay_client_toolkit::delegate_compositor!(@<A: App> AppData<A>);
smithay_client_toolkit::delegate_output!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_shm!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_seat!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_xdg_shell!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_xdg_window!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_registry!(@<A: App + 'static> AppData<A>);
