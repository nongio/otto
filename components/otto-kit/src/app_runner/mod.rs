//! AppRunner - High-level application framework
//!
//! Hides all Wayland boilerplate and provides a simple trait-based API
//! for creating window-based applications.

pub mod context;
mod handlers;

pub use context::AppContext;
pub use smithay_client_toolkit::seat::keyboard::Modifiers;

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
        keyboard::{KeyEvent, KeyboardHandler, Keysym},
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
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1::{self, ZwpPointerGestureHoldV1},
    zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1},
    zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
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

    /// Called when the compositor configures an `ext-session-lock-v1` surface.
    /// The surface itself acks the configure and resizes its canvas; this is
    /// only the app's chance to lay content out and paint.
    fn on_configure_lock_surface(
        &mut self,
        _ctx: &AppContext,
        _lock_surface: &wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        _width: i32,
        _height: i32,
        _serial: u32,
    ) {
        // Default: do nothing
    }

    /// The session is locked: nothing of it is visible on any output. Until
    /// this arrives a locker's surfaces may be drawn but the desktop behind
    /// them has not necessarily left the screen.
    fn on_session_locked(&mut self, _ctx: &AppContext) {
        // Default: do nothing
    }

    /// The lock request was refused, or the lock has ended. The session was
    /// never hidden, and the lock object must not be used again.
    fn on_session_lock_finished(&mut self, _ctx: &AppContext) {
        // Default: do nothing
    }

    /// Called when the user requests to close the app
    /// Return `true` to allow closing, `false` to prevent it
    fn on_close(&mut self) -> bool {
        true
    }

    /// Describes a surface for assistive technologies — a screen reader reads
    /// what this returns, and nothing else.
    ///
    /// Only asked for surfaces passed to `AppContext::enable_accessibility`,
    /// and only while an assistive technology is attached, so an application
    /// pays nothing for implementing it in a session with none. Build the tree
    /// from the same state the paint uses, so the two cannot disagree; the
    /// focused control is filled in from the surface's focus ring, and does not
    /// have to be set here.
    ///
    /// Returning `None` means "nothing to expose right now" — a window that is
    /// still loading, say. See [`crate::accessibility`].
    fn accessibility(
        &mut self,
        _ctx: &AppContext,
        _surface: &wayland_client::backend::ObjectId,
    ) -> Option<crate::accessibility::A11yTree> {
        None
    }

    /// An assistive technology asked for something: a control clicked, a value
    /// set, a focus moved. Act on it as though the user had done it by hand.
    fn on_accessibility_action(
        &mut self,
        _ctx: &AppContext,
        _surface: &wayland_client::backend::ObjectId,
        _request: &crate::accessibility::ActionRequest,
    ) {
        // Default: do nothing
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

    /// Called for the same events as [`App::on_keyboard_event`], but with the
    /// full `KeyEvent` — including `keysym` and the `utf8` text produced by the
    /// active keymap. Implement this instead when you need text entry rather
    /// than raw evdev codes.
    fn on_key_event(
        &mut self,
        _ctx: &AppContext,
        _event: &KeyEvent,
        _state: wl_keyboard::KeyState,
        _serial: u32,
    ) {
        // Default: do nothing
    }

    /// The modifier state changed. Sent by the compositor before the key
    /// event that the change belongs to, and again on focus, so an app that
    /// needs to know whether Ctrl is down should read it from here rather
    /// than tracking `Control_L`/`Control_R` presses — those miss a modifier
    /// held before the surface took focus, and any chord the compositor
    /// swallowed.
    fn on_modifiers(&mut self, _ctx: &AppContext, _modifiers: Modifiers) {
        // Default: do nothing
    }

    /// Called when keyboard focus is lost from a surface
    /// Override this to handle focus loss (e.g., close menus)
    fn on_keyboard_leave(&mut self, _ctx: &AppContext, _surface: &wl_surface::WlSurface) {
        // Default: do nothing
    }

    /// The theme changed underfoot: the user picked a different accent, colour
    /// scheme or icon theme and the portal said so.
    ///
    /// An app that paints from `AppContext::current_theme()` has to repaint the
    /// surfaces it caches — for a window that draws its own chrome, that means
    /// asking for a frame, which nothing else will do while the app is idle.
    fn on_theme_changed(&mut self, _ctx: &AppContext) {
        // Default: do nothing
    }

    /// Fingers came to rest on the touchpad without scrolling
    /// (`zwp_pointer_gesture_hold_v1`).
    ///
    /// This is how a user stops a view that is still gliding: laying a hand on
    /// the trackpad sends no motion and no button, so nothing else in the
    /// pointer stream reports it.
    fn on_pointer_hold_begin(&mut self, _ctx: &AppContext, _fingers: u32) {
        // Default: do nothing
    }

    /// The hold ended — the fingers lifted, or the gesture turned into a
    /// scroll or a swipe and was cancelled.
    fn on_pointer_hold_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
        // Default: do nothing
    }

    /// Two fingers came down and the compositor read them as a pinch
    /// (`zwp_pointer_gesture_pinch_v1`).
    ///
    /// Nothing is known about where the pinch is: the protocol reports the
    /// focal point only as a delta from wherever it began, so an app that
    /// wants to zoom about it has to have kept the pointer's last position
    /// itself.
    fn on_pointer_pinch_begin(&mut self, _ctx: &AppContext, _fingers: u32) {
        // Default: do nothing
    }

    /// The pinch moved. `scale` is measured against the gesture's *start*,
    /// not the last update — 1.0 is "back where it began" — and `dx`/`dy` are
    /// how far the focal point has travelled since then, in surface
    /// coordinates. Both are absolute rather than incremental, so a dropped
    /// update costs nothing.
    fn on_pointer_pinch_update(
        &mut self,
        _ctx: &AppContext,
        _dx: f64,
        _dy: f64,
        _scale: f64,
        _rotation: f64,
    ) {
        // Default: do nothing
    }

    /// The pinch ended — the fingers lifted, or the compositor decided the
    /// gesture was something else after all and cancelled it.
    fn on_pointer_pinch_end(&mut self, _ctx: &AppContext, _cancelled: bool) {
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

    /// File descriptors to watch alongside the Wayland connection.
    ///
    /// `on_update` runs whenever one of these becomes readable, so an app whose
    /// other input is a socket — an IPC connection, a pipe — can wait on it
    /// rather than polling it on a timer.
    fn poll_fds(&self) -> Vec<std::os::fd::RawFd> {
        Vec::new()
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

    fn on_configure_lock_surface(
        &mut self,
        ctx: &AppContext,
        lock_surface: &wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        width: i32,
        height: i32,
        serial: u32,
    ) {
        self.inner
            .on_configure_lock_surface(ctx, lock_surface, width, height, serial)
    }

    fn on_session_locked(&mut self, ctx: &AppContext) {
        self.inner.on_session_locked(ctx)
    }

    fn on_session_lock_finished(&mut self, ctx: &AppContext) {
        self.inner.on_session_lock_finished(ctx)
    }

    fn on_close(&mut self) -> bool {
        self.inner.on_close()
    }

    fn accessibility(
        &mut self,
        ctx: &AppContext,
        surface: &wayland_client::backend::ObjectId,
    ) -> Option<crate::accessibility::A11yTree> {
        self.inner.accessibility(ctx, surface)
    }

    fn on_accessibility_action(
        &mut self,
        ctx: &AppContext,
        surface: &wayland_client::backend::ObjectId,
        request: &crate::accessibility::ActionRequest,
    ) {
        self.inner.on_accessibility_action(ctx, surface, request)
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

    fn on_modifiers(&mut self, ctx: &AppContext, modifiers: Modifiers) {
        self.inner.on_modifiers(ctx, modifiers)
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
    fn on_theme_changed(&mut self, ctx: &AppContext) {
        self.inner.on_theme_changed(ctx)
    }
    fn on_pointer_hold_begin(&mut self, ctx: &AppContext, fingers: u32) {
        self.inner.on_pointer_hold_begin(ctx, fingers)
    }
    fn on_pointer_hold_end(&mut self, ctx: &AppContext, cancelled: bool) {
        self.inner.on_pointer_hold_end(ctx, cancelled)
    }
    fn on_pointer_pinch_begin(&mut self, ctx: &AppContext, fingers: u32) {
        self.inner.on_pointer_pinch_begin(ctx, fingers)
    }
    fn on_pointer_pinch_update(
        &mut self,
        ctx: &AppContext,
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
    ) {
        self.inner
            .on_pointer_pinch_update(ctx, dx, dy, scale, rotation)
    }
    fn on_pointer_pinch_end(&mut self, ctx: &AppContext, cancelled: bool) {
        self.inner.on_pointer_pinch_end(ctx, cancelled)
    }
    fn on_update(&mut self, ctx: &AppContext) {
        self.inner.on_update(ctx)
    }
    fn idle_timeout(&self) -> Option<std::time::Duration> {
        self.inner.idle_timeout()
    }
    fn poll_fds(&self) -> Vec<std::os::fd::RawFd> {
        self.inner.poll_fds()
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
        // Version 2 added `set_clip_children`, which a scrolling view needs to
        // make a pane a fixed clipping window for the subsurface moving inside
        // it; version 3 the output frame; version 4 the desktop frame, without
        // which this application cannot say where its controls are on screen
        // and no assistive technology can find them by pointing. An older
        // compositor still binds at what it has and simply lacks the rest.
        let surface_style_manager = globals.bind(&qh, 1..=4, ()).ok();
        let wlr_layer_shell: Option<ZwlrLayerShellV1> = globals.bind(&qh, 1..=4, ()).ok();
        let otto_dock_manager = globals.bind(&qh, 1..=1, ()).ok();
        let session_lock_manager = globals.bind(&qh, 1..=1, ()).ok();
        let subcompositor = globals.bind(&qh, 1..=1, ()).ok();
        let cursor_shape_manager: Option<wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1> =
            globals.bind(&qh, 1..=2, ()).ok();
        let fractional_scale_manager: Option<wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> =
            globals.bind(&qh, 1..=1, ()).ok();
        // Hold gestures arrived in version 3; pinch has been there since 1.
        // Binding at 3 is what the hold hooks need, and a compositor offering
        // less simply leaves both unbound rather than half-working.
        let pointer_gestures: Option<ZwpPointerGesturesV1> = globals.bind(&qh, 3..=3, ()).ok();

        // Get display pointer for creating surfaces
        let display_ptr = conn.backend().display_ptr() as *mut std::ffi::c_void;

        // Note: Layers renderer is now initialized via AppContext::enable_layer_engine()

        // Move states into the context data structure (box it to prevent movement)
        // Clipboard support. A compositor without `wl_data_device_manager` is
        // legal; the clipboard API then reports failure rather than panicking.
        let data_device_manager =
            smithay_client_toolkit::data_device_manager::DataDeviceManagerState::bind(
                &globals, &qh,
            )
            .ok();
        if data_device_manager.is_none() {
            tracing::debug!("no wl_data_device_manager; clipboard unavailable");
        }

        let context = Box::new(AppContextData {
            connection: conn.clone(),
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            output_state,
            surface_style_manager,
            wlr_layer_shell,
            subcompositor,
            otto_dock_manager,
            session_lock_manager,
            cursor_shape_manager,
            fractional_scale_manager,
            pointer_gestures,
            data_device_manager,
            data_device: None,
            display_ptr,
        });

        // Create the internal app data
        let mut app_data = AppData {
            app: self.app,
            registry_state,
            context_data: context,
            hold_gestures: Vec::new(),
            pinch_gestures: Vec::new(),
            exit: false,
        };

        // Initialize AppContext with context data pointer and queue handle
        // Box ensures context_data won't move even when app_data is moved
        AppContext::init::<A>(&app_data.context_data, &qh);

        // Background watchers. They find their own home — an app with a plain
        // `fn main` follows the portal just like an async one does.
        //
        // The generation is read *before* they start: an answer that lands
        // while `on_app_ready` runs has to reach the app as a change, not be
        // folded into the first pass. What the app pushed to the compositor
        // during `on_app_ready` — a window's material, say — was built from
        // the default scheme, and only `on_theme_changed` replaces it. Seen
        // as the settings app keeping a light frost under dark content.
        let theme_generation_seen = crate::portal_runtime::theme_generation();
        crate::color_scheme::spawn_color_scheme_watcher();
        crate::accent::spawn_accent_watcher();
        crate::icon_theme::spawn_icon_theme_watcher();
        crate::sound::spawn_theme_watcher();

        // Call the app's ready callback
        let ctx = AppContext::new(&app_data.context_data);
        app_data.app.on_app_ready(&ctx)?;

        Ok(AppRunnerInitialized {
            conn,
            event_queue,
            app_data,
            theme_generation_seen,
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
    /// The theme generation last delivered to the app — taken before the
    /// watchers were spawned, so nothing they answered during init is lost.
    theme_generation_seen: u64,
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
        let mut theme_generation_seen = self.theme_generation_seen;

        while !self.app_data.exit {
            // 1. Drain any events already queued (no I/O).
            self.event_queue.dispatch_pending(&mut self.app_data)?;
            self.conn.flush()?;

            AppContext::update_windows();

            let ctx = AppContext::new(&self.app_data.context_data);
            // A watcher may have changed the theme since the last pass. It
            // runs off this thread, so this is where the app hears about it.
            let generation = crate::portal_runtime::theme_generation();
            if generation != theme_generation_seen {
                theme_generation_seen = generation;
                self.app_data.app.on_theme_changed(&ctx);
            }
            self.app_data.app.on_update(&ctx);

            if self.app_data.exit {
                break;
            }

            // After `on_update`, so a tree describes the state the application
            // has just settled on rather than the previous pass's.
            pump_accessibility(&mut self.app_data);

            // An app that asked to stop has usually just sent something it
            // needs delivered — a locker's unlock request — so flush before
            // dropping the connection.
            if AppContext::exit_requested() {
                self.conn.flush()?;
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

            let mut fds = vec![
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
            // Whatever else the app wants woken for. Their readiness is not
            // reported back: `on_update` runs on every iteration anyway, which
            // is where an app reads its own descriptors.
            fds.extend(
                self.app_data
                    .app
                    .poll_fds()
                    .into_iter()
                    .map(|fd| libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    }),
            );

            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };

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
    /// One per seat pointer. Held only to keep the protocol objects alive —
    /// destroying them would silently end the hold events.
    hold_gestures: Vec<ZwpPointerGestureHoldV1>,
    /// The same, for pinch. Kept apart from the holds because the two are
    /// separate protocol objects with separate lifetimes, not because either
    /// list is ever read.
    pinch_gestures: Vec<ZwpPointerGesturePinchV1>,
    exit: bool,
}

// Wayland protocol handler implementations
impl<A: App + 'static> CompositorHandler for AppData<A> {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        tracing::debug!("scale_factor_changed: {new_factor}");
        AppContext::set_scale_factor(new_factor);
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

        // The last committed frame is on screen, so a client throttling itself
        // to the compositor may paint the next one.
        AppContext::clear_frame_in_flight(&surface.id());

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
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, window: &StkWindow) {
        use smithay_client_toolkit::shell::WaylandSurface;
        use wayland_client::Proxy;

        // An application with more than one window is asked about each of
        // them separately. A secondary window that has claimed its own close
        // takes it here; only a close nobody claimed is the application's,
        // and only that one can end the process.
        if AppContext::dispatch_close_request(&window.wl_surface().id()) {
            return;
        }
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
        use smithay_client_toolkit::shell::WaylandSurface;
        use wayland_client::Proxy;

        // Named, not null. Every registered handler is called for every
        // configure, so the id is the only thing that tells a window its own
        // configure from another window's — and an application with two of
        // them was otherwise resizing both to whatever the last one was told.
        AppContext::set_current_configure(window.wl_surface().id(), configure.clone(), serial);
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

        if capability == Capability::Pointer {
            match self.context_data.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    // One hold object per pointer, kept alive for the seat's
                    // lifetime: dropping it destroys the protocol object and
                    // the events stop.
                    if let Some(gestures) = &self.context_data.pointer_gestures {
                        self.hold_gestures
                            .push(gestures.get_hold_gesture(&pointer, qh, ()));
                        self.pinch_gestures
                            .push(gestures.get_pinch_gesture(&pointer, qh, ()));
                    }
                }
                Err(_) => eprintln!("Failed to create pointer"),
            }
        }

        // The clipboard lives on a seat, so the data device cannot be created
        // until one exists. Created once, on the first seat.
        if self.context_data.data_device.is_none() {
            if let Some(manager) = &self.context_data.data_device_manager {
                self.context_data.data_device = Some(manager.get_data_device(qh, &seat));
            }
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

/// Runs the accessibility side of one pass of the run loop: delivers whatever
/// assistive technologies asked for, then re-describes the surfaces one is
/// listening to.
///
/// Everything here is skipped when no assistive technology is attached — the
/// mailbox is empty and nothing is wanted — so the cost in an ordinary session
/// is one map lookup per accessible surface.
fn pump_accessibility<A: App + 'static>(data: &mut AppData<A>) {
    let surfaces = AppContext::accessible_surfaces();
    if surfaces.is_empty() {
        return;
    }

    let keyboard_focus = AppContext::keyboard_focus();

    for surface in surfaces {
        let (wanted, actions) = AppContext::accessibility_pending(&surface);

        for request in &actions {
            let ctx = AppContext::new(&data.context_data);
            data.app.on_accessibility_action(&ctx, &surface, request);
        }

        AppContext::set_accessibility_window_focus(
            &surface,
            keyboard_focus.as_ref() == Some(&surface),
        );
        // Before the tree, not after: the bounds are what the coordinates in it
        // are read against, and a tree published against the wrong origin is
        // read against the wrong origin for as long as it stands.
        AppContext::sync_accessibility_desktop_frame(&surface);

        if !wanted {
            continue;
        }

        let ctx = AppContext::new(&data.context_data);
        let Some(mut tree) = data.app.accessibility(&ctx, &surface) else {
            continue;
        };

        // The focus ring is the default answer for what has the keyboard: it is
        // what Tab moved, and what the application drew a ring around. An
        // application that named a node itself keeps it — a list that is one
        // keyboard stop still has to report *which* row is current.
        if let Some(focused) = AppContext::focused_control(&surface) {
            tree.set_default_focus(focused);
        }

        AppContext::push_accessibility_tree(&surface, tree.finish());
    }
}

/// Tab and Shift+Tab walk the focused window's controls.
///
/// Returns whether the key was consumed. A window that declared no focusables
/// consumes nothing, so this is invisible to an application that does its own
/// keyboard handling — the toolkit only takes Tab once there is somewhere for
/// it to go.
fn move_focus_for_key(event: &KeyEvent) -> bool {
    use crate::focus::FocusMove;

    let Some(surface) = AppContext::keyboard_focus() else {
        return false;
    };

    let modifiers = AppContext::current_modifiers();
    // Ctrl+Tab and friends belong to the application (or the compositor).
    if modifiers.ctrl || modifiers.alt || modifiers.logo {
        return false;
    }

    let direction = match event.keysym {
        Keysym::Tab if modifiers.shift => FocusMove::Previous,
        Keysym::Tab => FocusMove::Next,
        // What most keymaps produce for Shift+Tab.
        Keysym::ISO_Left_Tab => FocusMove::Previous,
        _ => return false,
    };

    let moved = AppContext::with_focus_ring(&surface, |ring| {
        if ring.entries().is_empty() {
            None
        } else {
            ring.move_focus(direction)
        }
    });

    if moved.is_none() {
        return false;
    }

    AppContext::update_windows();
    // The ring moved, so the window looks different and has to paint: waking
    // the loop alone only gets the application asked whether anything changed,
    // and as far as it knows nothing has.
    AppContext::request_window_frame(&surface);
    AppContext::request_wakeup();
    true
}

impl<A: App + 'static> KeyboardHandler for AppData<A> {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        use wayland_client::Proxy;
        AppContext::set_keyboard_focus(Some(surface.id()));
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        use wayland_client::Proxy;
        // The window keeps its focus ring — it will still be focused where it
        // was when the keyboard comes back — but no surface of ours holds the
        // keyboard now.
        if AppContext::keyboard_focus().as_ref() == Some(&surface.id()) {
            AppContext::set_keyboard_focus(None);
        }
        AppContext::dispatch_keyboard_leave(&surface.id());
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
        // Tab moves the focus between the window's own controls before the
        // application sees the key — but only in a window that declared any,
        // so an application that handles Tab itself is unaffected.
        if move_focus_for_key(&event) {
            return;
        }

        let ctx = AppContext::new(&self.context_data);
        self.app
            .on_keyboard_event(&ctx, event.raw_code, wl_keyboard::KeyState::Pressed, serial);
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
        self.app.on_keyboard_event(
            &ctx,
            event.raw_code,
            wl_keyboard::KeyState::Released,
            serial,
        );
        self.app
            .on_key_event(&ctx, &event, wl_keyboard::KeyState::Released, serial);
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        AppContext::set_current_modifiers(modifiers);
        let ctx = AppContext::new(&self.context_data);
        self.app.on_modifiers(&ctx, modifiers);
    }
}

impl<A: App + 'static> PointerHandler for AppData<A> {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        use smithay_client_toolkit::seat::pointer::PointerEventKind;

        // Track enter serial and lazily create cursor shape device.
        for event in events {
            if let PointerEventKind::Enter { serial, .. } = event.kind {
                AppContext::set_last_pointer_enter_serial(serial);

                // Create cursor shape device if we have the manager but no device yet.
                AppContext::ensure_cursor_shape_device(&self.context_data, pointer, qh);
            }
        }

        AppContext::dispatch_pointer_callbacks(events);
        let ctx = AppContext::new(&self.context_data);
        self.app.on_pointer_event(&ctx, events);
        // Everything that had a claim on this batch has now had it, which is
        // when a popup can tell an outside press from a press on the control
        // that owns it.
        AppContext::dispatch_pointer_batch_end();
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

// ext-session-lock-v1: a screen locker's side of the protocol.
//
// The lock object outlives its surfaces and is what "locked" is attached to,
// so both events reach the app rather than being handled here — only the app
// knows whether a `finished` means "refused, give up" or "we just unlocked".
impl<A: App + 'static> Dispatch<ExtSessionLockV1, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        _proxy: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let ctx = AppContext::new(&state.context_data);
        match event {
            ext_session_lock_v1::Event::Locked => {
                tracing::info!("session locked");
                state.app.on_session_locked(&ctx);
            }
            ext_session_lock_v1::Event::Finished => {
                tracing::info!("session lock finished");
                state.app.on_session_lock_finished(&ctx);
            }
            _ => {}
        }
    }
}

impl<A: App + 'static> Dispatch<ExtSessionLockSurfaceV1, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        proxy: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;

        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            tracing::debug!("Lock surface configure: {width}x{height}");
            // The surface acks and resizes its canvas first, so the app is
            // called with somewhere to draw.
            AppContext::dispatch_lock_surface_configure(
                &proxy.id(),
                width as i32,
                height as i32,
                serial,
            );

            let ctx = AppContext::new(&state.context_data);
            state
                .app
                .on_configure_lock_surface(&ctx, proxy, width as i32, height as i32, serial);
        }
    }
}

wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore ExtSessionLockManagerV1);

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
smithay_client_toolkit::delegate_data_device!(@<A: App + 'static> AppData<A>);

// -- Clipboard and drag and drop -------------------------------------------
//
// Three traits, because `wl_data_device` carries both on the same object. The
// clipboard half lives in [`crate::clipboard`] and the drag half in
// [`crate::dnd`]; what follows is only the dispatch between them, which turns
// on one question — is this our own drag source, or the clipboard's?

impl<A: App + 'static> smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler
    for AppData<A>
{
    /// A drag came over one of our surfaces. The offered types are recorded
    /// here, once, because the application asks for them at every position.
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
        x: f64,
        y: f64,
        surface: &wl_surface::WlSurface,
    ) {
        use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
        use wayland_client::Proxy;

        let Some(device) = self.context_data.data_device.as_ref() else {
            return;
        };
        if device.inner() != data_device {
            return;
        }

        let mimes = DataDevice::data(device)
            .drag_offer()
            .map_or_else(Vec::new, |offer| offer.with_mime_types(<[String]>::to_vec));
        crate::dnd::set_offered_mime_types(mimes);
        crate::dnd::dispatch(crate::dnd::DragEvent::Enter {
            surface: surface.id(),
            x,
            y,
        });
    }

    /// The drag left, or was cancelled over us. Either way no drop is coming,
    /// and anything the application highlighted should stop being highlighted.
    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        crate::dnd::set_offered_mime_types(Vec::new());
        crate::dnd::dispatch(crate::dnd::DragEvent::Leave);
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
        x: f64,
        y: f64,
    ) {
        crate::dnd::dispatch(crate::dnd::DragEvent::Motion { x, y });
    }

    /// Dropped on us. The position is the offer's own — the drop event does not
    /// carry one, and the last motion is where the pointer was released.
    fn drop_performed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        use smithay_client_toolkit::data_device_manager::data_device::DataDevice;

        let Some(device) = self.context_data.data_device.as_ref() else {
            return;
        };
        if device.inner() != data_device {
            return;
        }

        let (x, y) = DataDevice::data(device)
            .drag_offer()
            .map_or((0.0, 0.0), |offer| (offer.x, offer.y));
        crate::dnd::dispatch(crate::dnd::DragEvent::Drop { x, y });
    }

    /// The clipboard changed. Record what it offers so a later paste knows
    /// which types it can ask for.
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &wayland_client::protocol::wl_data_device::WlDataDevice,
    ) {
        use smithay_client_toolkit::data_device_manager::data_device::DataDevice;

        let Some(device) = self.context_data.data_device.as_ref() else {
            return;
        };
        if device.inner() != data_device {
            return;
        }

        match DataDevice::data(device).selection_offer() {
            Some(offer) => {
                let mimes = offer.with_mime_types(<[String]>::to_vec);
                crate::clipboard::set_available(mimes);
                crate::app_runner::context::set_current_offer(Some(offer.inner().clone()));
            }
            None => {
                crate::clipboard::set_available(Vec::new());
                crate::app_runner::context::set_current_offer(None);
            }
        }
    }
}

impl<A: App + 'static> smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler
    for AppData<A>
{
    fn accept_mime(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wayland_client::protocol::wl_data_source::WlDataSource,
        _mime: Option<String>,
    ) {
    }

    /// Somebody pasted. Write the payload and close the pipe — the reader waits
    /// on EOF, so failing to close would hang it until its own timeout.
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wayland_client::protocol::wl_data_source::WlDataSource,
        mime: String,
        fd: smithay_client_toolkit::data_device_manager::WritePipe,
    ) {
        use std::io::Write;

        // A drag and the clipboard can both be live at once, and they keep
        // their payloads apart, so the source says which store to answer from.
        let payload = if crate::app_runner::context::is_drag_source(_source) {
            crate::dnd::payload_for(&mime)
        } else {
            crate::clipboard::offered_bytes(&mime)
        };
        let Some(bytes) = payload else {
            tracing::debug!(%mime, "asked for a type we do not offer");
            return;
        };
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        if let Err(err) = file.write_all(&bytes) {
            tracing::warn!(%err, %mime, "could not write the clipboard payload");
        }
        // `file` closes here, which is the EOF the reader is waiting for.
    }

    /// Our source is dead: someone else claimed the clipboard, or a drag ended
    /// over a target that refused it. Either way the payload goes.
    fn cancelled(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        if crate::app_runner::context::is_drag_source(source) {
            crate::app_runner::context::drop_drag_source();
        } else {
            crate::app_runner::context::drop_current_source();
        }
    }

    /// The button came up over a target that accepted. The data has *not*
    /// necessarily been read yet — the target may ask after the drop — so the
    /// source stays alive until `dnd_finished`.
    fn dnd_dropped(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
    }

    /// The target is done with the transfer. Nothing more can be asked for.
    ///
    /// A move is not acted on here: the target performs it. See the module
    /// documentation on [`crate::dnd`] for why the source does not delete.
    fn dnd_finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wayland_client::protocol::wl_data_source::WlDataSource,
    ) {
        if crate::app_runner::context::is_drag_source(source) {
            crate::app_runner::context::drop_drag_source();
        }
    }

    fn action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wayland_client::protocol::wl_data_source::WlDataSource,
        _action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

/// The negotiated action is read from the offer itself, at the moment it
/// matters — see [`crate::dnd::selected_action`]. These two only announce
/// changes SCTK has already recorded on the offer, so there is nothing to keep.
impl<A: App + 'static> smithay_client_toolkit::data_device_manager::data_offer::DataOfferHandler
    for AppData<A>
{
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut smithay_client_toolkit::data_device_manager::data_offer::DragOffer,
        _actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut smithay_client_toolkit::data_device_manager::data_offer::DragOffer,
        _action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

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
        proxy: &otto_surface_style_v1::OttoSurfaceStyleV1,
        event: otto_surface_style_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_client::Proxy;

        match event {
            otto_surface_style_v1::Event::OutputFrame {
                x,
                y,
                width,
                height,
            } => AppContext::set_output_frame(
                &proxy.id(),
                (x as f32, y as f32, width as f32, height as f32),
            ),
            // Where this window is on the desktop. Only accessibility uses it,
            // and only for a surface that is a window in its own right — see
            // `AppContext::desktop_frame`.
            otto_surface_style_v1::Event::DesktopFrame {
                x,
                y,
                width,
                height,
            } => AppContext::set_desktop_frame(
                &proxy.id(),
                (x as f32, y as f32, width as f32, height as f32),
            ),
        }
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

impl<A: App + 'static>
    Dispatch<
        wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
        (),
    > for AppData<A>
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::Event;
        if let Event::PreferredScale { scale } = event {
            tracing::debug!("preferred fractional scale: {}", scale as f64 / 120.0);
            AppContext::set_fractional_scale_120(scale);
        }
    }
}

// Delegate noop for protocols we don't handle
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_client::protocol::wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_client::protocol::wl_subsurface::WlSubsurface);
// Regions are write-only: a client builds one, hands it to `set_input_region`
// or `set_opaque_region`, and never hears from it again.
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_client::protocol::wl_region::WlRegion);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore ZwlrLayerShellV1);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(@<A: App + 'static> AppData<A>: ignore ZwpPointerGesturesV1);

impl<A: App + 'static> Dispatch<ZwpPointerGesturePinchV1, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        _pinch: &ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let ctx = AppContext::new(&state.context_data);
        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin { fingers, .. } => {
                state.app.on_pointer_pinch_begin(&ctx, fingers);
            }
            zwp_pointer_gesture_pinch_v1::Event::Update {
                dx,
                dy,
                scale,
                rotation,
                ..
            } => {
                state
                    .app
                    .on_pointer_pinch_update(&ctx, dx, dy, scale, rotation);
            }
            zwp_pointer_gesture_pinch_v1::Event::End { cancelled, .. } => {
                state.app.on_pointer_pinch_end(&ctx, cancelled != 0);
            }
            _ => {}
        }
    }
}

impl<A: App + 'static> Dispatch<ZwpPointerGestureHoldV1, ()> for AppData<A> {
    fn event(
        state: &mut Self,
        _hold: &ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let ctx = AppContext::new(&state.context_data);
        match event {
            zwp_pointer_gesture_hold_v1::Event::Begin { fingers, .. } => {
                state.app.on_pointer_hold_begin(&ctx, fingers);
            }
            zwp_pointer_gesture_hold_v1::Event::End { cancelled, .. } => {
                state.app.on_pointer_hold_end(&ctx, cancelled != 0);
            }
            _ => {}
        }
    }
}
