use std::fs::File;
use std::os::fd::AsFd;

use anyhow::{Context, Result};
use memmap2::MmapMut;
use tempfile::tempfile;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

const MAIN_SIZE: (u32, u32) = (360, 260);
const SUBMENU_SIZE: (u32, u32) = (140, 120);
const SUBMENU_OFFSET: (i32, i32) = (MAIN_SIZE.0 as i32 - SUBMENU_SIZE.0 as i32 - 12, 40);
const RIGHT_BUTTON: u32 = 0x111; // BTN_RIGHT from linux/input-event-codes.h

#[derive(Clone, Copy)]
struct Color(u8, u8, u8);

impl Color {
    fn packed(self) -> u32 {
        0xff00_0000 | ((self.0 as u32) << 16) | ((self.1 as u32) << 8) | self.2 as u32
    }
}

struct ShmBuffer {
    width: u32,
    height: u32,
    mmap: MmapMut,
    buffer: WlBuffer,
    _file: File,
    _pool: WlShmPool,
}

impl ShmBuffer {
    fn fill(&mut self, color: Color) {
        self.mmap
            .chunks_exact_mut(4)
            .for_each(|chunk| chunk.copy_from_slice(&color.packed().to_le_bytes()));
    }

    fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        let pixel = color.packed().to_le_bytes();
        for row in y..(y + height).min(self.height) {
            let start = (row * self.width + x).min(self.width * self.height) as usize * 4;
            let end = (row * self.width + (x + width).min(self.width)) as usize * 4;
            let row_slice = &mut self.mmap[start..end];
            for chunk in row_slice.chunks_exact_mut(4) {
                chunk.copy_from_slice(&pixel);
            }
        }
    }
}

struct AppState {
    running: bool,
    configured: bool,
    needs_redraw: bool,
    main_surface: WlSurface,
    submenu_surface: WlSurface,
    _subsurface: WlSubsurface,
    _xdg_surface: XdgSurface,
    _xdg_toplevel: XdgToplevel,
    main_buffer: ShmBuffer,
    submenu_buffer: ShmBuffer,
    submenu_visible: bool,
    hover_item: Option<usize>,
    pointer_on_submenu: bool,
}

impl AppState {
    fn new(
        main_surface: WlSurface,
        submenu_surface: WlSurface,
        subsurface: WlSubsurface,
        xdg_surface: XdgSurface,
        xdg_toplevel: XdgToplevel,
        main_buffer: ShmBuffer,
        submenu_buffer: ShmBuffer,
    ) -> Self {
        Self {
            running: true,
            configured: false,
            needs_redraw: true,
            main_surface,
            submenu_surface,
            _subsurface: subsurface,
            _xdg_surface: xdg_surface,
            _xdg_toplevel: xdg_toplevel,
            main_buffer,
            submenu_buffer,
            submenu_visible: false,
            hover_item: None,
            pointer_on_submenu: false,
        }
    }

    fn update_hover(&mut self, y: f64) {
        if !self.submenu_visible {
            return;
        }

        let item_height = SUBMENU_SIZE.1 as f64 / 2.0;
        let new_hover = if y >= 0.0 && y < SUBMENU_SIZE.1 as f64 {
            if y < item_height {
                Some(0)
            } else if y < item_height * 2.0 {
                Some(1)
            } else {
                None
            }
        } else {
            None
        };

        if self.hover_item != new_hover {
            self.hover_item = new_hover;
            self.needs_redraw = true;
        }
    }

    fn redraw(&mut self) {
        if !self.configured {
            return;
        }

        self.draw_main_surface();
        self.draw_submenu();
        self.needs_redraw = false;
    }

    fn draw_main_surface(&mut self) {
        let background = Color(0x1e, 0x48, 0x73);
        let accent = Color(0x52, 0xa5, 0x62);

        self.main_buffer.fill(background);
        let margin = 18;
        let inner_width = MAIN_SIZE.0 - margin * 2;
        let inner_height = MAIN_SIZE.1 - margin * 2;
        self.main_buffer
            .fill_rect(margin, margin, inner_width, inner_height, accent);

        self.main_surface
            .attach(Some(&self.main_buffer.buffer), 0, 0);
        self.main_surface.damage_buffer(
            0,
            0,
            self.main_buffer.width as i32,
            self.main_buffer.height as i32,
        );
        self.main_surface.commit();
    }

    fn draw_submenu(&mut self) {
        if !self.submenu_visible {
            self.submenu_surface.attach(None, 0, 0);
            self.submenu_surface.commit();
            return;
        }

        let base = Color(0x36, 0x37, 0x3a);
        let item = Color(0x58, 0x59, 0x5c);
        let highlight = Color(0x78, 0x9c, 0xcf);

        self.submenu_buffer.fill(base);
        let item_height = SUBMENU_SIZE.1 / 2;

        let first_color = if self.hover_item == Some(0) {
            highlight
        } else {
            item
        };
        let second_color = if self.hover_item == Some(1) {
            highlight
        } else {
            item
        };

        self.submenu_buffer
            .fill_rect(8, 8, SUBMENU_SIZE.0 - 16, item_height - 12, first_color);
        self.submenu_buffer.fill_rect(
            8,
            item_height + 4,
            SUBMENU_SIZE.0 - 16,
            item_height - 12,
            second_color,
        );

        self.submenu_surface
            .attach(Some(&self.submenu_buffer.buffer), 0, 0);
        self.submenu_surface.damage_buffer(
            0,
            0,
            self.submenu_buffer.width as i32,
            self.submenu_buffer.height as i32,
        );
        self.submenu_surface.commit();
    }
}

fn main() -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to Wayland compositor")?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = globals
        .bind::<WlCompositor, _, _>(&qh, 4..=6, ())
        .context("binding wl_compositor")?;
    let subcompositor = globals
        .bind::<WlSubcompositor, _, _>(&qh, 1..=1, ())
        .context("binding wl_subcompositor")?;
    let shm = globals
        .bind::<WlShm, _, _>(&qh, 1..=1, ())
        .context("binding wl_shm")?;
    let xdg_wm_base = globals
        .bind::<XdgWmBase, _, _>(&qh, 1..=6, ())
        .context("binding xdg_wm_base")?;
    let seat = globals
        .bind::<WlSeat, _, _>(&qh, 5..=9, ())
        .context("binding wl_seat")?;

    let main_surface = compositor.create_surface(&qh, ());
    let submenu_surface = compositor.create_surface(&qh, ());
    let subsurface = subcompositor.get_subsurface(&submenu_surface, &main_surface, &qh, ());
    subsurface.set_position(SUBMENU_OFFSET.0, SUBMENU_OFFSET.1);
    subsurface.set_desync();

    let xdg_surface = xdg_wm_base.get_xdg_surface(&main_surface, &qh, ());
    let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());
    xdg_toplevel.set_title("Submenu demo".to_string());
    main_surface.commit();

    let main_buffer = create_shm_buffer(&shm, MAIN_SIZE.0, MAIN_SIZE.1, &qh)?;
    let submenu_buffer = create_shm_buffer(&shm, SUBMENU_SIZE.0, SUBMENU_SIZE.1, &qh)?;

    let mut state = AppState::new(
        main_surface,
        submenu_surface,
        subsurface,
        xdg_surface,
        xdg_toplevel,
        main_buffer,
        submenu_buffer,
    );

    let _pointer = seat.get_pointer(&qh, ());

    event_queue.roundtrip(&mut state)?;
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }

    state.redraw();

    while state.running {
        event_queue.blocking_dispatch(&mut state)?;
        if state.needs_redraw {
            state.redraw();
        }
    }

    Ok(())
}

impl Dispatch<WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut Self,
        _registry: &WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgWmBase, ()> for AppState {
    fn event(
        _state: &mut Self,
        proxy: &XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
            state.needs_redraw = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.running = false
        }
    }
}

impl Dispatch<WlPointer, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface, surface_y, ..
            } => {
                state.pointer_on_submenu = surface == state.submenu_surface;
                if state.pointer_on_submenu {
                    state.update_hover(surface_y);
                } else {
                    if state.hover_item.take().is_some() {
                        state.needs_redraw = true;
                    }
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if surface == state.submenu_surface {
                    state.pointer_on_submenu = false;
                    if state.hover_item.take().is_some() {
                        state.needs_redraw = true;
                    }
                }
            }
            wl_pointer::Event::Motion { surface_y, .. } => {
                if state.pointer_on_submenu {
                    state.update_hover(surface_y);
                }
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } if button == RIGHT_BUTTON
                && btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed) =>
            {
                state.submenu_visible = !state.submenu_visible;
                if !state.submenu_visible {
                    state.pointer_on_submenu = false;
                    state.hover_item = None;
                }
                state.needs_redraw = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(AppState: WlCompositor);
delegate_noop!(AppState: WlSubcompositor);
delegate_noop!(AppState: WlSubsurface);
delegate_noop!(AppState: WlShmPool);

impl Dispatch<WlSurface, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: wayland_client::protocol::wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Ignore output enter/leave notifications for this sample.
    }
}

impl Dispatch<WlShm, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Ignore format announcements; we always use ARGB8888.
    }
}

impl Dispatch<WlSeat, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: wayland_client::protocol::wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Seat capability/name updates are ignored in this simple sample.
    }
}

impl Dispatch<WlBuffer, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Release events are ignored; buffers are reused.
    }
}

fn create_shm_buffer(
    shm: &WlShm,
    width: u32,
    height: u32,
    qh: &QueueHandle<AppState>,
) -> Result<ShmBuffer> {
    let stride = (width * 4) as i32;
    let size = stride as i64 * height as i64;
    let file = tempfile().context("creating shared memory file")?;
    file.set_len(size as u64)
        .context("sizing shared memory file")?;

    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    let mmap = unsafe { MmapMut::map_mut(&file) }.context("mapping buffer")?;

    Ok(ShmBuffer {
        width,
        height,
        mmap,
        buffer,
        _file: file,
        _pool: pool,
    })
}
