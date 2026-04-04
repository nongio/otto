//! AppRunner - High-level application framework
//!
//! Hides all Wayland boilerplate and provides a simple trait-based API
//! for creating window-based applications.

pub mod context;
mod handlers;

pub use context::AppContext;
pub use smithay_client_toolkit::seat::keyboard::KeyEvent;
use wayland_client::backend::ObjectId;

use crate::protocols::{
    otto_dock_item_v1, otto_dock_manager_v1, otto_style_transaction_v1,
    otto_surface_style_manager_v1, otto_surface_style_v1, otto_timing_function_v1,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::xdg::{
        popup::{Popup, PopupConfigure, PopupHandler},
        window::{Window as StkWindow, WindowConfigure, WindowHandler},
        XdgShell,
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1, zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
};

// Re-export context items
use context::AppContextData;

/// The App trait - implement this to create a runnable application
///
/// This trait defines the lifecycle of your application:
/// - `on_app_ready()`: Called once when the app launches
/// - `on_configure()`: Called when a window configure event occurs
/// - `on_close()`: Called when the user tries to close the app
pub trait App {
    fn on_start(&mut self) {
        // Default implementation does nothing - override if you want a startup callback
    }
    /// Called when the app is ready to run
    /// This is where you create your window and setup your UI
    fn on_app_ready(&mut self, ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>>;

    /// Called when a window configure event occurs
    /// Override this to handle window configuration
    fn on_configure(&mut self, _ctx: &AppContext, _configure: WindowConfigure, _serial: u32) {
        // Default: do nothing
    }

    /// Called when a layer shell surface configure event occurs
    /// Override this to handle layer surface configuration
    fn on_configure_layer(&mut self, _ctx: &AppContext, _width: i32, _height: i32, _serial: u32) {
        // Default: do nothing
    }

    /// Called when the user requests to close the app
    /// Return `true` to allow closing, `false` to prevent it
    fn on_close(&mut self) -> bool {
        true
    }

    /// Called when a keyboard event occurs
    /// Override this to handle keyboard input
    /// `serial` is the input serial from the Wayland compositor - save this to use for popup grabs!
    fn on_keyboard_event(
        &mut self,
        _ctx: &AppContext,
        _key: u32,
        _state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // Default: do nothing
    }

    /// Called when a keyboard event occurs, with full key event data
    ///
    /// This provides the keysym and UTF-8 text in addition to the raw keycode.
    /// Override this instead of `on_keyboard_event` when you need character input
    /// (e.g., for text fields). The default implementation delegates to `on_keyboard_event`.
    fn on_key_event(
        &mut self,
        ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        self.on_keyboard_event(ctx, event.raw_code, state, serial);
    }

    /// Called when keyboard focus is lost from a surface
    /// Override this to handle focus loss (e.g., close menus)
    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        // Default: do nothing
    }

    /// Called when a pointer event occurs
    fn on_pointer_event(&mut self, _ctx: &AppContext, _events: &[PointerEvent]) {
        // Default: do nothing
    }

    /// Called when the compositor requests to show a dock menu at coordinates (x, y)
    fn on_dock_menu_requested(&mut self, _ctx: &AppContext, _x: i32, _y: i32) {
        // Default: do nothing
    }

    /// Called once per event loop iteration, after dispatching Wayland events.
    /// Use for periodic checks (timers, polling state changes) without frame callbacks.
    fn on_update(&mut self, _ctx: &AppContext) {
        // Default: do nothing
    }

    /// Maximum time to sleep between `on_update` calls.
    ///
    /// Return `Some(duration)` to ensure `on_update` fires at least every `duration`
    /// (e.g., for clock ticks). Return `None` to block indefinitely until a Wayland
    /// event or `AppContext::request_wakeup()` wakes the loop.
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        None
    }
}

/// DefaultApp - Wrapper for using App trait objects with AppRunner
///
/// This type allows AppRunner to work without generics by wrapping
/// any App implementation in a concrete type via `Box<dyn App>`.
pub struct DefaultApp {
    inner: Box<dyn App>,
}

impl DefaultApp {
    /// Create a new DefaultApp wrapping any App implementation
    pub fn new<A: App + 'static>(app: A) -> Self {
        Self {
            inner: Box::new(app),
        }
    }
}

// Implement App for DefaultApp by delegating to the inner trait object
impl App for DefaultApp {
    fn on_start(&mut self) {
        self.inner.on_start();
    }
    fn on_app_ready(&mut self, ctx: &AppContext) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.on_app_ready(ctx)
    }

    fn on_configure(&mut self, ctx: &AppContext, configure: WindowConfigure, serial: u32) {
        self.inner.on_configure(ctx, configure, serial)
    }

    fn on_configure_layer(&mut self, ctx: &AppContext, width: i32, height: i32, serial: u32) {
        self.inner.on_configure_layer(ctx, width, height, serial)
    }

    fn on_close(&mut self) -> bool {
        self.inner.on_close()
    }

    fn on_keyboard_event(
        &mut self,
        ctx: &AppContext,
        key: u32,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        self.inner.on_keyboard_event(ctx, key, state, serial)
    }

    fn on_key_event(
        &mut self,
        ctx: &AppContext,
        event: &KeyEvent,
        state: wl_keyboard::KeyState,
        serial: u32,
    ) {
        self.inner.on_key_event(ctx, event, state, serial)
    }

    fn on_keyboard_leave(&mut self, ctx: &AppContext, surface: &wl_surface::WlSurface) {
        self.inner.on_keyboard_leave(ctx, surface)
    }

    fn on_dock_menu_requested(&mut self, ctx: &AppContext, x: i32, y: i32) {
        self.inner.on_dock_menu_requested(ctx, x, y)
    }
    fn on_pointer_event(&mut self, ctx: &AppContext, events: &[PointerEvent]) {
        self.inner.on_pointer_event(ctx, events)
    }
    fn on_update(&mut self, ctx: &AppContext) {
        self.inner.on_update(ctx)
    }
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        self.inner.idle_timeout()
    }
}

/// AppRunner - manages the Wayland event loop and application lifecycle (no generics version)
///
/// This is the recommended version for most use cases. It uses AppRunner<DefaultApp> internally
/// to avoid complex generic types in your code.
pub struct AppRunner {
    runner: AppRunnerWithType<DefaultApp>,
}

impl AppRunner {
    /// Create a new AppRunner with your App instance
    pub fn new<A: App + 'static>(app: A) -> Self {
        Self {
            runner: AppRunnerWithType::new(DefaultApp::new(app)),
        }
    }

    /// Initialize the application
    ///
    /// This method:
    /// 1. Connects to Wayland
    /// 2. Initializes all required protocols (compositor, xdg-shell, etc.)
    /// 3. Calls your app's `on_app_ready()` method
    ///
    /// Returns an initialized runner ready to start the event loop.
    pub fn init(self) -> Result<AppRunnerDefaultInitialized, Box<dyn std::error::Error>> {
        Ok(AppRunnerDefaultInitialized {
            runner: self.runner.init()?,
        })
    }

    /// Run the application (init + event loop)
    ///
    /// This is a convenience method that calls `init()` then `run()`.
    /// For more control, call `init()` and `run()` separately.
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        self.init()?.run()
    }
}

/// Initialized AppRunner ready to run the event loop
pub struct AppRunnerDefaultInitialized {
    runner: AppRunnerInitialized<DefaultApp>,
}

impl AppRunnerDefaultInitialized {
    /// Run the event loop until the app exits
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        self.runner.run()
    }
}

/// AppRunner - manages the Wayland event loop and application lifecycle (generic version)
///
/// This is the generic version that keeps your App type in the event queue type.
/// Most users should use `AppRunnerDefault` instead unless they need the generic version
/// for specific use cases.
pub struct AppRunnerWithType<A: App + 'static> {
    app: A,
}

impl<A: App + 'static> AppRunnerWithType<A> {
    /// Create a new AppRunner with your App instance
    pub fn new(app: A) -> Self {
        Self { app }
    }

    /// Initialize the application
    ///
    /// This method:
    /// 1. Connects to Wayland
    /// 2. Initializes all required protocols (compositor, xdg-shell, etc.)
    /// 3. Calls your app's `on_app_ready()` method
    ///
    /// Returns an initialized runner ready to start the event loop.
    pub fn init(self) -> Result<AppRunnerInitialized<A>, Box<dyn std::error::Error>> {
        // Connect to Wayland
        let conn = Connection::connect_to_env()?;
        let (globals, event_queue) = registry_queue_init::<AppData<A>>(&conn)?;
        let qh = event_queue.handle();

        // Initialize Wayland protocol states
        let compositor_state = CompositorState::bind(&globals, &qh)?;
        let xdg_shell_state = XdgShell::bind(&globals, &qh)?;
        let shm_state = Shm::bind(&globals, &qh)?;
        let seat_state = SeatState::new(&globals, &qh);
        let output_state = OutputState::new(&globals, &qh);
        let registry_state = RegistryState::new(&globals);
        let surface_style_manager = globals.bind(&qh, 1..=1, ()).ok();
        let wlr_layer_shell: Option<ZwlrLayerShellV1> = globals.bind(&qh, 1..=4, ()).ok();
        let otto_dock_manager = globals.bind(&qh, 1..=1, ()).ok();
        let subcompositor = globals.bind(&qh, 1..=1, ()).ok();

        // Get display pointer for creating surfaces
        let display_ptr = conn.backend().display_ptr() as *mut std::ffi::c_void;

        // Note: Layers renderer is now initialized via AppContext::enable_layer_engine()

        // Move states into the context data structure (box it to prevent movement)
        let context = Box::new(AppContextData {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            output_state,
            surface_style_manager,
            wlr_layer_shell,
            subcompositor,
            otto_dock_manager,
            display_ptr,
        });

        // Create the internal app data
        let mut app_data = AppData {
            app: self.app,
            registry_state,
            context_data: context,
            exit: false,
        };

        // Initialize AppContext with context data pointer and queue handle
        // Box ensures context_data won't move even when app_data is moved
        AppContext::init::<A>(&app_data.context_data, &qh);

        // Start background watchers if a tokio runtime is available
        if tokio::runtime::Handle::try_current().is_ok() {
            crate::color_scheme::spawn_color_scheme_watcher();
            crate::icon_theme::spawn_icon_theme_watcher();
        } else {
            tracing::debug!("no tokio runtime found, skipping portal watchers");
        }

        // Call the app's ready callback
        let ctx = AppContext::new(&app_data.context_data);
        app_data.app.on_app_ready(&ctx)?;

        Ok(AppRunnerInitialized {
            conn,
            event_queue,
            app_data,
        })
    }

    /// Run the application (init + event loop)
    ///
    /// This is a convenience method that calls `init()` then `run()`.
    /// For more control, call `init()` and `run()` separately.
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        self.init()?.run()
    }
}

/// Initialized AppRunner ready to run the event loop
pub struct AppRunnerInitialized<A: App + 'static> {
    conn: Connection,
    event_queue: wayland_client::EventQueue<AppData<A>>,
    app_data: AppData<A>,
}

impl<A: App + 'static> AppRunnerInitialized<A> {
    /// Run the event loop until the app exits
    ///
    /// Uses `prepare_read` + `poll` so the loop can be woken by:
    /// - Wayland events (compositor, input, frame callbacks)
    /// - `AppContext::request_wakeup()` from background threads / tokio tasks
    /// - `App::idle_timeout()` expiry (e.g., clock ticks)
    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::fd::AsRawFd;

        // Ensure wakeup pipe exists before entering the loop.
        let wake_fd = AppContext::wakeup_read_fd();

        while !self.app_data.exit {
            // 1. Drain any events already queued (no I/O).
            self.event_queue.dispatch_pending(&mut self.app_data)?;
            self.conn.flush()?;

            AppContext::update_windows();

            let ctx = AppContext::new(&self.app_data.context_data);
            self.app_data.app.on_update(&ctx);

            if self.app_data.exit {
                break;
            }

            // 2. Prepare to block for the next batch of events.
            let guard = loop {
                match self.event_queue.prepare_read() {
                    Some(guard) => break guard,
                    None => {
                        // Internal queue still has pending events — drain first.
                        self.event_queue.dispatch_pending(&mut self.app_data)?;
                    }
                }
            };

            let wl_fd = guard.connection_fd().as_raw_fd();
            let timeout_ms = self
                .app_data
                .app
                .idle_timeout()
                .map(|d| d.as_millis().min(i32::MAX as u128) as i32)
                .unwrap_or(-1); // -1 = block forever

            let mut fds = [
                libc::pollfd {
                    fd: wl_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];

            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };

            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                tracing::error!("poll error: {err}");
                break;
            }

            if n > 0 && fds[1].revents & libc::POLLIN != 0 {
                AppContext::drain_wakeup();
            }

            if n > 0 && fds[0].revents & libc::POLLIN != 0 {
                // Data arrived on the Wayland fd — read & enqueue.
                if let Err(e) = guard.read() {
                    tracing::error!("wayland read error: {e}");
                    break;
                }
            }
            // Otherwise (timeout or only wakeup), guard drops and cancels the read.
        }

        AppContext::clear();
        Ok(())
    }
}

/// Internal app data that wraps the user's App and handles Wayland protocols
pub struct AppData<A: App + 'static> {
    app: A,
    registry_state: RegistryState,
    pub(super) context_data: Box<AppContextData>, // Box prevents movement after pointer is stored
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
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        use wayland_client::Proxy;

        let has_callback = AppContext::has_frame_callback(&surface.id());

        if has_callback {
            AppContext::request_frame(surface);
        }

        AppContext::dispatch_frame_callback(&surface.id());
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
        _window: &StkWindow,
        configure: WindowConfigure,
        serial: u32,
    ) {
        AppContext::set_current_configure(ObjectId::null(), configure.clone(), serial);
        AppContext::dispatch_configure_handlers();

        let ctx = AppContext::new(&self.context_data);
        self.app.on_configure(&ctx, configure, serial);

        AppContext::clear_current_configure();
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
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && self
                .context_data
                .seat_state
                .get_keyboard(qh, &seat, None)
                .is_err()
        {
            eprintln!("Failed to create keyboard");
        }

        if capability == Capability::Pointer
            && self.context_data.seat_state.get_pointer(qh, &seat).is_err()
        {
            eprintln!("Failed to create pointer");
        }
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

impl<A: App + 'static> KeyboardHandler for AppData<A> {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        let ctx = AppContext::new(&self.context_data);
        self.app.on_keyboard_leave(&ctx, surface);
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        let ctx = AppContext::new(&self.context_data);
        self.app
            .on_key_event(&ctx, &event, wl_keyboard::KeyState::Pressed, serial);
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        let ctx = AppContext::new(&self.context_data);
        self.app
            .on_key_event(&ctx, &event, wl_keyboard::KeyState::Released, serial);
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _layout: u32,
    ) {
    }
}

impl<A: App + 'static> PointerHandler for AppData<A> {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        AppContext::dispatch_pointer_callbacks(events);
        let ctx = AppContext::new(&self.context_data);
        self.app.on_pointer_event(&ctx, events);
    }
}

impl<A: App + 'static> PopupHandler for AppData<A> {
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        popup: &Popup,
        config: PopupConfigure,
    ) {
        use wayland_client::Proxy;
        AppContext::dispatch_popup_configure(&popup.wl_surface().id(), config.serial);
    }

    fn done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, popup: &Popup) {
        use wayland_client::Proxy;
        AppContext::dispatch_popup_done(&popup.wl_surface().id());
    }
}

impl<A: App + 'static> ProvidesRegistryState for AppData<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl<A: App + 'static> wayland_client::Dispatch<wl_keyboard::WlKeyboard, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: wayland_client::WEnum::Value(state_val),
            ..
        } = event
        {
            let ctx = AppContext::new(&state.context_data);
            state.app.on_keyboard_event(&ctx, key, state_val, 0);
        }
    }
}

impl<A: App + 'static> wayland_client::Dispatch<ZwlrLayerSurfaceV1, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;
        use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event;

        match event {
            Event::Configure {
                serial,
                width,
                height,
            } => {
                tracing::debug!("Layer surface configure: {}x{}", width, height);
                AppContext::dispatch_layer_configure(
                    &proxy.id(),
                    width as i32,
                    height as i32,
                    serial,
                );

                let ctx = AppContext::new(&state.context_data);
                state
                    .app
                    .on_configure_layer(&ctx, width as i32, height as i32, serial);
            }
            Event::Closed => {
                tracing::debug!("Layer surface closed");
            }
            _ => {}
        }
    }
}

smithay_client_toolkit::delegate_compositor!(@<A: App> AppData<A>);
smithay_client_toolkit::delegate_output!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_shm!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_seat!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_keyboard!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_pointer!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_xdg_shell!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_xdg_window!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_xdg_popup!(@<A: App + 'static> AppData<A>);
smithay_client_toolkit::delegate_registry!(@<A: App + 'static> AppData<A>);

// ============================================================================
// Otto Protocol Handlers (merged from wayland_handlers.rs)
// ============================================================================

// SC Layer protocol handlers - must be generic over A: App to match AppData<A>
impl<A: App + 'static> Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
    for AppData<A>
{
    fn event(
        _state: &mut Self,
        _proxy: &otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1,
        _event: otto_surface_style_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl<A: App + 'static> Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()> for AppData<A> {
    fn event(
        _state: &mut Self,
        _proxy: &otto_surface_style_v1::OttoSurfaceStyleV1,
        _event: otto_surface_style_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl<A: App + 'static> Dispatch<otto_style_transaction_v1::OttoStyleTransactionV1, ()>
    for AppData<A>
{
    fn event(
        _state: &mut Self,
        proxy: &otto_style_transaction_v1::OttoStyleTransactionV1,
        event: otto_style_transaction_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;

        match event {
            otto_style_transaction_v1::Event::Completed => {
                tracing::debug!("Transaction completed event received");
                AppContext::dispatch_transaction_completed(&proxy.id());
            }
        }
    }
}

impl<A: App + 'static> Dispatch<otto_timing_function_v1::OttoTimingFunctionV1, ()> for AppData<A> {
    fn event(
        _state: &mut Self,
        _proxy: &otto_timing_function_v1::OttoTimingFunctionV1,
        _event: otto_timing_function_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl<A: App + 'static> Dispatch<otto_dock_manager_v1::OttoDockManagerV1, ()> for AppData<A> {
    fn event(
        _state: &mut Self,
        _proxy: &otto_dock_manager_v1::OttoDockManagerV1,
        _event: otto_dock_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl<A: App + 'static> Dispatch<otto_dock_item_v1::OttoDockItemV1, ()> for AppData<A> {
    fn event(
        _state: &mut Self,
        _proxy: &otto_dock_item_v1::OttoDockItemV1,
        _event: otto_dock_item_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

// Delegate noop for protocols we don't handle
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_client::protocol::wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_client::protocol::wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore ZwlrLayerShellV1);
