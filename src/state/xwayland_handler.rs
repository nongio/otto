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
    utils::SERIAL_COUNTER,
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
        let window_element = WindowElement::new(
            Window::new_x11_window(window.clone()),
            window_layer,
            mirror_layer,
        );

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
        let bbox = self
            .workspaces
            .space()
            .and_then(|s| s.element_bbox(&window_element));
        if let WindowSurface::X11(xsurface) = window_element.underlying_surface() {
            let _ = xsurface.configure(bbox);
        }

        if !is_override_redirect {
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(
                self,
                Some(window_element.into()),
                SERIAL_COUNTER.next_serial(),
            );
        }
    }
}

#[cfg(feature = "xwayland")]
delegate_xwayland_keyboard_grab!(@<BackendData: Backend + 'static> Otto<BackendData>);

#[cfg(feature = "xwayland")]
delegate_xwayland_shell!(@<BackendData: Backend + 'static> Otto<BackendData>);
