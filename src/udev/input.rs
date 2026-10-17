//! The udev backend's input dispatcher: routes libinput events to the
//! handler modules (keyboard, pointer, gestures, tablet) and handles what
//! only a session on the hardware can do, like switching VTs.

use smithay::{
    backend::{
        input::{
            Device, DeviceCapability, InputBackend, InputEvent, Switch, SwitchState,
            SwitchToggleEvent,
        },
        session::Session,
    },
    input::tablet::{TabletDescriptor, TabletSeatTrait},
    output::Scale,
    reexports::wayland_server::DisplayHandle,
};

use super::UdevData;
use crate::{input::KeyAction, renderer::active::RendererApi, state::Backend, Otto};

impl<A: RendererApi> Otto<UdevData<A>> {
    pub fn process_input_event<B: InputBackend>(
        &mut self,
        dh: &DisplayHandle,
        event: InputEvent<B>,
    ) {
        // Every event, whatever it turns into: auto-lock measures idleness
        // from the last one (`lock.auto_lock_timeout`).
        self.note_input_activity();
        self.note_press(&event);
        match event {
            InputEvent::Keyboard { event, .. } => {
                let action = self.keyboard_key_to_action::<B>(event);
                match self.dispatch_key_action(action) {
                    None => (),
                    Some(KeyAction::VtSwitch(vt)) => {
                        tracing::info!(to = vt, "Trying to switch vt");
                        if let Err(err) = self.backend_data.session.change_vt(vt) {
                            tracing::error!(vt, "Error switching vt: {}", err);
                        }
                    }
                    Some(KeyAction::Screen(num)) => {
                        let geometry = self
                            .workspaces
                            .outputs()
                            .nth(num)
                            .map(|o| self.workspaces.output_geometry(o).unwrap());

                        if let Some(geometry) = geometry {
                            let x = geometry.loc.x as f64 + geometry.size.w as f64 / 2.0;
                            let y = geometry.size.h as f64 / 2.0;
                            self.warp_pointer_to((x, y).into());
                        }
                    }
                    Some(KeyAction::ScaleUp) => {
                        self.change_output_scale_under_pointer(|scale| scale + 0.25)
                    }
                    Some(KeyAction::ScaleDown) => {
                        self.change_output_scale_under_pointer(|scale| f64::max(1.0, scale - 0.25))
                    }
                    Some(KeyAction::RotateOutput) => {
                        if let Some(output) = self.output_under_pointer() {
                            self.rotate_output(&output);
                        }
                    }
                    // A bound action this dispatcher has no arm for must not
                    // take the session down with it: every builtin lands
                    // here first on the udev backend.
                    Some(action) => {
                        tracing::warn!(?action, "Key action unsupported on this backend.")
                    }
                }
            }
            InputEvent::PointerMotion { event, .. } => self.on_pointer_move::<B>(dh, event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.on_pointer_move_absolute::<B>(dh, event)
            }
            InputEvent::PointerButton { event, .. } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. } => self.on_pointer_axis::<B>(event),
            InputEvent::TabletToolAxis { event, .. } => self.on_tablet_tool_axis::<B>(event),
            InputEvent::TabletToolProximity { event, .. } => {
                self.on_tablet_tool_proximity::<B>(dh, event)
            }
            InputEvent::TabletToolTip { event, .. } => self.on_tablet_tool_tip::<B>(event),
            InputEvent::TabletToolButton { event, .. } => self.on_tablet_button::<B>(event),
            InputEvent::GestureSwipeBegin { event, .. } => self.on_gesture_swipe_begin::<B>(event),
            InputEvent::GestureSwipeUpdate { event, .. } => {
                self.on_gesture_swipe_update::<B>(event)
            }
            InputEvent::GestureSwipeEnd { event, .. } => self.on_gesture_swipe_end::<B>(event),
            InputEvent::GesturePinchBegin { event, .. } => self.on_gesture_pinch_begin::<B>(event),
            InputEvent::GesturePinchUpdate { event, .. } => {
                self.on_gesture_pinch_update::<B>(event)
            }
            InputEvent::GesturePinchEnd { event, .. } => self.on_gesture_pinch_end::<B>(event),
            InputEvent::GestureHoldBegin { event, .. } => self.on_gesture_hold_begin::<B>(event),
            InputEvent::GestureHoldEnd { event, .. } => self.on_gesture_hold_end::<B>(event),
            InputEvent::SwitchToggle { event } => {
                if let Some(switch) = event.switch() {
                    if switch == Switch::Lid {
                        let is_closed = event.state() == SwitchState::On;
                        tracing::info!(
                            is_closed,
                            "Lid switch {}",
                            if is_closed { "closed" } else { "opened" }
                        );
                        self.is_lid_closed = is_closed;
                        self.update_display_power_state();
                    }
                }
            }
            InputEvent::DeviceAdded { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    self.seat
                        .tablet_seat()
                        .add_wp_tablet(dh, &TabletDescriptor::from(&device));
                }
            }
            InputEvent::DeviceRemoved { device }
                if device.has_capability(DeviceCapability::TabletTool) =>
            {
                let tablet_seat = self.seat.tablet_seat();

                tablet_seat.remove_tablet(&TabletDescriptor::from(&device));

                // If there are no tablets in seat we can remove all tools
                if tablet_seat.count_tablets() == 0 {
                    tablet_seat.clear_tools();
                }
            }
            _ => {
                // other events are not handled (yet)
            }
        }
    }

    /// The output whose geometry contains the pointer.
    fn output_under_pointer(&self) -> Option<smithay::output::Output> {
        let pos = self.pointer.current_location().to_i32_round();
        self.workspaces
            .outputs()
            .find(|o| self.workspaces.output_geometry(o).unwrap().contains(pos))
            .cloned()
    }

    /// Move the pointer to `location` as if the user had, so focus and the
    /// cursor follow.
    fn warp_pointer_to(&mut self, location: smithay::utils::Point<f64, smithay::utils::Logical>) {
        let pointer = self.pointer.clone();
        let under = self.surface_under(location);
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: smithay::backend::input::InputTime::from_millis(0),
            },
        );
        pointer.frame(self);
    }

    /// Step the scale of the output under the pointer to `new_scale(current)`,
    /// keeping the pointer over the same spot of that output's content.
    fn change_output_scale_under_pointer(&mut self, new_scale: impl Fn(f64) -> f64) {
        let Some(output) = self.output_under_pointer() else {
            return;
        };
        let (output_location, scale) = (
            self.workspaces.output_geometry(&output).unwrap().loc,
            output.current_scale().fractional_scale(),
        );
        let new_scale = new_scale(scale);
        output.change_current_state(None, None, Some(Scale::Fractional(new_scale)), None);

        let rescale = scale / new_scale;
        let output_location = output_location.to_f64();
        let mut pointer_output_location = self.pointer.current_location() - output_location;
        pointer_output_location.x *= rescale;
        pointer_output_location.y *= rescale;
        let pointer_location = output_location + pointer_output_location;

        crate::shell::fixup_positions(&mut self.workspaces, pointer_location);
        self.warp_pointer_to(pointer_location);
        self.backend_data.reset_buffers(&output);
    }
}
