use layers::prelude::{TimingFunction, Transition};
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

/// How long the icon takes to fly back to where the drag started, when the
/// drop is refused. Long enough to read as a return rather than a glitch,
/// short enough not to make the user wait to try again.
const SNAP_BACK: f32 = 0.35;

impl<BackendData: Backend> WaylandDndGrabHandler for Otto<BackendData> {
    fn dnd_requested<S: smithay::input::dnd::Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        // Whatever the last refused drag left behind, now that its flight home
        // is over and its layers are no longer being looked at.
        if let Some(surface) = self.pending_dnd_cleanup.take() {
            self.cleanup_dnd_layers(&surface);
        }

        self.dnd_icon = icon;
        let p = self.get_cursor_position();
        let p = (p.x as f32, p.y as f32).into();
        self.workspaces.dnd_view.set_initial_position(p);
        self.workspaces.dnd_view.layer.set_scale((1.0, 1.0), None);

        self.workspaces
            .dnd_view
            .layer
            .set_opacity(0.8_f32, Some(Transition::default()));

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
        validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        let dnd_surface = self.dnd_icon.clone();
        // Cleared first either way: `update_dnd` snaps the view to the cursor
        // every frame while an icon exists, and would fight the animation.
        self.dnd_icon = None;

        let view = self.workspaces.dnd_view.clone();
        if validated {
            // Taken. The icon has arrived, so it gets out of the way where it
            // landed: a fade, growing slightly as it goes.
            if let Some(ref surface) = dnd_surface {
                self.cleanup_dnd_layers(surface);
            }
            view.layer.set_opacity(0.0_f32, Some(Transition::default()));
            view.layer
                .set_scale((1.2, 1.2), Some(Transition::default()));
        } else {
            // Refused — nothing happened, and the files are still where they
            // started. Fly the icon home to say so, the way a dragged thing
            // that is not accepted goes back.
            //
            // The content layers are torn down when the flight ends rather
            // than now: cleaning them up here would animate an empty layer.
            let flight = Transition {
                delay: 0.0,
                timing: TimingFunction::ease_out_quad(SNAP_BACK),
            };
            view.layer.set_scale((1.0, 1.0), Some(flight.clone()));
            view.layer.set_position(view.initial_position, Some(flight));
            view.layer.set_opacity(
                0.0_f32,
                Some(Transition {
                    // Held opaque for most of the way: fading immediately
                    // would make it vanish before it arrives, and the point of
                    // the gesture is seeing where it goes back to.
                    delay: SNAP_BACK * 0.6,
                    timing: TimingFunction::ease_out_quad(SNAP_BACK * 0.4),
                }),
            );
            // Swept up when the next drag starts rather than now: removing the
            // content layers here would leave an empty layer to fly home.
            self.pending_dnd_cleanup = dnd_surface;
        }

        // Reset cursor to default
        self.set_cursor(&CursorImageStatus::default_named());
    }
}
