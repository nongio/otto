//! Foreign toplevel focus tracker.
//!
//! Spawns a background thread with its own Wayland connection that binds
//! `zwlr_foreign_toplevel_manager_v1` and watches for activated state changes.
//! The focused app's title and app_id are stored in a global `Mutex` for the
//! main thread to read, and any tracked window can be activated with
//! [`activate_window`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

use wayland_client::{
    protocol::{wl_registry, wl_seat::WlSeat},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

/// Info about the currently focused (activated) toplevel.
#[derive(Clone, Debug, Default)]
pub struct FocusedApp {
    pub app_id: String,
    pub title: String,
}

/// Global state: the currently focused app.
static FOCUSED_APP: LazyLock<Mutex<FocusedApp>> =
    LazyLock::new(|| Mutex::new(FocusedApp::default()));

/// Generation counter — bumped on every focus change.
static FOCUS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Read the current focused app info.
pub fn current_focused_app() -> FocusedApp {
    FOCUSED_APP.lock().unwrap().clone()
}

/// The watcher's connection and seat, which activating a window needs.
static CONTROL: OnceLock<Connection> = OnceLock::new();
static SEAT: Mutex<Option<WlSeat>> = Mutex::new(None);

/// Every open window, for [`activate_window`] to pick from.
static WINDOWS: Mutex<Vec<(ZwlrForeignToplevelHandleV1, FocusedApp)>> = Mutex::new(Vec::new());

/// Every open window, as of the last `done` event.
///
/// Empty until [`spawn_focus_watcher`] has run and the compositor has listed
/// its windows.
pub fn windows() -> Vec<FocusedApp> {
    WINDOWS
        .lock()
        .unwrap()
        .iter()
        .map(|(_, app)| app.clone())
        .collect()
}

/// Activate the first window for which `matches(app_id, title)` holds, through
/// `zwlr_foreign_toplevel_handle_v1.activate` on the first seat.
///
/// Returns whether a window matched and the request was sent. It is `false`
/// before [`spawn_focus_watcher`] has connected. The compositor decides whether
/// to honour the request.
pub fn activate_window(matches: impl Fn(&str, &str) -> bool) -> bool {
    let Some(conn) = CONTROL.get() else {
        return false;
    };
    let Some(seat) = SEAT.lock().unwrap().clone() else {
        return false;
    };
    let windows = WINDOWS.lock().unwrap();
    let Some((handle, _)) = windows
        .iter()
        .find(|(_, app)| matches(&app.app_id, &app.title))
    else {
        return false;
    };
    handle.activate(&seat);
    conn.flush().is_ok()
}

/// Read the generation counter.
pub fn generation() -> u64 {
    FOCUS_GENERATION.load(Ordering::Relaxed)
}

/// Spawn the focus watcher on a background thread.
pub fn spawn_focus_watcher() {
    std::thread::spawn(|| {
        if let Err(e) = run_watcher() {
            tracing::warn!("focus watcher stopped: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// Internal Wayland client state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ToplevelInfo {
    handle: ZwlrForeignToplevelHandleV1,
    app_id: Option<String>,
    title: Option<String>,
    activated: bool,
}

struct FocusState {
    toplevels: std::collections::HashMap<u32, ToplevelInfo>,
}

impl FocusState {
    fn new() -> Self {
        Self {
            toplevels: std::collections::HashMap::new(),
        }
    }

    /// Called after a `done` event — check if activated state changed globally.
    fn update_focused(&self) {
        *WINDOWS.lock().unwrap() = self
            .toplevels
            .values()
            .map(|t| {
                let app = FocusedApp {
                    app_id: t.app_id.clone().unwrap_or_default(),
                    title: t.title.clone().unwrap_or_default(),
                };
                (t.handle.clone(), app)
            })
            .collect();

        let focused = self.toplevels.values().find(|t| t.activated);

        let app = match focused {
            Some(t) => FocusedApp {
                app_id: t.app_id.clone().unwrap_or_default(),
                title: t.title.clone().unwrap_or_default(),
            },
            None => FocusedApp::default(),
        };

        let mut current = FOCUSED_APP.lock().unwrap();
        if current.app_id != app.app_id || current.title != app.title {
            *current = app;
            FOCUS_GENERATION.fetch_add(1, Ordering::Relaxed);
            crate::AppContext::request_wakeup();
        }
    }
}

// --- Registry dispatch ---

impl Dispatch<wl_registry::WlRegistry, ()> for FocusState {
    fn event(
        _state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zwlr_foreign_toplevel_manager_v1" {
                registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
            } else if interface == "wl_seat" {
                let mut seat = SEAT.lock().unwrap();
                if seat.is_none() {
                    *seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(1), qh, ()));
                }
            }
        }
    }
}

// --- Manager dispatch ---

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for FocusState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                let id = toplevel.id().protocol_id();
                _state.toplevels.insert(
                    id,
                    ToplevelInfo {
                        handle: toplevel,
                        app_id: None,
                        title: None,
                        activated: false,
                    },
                );
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {}
            _ => {}
        }
    }

    wayland_client::event_created_child!(FocusState, ZwlrForeignToplevelManagerV1, [
        0 => (ZwlrForeignToplevelHandleV1, ())
    ]);
}

// --- Handle dispatch ---

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for FocusState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    info.title = Some(title);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    info.app_id = Some(app_id);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: raw_state } => {
                if let Some(info) = state.toplevels.get_mut(&id) {
                    let activated = raw_state.chunks_exact(4).any(|chunk| {
                        let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        val == zwlr_foreign_toplevel_handle_v1::State::Activated as u32
                    });
                    info.activated = activated;

                    if activated {
                        for (&other_id, other) in state.toplevels.iter_mut() {
                            if other_id != id {
                                other.activated = false;
                            }
                        }
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.update_focused();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                // The handle is inert once closed; destroying it lets the
                // compositor free the object.
                proxy.destroy();
                if state.toplevels.remove(&id).is_some() {
                    state.update_focused();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for FocusState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn run_watcher() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let _ = CONTROL.set(conn.clone());
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = FocusState::new();

    loop {
        event_queue.blocking_dispatch(&mut state)?;
    }
}
