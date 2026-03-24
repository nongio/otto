//! Foreign toplevel focus tracker.
//!
//! Spawns a background thread with its own Wayland connection that binds
//! `zwlr_foreign_toplevel_manager_v1` and watches for activated state changes.
//! The focused app's title and app_id are stored in a global `Mutex` for the
//! main thread to read.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use wayland_client::{protocol::wl_registry, Connection, Dispatch, Proxy, QueueHandle};
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
static FOCUSED_APP: LazyLock<Mutex<FocusedApp>> = LazyLock::new(|| Mutex::new(FocusedApp::default()));

/// Generation counter — bumped on every focus change.
static FOCUS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Read the current focused app info.
pub fn current_focused_app() -> FocusedApp {
    FOCUSED_APP.lock().unwrap().clone()
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
        // Find the activated toplevel
        let focused = self
            .toplevels
            .values()
            .find(|t| t.activated);

        let app = match focused {
            Some(t) => FocusedApp {
                app_id: t.app_id.clone().unwrap_or_default(),
                title: t.title.clone().unwrap_or_default(),
            },
            None => FocusedApp::default(),
        };

        let mut current = FOCUSED_APP.lock().unwrap();
        if current.app_id != app.app_id || current.title != app.title {
            tracing::debug!("focus changed: app_id={:?} title={:?}", app.app_id, app.title);
            *current = app;
            FOCUS_GENERATION.fetch_add(1, Ordering::Relaxed);
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
                tracing::info!("found zwlr_foreign_toplevel_manager_v1 v{version}");
                registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(
                    name,
                    version.min(3),
                    qh,
                    (),
                );
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
                tracing::debug!("new toplevel #{id}");
                // Info populated via handle events
                _state.toplevels.insert(
                    id,
                    ToplevelInfo {
                        app_id: None,
                        title: None,
                        activated: false,
                    },
                );
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                tracing::debug!("foreign toplevel manager finished");
            }
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
                    // State is a list of u32 values; Activated = 2
                    let activated = raw_state
                        .chunks_exact(4)
                        .any(|chunk| {
                            let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                            val == zwlr_foreign_toplevel_handle_v1::State::Activated as u32
                        });
                    info.activated = activated;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.update_focused();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(_info) = state.toplevels.remove(&id) {
                    tracing::debug!("toplevel #{id} closed");
                    state.update_focused();
                }
            }
            _ => {}
        }
    }
}

fn run_watcher() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = FocusState::new();

    tracing::info!("focus watcher started");

    loop {
        event_queue.blocking_dispatch(&mut state)?;
    }
}
