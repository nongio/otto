#[cfg(feature = "xwayland")]
use crate::{
    focus::KeyboardFocusTarget,
    shell::WindowElement,
    state::{Backend, Otto},
};
#[cfg(feature = "xwayland")]
use smithay::{
    delegate_xwayland_keyboard_grab, delegate_xwayland_shell,
    desktop::{Window, WindowSurface},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
    wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    xwayland::xwm::XwmId,
};

#[cfg(feature = "xwayland")]
impl<BackendData: Backend + 'static> XWaylandKeyboardGrabHandler for Otto<BackendData> {
    fn keyboard_focus_for_xsurface(
        &self,
        surface: &WlSurface,
    ) -> Option<KeyboardFocusTarget<BackendData>> {
        let elem = self
            .workspaces
            .space()?
            .elements()
            .find(|elem| elem.wl_surface().as_deref() == Some(surface))?
            .clone();
        Some(KeyboardFocusTarget::Window(elem))
    }
}

#[cfg(feature = "xwayland")]
impl<BackendData: Backend + 'static> XWaylandShellHandler for Otto<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(
        &mut self,
        _xwm_id: XwmId,
        _surface: WlSurface,
        window: smithay::xwayland::X11Surface,
    ) {
        // wl_surface is now set on the X11Surface — safe to call window_element.id().
        // Handle both regular and override-redirect windows here.
        if !window.is_mapped() && !window.is_override_redirect() {
            return;
        }
        let is_override_redirect = window.is_override_redirect();
        let window_layer = self.layers_engine.new_layer();
        let mirror_layer = self.layers_engine.new_layer();

        mirror_layer.set_draw_content(window_layer.as_content());
        mirror_layer.set_picture_cached(false);
        mirror_layer.set_layout_style(layers::prelude::taffy::Style {
            position: layers::prelude::taffy::Position::Absolute,
            ..Default::default()
        });
        window_layer.add_follower_node(&mirror_layer);

        let window_element = WindowElement::new(
            Window::new_x11_window(window.clone()),
            window_layer.clone(),
            mirror_layer.clone(),
        );

        // Set keys after construction so we can use window_element.id() for consistency
        let surface_id = window_element.id();
        window_layer.set_key(format!("surface_{:?}", surface_id));
        mirror_layer.set_key(format!("mirror_window_{}", window_layer.id.0));

        let location = if is_override_redirect {
            // Override-redirect windows self-position; use their declared geometry.
            window.geometry().loc
        } else {
            let loc = self.pointer.current_location();
            let (_, location) = self.workspaces.new_window_placement_at(loc);
            location
        };

        // Override-redirect popups must not steal focus (activate=false).
        self.workspaces
            .map_window(&window_element, location, !is_override_redirect, None);

        // A client may map with an initial _NET_WM_STATE_FULLSCREEN (Unity/Proton
        // games set it via XChangeProperty before mapping). In that case fullscreen
        // owns the geometry: sending the windowed-placement configure here would race
        // the fullscreen configure and leave the window mis-sized at the windowed
        // spawn point (collapsed -> black). Skip the windowed configure and let
        // apply_x11_fullscreen be the sole authority on the X11 geometry.
        let wants_fullscreen = !is_override_redirect && window.is_fullscreen();

        if !wants_fullscreen {
            let bbox = self
                .workspaces
                .space()
                .and_then(|s| s.element_bbox(&window_element));
            if let WindowSurface::X11(xsurface) = window_element.underlying_surface() {
                let _ = xsurface.configure(bbox);
            }
        }

        if !is_override_redirect {
            // Focus the window at map. For self-managing X11 games this routes
            // keyboard delivery to the `wl_surface` (see src/focus.rs) — the game
            // gets wl_keyboard focus (so XWayland can deliver keys) WITHOUT a
            // WM_TAKE_FOCUS, which would break its render loop. Activation
            // (_NET_ACTIVE_WINDOW) is set inside the helper too.
            self.set_keyboard_focus_on_window(&window_element);

            // Replay a fullscreen request that arrived before the element was
            // mapped. Otto defers X11 mapping to here, so a client that maps and
            // immediately sets _NET_WM_STATE_FULLSCREEN (Unity/Proton games) had
            // its fullscreen_request dropped — the initial _NET_WM_STATE property is
            // seeded into the X11Surface (smithay update_net_wm_state at MapRequest),
            // and we apply it now that the element exists.
            if window.is_fullscreen() && !window_element.is_fullscreen() {
                self.apply_x11_fullscreen(&window_element, &window);
            }
        }
    }
}

#[cfg(feature = "xwayland")]
delegate_xwayland_keyboard_grab!(@<BackendData: Backend + 'static> Otto<BackendData>);

#[cfg(feature = "xwayland")]
delegate_xwayland_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
