use smithay::{
    delegate_input_method_manager, delegate_pointer_constraints,
    desktop::{PopupKind, PopupManager},
    input::pointer::PointerHandle,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Rectangle,
    wayland::{
        input_method::{InputMethodHandler, PopupSurface},
        pointer_constraints::{with_pointer_constraint, PointerConstraintsHandler},
        seat::WaylandFocus,
    },
};
use tracing::warn;

use super::{Backend, Otto};

impl<BackendData: Backend> InputMethodHandler for Otto<BackendData> {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!("Failed to track popup: {}", err);
        }
    }

    fn popup_repositioned(&mut self, _: PopupSurface) {}

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
        // Note: we intentionally do NOT release grabs or restore pointer focus here.
        // This callback is invoked while smithay's input_method mutex (and sometimes the
        // keyboard mutex) is held — touching keyboard/pointer handles would deadlock.
        // Stale grabs are cleaned up proactively in xdg::grab and in popup_destroyed.
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, smithay::utils::Logical> {
        self.workspaces
            .spaces_elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(parent)).then(|| window.geometry())
            })
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(@<BackendData: Backend + 'static> Otto<BackendData>);

impl<BackendData: Backend> PointerConstraintsHandler for Otto<BackendData> {
    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        // Handle cursor position hint if needed
    }

    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // XXX region
        let Some(current_focus) = pointer.current_focus() else {
            return;
        };
        if current_focus.wl_surface().as_deref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                constraint.unwrap().activate();
            });
        }
    }
}
delegate_pointer_constraints!(@<BackendData: Backend + 'static> Otto<BackendData>);
