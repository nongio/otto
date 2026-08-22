use smithay::{
    delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::{
        self as xdg_decoration, zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
    },
    reexports::wayland_server::Resource,
    wayland::{
        compositor::with_states,
        shell::xdg::{decoration::XdgDecorationHandler, ToplevelSurface, XdgToplevelSurfaceData},
    },
};

use super::{Backend, Otto};

impl<BackendData: Backend> Otto<BackendData> {
    /// Apply a negotiated decoration mode: record it on the toplevel, flag the
    /// window so its geometry accounts for the titlebar, and re-layout.
    fn apply_decoration_mode(&mut self, toplevel: &ToplevelSurface, mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });

        let server_side = mode == DecorationMode::ServerSide;
        let id = toplevel.wl_surface().id();
        let changed = self
            .workspaces
            .get_window_for_surface(&id)
            .map(|window| window.set_decorated(server_side) != server_side)
            .unwrap_or(false);

        let initial_configure_sent = with_states(toplevel.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });
        if initial_configure_sent {
            toplevel.send_pending_configure();
        }

        // The window just grew or lost a titlebar, so its geometry changed
        // under the layout — and the view needs to show or hide the bar.
        if changed {
            if let Some(window) = self.workspaces.get_window_for_surface(&id).cloned() {
                if let Some(view) = self.workspaces.get_window_view(&id) {
                    view.set_decorated(server_side);
                }
                self.update_window_view(&window);
                self.workspaces.update_workspace_model();
            }
        }
    }
}

impl<BackendData: Backend> XdgDecorationHandler for Otto<BackendData> {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        // A client that binds the decoration protocol without asking for a
        // mode gets a server-side titlebar: Otto would rather draw the
        // decoration itself than have every app invent its own.
        self.apply_decoration_mode(&toplevel, Mode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        // Honour what the client asks for. Clients that draw their own
        // titlebar (GTK and friends) ask for ClientSide and keep doing it.
        self.apply_decoration_mode(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        // No preference from the client means Otto's preference applies.
        self.apply_decoration_mode(&toplevel, Mode::ServerSide);
    }
}
delegate_xdg_decoration!(@<BackendData: Backend + 'static> Otto<BackendData>);
