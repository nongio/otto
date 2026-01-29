#[cfg(feature = "udev")]
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Event, InputBackend, ProximityState, TabletToolButtonEvent,
        TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState,
    },
    input::pointer::MotionEvent,
    reexports::wayland_server::DisplayHandle,
    utils::SERIAL_COUNTER as SCOUNTER,
    wayland::{
        seat::WaylandFocus,
        tablet_manager::{TabletDescriptor, TabletSeatTrait},
    },
};

#[cfg(feature = "udev")]
impl crate::Otto<crate::udev::UdevData> {
    pub(crate) fn on_tablet_tool_axis<B: InputBackend>(&mut self, evt: B::TabletToolAxisEvent) {
        let tablet_seat = self.seat.tablet_seat();

        let output_geometry = self
            .workspaces
            .outputs()
            .next()
            .map(|o| self.workspaces.output_geometry(o).unwrap());

        if let Some(rect) = output_geometry {
            let pointer_location = evt.position_transformed(rect.size) + rect.loc.to_f64();

            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&evt.tool());

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: 0,
                },
            );

            if let (Some(tablet), Some(tool)) = (tablet, tool) {
                if evt.pressure_has_changed() {
                    tool.pressure(evt.pressure());
                }
                if evt.distance_has_changed() {
                    tool.distance(evt.distance());
                }
                if evt.tilt_has_changed() {
                    tool.tilt(evt.tilt());
                }
                if evt.slider_has_changed() {
                    tool.slider_position(evt.slider_position());
                }
                if evt.rotation_has_changed() {
                    tool.rotation(evt.rotation());
                }
                if evt.wheel_has_changed() {
                    tool.wheel(evt.wheel_delta(), evt.wheel_delta_discrete());
                }

                tool.motion(
                    pointer_location,
                    under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                    &tablet,
                    SCOUNTER.next_serial(),
                    evt.time_msec(),
                );
            }

            pointer.frame(self);
        }
    }

    pub(crate) fn on_tablet_tool_proximity<B: InputBackend>(
        &mut self,
        _dh: &DisplayHandle,
        evt: B::TabletToolProximityEvent,
    ) {
        let tablet_seat = self.seat.tablet_seat();

        let output_geometry = self
            .workspaces
            .outputs()
            .next()
            .map(|o| self.workspaces.output_geometry(o).unwrap());

        if let Some(rect) = output_geometry {
            let tool = evt.tool();
            // FIXME: tablet handling on proximity
            // tablet_seat.add_tool::<Self>(dh, &tool);

            let pointer_location = evt.position_transformed(rect.size) + rect.loc.to_f64();

            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&tool);

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: 0,
                },
            );
            pointer.frame(self);

            if let (Some(under), Some(tablet), Some(tool)) = (
                under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                tablet,
                tool,
            ) {
                match evt.state() {
                    ProximityState::In => tool.proximity_in(
                        pointer_location,
                        under,
                        &tablet,
                        SCOUNTER.next_serial(),
                        evt.time_msec(),
                    ),
                    ProximityState::Out => tool.proximity_out(evt.time_msec()),
                }
            }
        }
    }

    pub(crate) fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            match evt.tip_state() {
                TabletToolTipState::Down => {
                    let serial = SCOUNTER.next_serial();
                    tool.tip_down(serial, evt.time_msec());

                    self.focus_window_under_cursor(serial);
                }
                TabletToolTipState::Up => {
                    tool.tip_up(evt.time_msec());
                }
            }
        }
    }

    pub(crate) fn on_tablet_button<B: InputBackend>(&mut self, evt: B::TabletToolButtonEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            tool.button(
                evt.button(),
                evt.button_state(),
                SCOUNTER.next_serial(),
                evt.time_msec(),
            );
        }
    }
}
