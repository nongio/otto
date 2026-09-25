#[cfg(feature = "udev")]
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Event, InputBackend, InputTime, ProximityState,
        TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent,
        TabletToolTipState,
    },
    input::{
        pointer::MotionEvent,
        tablet::{self, TabletDescriptor, TabletSeatTrait},
    },
    reexports::wayland_server::DisplayHandle,
    utils::SERIAL_COUNTER as SCOUNTER,
};

#[cfg(feature = "udev")]
impl<A: crate::renderer::active::RendererApi> crate::Otto<crate::udev::UdevData<A>> {
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
            let tool = tablet_seat.get_tool(&evt.tool());

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: InputTime::from_millis(0),
                },
            );

            if let Some(tool) = tool {
                let frame = tablet::tool::AxisFrame {
                    pressure: evt.pressure_has_changed().then(|| evt.pressure()),
                    distance: evt.distance_has_changed().then(|| evt.distance()),
                    tilt: evt.tilt_has_changed().then(|| evt.tilt()),
                    rotation: evt.rotation_has_changed().then(|| evt.rotation()),
                    slider: evt.slider_has_changed().then(|| evt.slider_position()),
                    wheel: evt
                        .wheel_has_changed()
                        .then(|| (evt.wheel_delta(), evt.wheel_delta_discrete())),
                };

                tool.axis(self, frame);
                tool.motion(
                    self,
                    under,
                    &tablet::tool::MotionEvent {
                        location: pointer_location,
                        serial: SCOUNTER.next_serial(),
                        time: evt.time(),
                    },
                );
                tool.frame(self, evt.time());
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
                    time: InputTime::from_millis(0),
                },
            );
            pointer.frame(self);

            if let (Some(tablet), Some(tool)) = (tablet, tool) {
                let frame = tablet::tool::AxisFrame {
                    pressure: evt.pressure_has_changed().then(|| evt.pressure()),
                    distance: evt.distance_has_changed().then(|| evt.distance()),
                    tilt: evt.tilt_has_changed().then(|| evt.tilt()),
                    rotation: evt.rotation_has_changed().then(|| evt.rotation()),
                    slider: evt.slider_has_changed().then(|| evt.slider_position()),
                    wheel: evt
                        .wheel_has_changed()
                        .then(|| (evt.wheel_delta(), evt.wheel_delta_discrete())),
                };

                match evt.state() {
                    ProximityState::In => tool.proximity_in(
                        self,
                        under,
                        tablet,
                        &tablet::tool::ProximityInEvent {
                            location: pointer_location,
                            axis: Some(frame),
                            serial: SCOUNTER.next_serial(),
                            time: evt.time(),
                        },
                    ),
                    ProximityState::Out => tool.proximity_out(
                        self,
                        &tablet::tool::ProximityOutEvent {
                            serial: SCOUNTER.next_serial(),
                            time: evt.time(),
                        },
                    ),
                }
                tool.frame(self, evt.time());
            }
        }
    }

    pub(crate) fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            let serial = SCOUNTER.next_serial();
            match evt.tip_state() {
                TabletToolTipState::Down => {
                    tool.down(
                        self,
                        &tablet::tool::DownEvent {
                            serial,
                            time: evt.time(),
                        },
                    );

                    self.focus_window_under_cursor(
                        serial,
                        crate::input::pointer::RaiseTiming::Press,
                    );
                }
                TabletToolTipState::Up => {
                    tool.up(
                        self,
                        &tablet::tool::UpEvent {
                            serial,
                            time: evt.time(),
                        },
                    );
                }
            }
            tool.frame(self, evt.time());
        }
    }

    pub(crate) fn on_tablet_button<B: InputBackend>(&mut self, evt: B::TabletToolButtonEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            tool.button(
                self,
                &tablet::tool::ButtonEvent {
                    serial: SCOUNTER.next_serial(),
                    button: evt.button(),
                    state: evt.button_state(),
                    time: evt.time(),
                },
            );
            tool.frame(self, evt.time());
        }
    }
}
