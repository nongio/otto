use smithay::{
    delegate_kde_decoration, delegate_xdg_decoration,
    reexports::wayland_protocols::xdg::decoration::{
        self as xdg_decoration, zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
    },
    reexports::wayland_protocols_misc::server_decoration::server::{
        org_kde_kwin_server_decoration::{Mode as KdeMode, OrgKdeKwinServerDecoration},
    },
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource, WEnum},
    wayland::{
        compositor::with_states,
        shell::{
            kde::decoration::{KdeDecorationHandler, KdeDecorationState},
            xdg::{decoration::XdgDecorationHandler, ToplevelSurface, XdgToplevelSurfaceData},
        },
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

        // Flag the window first: a maximized or tiled window is re-fitted to
        // its zone by `set_surface_decorated`, and that size must ride the
        // same configure as the mode.
        self.set_surface_decorated(toplevel.wl_surface(), mode == DecorationMode::ServerSide);

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
    }

    /// Show or hide Otto's titlebar for a surface, whichever protocol asked.
    /// The KDE path has no toplevel state to configure, so this is the half
    /// both protocols share.
    pub fn set_surface_decorated(&mut self, surface: &WlSurface, server_side: bool) {
        let id = surface.id();
        let Some(window) = self.workspaces.get_window_for_surface(&id) else {
            // No window yet — the KDE protocol lets a client negotiate before
            // it has an `xdg_toplevel`. Remember the answer for `new_toplevel`.
            self.pending_kde_decorations.insert(id, server_side);
            return;
        };
        let changed = window.set_decorated(server_side) != server_side;

        // The window just grew or lost a titlebar, so its geometry changed
        // under the layout — and the view needs to show or hide the bar.
        if changed {
            if let Some(window) = self.workspaces.get_window_for_surface(&id).cloned() {
                if let Some(view) = self.workspaces.get_window_view(&id) {
                    // Not `server_side`: a fullscreen window wears no bar
                    // whatever it just negotiated, and gets it when it
                    // leaves fullscreen.
                    view.set_decorated(window.is_decorated());
                }
                self.update_window_view(&window);
                self.workspaces.update_workspace_model();
                // A zone-owned window's client size is the zone minus the
                // titlebar it just gained or lost — re-derive it, whether
                // the change lands mid-animation (Chrome restoring a
                // maximized window negotiates client-side decorations
                // AFTER asking to be maximized) or long after.
                self.refit_window_to_zone(&window);
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

/// KDE's older server-decoration protocol, kept alive because it is the only
/// one some toolkits look for. GTK never binds `xdg-decoration`, so apps built
/// on it — ghostty, for one — can only be server-decorated through here.
impl<BackendData: Backend> KdeDecorationHandler for Otto<BackendData> {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }

    fn new_decoration(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
        // Same preference as the xdg path: tell the client Otto decorates it,
        // and start drawing the bar. A client that disagrees answers with a
        // request_mode of its own.
        decoration.mode(KdeMode::Server);
        self.set_surface_decorated(surface, true);
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeMode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };
        // Acknowledge the request before acting on it — the client waits for
        // the mode event to decide whether to draw its own titlebar.
        decoration.mode(mode);
        self.set_surface_decorated(surface, mode == KdeMode::Server);
    }

    fn release(&mut self, _decoration: &OrgKdeKwinServerDecoration, surface: &WlSurface) {
        // The client dropped the object: fall back to Otto's own preference,
        // the same one the manager advertises as its default mode.
        self.set_surface_decorated(surface, true);
    }
}
delegate_kde_decoration!(@<BackendData: Backend + 'static> Otto<BackendData>);
