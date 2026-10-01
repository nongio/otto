/// Handler for wlr-foreign-toplevel-management-unstable-v1 protocol
///
/// This implements the older wlroots protocol for taskbars and window management.
/// Used by rofi, waybar, and other wlroots-based tools.
use std::sync::{Arc, Mutex};

use smithay::output::Output;
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

    /// Announce a new toplevel to every manager whose client `visible_to`
    /// accepts: an agent's connection is told of the windows within its
    /// scope alone (`crate::state::agent_seats`).
    #[allow(private_bounds)]
    pub fn new_toplevel<D>(
        &mut self,
        dh: &DisplayHandle,
        app_id: &str,
        title: &str,
        window_id: ObjectId,
        output: Option<&Output>,
        visible_to: &dyn Fn(&Client) -> bool,
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
            if let Some(client) = manager.client().filter(|client| visible_to(client)) {
                let handle = client
                    .create_resource::<ZwlrForeignToplevelHandleV1, _, D>(
                        dh,
                        manager.version(),
                        handle_data.clone(),
                    )
                    .ok();

                if let Some(handle) = handle {
                    manager.toplevel(&handle);

                    // Send initial properties + output in one batch before done
                    handle.app_id(app_id.to_string());
                    handle.title(title.to_string());
                    if let Some(output) = output {
                        for wl_output in output.client_outputs(&client) {
                            handle.output_enter(&wl_output);
                        }
                    }
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

    pub fn send_output_enter(&self, output: &Output) {
        let data = self.data.lock().unwrap();
        for resource in &data.resources {
            if let Some(client) = resource.client() {
                for wl_output in output.client_outputs(&client) {
                    resource.output_enter(&wl_output);
                }
                resource.done();
            }
        }
    }

    pub fn send_output_leave(&self, output: &Output) {
        let data = self.data.lock().unwrap();
        for resource in &data.resources {
            if let Some(client) = resource.client() {
                for wl_output in output.client_outputs(&client) {
                    resource.output_leave(&wl_output);
                }
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
    /// For the clients that get the privileged interfaces, and agents'
    /// connections, which see their own scope (see `src/sandbox.rs`).
    fn can_view(client: Client, _global_data: &()) -> bool {
        crate::sandbox::is_privileged_client(&client)
            || crate::state::ClientState::agent_seat_of(&client).is_some()
    }

    fn bind(
        state: &mut Otto<BackendData>,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let manager = data_init.init(resource, ());
        state
            .wlr_foreign_toplevel_state
            .register_manager(manager.clone());

        // An agent's connection is told of the windows within its scope.
        let scope = crate::state::ClientState::agent_seat_of(client)
            .map(|seat| state.agent_scope_window_ids(seat));

        // Send all existing toplevels to this new manager
        for handles in state.foreign_toplevels.values() {
            if let Some(wlr_handle) = &handles.wlr {
                let in_scope = scope
                    .as_ref()
                    .is_none_or(|scope| scope.contains(&wlr_handle.data.lock().unwrap().window_id));
                if !in_scope {
                    continue;
                }
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
                        if let Some(output) = &handles.output {
                            for wl_output in output.client_outputs(&client) {
                                handle.output_enter(&wl_output);
                            }
                        }
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
        client: &Client,
        _resource: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &Arc<Mutex<WlrToplevelData>>,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<BackendData>>,
    ) {
        let window_id = data.lock().unwrap().window_id.clone();

        // An agent's connection: activating gives the agent's keyboard to a
        // window within its scope, closing closes one, and the rest is
        // ignored — nothing an agent holds changes what the user sees.
        if let Some(seat) = crate::state::ClientState::agent_seat_of(client) {
            let seat = seat.to_string();
            match request {
                zwlr_foreign_toplevel_handle_v1::Request::Activate { .. } => {
                    state.focus_on_agent_seat(&seat, &window_id);
                }
                zwlr_foreign_toplevel_handle_v1::Request::Close
                    if state.agent_scope_window_ids(&seat).contains(&window_id) =>
                {
                    close_window(state, &window_id);
                }
                _ => {}
            }
            return;
        }

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
                    state.demote_scanout_window(&window);
                    state.workspaces.minimize_window(&window);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetMinimized => {
                state.workspaces.unminimize_window(&window_id);
                state.set_keyboard_focus_on_surface(&window_id);
            }
            zwlr_foreign_toplevel_handle_v1::Request::Activate { seat: _seat } => {
                state.activate_window(&window_id);
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => close_window(state, &window_id),
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

/// Ask the window `window_id` to close.
fn close_window<BackendData: Backend>(state: &Otto<BackendData>, window_id: &ObjectId) {
    if let Some(window) = state.workspaces.get_window_for_surface(window_id) {
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
