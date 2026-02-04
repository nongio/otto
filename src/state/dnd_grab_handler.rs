use layers::prelude::Transition;
use smithay::{
    input::{
        dnd::{DnDGrab, DndGrabHandler, GrabType},
        pointer::{CursorImageStatus, Focus},
        Seat,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Serial},
    wayland::selection::data_device::WaylandDndGrabHandler,
};

use super::{Backend, Otto};

impl<BackendData: Backend> WaylandDndGrabHandler for Otto<BackendData> {
    fn dnd_requested<S: smithay::input::dnd::Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        self.dnd_icon = icon;
        let p = self.get_cursor_position();
        let p = (p.x as f32, p.y as f32).into();
        self.workspaces.dnd_view.set_initial_position(p);
        self.workspaces.dnd_view.layer.set_scale((1.0, 1.0), None);

        self.workspaces
            .dnd_view
            .layer
            .set_opacity(0.8, Some(Transition::default()));

        // Actually start the DnD grab!
        match type_ {
            GrabType::Pointer => {
                let pointer = seat.get_pointer().unwrap();
                let start_data = pointer.grab_start_data().unwrap();
                pointer.set_grab(
                    self,
                    DnDGrab::new_pointer(&self.display_handle, start_data, source, seat),
                    serial,
                    Focus::Keep,
                );
            }
            GrabType::Touch => {
                let touch = seat.get_touch().unwrap();
                let start_data = touch.grab_start_data().unwrap();
                touch.set_grab(
                    self,
                    DnDGrab::new_touch(&self.display_handle, start_data, source, seat),
                    serial,
                );
            }
        }
    }
}

impl<BackendData: Backend> DndGrabHandler for Otto<BackendData> {
    fn dropped(
        &mut self,
        _target: Option<smithay::input::dnd::DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        // Clean up layers before clearing dnd_icon
        let dnd_surface = self.dnd_icon.clone();
        if let Some(ref surface) = dnd_surface {
            self.cleanup_dnd_layers(surface);
        }

        self.dnd_icon = None;
        self.workspaces
            .dnd_view
            .layer
            .set_opacity(0.0, Some(Transition::default()));
        self.workspaces
            .dnd_view
            .layer
            .set_scale((1.2, 1.2), Some(Transition::default()));

        // Reset cursor to default
        self.set_cursor(&CursorImageStatus::default_named());
    }
}
