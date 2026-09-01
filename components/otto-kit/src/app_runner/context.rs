//! Application context - manages global state and Wayland protocol access

use super::{App, AppData};
use crate::protocols::otto_surface_style_manager_v1;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::{OutputInfo, OutputState},
    seat::SeatState,
    shell::xdg::{window::WindowConfigure, XdgShell},
    shm::Shm,
};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    /// Per-window handlers for the compositor's "please close" request, keyed
    /// by the window's own surface. A window that registers one owns its
    /// close: the application is not asked, and the process does not exit.
    #[allow(clippy::type_complexity)]
    static CLOSE_HANDLERS: RefCell<HashMap<ObjectId, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());

    /// Keyboard focus below the window: which control in each surface has it,
    /// and in what order Tab visits them. See [`crate::focus`].
    static FOCUS_RINGS: RefCell<HashMap<ObjectId, crate::focus::FocusRing>> = RefCell::new(HashMap::new());
    /// The surface the compositor last gave the keyboard to, if it is ours.
    static KEYBOARD_FOCUS: RefCell<Option<ObjectId>> = const { RefCell::new(None) };
    /// Modifier state from the last `wl_keyboard.modifiers`. Kept because a key
    /// event does not carry it, and Shift+Tab has to be told from Tab.
    static CURRENT_MODIFIERS: RefCell<super::Modifiers> = RefCell::new(super::Modifiers::default());

    /// One AT-SPI adapter per surface the application made accessible. See
    /// [`crate::accessibility`].
    static A11Y_ADAPTERS: RefCell<HashMap<ObjectId, crate::accessibility::SurfaceAdapter>> = RefCell::new(HashMap::new());
}

/// The data source backing our claim on the clipboard, and the offer backing
/// the current selection.
///
/// `Mutex` rather than `thread_local!` only because they are read from the
/// clipboard module's plain functions; every write still happens on the UI
/// thread through the dispatch handlers.
static CURRENT_SOURCE: std::sync::Mutex<
    Option<smithay_client_toolkit::data_device_manager::data_source::CopyPasteSource>,
> = std::sync::Mutex::new(None);
static CURRENT_OFFER: std::sync::Mutex<
    Option<wayland_client::protocol::wl_data_offer::WlDataOffer>,
> = std::sync::Mutex::new(None);

/// The source backing a drag *this* application started. Held for the same
/// reason as `CURRENT_SOURCE` — dropping it destroys the `wl_data_source`, and
/// with it the drag, mid-gesture.
static CURRENT_DRAG_SOURCE: std::sync::Mutex<
    Option<smithay_client_toolkit::data_device_manager::data_source::DragSource>,
> = std::sync::Mutex::new(None);

/// Whether `source` is the one backing our current drag, so the send handler
/// knows which payload store to answer from.
pub(crate) fn is_drag_source(
    source: &wayland_client::protocol::wl_data_source::WlDataSource,
) -> bool {
    CURRENT_DRAG_SOURCE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|drag| drag.inner() == source)
}

/// The drag we started is over — dropped, cancelled or refused.
pub(crate) fn drop_drag_source() {
    *CURRENT_DRAG_SOURCE.lock().unwrap() = None;
    crate::dnd::clear_payload();
}

/// Release our claim on the clipboard. Called when the compositor cancels the
/// source, which is how it says someone else copied.
pub(crate) fn drop_current_source() {
    *CURRENT_SOURCE.lock().unwrap() = None;
    crate::clipboard::clear_offered();
}

/// Record the offer that now owns the selection.
pub(crate) fn set_current_offer(
    offer: Option<wayland_client::protocol::wl_data_offer::WlDataOffer>,
) {
    *CURRENT_OFFER.lock().unwrap() = offer;
}

/// A pipe for receiving a selection: `(read, write)`.
fn rustix_pipe() -> Option<(std::fs::File, std::fs::File)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `pipe2` fills both entries or returns an error; the descriptors
    // are immediately owned by `File`, which closes them.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        tracing::warn!("could not create a pipe for the clipboard");
        return None;
    }
    unsafe {
        Some((
            std::fs::File::from_raw_fd(fds[0]),
            std::fs::File::from_raw_fd(fds[1]),
        ))
    }
}

// -- Callback registries --

thread_local! {
    static CONFIGURE_HANDLERS: RefCell<Vec<Box<dyn FnMut()>>> = const { RefCell::new(Vec::new()) };
    #[allow(clippy::type_complexity)]
    static POINTER_CALLBACKS: RefCell<Vec<Box<dyn FnMut(&[smithay_client_toolkit::seat::pointer::PointerEvent])>>> = const { RefCell::new(Vec::new()) };
    /// Run after every pointer callback *and* the app's own `on_pointer_event`
    /// for a batch. See [`AppContext::register_pointer_batch_end_callback`].
    static POINTER_BATCH_END_CALLBACKS: RefCell<Vec<Box<dyn FnMut()>>> = const { RefCell::new(Vec::new()) };
    static FRAME_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut()>>> = RefCell::new(HashMap::new());
    /// Surfaces that have committed a frame the compositor has not yet said it
    /// presented. See [`AppContext::frame_in_flight`].
    static FRAMES_IN_FLIGHT: RefCell<HashSet<ObjectId>> = RefCell::new(HashSet::new());
    /// The last `output_frame` a style surface was told, keyed by the style
    /// object. See [`AppContext::output_frame`].
    static OUTPUT_FRAMES: RefCell<HashMap<ObjectId, (f32, f32, f32, f32)>> = RefCell::new(HashMap::new());
    /// Which `wl_surface` a style object augments, for the style objects on a
    /// window's own surface. Only those: the compositor reports a desktop
    /// position for a surface that is a window in its own right, never for a
    /// popup or a subsurface, which move with their parent.
    static STYLE_SURFACES: RefCell<HashMap<ObjectId, ObjectId>> = RefCell::new(HashMap::new());
    /// Where the compositor is drawing each window, keyed by `wl_surface`, in
    /// physical pixels. See [`AppContext::desktop_frame`].
    static DESKTOP_FRAMES: RefCell<HashMap<ObjectId, (f32, f32, f32, f32)>> = RefCell::new(HashMap::new());
    /// The serial of the most recent input event. See
    /// [`AppContext::last_input_serial`].
    static LAST_INPUT_SERIAL: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    #[allow(clippy::type_complexity)]
    static POPUP_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce(u32)>>> = RefCell::new(HashMap::new());
    static POPUP_DONE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
    #[allow(clippy::type_complexity)]
    static LAYER_SHELL_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut(i32, i32, u32)>>> = RefCell::new(HashMap::new());
    /// Keyed by `ext_session_lock_surface_v1` object, not by the `wl_surface`:
    /// the configure arrives on the lock surface, and the callback acks it.
    #[allow(clippy::type_complexity)]
    static LOCK_SURFACE_CONFIGURE_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnMut(i32, i32, u32)>>> = RefCell::new(HashMap::new());
    static TRANSACTION_COMPLETION_CALLBACKS: RefCell<HashMap<ObjectId, Box<dyn FnOnce()>>> = RefCell::new(HashMap::new());
    /// Called with the `wl_surface` that just lost keyboard focus. Components
    /// that must not outlive the focus (menus, popovers) subscribe here.
    #[allow(clippy::type_complexity)]
    static KEYBOARD_LEAVE_CALLBACKS: RefCell<Vec<Box<dyn FnMut(&ObjectId)>>> = const { RefCell::new(Vec::new()) };
}

// -- Cursor shape state --

thread_local! {
    static CURSOR_SHAPE_DEVICE: RefCell<Option<wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>> = const { RefCell::new(None) };
    static LAST_POINTER_ENTER_SERIAL: RefCell<u32> = const { RefCell::new(0) };
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

/// Set by [`AppContext::request_exit`]; the run loop stops at the next
/// iteration, after flushing whatever the app asked for last.
static EXIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// -- Display scale factor (updated by compositor, default 1) --

static DISPLAY_SCALE_FACTOR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(2);

/// Whether [`AppContext::set_scale_factor`] has already taken a value. The
/// scale is latched for the lifetime of the process — see
/// [`AppContext::fractional_scale`] for why.
static SCALE_FACTOR_LATCHED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Preferred fractional scale in 120ths, as sent by `wp_fractional_scale_v1`.
/// 0 means the compositor has not sent one yet — callers fall back to the
/// integer `wl_surface` scale.
static DISPLAY_FRACTIONAL_SCALE_120: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

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
    /// The connection itself, so a request that must not wait for the run
    /// loop's next flush can be pushed out where it is made.
    pub connection: wayland_client::Connection,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShell,
    pub shm_state: Shm,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub surface_style_manager: Option<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1>,
    pub wlr_layer_shell: Option<ZwlrLayerShellV1>,
    pub subcompositor: Option<wayland_client::protocol::wl_subcompositor::WlSubcompositor>,
    pub otto_dock_manager: Option<crate::protocols::otto_dock_manager_v1::OttoDockManagerV1>,
    pub session_lock_manager: Option<wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    pub cursor_shape_manager: Option<wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub fractional_scale_manager: Option<wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    /// `None` on a compositor without pointer gestures at version 3, where a
    /// touchpad hold is simply not reported.
    pub pointer_gestures: Option<wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    /// Clipboard and drag-and-drop. `None` on a compositor without
    /// `wl_data_device_manager`, which is legal — clipboard calls then fail
    /// cleanly rather than panicking.
    pub data_device_manager:
        Option<smithay_client_toolkit::data_device_manager::DataDeviceManagerState>,
    pub data_device: Option<smithay_client_toolkit::data_device_manager::data_device::DataDevice>,
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

    /// Everything the compositor has said about the outputs it is driving:
    /// connector name, make and model, position, and the full mode list with
    /// which one is current and which is preferred.
    ///
    /// This is a client's display probe. A settings app offering a resolution
    /// or a refresh rate has to know what the hardware actually supports, and
    /// `wl_output` already carries it — no DRM access, no second session, and
    /// it follows hotplug because the compositor keeps sending events.
    /// Empty before the context exists, and again before the compositor has
    /// announced anything — a caller has to be able to ask again rather than
    /// keep the first answer, since outputs arrive after the app starts and
    /// come and go with hotplug.
    pub fn outputs() -> Vec<OutputInfo> {
        APP_CONTEXT_PTR.with(|ptr| {
            let Some(data_ptr) = *ptr.borrow() else {
                return Vec::new();
            };
            let state = &unsafe { &*data_ptr }.output_state;
            // An output is listed as soon as its global is bound but has no
            // `OutputInfo` until its `done` event lands a few dispatches
            // later, so this is empty for the app's first frames even though
            // the compositor is driving a display.
            state.outputs().filter_map(|o| state.info(&o)).collect()
        })
    }

    pub fn compositor_state() -> &'static CompositorState {
        Self::with_global(|ctx| unsafe { &*(ctx.compositor_state_ref() as *const CompositorState) })
    }

    /// Returns the current display scale factor (updated by the compositor).
    /// Defaults to 1 if no scale_factor_changed event has been received yet.
    pub fn scale_factor() -> i32 {
        DISPLAY_SCALE_FACTOR.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record the display scale factor (called from CompositorHandler).
    ///
    /// Only the first value is kept; later `scale_factor_changed` events are
    /// ignored until the process restarts.
    pub(crate) fn set_scale_factor(factor: i32) {
        if SCALE_FACTOR_LATCHED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        DISPLAY_SCALE_FACTOR.store(factor, std::sync::atomic::Ordering::Relaxed);
    }

    /// The output scale as a fraction (physical pixels per logical point).
    ///
    /// Buffers are still rendered at the integer [`scale_factor`], but geometry
    /// handed to the compositor in physical pixels — surface-style sizes and
    /// positions — must use this: on a 1.5x output the integer factor is 2, and
    /// scaling by it makes a surface a third too large.
    ///
    /// Latched at startup. A surface fixes its buffer scale when it is created
    /// and never re-rasters, so letting this follow a live scale change would
    /// move the geometry out from under a buffer that stayed put — the panel
    /// keeps its old pixels at a new size. A scale change takes effect on the
    /// next restart, which is how the compositor-side chrome treats it too.
    pub fn fractional_scale() -> f64 {
        match DISPLAY_FRACTIONAL_SCALE_120.load(std::sync::atomic::Ordering::Relaxed) {
            0 => Self::scale_factor().max(1) as f64,
            n => n as f64 / 120.0,
        }
    }

    /// Store the preferred scale from `wp_fractional_scale_v1` (in 120ths).
    ///
    /// First one wins: 0 is the "nothing received yet" sentinel, so the
    /// compare-exchange takes the compositor's opening value and ignores every
    /// later `preferred_scale`. See [`Self::fractional_scale`].
    pub(crate) fn set_fractional_scale_120(scale_120: u32) {
        let _ = DISPLAY_FRACTIONAL_SCALE_120.compare_exchange(
            0,
            scale_120,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
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

    pub fn fractional_scale_manager() -> Option<&'static wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>{
        use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
        Self::with_global(|ctx| unsafe {
            ctx.data
                .fractional_scale_manager
                .as_ref()
                .map(|r| &*(r as *const WpFractionalScaleManagerV1))
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

    pub fn session_lock_manager() -> Option<
        &'static wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    >{
        Self::with_global(|ctx| unsafe {
            ctx.session_lock_manager_ref().map(|r| &*(r as *const _))
        })
    }

    /// Push everything queued out to the compositor now.
    ///
    /// The run loop flushes once per iteration, which is right for anything
    /// whose effect the client will still be around to see. A request the
    /// client is about to exit on — a locker handing the session back — has to
    /// go out where it is made.
    /// Claim the clipboard, offering `mime_types`.
    ///
    /// The payload itself lives in [`crate::clipboard`]; this only creates the
    /// source and hands it to the compositor. Returns `false` when there is no
    /// data device — no seat, or a compositor without the protocol.
    pub fn set_selection(mime_types: Vec<String>, serial: u32) -> bool {
        Self::with_global(|ctx| {
            let (Some(manager), Some(device)) =
                (&ctx.data.data_device_manager, &ctx.data.data_device)
            else {
                tracing::debug!("no data device; cannot claim the selection");
                return false;
            };

            let qh = Self::queue_handle();
            let types: Vec<&str> = mime_types.iter().map(String::as_str).collect();
            let source = manager.create_copy_paste_source(qh, types);
            source.set_selection(device, serial);

            // Hold the source alive: dropping it destroys the wl_data_source
            // and the selection with it. It is released when the compositor
            // cancels it, which is what `cancelled` does.
            *CURRENT_SOURCE.lock().unwrap() = Some(source);

            // Push it out now. Waiting for the run loop's next flush would
            // leave the clipboard unclaimed for a frame, and a paste in that
            // window would see the previous owner's data.
            if let Err(err) = ctx.data.connection.flush() {
                tracing::warn!(%err, "could not flush after claiming the selection");
            }
            true
        })
    }

    /// Ask the current selection's owner for `mime`, returning the read end of
    /// the pipe. The caller reads it — see [`crate::clipboard::read`].
    pub fn receive_selection(mime: &str) -> Option<std::fs::File> {
        use std::os::fd::AsFd;
        Self::with_global(|ctx| {
            let offer = CURRENT_OFFER.lock().unwrap();
            let offer = offer.as_ref()?;

            // A pipe, one end to the compositor, the other read here.
            let (read_fd, write_fd) = match rustix_pipe() {
                Some(pair) => pair,
                None => return None,
            };

            offer.receive(mime.to_string(), write_fd.as_fd());
            // The write end must be closed here or the read never sees EOF:
            // the source client holds the only other copy.
            drop(write_fd);

            // The request has to reach the compositor before we block reading.
            if let Err(err) = ctx.data.connection.flush() {
                tracing::warn!(%err, "could not flush before reading the selection");
            }
            Some(read_fd)
        })
    }

    // ------------------------------------------------------------------
    // Drag and drop. The application-facing API is [`crate::dnd`]; these are
    // the protocol calls behind it.
    // ------------------------------------------------------------------

    /// Start a drag from `origin`, offering `mime_types` and `actions`.
    ///
    /// See [`crate::dnd::start`] for what the arguments mean.
    pub(crate) fn start_drag(
        mime_types: Vec<String>,
        actions: wayland_client::protocol::wl_data_device_manager::DndAction,
        origin: &wl_surface::WlSurface,
        icon: Option<&wl_surface::WlSurface>,
        serial: u32,
    ) -> bool {
        Self::with_global(|ctx| {
            let (Some(manager), Some(device)) =
                (&ctx.data.data_device_manager, &ctx.data.data_device)
            else {
                tracing::debug!("no data device; cannot start a drag");
                return false;
            };

            let qh = Self::queue_handle();
            let types: Vec<&str> = mime_types.iter().map(String::as_str).collect();
            // This already sends `set_actions`. `DragSource::set_actions` must
            // not be called afterwards: a second one on the same source is a
            // protocol error, and it issues the request twice besides.
            let source = manager.create_drag_and_drop_source(qh, types, actions);
            source.start_drag(device, origin, icon, serial);
            *CURRENT_DRAG_SOURCE.lock().unwrap() = Some(source);

            // The grab has to reach the compositor now. Waiting for the run
            // loop's next flush would drop the first stretch of pointer motion
            // out of the drag.
            if let Err(err) = ctx.data.connection.flush() {
                tracing::warn!(%err, "could not flush after starting a drag");
            }
            true
        })
    }

    /// The offer for the drag currently over one of our surfaces.
    fn drag_offer() -> Option<smithay_client_toolkit::data_device_manager::data_offer::DragOffer> {
        use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
        Self::with_global(|ctx| DataDevice::data(ctx.data.data_device.as_ref()?).drag_offer())
    }

    /// Answer the source at the current position: which type we would take, and
    /// which action we would perform. See [`crate::dnd::accept`].
    pub(crate) fn accept_drag(
        mime: Option<String>,
        actions: wayland_client::protocol::wl_data_device_manager::DndAction,
        preferred: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
        let Some(offer) = Self::drag_offer() else {
            return;
        };
        // The serial is the enter's, and every accept for this drag carries it.
        offer.accept_mime_type(offer.serial, mime);
        offer.set_actions(actions, preferred);
    }

    /// The action the compositor settled on for the current drag.
    pub(crate) fn drag_selected_action(
    ) -> wayland_client::protocol::wl_data_device_manager::DndAction {
        Self::drag_offer().map_or(
            wayland_client::protocol::wl_data_device_manager::DndAction::empty(),
            |offer| offer.selected_action,
        )
    }

    /// Ask the drag source for `mime`, returning the read end of the pipe.
    pub(crate) fn receive_drag(mime: &str) -> Option<std::fs::File> {
        let offer = Self::drag_offer()?;
        let pipe = match offer.receive(mime.to_string()) {
            Ok(pipe) => pipe,
            Err(err) => {
                tracing::warn!(%err, %mime, "could not receive the dropped data");
                return None;
            }
        };
        Self::with_global(|ctx| {
            // The request has to reach the compositor before we block reading.
            if let Err(err) = ctx.data.connection.flush() {
                tracing::warn!(%err, "could not flush before reading a drop");
            }
        });
        Some(std::fs::File::from(std::os::fd::OwnedFd::from(pipe)))
    }

    /// Tell the source the drop was taken, and let the offer go.
    pub(crate) fn finish_drag() {
        let Some(offer) = Self::drag_offer() else {
            return;
        };
        offer.finish();
        offer.destroy();
        Self::flush();
    }

    pub fn flush() {
        Self::with_global(|ctx| {
            if let Err(err) = ctx.data.connection.flush() {
                tracing::error!(%err, "could not flush the Wayland connection");
            }
        });
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

    pub fn session_lock_manager_ref(
        &self,
    ) -> Option<
        &wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    >{
        self.data.session_lock_manager.as_ref()
    }

    /// The outputs the compositor has announced.
    ///
    /// A client that must cover every screen — a screen locker — reconciles
    /// its surfaces against this rather than tracking hotplug itself.
    pub fn output_state_ref(&self) -> &OutputState {
        &self.data.output_state
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
    // Cursor shape (wp_cursor_shape_v1)
    // ========================================================================

    pub(crate) fn set_last_pointer_enter_serial(serial: u32) {
        LAST_POINTER_ENTER_SERIAL.with(|s| *s.borrow_mut() = serial);
    }

    pub(crate) fn ensure_cursor_shape_device<A: super::App + 'static>(
        context_data: &AppContextData,
        pointer: &wayland_client::protocol::wl_pointer::WlPointer,
        qh: &QueueHandle<super::AppData<A>>,
    ) {
        CURSOR_SHAPE_DEVICE.with(|d| {
            if d.borrow().is_some() {
                return;
            }
            if let Some(ref manager) = context_data.cursor_shape_manager {
                let device = manager.get_pointer(pointer, qh, ());
                *d.borrow_mut() = Some(device);
            }
        });
    }

    /// Set the cursor shape for the current pointer.
    ///
    /// Uses `wp_cursor_shape_v1` — no bitmap loading needed.
    /// Call this from `on_pointer_event` on Enter/Motion events.
    pub fn set_cursor_shape(
        shape: wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape,
    ) {
        let serial = LAST_POINTER_ENTER_SERIAL.with(|s| *s.borrow());
        CURSOR_SHAPE_DEVICE.with(|d| {
            if let Some(ref device) = *d.borrow() {
                device.set_shape(serial, shape);
            }
        });
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

    /// Run `callback` once at the end of every pointer batch, after every
    /// registered pointer callback and after the app's own
    /// `on_pointer_event`.
    ///
    /// This is where a popup decides whether a press landed outside it: the
    /// decision has to be made *after* the owner of the control under the
    /// pointer has had the batch, so a field toggling its own menu shut is
    /// not undone, and it must not wait for a later batch, which may never
    /// come.
    pub fn register_pointer_batch_end_callback<F>(callback: F)
    where
        F: FnMut() + 'static,
    {
        POINTER_BATCH_END_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().push(Box::new(callback));
        });
    }

    /// Subscribe to keyboard-focus loss. The callback receives the id of the
    /// `wl_surface` that lost focus — note that with an active popup grab that
    /// is the *popup's* surface, not the toplevel's.
    pub fn register_keyboard_leave_callback<F>(callback: F)
    where
        F: FnMut(&ObjectId) + 'static,
    {
        KEYBOARD_LEAVE_CALLBACKS.with(|callbacks| {
            callbacks.borrow_mut().push(Box::new(callback));
        });
    }

    pub fn register_window(window: crate::components::window::Window) {
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            // A window closed while it was registered leaves a husk behind —
            // no surface, nothing to draw — so it is swept up here rather
            // than being walked over on every update for the rest of the
            // process's life.
            windows.retain(crate::components::window::Window::is_alive);
            windows.push(window);
        });
    }

    /// Claim the compositor's close request for one window.
    ///
    /// Without a handler a close request is the *application's* — the runner
    /// asks `App::on_close` and, if it agrees, exits. That is right for the
    /// window an application is, and wrong for every secondary window it
    /// opens: closing an inspector is not quitting.
    pub fn register_close_handler<F>(surface_id: ObjectId, handler: F)
    where
        F: FnMut() + 'static,
    {
        CLOSE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().insert(surface_id, Box::new(handler));
        });
    }

    pub fn unregister_close_handler(surface_id: &ObjectId) {
        CLOSE_HANDLERS.with(|handlers| {
            handlers.borrow_mut().remove(surface_id);
        });
    }

    /// Run the close handler for `surface_id`, if it has one. Returns whether
    /// the window took the request.
    pub(crate) fn dispatch_close_request(surface_id: &ObjectId) -> bool {
        // Taken out of the map for the call: a handler that closes its own
        // window unregisters itself, and doing that while the map is borrowed
        // would panic.
        let handler = CLOSE_HANDLERS.with(|handlers| handlers.borrow_mut().remove(surface_id));
        let Some(mut handler) = handler else {
            return false;
        };
        handler();
        // Unless the handler already dropped the window, it is still up and
        // still owns its next close request.
        CLOSE_HANDLERS.with(|handlers| {
            let mut handlers = handlers.borrow_mut();
            handlers.entry(surface_id.clone()).or_insert(handler);
        });
        true
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

    /// Called when the compositor configures a session lock surface. Keyed by
    /// the `ext_session_lock_surface_v1` object.
    pub fn register_lock_surface_configure_callback<F>(lock_surface_id: ObjectId, callback: F)
    where
        F: FnMut(i32, i32, u32) + 'static,
    {
        LOCK_SURFACE_CONFIGURE_CALLBACKS.with(|callbacks| {
            callbacks
                .borrow_mut()
                .insert(lock_surface_id, Box::new(callback));
        });
    }

    pub fn unregister_lock_surface_configure_callback(lock_surface_id: &ObjectId) {
        let _ = LOCK_SURFACE_CONFIGURE_CALLBACKS.try_with(|callbacks| {
            callbacks.borrow_mut().remove(lock_surface_id);
        });
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

    /// Note that `style` augments `surface`, so a `desktop_frame` arriving on
    /// the style object can be filed against the window it is about.
    ///
    /// Called where a style is created for a surface that is a window — a
    /// toplevel or a layer surface. A popup or a subsurface never receives a
    /// desktop position, so registering one would only be misleading.
    pub fn note_style_surface(style: &ObjectId, surface: &ObjectId) {
        STYLE_SURFACES.with(|styles| {
            styles.borrow_mut().insert(style.clone(), surface.clone());
        });
    }

    /// Record where the compositor says it is drawing a window.
    pub(crate) fn set_desktop_frame(style: &ObjectId, frame: (f32, f32, f32, f32)) {
        let surface = STYLE_SURFACES.with(|styles| styles.borrow().get(style).cloned());
        let Some(surface) = surface else {
            return;
        };
        DESKTOP_FRAMES.with(|frames| {
            frames.borrow_mut().insert(surface, frame);
        });
    }

    /// Where the compositor is drawing this window on the desktop, in physical
    /// pixels — `None` until it has said, and on a compositor that does not
    /// speak version 4 of the style protocol, never.
    ///
    /// The desktop's coordinate space, not the window's: this is the one thing
    /// a Wayland client cannot work out for itself, and without it a window
    /// cannot answer "what is at this point on the screen" — which is how an
    /// assistive technology reads a desktop. See
    /// [`crate::accessibility`].
    pub fn desktop_frame(surface: &ObjectId) -> Option<(f32, f32, f32, f32)> {
        DESKTOP_FRAMES.with(|frames| frames.borrow().get(surface).copied())
    }

    /// Remember the serial of an input event, for a request that needs one
    /// later.
    pub(crate) fn note_input_serial(serial: u32) {
        LAST_INPUT_SERIAL.with(|last| last.set(serial));
    }

    /// The serial of the most recent input event, or 0 if there has not been
    /// one.
    ///
    /// Opening a pop-up needs a serial the compositor recognises, and the
    /// press that asked for it is usually right there. This is for the cases
    /// where it is not: an assistive technology asking for a control to be
    /// activated sends no input event of its own, and a request carrying no
    /// serial at all would simply be refused.
    pub fn last_input_serial() -> u32 {
        LAST_INPUT_SERIAL.with(|last| last.get())
    }

    /// Record the output geometry the compositor reported for a style surface.
    pub fn set_output_frame(style: &ObjectId, frame: (f32, f32, f32, f32)) {
        OUTPUT_FRAMES.with(|frames| {
            frames.borrow_mut().insert(style.clone(), frame);
        });
    }

    /// Forget what the compositor last said, so [`AppContext::output_frame`]
    /// reads `None` until a fresh answer arrives.
    ///
    /// The answer is relative to the surface's parent, so moving the window
    /// invalidates it. A caller that is about to ask again clears it first,
    /// which is how it can tell the new reply from the old one.
    pub fn clear_output_frame(style: &ObjectId) {
        OUTPUT_FRAMES.with(|frames| {
            frames.borrow_mut().remove(style);
        });
    }

    /// Where the output is, in the coordinates this surface's positions are
    /// set in — `None` until the compositor has answered a
    /// `request_output_frame`.
    ///
    /// This is the one thing a Wayland client cannot work out for itself: it
    /// is never told where its own window sits, so a surface that has to be
    /// placed against the display rather than against its parent has to ask.
    pub fn output_frame(style: &ObjectId) -> Option<(f32, f32, f32, f32)> {
        OUTPUT_FRAMES.with(|frames| frames.borrow().get(style).copied())
    }

    /// Ask for a frame callback and remember that one is outstanding.
    ///
    /// Painting again before that callback arrives only queues work the
    /// compositor has not asked for, so a client with continuous content
    /// should hold off while [`AppContext::frame_in_flight`] is true.
    pub fn request_throttled_frame(surface: &wl_surface::WlSurface) {
        use wayland_client::Proxy;

        Self::request_frame(surface);
        FRAMES_IN_FLIGHT.with(|surfaces| {
            surfaces.borrow_mut().insert(surface.id());
        });
    }

    /// Whether a frame committed on this surface has yet to be presented.
    pub fn frame_in_flight(surface_id: &ObjectId) -> bool {
        FRAMES_IN_FLIGHT.with(|surfaces| surfaces.borrow().contains(surface_id))
    }

    pub(crate) fn clear_frame_in_flight(surface_id: &ObjectId) {
        FRAMES_IN_FLIGHT.with(|surfaces| {
            surfaces.borrow_mut().remove(surface_id);
        });
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

    /// Ask the run loop to stop.
    ///
    /// The loop finishes the current iteration and flushes first, so a request
    /// made just before this — a screen locker's `unlock` — reaches the
    /// compositor rather than dying with the connection.
    pub fn request_exit() {
        EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        Self::request_wakeup();
    }

    pub(crate) fn exit_requested() -> bool {
        EXIT_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
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

    pub(crate) fn dispatch_pointer_batch_end() {
        POINTER_BATCH_END_CALLBACKS.with(|callbacks| {
            // Taken out of the RefCell first: a callback closing a popup can
            // register further callbacks, and would otherwise borrow this
            // list while it is still borrowed here.
            let mut taken = std::mem::take(&mut *callbacks.borrow_mut());
            for callback in taken.iter_mut() {
                callback();
            }
            let mut slot = callbacks.borrow_mut();
            taken.append(&mut slot);
            *slot = taken;
        });
    }

    pub(crate) fn dispatch_keyboard_leave(surface_id: &ObjectId) {
        KEYBOARD_LEAVE_CALLBACKS.with(|callbacks| {
            // Copy out of the RefCell borrow first: a callback may close a menu,
            // which can register further callbacks.
            let mut taken = std::mem::take(&mut *callbacks.borrow_mut());
            for callback in taken.iter_mut() {
                callback(surface_id);
            }
            let mut slot = callbacks.borrow_mut();
            taken.append(&mut slot);
            *slot = taken;
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

    pub(crate) fn dispatch_lock_surface_configure(
        lock_surface_id: &ObjectId,
        width: i32,
        height: i32,
        serial: u32,
    ) {
        LOCK_SURFACE_CONFIGURE_CALLBACKS.with(|callbacks| {
            if let Some(callback) = callbacks.borrow_mut().get_mut(lock_surface_id) {
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
    // Keyboard focus
    // ========================================================================

    /// Works on a surface's focus ring, creating it on first use.
    ///
    /// Called from the application's build pass to declare what can be focused
    /// (`begin`, `add`.., `end`) and again while drawing to ask what is.
    pub fn with_focus_ring<R>(
        surface_id: &ObjectId,
        f: impl FnOnce(&mut crate::focus::FocusRing) -> R,
    ) -> R {
        FOCUS_RINGS.with(|rings| {
            let mut rings = rings.borrow_mut();
            let ring = rings.entry(surface_id.clone()).or_default();
            f(ring)
        })
    }

    /// Which control in a surface has the keyboard, if any.
    /// Moves the keyboard focus within a surface, as Tab would.
    ///
    /// What an assistive technology's `Focus` request goes through, so a
    /// screen reader moving the focus and the user pressing Tab leave the
    /// application in the same state. Returns whether the control was there to
    /// be focused.
    pub fn focus_control(surface_id: &ObjectId, id: Option<crate::focus::FocusId>) -> bool {
        FOCUS_RINGS.with(|rings| {
            let mut rings = rings.borrow_mut();
            let Some(ring) = rings.get_mut(surface_id) else {
                return false;
            };
            match id {
                Some(id) => ring.focus(id),
                None => ring.clear(),
            }
        })
    }

    pub fn focused_control(surface_id: &ObjectId) -> Option<crate::focus::FocusId> {
        FOCUS_RINGS.with(|rings| rings.borrow().get(surface_id).and_then(|r| r.focused()))
    }

    /// Drops a surface's focus ring. Called when the surface goes away.
    pub fn forget_focus_ring(surface_id: &ObjectId) {
        let _ = FOCUS_RINGS.try_with(|rings| {
            rings.borrow_mut().remove(surface_id);
        });
        let _ = KEYBOARD_FOCUS.try_with(|focus| {
            let mut focus = focus.borrow_mut();
            if focus.as_ref() == Some(surface_id) {
                *focus = None;
            }
        });
    }

    /// The surface holding the keyboard, or `None` when no window of this
    /// application does.
    pub fn keyboard_focus() -> Option<ObjectId> {
        KEYBOARD_FOCUS.with(|focus| focus.borrow().clone())
    }

    pub(crate) fn set_keyboard_focus(surface_id: Option<ObjectId>) {
        KEYBOARD_FOCUS.with(|focus| *focus.borrow_mut() = surface_id);
    }

    /// Modifier state as of the last `wl_keyboard.modifiers` event.
    pub fn current_modifiers() -> super::Modifiers {
        CURRENT_MODIFIERS.with(|modifiers| *modifiers.borrow())
    }

    pub(crate) fn set_current_modifiers(modifiers: super::Modifiers) {
        CURRENT_MODIFIERS.with(|current| *current.borrow_mut() = modifiers);
    }

    // ========================================================================
    // Accessibility
    // ========================================================================

    /// Makes a surface visible to assistive technologies.
    ///
    /// Call it once the surface exists. From then on the run loop asks the
    /// application to describe it through [`crate::App::accessibility`] —
    /// but only while something is actually listening, so a session with no
    /// screen reader pays for an idle D-Bus connection and nothing else.
    pub fn enable_accessibility(surface_id: &ObjectId) {
        A11Y_ADAPTERS.with(|adapters| {
            adapters
                .borrow_mut()
                .entry(surface_id.clone())
                .or_insert_with(crate::accessibility::SurfaceAdapter::new);
        });
    }

    /// Drops a surface's adapter, ending its AT-SPI presence.
    ///
    /// Deliberately not called when a render surface goes away: a Skia surface
    /// is rebuilt on the first configure and on every resize, while the window
    /// — and what a screen reader is reading — lives on. It ends with the
    /// window, in [`crate::components::window::Window::close`].
    pub fn disable_accessibility(surface_id: &ObjectId) {
        let _ = A11Y_ADAPTERS.try_with(|adapters| {
            adapters.borrow_mut().remove(surface_id);
        });
    }

    /// The surfaces made accessible, in no particular order.
    pub(crate) fn accessible_surfaces() -> Vec<ObjectId> {
        A11Y_ADAPTERS.with(|adapters| adapters.borrow().keys().cloned().collect())
    }

    /// Whether an assistive technology is waiting for `surface`'s tree, and
    /// whatever it has asked the application to do since the last pass.
    pub(crate) fn accessibility_pending(
        surface_id: &ObjectId,
    ) -> (bool, Vec<crate::accessibility::ActionRequest>) {
        A11Y_ADAPTERS.with(|adapters| {
            let adapters = adapters.borrow();
            let Some(adapter) = adapters.get(surface_id) else {
                return (false, Vec::new());
            };
            (adapter.mailbox.is_wanted(), adapter.mailbox.take_actions())
        })
    }

    /// Hands a freshly built tree to the adapter.
    pub(crate) fn push_accessibility_tree(surface_id: &ObjectId, update: accesskit::TreeUpdate) {
        A11Y_ADAPTERS.with(|adapters| {
            if let Some(adapter) = adapters.borrow_mut().get_mut(surface_id) {
                adapter.update(|| update);
            }
        });
    }

    /// Tells the adapter whether its surface has the keyboard. A screen reader
    /// reads the focused window, so without this it has no reason to read any.
    pub(crate) fn set_accessibility_window_focus(surface_id: &ObjectId, focused: bool) {
        A11Y_ADAPTERS.with(|adapters| {
            if let Some(adapter) = adapters.borrow_mut().get_mut(surface_id) {
                adapter.set_window_focused(focused);
            }
        });
    }

    /// Tell a surface's adapter where its window is, so what it describes can
    /// be found by pointing at the screen.
    ///
    /// The compositor answers in physical pixels, like the rest of the style
    /// protocol; an assistive technology works in logical ones, which is also
    /// the space an application draws and describes in. Dividing here is what
    /// keeps that conversion in one place.
    ///
    /// Does nothing until the compositor has said where the window is — on one
    /// that does not speak version 4 of the style protocol, never. The tree is
    /// still published; only its coordinates are then the window's own, which
    /// is what every kit application did before this existed.
    pub(crate) fn sync_accessibility_desktop_frame(surface_id: &ObjectId) {
        let Some((x, y, width, height)) = Self::desktop_frame(surface_id) else {
            return;
        };
        let scale = Self::fractional_scale().max(0.1);
        let frame = accesskit::Rect::new(
            f64::from(x) / scale,
            f64::from(y) / scale,
            f64::from(x + width) / scale,
            f64::from(y + height) / scale,
        );
        A11Y_ADAPTERS.with(|adapters| {
            if let Some(adapter) = adapters.borrow_mut().get_mut(surface_id) {
                adapter.set_desktop_frame(frame);
            }
        });
    }

    // ========================================================================
    // Window update loop
    // ========================================================================

    /// Asks the window owning `surface_id` to paint a frame.
    ///
    /// Moving the keyboard focus changes what the window looks like — the ring
    /// is around a different control — but nothing else about the window has
    /// changed, so without this the new ring is not drawn until something else
    /// happens to repaint. See `move_focus_for_key`.
    pub fn request_window_frame(surface_id: &ObjectId) {
        WINDOWS.with(|windows| {
            for window in windows.borrow().iter() {
                if window.surface_id().as_ref() == Some(surface_id) {
                    window.request_frame();
                }
            }
        });
    }

    /// Whether `surface_id` is one of this application's toplevel windows.
    ///
    /// Layer surfaces and popups are not registered here, which is what makes
    /// this the test for "is the keyboard on something Cmd+W should close".
    pub fn is_toplevel_surface(surface_id: &ObjectId) -> bool {
        WINDOWS.with(|windows| {
            windows
                .borrow()
                .iter()
                .any(|window| window.surface_id().as_ref() == Some(surface_id))
        })
    }

    pub fn update_windows() {
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            windows.retain(crate::components::window::Window::is_alive);
            for window in windows.iter_mut() {
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
        POINTER_BATCH_END_CALLBACKS.with(|c| c.borrow_mut().clear());
        KEYBOARD_LEAVE_CALLBACKS.with(|c| c.borrow_mut().clear());
        FRAME_CALLBACKS.with(|c| c.borrow_mut().clear());
        POPUP_CONFIGURE_CALLBACKS.with(|c| c.borrow_mut().clear());
        POPUP_DONE_CALLBACKS.with(|c| c.borrow_mut().clear());
        LAYER_SHELL_CONFIGURE_CALLBACKS.with(|c| c.borrow_mut().clear());
        LOCK_SURFACE_CONFIGURE_CALLBACKS.with(|c| c.borrow_mut().clear());
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

#[cfg(test)]
mod scale_latch_tests {
    use super::*;

    /// The compositor's opening `preferred_scale` is the one the process keeps:
    /// a later change must not move geometry under buffers that never re-raster.
    #[test]
    fn fractional_scale_ignores_later_changes() {
        DISPLAY_FRACTIONAL_SCALE_120.store(0, std::sync::atomic::Ordering::Relaxed);

        AppContext::set_fractional_scale_120(180); // 1.5x
        assert_eq!(AppContext::fractional_scale(), 1.5);

        AppContext::set_fractional_scale_120(240); // user switches to 2x
        assert_eq!(
            AppContext::fractional_scale(),
            1.5,
            "scale change should wait for a restart"
        );
    }
}
