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
        wl_buffer, wl_callback, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Shared state for the test client's Wayland event dispatching.
#[derive(Debug)]
pub struct TestClientState {
    pub wl_compositor: Option<wl_compositor::WlCompositor>,
    pub wl_shm: Option<wl_shm::WlShm>,
    pub wl_seat: Option<wl_seat::WlSeat>,
    pub xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub shm_formats: Vec<wl_shm::Format>,
}

impl TestClientState {
    fn new() -> Self {
        Self {
            wl_compositor: None,
            wl_shm: None,
            wl_seat: None,
            xdg_wm_base: None,
            shm_formats: Vec::new(),
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
            closed: false,
            title: title.to_string(),
        }));

        let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &self.qh, toplevel_state.clone());
        let toplevel = xdg_surface.get_toplevel(&self.qh, toplevel_state.clone());
        toplevel.set_title(title.to_string());

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

        // Roundtrip to receive configure
        let _ = self.roundtrip();

        // Commit the buffer after configure
        surface.commit();

        toplevel_state
    }
}

/// Tracks the state of a test XDG toplevel.
#[derive(Debug)]
pub struct TestToplevel {
    pub configured: bool,
    pub width: i32,
    pub height: i32,
    pub closed: bool,
    pub title: String,
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
        _state: &mut Self,
        _proxy: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
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
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    let mut tl = data.lock().unwrap();
                    tl.width = width;
                    tl.height = height;
                }
            }
            xdg_toplevel::Event::Close => {
                data.lock().unwrap().closed = true;
            }
            _ => {}
        }
    }
}
