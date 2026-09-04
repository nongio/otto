use smithay::{
    backend::input::TabletToolDescriptor,
    delegate_seat, delegate_tablet_manager,
    desktop::{find_popup_root_surface, space::SpaceElement},
    input::{pointer::CursorImageStatus, SeatHandler, SeatState},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    wayland::{
        seat::WaylandFocus,
        selection::{data_device::set_data_device_focus, primary_selection::set_primary_focus},
        tablet_manager::TabletSeatHandler,
    },
};

use crate::focus::{KeyboardFocusTarget, PointerFocusTarget};

use super::{Backend, Otto};

impl<BackendData: Backend> SeatHandler for Otto<BackendData> {
    type KeyboardFocus = KeyboardFocusTarget<BackendData>;
    type PointerFocus = PointerFocusTarget<BackendData>;
    type TouchFocus = PointerFocusTarget<BackendData>;

    fn seat_state(&mut self) -> &mut SeatState<Otto<BackendData>> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        seat: &smithay::input::Seat<Self>,
        target: Option<&KeyboardFocusTarget<BackendData>>,
    ) {
        let dh = &self.display_handle;

        let wl_surface = target.and_then(WaylandFocus::wl_surface);

        let focus = wl_surface.as_ref().and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, focus.clone());
        set_primary_focus(dh, seat, focus);

        self.update_toplevel_activation(target);
    }

    fn cursor_image(&mut self, _seat: &smithay::input::Seat<Self>, image: CursorImageStatus) {
        *self.cursor_status.lock().unwrap() = image.clone();
        self.cursor_manager.set_cursor_image(image);
    }
    fn led_state_changed(
        &mut self,
        _seat: &smithay::input::Seat<Self>,
        led_state: smithay::input::keyboard::LedState,
    ) {
        self.backend_data.update_keyboard_leds(led_state);
    }
}

/// The surface of the window the keyboard focus belongs to, which is not the
/// same as the surface holding the focus.
///
/// A popup is chrome the focused window put up — a menu, a tooltip — so the
/// window behind it is still the focused window. Anything else with the
/// keyboard (a layer surface, the lock screen, Otto's own views) means no
/// window has it.
///
/// Everything that draws a window differently when it is focused must agree on
/// this, or a window dims itself the moment one of its own menus opens.
pub fn focused_window_surface<BackendData: Backend>(
    target: Option<&KeyboardFocusTarget<BackendData>>,
) -> Option<WlSurface> {
    match target {
        Some(KeyboardFocusTarget::Window(window)) => {
            window.wl_surface().map(|surface| surface.into_owned())
        }
        Some(KeyboardFocusTarget::Popup(popup)) => find_popup_root_surface(popup).ok(),
        _ => None,
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    /// Keep every toplevel's `activated` state in step with the keyboard.
    ///
    /// Clients draw themselves differently when they lose the focus — a
    /// grayed-out title and traffic lights, and no blurred backdrop, in the
    /// otto-kit apps — so the state has to reach the windows being deactivated
    /// as much as the one taking the focus. Raising a window only settles
    /// activation within its own space, which leaves a window on another
    /// workspace or another output believing it is still the focused one.
    fn update_toplevel_activation(&self, target: Option<&KeyboardFocusTarget<BackendData>>) {
        let focused = focused_window_surface(target);

        for window in self.workspaces.spaces_elements() {
            let active = match (&focused, window.wl_surface()) {
                (Some(focused), Some(surface)) => *surface == *focused,
                _ => false,
            };
            SpaceElement::set_activate(window, active);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }
}

impl<BackendData: Backend> TabletSeatHandler for Otto<BackendData> {
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        let mut cursor_status = self.cursor_status.lock().unwrap();
        *cursor_status = image.clone();
        self.cursor_manager.set_cursor_image(image);
    }
}

delegate_seat!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_tablet_manager!(@<BackendData: Backend + 'static> Otto<BackendData>);
