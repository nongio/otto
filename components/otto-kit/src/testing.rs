//! Test utilities for writing integration tests against a Wayland compositor.
//!
//! This module provides lightweight Wayland client primitives designed for use
//! in integration tests. Unlike the full `AppRunner` stack, these utilities
//! don't require EGL/Skia — they use SHM buffers and raw protocol interactions.
//!
//! # Example
//!
//! ```no_run
//! use otto_kit::testing::TestClient;
//!
//! let mut client = TestClient::connect("wayland-1").unwrap();
//! let toplevel = client.create_toplevel("test-window", 200, 150);
//! client.roundtrip().unwrap();
//! assert!(toplevel.lock().unwrap().configured);
//! ```

use std::{
    os::unix::io::AsFd,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
};

use wayland_client::{
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager,
        wl_data_offer, wl_data_source, wl_keyboard, wl_pointer, wl_region, wl_registry, wl_seat,
        wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use crate::protocols::{otto_surface_style_manager_v1, otto_surface_style_v1};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1, ext_background_effect_surface_v1,
};

/// Shared state for the test client's Wayland event dispatching.
#[derive(Debug)]
pub struct TestClientState {
    pub wl_compositor: Option<wl_compositor::WlCompositor>,
    pub wl_shm: Option<wl_shm::WlShm>,
    pub wl_seat: Option<wl_seat::WlSeat>,
    pub xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// The compositor-side material protocol, when the compositor advertises it.
    pub otto_surface_style_manager:
        Option<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1>,
    pub ext_background_effect_manager:
        Option<ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1>,
    /// The `capabilities` bitmask the background-effect global sent on bind.
    pub background_effect_capabilities: Option<u32>,
    pub shm_formats: Vec<wl_shm::Format>,
    /// The seat's keyboard, once the compositor announces the capability.
    pub wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Keys delivered to this client, as `(evdev code, pressed)` in arrival
    /// order — what a remote input injector's key presses look like from the
    /// application's side.
    pub keys: Vec<(u32, bool)>,
    /// Whether the compositor has given this client's surface keyboard focus.
    pub keyboard_focused: bool,
    /// The seat's pointer, once the compositor announces the capability.
    pub wl_pointer: Option<wl_pointer::WlPointer>,
    /// The serial of the most recent pointer button press. Starting a drag
    /// needs one: the compositor checks it against the grab it handed out, and
    /// a made-up number is refused.
    pub last_button_serial: Option<u32>,
    /// Drag and drop, when the compositor advertises it.
    pub wl_data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub wl_data_device: Option<wl_data_device::WlDataDevice>,
    /// Set when a drag this client started was cancelled by the compositor.
    pub drag_cancelled: bool,
}

impl TestClientState {
    fn new() -> Self {
        Self {
            wl_compositor: None,
            wl_shm: None,
            wl_seat: None,
            xdg_wm_base: None,
            otto_surface_style_manager: None,
            ext_background_effect_manager: None,
            background_effect_capabilities: None,
            shm_formats: Vec::new(),
            wl_keyboard: None,
            keys: Vec::new(),
            keyboard_focused: false,
            wl_pointer: None,
            last_button_serial: None,
            wl_data_device_manager: None,
            wl_data_device: None,
            drag_cancelled: false,
        }
    }
}

/// A lightweight Wayland client for integration testing.
///
/// Connects to a compositor via the given socket name and provides methods
/// for creating surfaces, toplevels, and performing roundtrips.
pub struct TestClient {
    pub conn: Connection,
    pub queue: EventQueue<TestClientState>,
    pub qh: QueueHandle<TestClientState>,
    pub state: TestClientState,
}

impl TestClient {
    /// Connect to the compositor at the given socket name.
    pub fn connect(socket_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let socket_path = std::env::var("XDG_RUNTIME_DIR")
            .map(|dir| format!("{}/{}", dir, socket_name))
            .unwrap_or_else(|_| {
                let uid = rustix::process::getuid().as_raw();
                format!("/run/user/{}/{}", uid, socket_name)
            });

        let stream = UnixStream::connect(&socket_path)?;
        let conn = Connection::from_socket(stream)?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = TestClientState::new();
        // Initial roundtrip to bind globals
        queue.roundtrip(&mut state)?;

        Ok(Self {
            conn,
            queue,
            qh,
            state,
        })
    }

    /// Perform a blocking roundtrip — sends pending requests and waits for
    /// the compositor to process them and respond.
    pub fn roundtrip(&mut self) -> Result<usize, Box<dyn std::error::Error>> {
        Ok(self.queue.roundtrip(&mut self.state)?)
    }

    /// Dispatch pending events without blocking.
    pub fn dispatch_pending(&mut self) -> Result<usize, Box<dyn std::error::Error>> {
        let n = self.queue.dispatch_pending(&mut self.state)?;
        self.conn.flush()?;
        Ok(n)
    }

    /// Create a wl_surface.
    pub fn create_surface(&self) -> wl_surface::WlSurface {
        self.state
            .wl_compositor
            .as_ref()
            .expect("compositor not bound")
            .create_surface(&self.qh, ())
    }

    /// Ask the compositor to give a surface a translucent, blurred material,
    /// the way a real otto-kit app dresses its popups.
    ///
    /// Returns the style object so the caller can keep it alive — dropping it
    /// destroys the style and the surface goes back to plain. Returns `None`
    /// when the compositor does not advertise `otto_surface_style_manager_v1`.
    pub fn request_material(
        &self,
        surface: &wl_surface::WlSurface,
    ) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        let manager = self.state.otto_surface_style_manager.as_ref()?;
        let style = manager.get_surface_style(surface, &self.qh, ());
        style.set_background_color(0.9, 0.9, 0.9, 0.85);
        style.set_blend_mode(otto_surface_style_v1::BlendMode::BackgroundBlur);
        Some(style)
    }

    /// Ask for the standard `ext-background-effect-v1` blur behind `surface`,
    /// the way foot or wezterm do with a translucent background: the whole
    /// surface as the blur region. Takes effect on the surface's next commit.
    ///
    /// Returns the effect object so the caller can keep it alive — dropping
    /// it removes the blur on the next commit. Returns `None` when the
    /// compositor does not advertise `ext_background_effect_manager_v1`.
    pub fn request_background_blur(
        &self,
        surface: &wl_surface::WlSurface,
        width: i32,
        height: i32,
    ) -> Option<ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1> {
        let manager = self.state.ext_background_effect_manager.as_ref()?;
        let effect = manager.get_background_effect(surface, &self.qh, ());
        let region = self
            .state
            .wl_compositor
            .as_ref()
            .expect("compositor not bound")
            .create_region(&self.qh, ());
        region.add(0, 0, width, height);
        effect.set_blur_region(Some(&region));
        region.destroy();
        Some(effect)
    }

    /// Ask the compositor to round the surface's corners, the way every
    /// otto-kit window does. The client's buffer keeps its square corners —
    /// the rounding is a clip the compositor applies — which is exactly why
    /// such a window cannot be scanned out raw.
    pub fn request_rounded(
        &self,
        surface: &wl_surface::WlSurface,
    ) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        let manager = self.state.otto_surface_style_manager.as_ref()?;
        let style = manager.get_surface_style(surface, &self.qh, ());
        style.set_corner_radius(36.0);
        style.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
        Some(style)
    }

    /// A drag icon: a plain surface with a buffer already attached, ready to
    /// be handed to [`Self::start_drag`].
    ///
    /// It is *not* committed here. A surface only takes the drag-icon role
    /// when `start_drag` gives it one, and committing a buffer before that
    /// would map it as an ordinary surface instead.
    pub fn create_drag_icon(
        &mut self,
        width: u32,
        height: u32,
    ) -> (wl_surface::WlSurface, ShmBuffer) {
        let shm = self.state.wl_shm.clone().expect("shm not bound");
        let buffer = ShmBuffer::new(&shm, &self.qh, width, height);
        let surface = self.create_surface();
        surface.attach(Some(buffer.buffer()), 0, 0);
        (surface, buffer)
    }

    /// Start a drag from `origin`, optionally carrying `icon` under the
    /// cursor — what a file manager does when a press turns into a drag.
    ///
    /// Needs a real press serial, so the caller has to have pressed the
    /// pointer over `origin` first; returns `None` when no press has been
    /// seen, rather than inventing one the compositor would refuse.
    pub fn start_drag(
        &mut self,
        origin: &wl_surface::WlSurface,
        icon: Option<&wl_surface::WlSurface>,
        mime_types: &[&str],
    ) -> Option<wl_data_source::WlDataSource> {
        let manager = self.state.wl_data_device_manager.clone()?;
        let seat = self.state.wl_seat.clone()?;
        let serial = self.state.last_button_serial?;

        let device = match self.state.wl_data_device.clone() {
            Some(device) => device,
            None => {
                let device = manager.get_data_device(&seat, &self.qh, ());
                self.state.wl_data_device = Some(device.clone());
                device
            }
        };

        let source = manager.create_data_source(&self.qh, ());
        for mime in mime_types {
            source.offer(mime.to_string());
        }
        device.start_drag(Some(&source), origin, icon, serial);
        // The icon has its role now, so its buffer can go up.
        if let Some(icon) = icon {
            icon.damage(0, 0, i32::MAX, i32::MAX);
            icon.commit();
        }
        Some(source)
    }

    /// Create an XDG toplevel window and attach a minimal SHM buffer.
    ///
    /// Returns a shared reference to the toplevel state which tracks
    /// configure events.
    pub fn create_toplevel(
        &mut self,
        title: &str,
        width: u32,
        height: u32,
    ) -> Arc<Mutex<TestToplevel>> {
        self.create_toplevel_inner(title, None, width, height, false)
    }

    /// Create a toplevel that also announces an `app_id`, the way a real
    /// application does. The id is set before the first commit, so it is
    /// already there when the compositor maps the window and registers it with
    /// the foreign-toplevel protocols.
    pub fn create_toplevel_with_app_id(
        &mut self,
        title: &str,
        app_id: &str,
        width: u32,
        height: u32,
    ) -> Arc<Mutex<TestToplevel>> {
        self.create_toplevel_inner(title, Some(app_id), width, height, false)
    }

    /// Create a toplevel that asks to be maximized before its first commit,
    /// the way a browser restoring a maximized window from its last session
    /// does. The request arrives before the surface has ever been mapped.
    pub fn create_maximized_toplevel(
        &mut self,
        title: &str,
        width: u32,
        height: u32,
    ) -> Arc<Mutex<TestToplevel>> {
        self.create_toplevel_inner(title, None, width, height, true)
    }

    fn create_toplevel_inner(
        &mut self,
        title: &str,
        app_id: Option<&str>,
        width: u32,
        height: u32,
        maximized: bool,
    ) -> Arc<Mutex<TestToplevel>> {
        let surface = self.create_surface();

        let xdg_wm_base = self
            .state
            .xdg_wm_base
            .as_ref()
            .expect("xdg_wm_base not bound");

        let toplevel_state = Arc::new(Mutex::new(TestToplevel {
            configured: false,
            width: width as i32,
            height: height as i32,
            maximized: false,
            closed: false,
            title: title.to_string(),
            surface: surface.clone(),
            buffer: None,
            xdg_surface: None,
        }));

        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &self.qh, toplevel_state.clone());
        toplevel_state.lock().unwrap().xdg_surface = Some(xdg_surface.clone());
        let toplevel = xdg_surface.get_toplevel(&self.qh, toplevel_state.clone());
        toplevel.set_title(title.to_string());
        if let Some(app_id) = app_id {
            toplevel.set_app_id(app_id.to_string());
        }
        if maximized {
            toplevel.set_maximized();
        }

        // Commit to trigger the initial configure
        surface.commit();

        // Attach a minimal SHM buffer
        let buffer = ShmBuffer::new(
            self.state.wl_shm.as_ref().expect("wl_shm not bound"),
            &self.qh,
            width,
            height,
        );
        surface.attach(Some(buffer.buffer()), 0, 0);
        toplevel_state.lock().unwrap().buffer = Some(buffer.buffer().clone());

        // Roundtrip to receive configure
        let _ = self.roundtrip();

        // Commit the buffer after configure
        surface.commit();

        toplevel_state
    }

    /// Create an XDG popup anchored to `parent`, the way a client puts up a
    /// tooltip or a context menu, and attach a minimal SHM buffer.
    ///
    /// `x`/`y` are the anchor rect's origin in the parent's surface-local
    /// coordinates.
    pub fn create_popup(
        &mut self,
        parent: &Arc<Mutex<TestToplevel>>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Arc<Mutex<TestPopup>> {
        let surface = self.create_surface();
        let xdg_wm_base = self
            .state
            .xdg_wm_base
            .as_ref()
            .expect("xdg_wm_base not bound");

        let positioner = xdg_wm_base.create_positioner(&self.qh, ());
        positioner.set_size(width as i32, height as i32);
        positioner.set_anchor_rect(x, y, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::BottomLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);

        let popup_state = Arc::new(Mutex::new(TestPopup {
            configured: false,
            width: width as i32,
            height: height as i32,
            done: false,
            surface: surface.clone(),
            buffer: None,
        }));

        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &self.qh, popup_state.clone());
        let parent_xdg = parent
            .lock()
            .unwrap()
            .xdg_surface
            .clone()
            .expect("parent toplevel has no xdg_surface");
        let _popup = xdg_surface.get_popup(
            Some(&parent_xdg),
            &positioner,
            &self.qh,
            popup_state.clone(),
        );
        positioner.destroy();

        // Commit to trigger the initial configure, then attach and commit.
        surface.commit();
        let buffer = ShmBuffer::new(
            self.state.wl_shm.as_ref().expect("wl_shm not bound"),
            &self.qh,
            width,
            height,
        );
        surface.attach(Some(buffer.buffer()), 0, 0);
        popup_state.lock().unwrap().buffer = Some(buffer.buffer().clone());
        let _ = self.roundtrip();
        surface.commit();

        popup_state
    }
}

/// Tracks the state of a test XDG popup.
#[derive(Debug)]
pub struct TestPopup {
    pub configured: bool,
    pub width: i32,
    pub height: i32,
    /// The compositor dismissed the popup.
    pub done: bool,
    pub surface: wl_surface::WlSurface,
    pub buffer: Option<wl_buffer::WlBuffer>,
}

impl TestPopup {
    /// Re-attach the buffer, damage the whole surface and commit — a popup
    /// redrawing its own content.
    pub fn commit_frame(&self) {
        self.surface.attach(self.buffer.as_ref(), 0, 0);
        self.surface.damage(0, 0, self.width, self.height);
        self.surface.commit();
    }
}

/// Tracks the state of a test XDG toplevel.
#[derive(Debug)]
pub struct TestToplevel {
    pub configured: bool,
    /// Size the compositor last configured, which is not the size of the
    /// attached buffer: this client never resizes itself.
    pub width: i32,
    pub height: i32,
    /// Whether the last configure carried the `maximized` state.
    pub maximized: bool,
    pub closed: bool,
    pub title: String,
    /// The toplevel's wl_surface, so tests can push further commits.
    pub surface: wl_surface::WlSurface,
    /// The buffer attached at map time, so tests can re-attach it.
    pub buffer: Option<wl_buffer::WlBuffer>,
    /// The toplevel's xdg_surface, so tests can hang popups off it.
    pub xdg_surface: Option<xdg_surface::XdgSurface>,
}

impl TestToplevel {
    /// Re-attach the buffer, damage the whole surface and commit,
    /// simulating a client redraw.
    pub fn commit_frame(&self) {
        self.surface.attach(self.buffer.as_ref(), 0, 0);
        self.surface.damage(0, 0, self.width, self.height);
        self.surface.commit();
    }
}

/// A minimal SHM buffer backed by a memfd.
pub struct ShmBuffer {
    _pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
}

impl ShmBuffer {
    pub fn new(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<TestClientState>,
        width: u32,
        height: u32,
    ) -> Self {
        let stride = width * 4;
        let size = (stride * height) as usize;

        // Create a memfd for the SHM pool
        let fd = rustix::fs::memfd_create(c"otto-test-shm", rustix::fs::MemfdFlags::CLOEXEC)
            .expect("memfd_create failed");

        rustix::io::retry_on_intr(|| rustix::fs::ftruncate(&fd, size as u64))
            .expect("ftruncate failed");

        let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );

        Self {
            _pool: pool,
            buffer,
        }
    }

    pub fn buffer(&self) -> &wl_buffer::WlBuffer {
        &self.buffer
    }
}

// --- Wayland dispatch implementations ---

impl Dispatch<wl_registry::WlRegistry, ()> for TestClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.wl_compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    state.wl_shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_seat" => {
                    state.wl_seat = Some(registry.bind(name, version.min(9), qh, ()));
                }
                "xdg_wm_base" => {
                    state.xdg_wm_base = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "wl_data_device_manager" => {
                    state.wl_data_device_manager =
                        Some(registry.bind(name, version.min(3), qh, ()));
                }
                "otto_surface_style_manager_v1" => {
                    state.otto_surface_style_manager =
                        Some(registry.bind(name, version.min(2), qh, ()));
                }
                "ext_background_effect_manager_v1" => {
                    state.ext_background_effect_manager =
                        Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

// No-op dispatchers for bound globals
impl Dispatch<wl_compositor::WlCompositor, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _proxy: &wl_shm::WlShm,
        event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format { format } = event {
            if let wayland_client::WEnum::Value(fmt) = format {
                state.shm_formats.push(fmt);
            }
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm_pool::WlShmPool,
        _event: wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TestClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Take the keyboard as soon as the seat announces one, so tests can
        // observe what actually reaches the application.
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) && state.wl_keyboard.is_none() {
                state.wl_keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.wl_pointer.is_none() {
                state.wl_pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.keyboard_focused = true,
            wl_keyboard::Event::Leave { .. } => state.keyboard_focused = false,
            wl_keyboard::Event::Key { key, state: s, .. } => {
                let pressed = matches!(
                    s,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                );
                state.keys.push((key, pressed));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Only the press serials matter here: they are what authorises a drag.
        if let wl_pointer::Event::Button {
            serial,
            state: wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            state.last_button_serial = Some(serial);
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_data_device_manager::WlDataDeviceManager,
        _event: wl_data_device_manager::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // A drag this client is the source of still gets its own offers back
        // when it passes over its own surfaces; nothing here needs them.
        if let wl_data_device::Event::DataOffer { id } = event {
            id.destroy();
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_data_offer::WlDataOffer,
        _event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_data_source::Event::Cancelled) {
            state.drag_cancelled = true;
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_callback::WlCallback,
        _event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()> for TestClientState {
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

impl Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()> for TestClientState {
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

impl Dispatch<wl_region::WlRegion, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1, ()>
    for TestClientState
{
    fn event(
        state: &mut Self,
        _proxy: &ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.background_effect_capabilities = Some(u32::from(flags));
        }
    }
}

impl Dispatch<ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1, ()>
    for TestClientState
{
    fn event(
        _state: &mut Self,
        _proxy: &ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
        _event: ext_background_effect_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_surface::XdgSurface, Arc<Mutex<TestToplevel>>> for TestClientState {
    fn event(
        _state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        data: &Arc<Mutex<TestToplevel>>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            data.lock().unwrap().configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, Arc<Mutex<TestToplevel>>> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        data: &Arc<Mutex<TestToplevel>>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                let mut tl = data.lock().unwrap();
                if width > 0 && height > 0 {
                    tl.width = width;
                    tl.height = height;
                }
                // States arrive as a flat array of little-endian u32 enum values.
                tl.maximized = states
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .any(|s| s == u32::from(xdg_toplevel::State::Maximized));
            }
            xdg_toplevel::Event::Close => {
                data.lock().unwrap().closed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_positioner::XdgPositioner, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &xdg_positioner::XdgPositioner,
        _event: xdg_positioner::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_surface::XdgSurface, Arc<Mutex<TestPopup>>> for TestClientState {
    fn event(
        _state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        data: &Arc<Mutex<TestPopup>>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            data.lock().unwrap().configured = true;
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, Arc<Mutex<TestPopup>>> for TestClientState {
    fn event(
        _state: &mut Self,
        _proxy: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        data: &Arc<Mutex<TestPopup>>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                let mut p = data.lock().unwrap();
                if width > 0 && height > 0 {
                    p.width = width;
                    p.height = height;
                }
            }
            xdg_popup::Event::PopupDone => {
                data.lock().unwrap().done = true;
            }
            _ => {}
        }
    }
}
