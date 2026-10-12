//! Input event dispatching
//!
//! This file contains the windowed backends' input dispatcher, which routes
//! events to the handler modules (keyboard, pointer); the udev backend's
//! lives in `udev::input`.

use smithay::utils::Transform;
#[cfg(any(feature = "winit", feature = "x11"))]
use smithay::{
    backend::input::{InputBackend, InputEvent},
    output::Scale,
};

#[cfg(any(feature = "winit", feature = "x11"))]
use crate::input::KeyAction;
use crate::Otto;

#[cfg(any(feature = "winit", feature = "x11"))]
impl<Backend: crate::state::Backend> Otto<Backend> {
    pub fn process_input_event_windowed<B: InputBackend>(
        &mut self,
        event: InputEvent<B>,
        output_name: &str,
    ) {
        // Every event, whatever it turns into: auto-lock measures idleness
        // from the last one (`lock.auto_lock_timeout`).
        self.note_input_activity();
        self.note_press(&event);
        match event {
            InputEvent::Keyboard { event } => {
                let action = self.keyboard_key_to_action::<B>(event);
                match self.dispatch_key_action(action) {
                    None => (),
                    // Nested in a host window, the output is the one the
                    // backend names, not the one under the pointer.
                    Some(KeyAction::ScaleUp) => {
                        self.change_windowed_output_scale(output_name, |scale| scale + 0.25)
                    }
                    Some(KeyAction::ScaleDown) => self
                        .change_windowed_output_scale(output_name, |scale| {
                            f64::max(1.0, scale - 0.25)
                        }),
                    Some(KeyAction::RotateOutput) => {
                        let output = self
                            .workspaces
                            .outputs()
                            .find(|o| o.name() == output_name)
                            .unwrap()
                            .clone();
                        self.rotate_output(&output);
                    }
                    Some(action) => tracing::warn!(
                        ?action,
                        output_name,
                        "Key action unsupported on output backend.",
                    ),
                }
            }

            InputEvent::PointerMotionAbsolute { event } => {
                let output = self
                    .workspaces
                    .outputs()
                    .find(|o| o.name() == output_name)
                    .unwrap()
                    .clone();
                self.on_pointer_move_absolute_windowed::<B>(event, &output)
            }
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            _ => (), // other events are not handled (yet)
        }
    }

    /// Step the scale of the backend's output `output_name` to
    /// `new_scale(current)`.
    fn change_windowed_output_scale(&mut self, output_name: &str, new_scale: impl Fn(f64) -> f64) {
        let output = self
            .workspaces
            .outputs()
            .find(|o| o.name() == output_name)
            .unwrap()
            .clone();

        let current_scale = output.current_scale().fractional_scale();
        output.change_current_state(
            None,
            None,
            Some(Scale::Fractional(new_scale(current_scale))),
            None,
        );
        let current_location = self.pointer.current_location();

        crate::shell::fixup_positions(&mut self.workspaces, current_location);
        self.backend_data.reset_buffers(&output);
        #[cfg(feature = "xwayland")]
        self.update_xwayland_scale();
    }
}

#[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
impl<Backend: crate::state::Backend> Otto<Backend> {
    /// Turn `output` a quarter turn clockwise (`RotateOutput`).
    pub(crate) fn rotate_output(&mut self, output: &smithay::output::Output) {
        let new_transform = match output.current_transform() {
            Transform::Normal => Transform::_90,
            Transform::_90 => Transform::_180,
            Transform::_180 => Transform::_270,
            Transform::_270 => Transform::Normal,
            _ => Transform::Normal,
        };
        output.change_current_state(None, Some(new_transform), None, None);
        let current_location = self.pointer.current_location();
        crate::shell::fixup_positions(&mut self.workspaces, current_location);
        self.backend_data.reset_buffers(output);
    }
}
