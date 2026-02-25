/// Handler for wlr-foreign-toplevel-management-unstable-v1 protocol
///
/// This implements the older wlroots protocol for taskbars and window management.
/// Used by rofi, waybar, and other wlroots-based tools.
use std::sync::{Arc, Mutex};

use wayland_server::{
    backend::ObjectId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::state::{Backend, Otto};

/// Global state for wlr foreign toplevel management
pub struct WlrForeignToplevelManagerState {
    instances: Vec<ZwlrForeignToplevelManagerV1>,
}

impl WlrForeignToplevelManagerState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ()>
            + Dispatch<ZwlrForeignToplevelManagerV1, ()>
            + 'static,
    {
        display.create_global::<D, ZwlrForeignToplevelManagerV1, ()>(3, ());

        Self {
            instances: Vec::new(),
        }
    }

    #[allow(private_bounds)]
    pub fn new_toplevel<D>(
        &mut self,
        dh: &DisplayHandle,
        app_id: &str,
        title: &str,
        window_id: ObjectId,
    ) -> WlrForeignToplevelHandle
    where
        D: Dispatch<ZwlrForeignToplevelHandleV1, Arc<Mutex<WlrToplevelData>>> + 'static,
    {
        let handle_data = Arc::new(Mutex::new(WlrToplevelData {
            app_id: app_id.to_string(),
            title: title.to_string(),
            window_id,
            current_state: Vec::new(),
            resources: Vec::new(),
        }));

        // Send toplevel to all manager instances
        for manager in &self.instances {
            if let Some(client) = manager.client() {
                let handle = client
                    .create_resource::<ZwlrForeignToplevelHandleV1, _, D>(
                        dh,
                        manager.version(),
                        handle_data.clone(),
                    )
                    .ok();

                if let Some(handle) = handle {
                    manager.toplevel(&handle);

                    // Send initial state
                    handle.app_id(app_id.to_string());
                    handle.title(title.to_string());
                    handle.done();

                    handle_data.lock().unwrap().resources.push(handle);
                }
            }
        }

        WlrForeignToplevelHandle { data: handle_data }
    }

    fn register_manager(&mut self, manager: ZwlrForeignToplevelManagerV1) {
        self.instances.push(manager);
    }

    fn unregister_manager(&mut self, manager: &ZwlrForeignToplevelManagerV1) {
        self.instances.retain(|m| m.id() != manager.id());
    }
}

/// Data associated with a wlr foreign toplevel handle
#[derive(Debug)]
struct WlrToplevelData {
    app_id: String,
    title: String,
    /// ObjectId of the corresponding compositor window surface
    window_id: ObjectId,
    /// Cached state bytes (array of u32 state enum values) for late-joining clients
    current_state: Vec<u8>,
    resources: Vec<ZwlrForeignToplevelHandleV1>,
}

/// Handle for a wlr foreign toplevel
#[derive(Debug, Clone)]
pub struct WlrForeignToplevelHandle {
    data: Arc<Mutex<WlrToplevelData>>,
}

impl WlrForeignToplevelHandle {
    pub fn send_title(&self, title: String) {
        let mut data = self.data.lock().unwrap();
        if data.title != title {
            data.title = title.clone();
            for resource in &data.resources {
                resource.title(title.clone());
                resource.done();
            }
        }
    }

    pub fn send_app_id(&self, app_id: String) {
        let mut data = self.data.lock().unwrap();
        if data.app_id != app_id {
            data.app_id = app_id.clone();
            for resource in &data.resources {
                resource.app_id(app_id.clone());
                resource.done();
            }
        }
    }

    pub fn send_closed(&self) {
        let data = self.data.lock().unwrap();
        for resource in &data.resources {
            resource.closed();
        }
    }

    pub fn send_state(&self, activated: bool, minimized: bool, maximized: bool, fullscreen: bool) {
        // Pack active state enum values as u32 little-endian bytes (wlr protocol array)
        let mut vals: Vec<u32> = Vec::new();
        if maximized {
            vals.push(0);
        }
        if minimized {
            vals.push(1);
        }
        if activated {
            vals.push(2);
        }
        if fullscreen {
            vals.push(3);
        }
        let state_bytes: Vec<u8> = vals.iter().flat_map(|v| v.to_ne_bytes()).collect();

        let mut data = self.data.lock().unwrap();
        data.current_state = state_bytes.clone();
        for resource in &data.resources {
            resource.state(state_bytes.clone());
            resource.done();
        }
    }

    pub fn window_id(&self) -> ObjectId {
        self.data.lock().unwrap().window_id.clone()
    }

    pub fn title(&self) -> String {
        self.data.lock().unwrap().title.clone()
    }

    pub fn app_id(&self) -> String {
        self.data.lock().unwrap().app_id.clone()
    }
}

// Implement GlobalDispatch for manager
impl<BackendData: Backend> GlobalDispatch<ZwlrForeignToplevelManagerV1, (), Otto<BackendData>>
    for Otto<BackendData>
{
    fn bind(
        state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let manager = data_init.init(resource, ());
        state
            .wlr_foreign_toplevel_state
            .register_manager(manager.clone());

        // Send all existing toplevels to this new manager
        for handles in state.foreign_toplevels.values() {
            if let Some(wlr_handle) = &handles.wlr {
                // Create a new handle resource for this manager
                if let Some(client) = manager.client() {
                    let handle = client
                        .create_resource::<ZwlrForeignToplevelHandleV1, _, Otto<BackendData>>(
                            _handle,
                            manager.version(),
                            wlr_handle.data.clone(),
                        )
                        .ok();

                    if let Some(handle) = handle {
                        manager.toplevel(&handle);

                        // Send initial state
                        let data = wlr_handle.data.lock().unwrap();
                        handle.app_id(data.app_id.clone());
                        handle.title(data.title.clone());
                        handle.state(data.current_state.clone());
                        handle.done();

                        // Store handle reference
                        drop(data);
                        wlr_handle.data.lock().unwrap().resources.push(handle);
                    }
                }
            }
        }
    }
}

// Implement Dispatch for manager
impl<BackendData: Backend> Dispatch<ZwlrForeignToplevelManagerV1, (), Otto<BackendData>>
    for Otto<BackendData>
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            state
                .wlr_foreign_toplevel_state
                .unregister_manager(resource);
        }
    }

    fn destroyed(
        state: &mut Otto<BackendData>,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state
            .wlr_foreign_toplevel_state
            .unregister_manager(resource);
    }
}

// Implement Dispatch for handle
impl<BackendData: Backend>
    Dispatch<ZwlrForeignToplevelHandleV1, Arc<Mutex<WlrToplevelData>>, Otto<BackendData>>
    for Otto<BackendData>
{
    fn request(
        state: &mut Otto<BackendData>,
        _client: &Client,
        _resource: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &Arc<Mutex<WlrToplevelData>>,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let window_id = data.lock().unwrap().window_id.clone();

        match request {
            zwlr_foreign_toplevel_handle_v1::Request::SetMaximized => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id) {
                    if let Some(toplevel) = window.toplevel().cloned() {
                        <Otto<BackendData> as smithay::wayland::shell::xdg::XdgShellHandler>::maximize_request(state, toplevel);
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMaximized => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id) {
                    if let Some(toplevel) = window.toplevel().cloned() {
                        <Otto<BackendData> as smithay::wayland::shell::xdg::XdgShellHandler>::unmaximize_request(state, toplevel);
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetMinimized => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id).cloned() {
                    state.workspaces.minimize_window(&window);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMinimized => {
                state.workspaces.unminimize_window(&window_id);
                state.set_keyboard_focus_on_surface(&window_id);
            }
            zwlr_foreign_toplevel_handle_v1::Request::Activate { seat: _seat } => {
                if let Some(wid) = state.workspaces.focus_app_with_window(&window_id) {
                    state.set_keyboard_focus_on_surface(&wid);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id) {
                    match window.underlying_surface() {
                        smithay::desktop::WindowSurface::Wayland(toplevel) => {
                            toplevel.send_close();
                        }
                        #[cfg(feature = "xwayland")]
                        smithay::desktop::WindowSurface::X11(surface) => {
                            let _ = surface.close();
                        }
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetRectangle { .. } => {
                // Hint for minimize animation target; not required by protocol
            }
            zwlr_foreign_toplevel_handle_v1::Request::Destroy => {
                // Handle is being destroyed by client
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetFullscreen { output: _output } => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id) {
                    if let Some(toplevel) = window.toplevel().cloned() {
                        <Otto<BackendData> as smithay::wayland::shell::xdg::XdgShellHandler>::fullscreen_request(state, toplevel, None);
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetFullscreen => {
                if let Some(window) = state.workspaces.get_window_for_surface(&window_id) {
                    if let Some(toplevel) = window.toplevel().cloned() {
                        <Otto<BackendData> as smithay::wayland::shell::xdg::XdgShellHandler>::unfullscreen_request(state, toplevel);
                    }
                }
            }
            _ => {}
        }
    }
}
