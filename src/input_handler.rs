//! Input event dispatching
//!
//! This file contains the top-level input event dispatchers that route events
//! to the appropriate handler modules (keyboard, pointer, gestures, tablet).

use smithay::{
    backend::input::{InputBackend, InputEvent},
    output::Scale,
    reexports::wayland_server::DisplayHandle,
    utils::Transform,
};

use crate::{input::KeyAction, state::Backend, Otto};

#[cfg(feature = "udev")]
use crate::udev::UdevData;

#[cfg(feature = "udev")]
use smithay::{
    backend::{
        input::{Device, DeviceCapability},
        session::Session,
    },
    wayland::tablet_manager::{TabletDescriptor, TabletSeatTrait},
};

#[cfg(any(feature = "winit", feature = "x11"))]
impl<Backend: crate::state::Backend> Otto<Backend> {
    pub fn process_input_event_windowed<B: InputBackend>(
        &mut self,
        event: InputEvent<B>,
        output_name: &str,
    ) {
        match event {
            InputEvent::Keyboard { event } => match self.keyboard_key_to_action::<B>(event) {
                KeyAction::ScaleUp => {
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_scale = output.current_scale().fractional_scale();
                    let new_scale = current_scale + 0.25;
                    output.change_current_state(
                        None,
                        None,
                        Some(Scale::Fractional(new_scale)),
                        None,
                    );
                    let current_location = self.pointer.current_location();

                    crate::shell::fixup_positions(&mut self.workspaces, current_location);
                    self.backend_data.reset_buffers(&output);
                }

                KeyAction::ScaleDown => {
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_scale = output.current_scale().fractional_scale();
                    let new_scale = f64::max(1.0, current_scale - 0.25);
                    output.change_current_state(
                        None,
                        None,
                        Some(Scale::Fractional(new_scale)),
                        None,
                    );
                    let current_location = self.pointer.current_location();
                    crate::shell::fixup_positions(&mut self.workspaces, current_location);
                    self.backend_data.reset_buffers(&output);
                }

                KeyAction::RotateOutput => {
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_transform = output.current_transform();
                    let new_transform = match current_transform {
                        Transform::Normal => Transform::_90,
                        Transform::_90 => Transform::_180,
                        Transform::_180 => Transform::_270,
                        Transform::_270 => Transform::Normal,
                        _ => Transform::Normal,
                    };
                    output.change_current_state(None, Some(new_transform), None, None);
                    let current_location = self.pointer.current_location();

                    crate::shell::fixup_positions(&mut self.workspaces, current_location);
                    self.backend_data.reset_buffers(&output);
                }
                KeyAction::ApplicationSwitchNext => {
                    self.handle_app_switcher_next();
                }
                KeyAction::ApplicationSwitchPrev => {
                    self.handle_app_switcher_prev();
                }
                KeyAction::ApplicationSwitchQuit => {
                    self.handle_app_switcher_quit();
                }
                KeyAction::ToggleMaximize => {
                    self.handle_toggle_maximize();
                }
                KeyAction::CloseWindow => {
                    self.handle_close_window();
                }
                KeyAction::ApplicationSwitchNextWindow => {
                    self.handle_app_switcher_next_window();
                }
                KeyAction::ExposeShowDesktop => {
                    self.handle_expose_show_desktop();
                }
                KeyAction::ExposeShowAll => {
                    self.handle_expose_show_all();
                }
                KeyAction::WorkspaceNum(n) => {
                    self.handle_workspace_num(n);
                }

                action => match action {
                    KeyAction::None
                    | KeyAction::Quit
                    | KeyAction::Run(_)
                    | KeyAction::ToggleDecorations
                    | KeyAction::SceneSnapshot => self.process_common_key_action(action),

                    _ => tracing::warn!(
                        ?action,
                        output_name,
                        "Key action unsupported on output backend.",
                    ),
                },
            },

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
}

#[cfg(feature = "udev")]
impl Otto<UdevData> {
    pub fn process_input_event<B: InputBackend>(
        &mut self,
        dh: &DisplayHandle,
        event: InputEvent<B>,
    ) {
        match event {
            InputEvent::Keyboard { event, .. } => match self.keyboard_key_to_action::<B>(event) {
                #[cfg(feature = "udev")]
                KeyAction::VtSwitch(vt) => {
                    tracing::info!(to = vt, "Trying to switch vt");
                    if let Err(err) = self.backend_data.session.change_vt(vt) {
                        tracing::error!(vt, "Error switching vt: {}", err);
                    }
                }
                KeyAction::Screen(num) => {
                    let geometry = self
                        .workspaces
                        .outputs()
                        .nth(num)
                        .map(|o| self.workspaces.output_geometry(o).unwrap());

                    if let Some(geometry) = geometry {
                        let x = geometry.loc.x as f64 + geometry.size.w as f64 / 2.0;
                        let y = geometry.size.h as f64 / 2.0;
                        let location = (x, y).into();
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(location);
                        pointer.motion(
                            self,
                            under,
                            &smithay::input::pointer::MotionEvent {
                                location,
                                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                                time: 0,
                            },
                        );
                        pointer.frame(self);
                    }
                }
                KeyAction::ScaleUp => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| self.workspaces.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let (output_location, scale) = (
                            self.workspaces.output_geometry(&output).unwrap().loc,
                            output.current_scale().fractional_scale(),
                        );
                        let new_scale = scale + 0.25;
                        output.change_current_state(
                            None,
                            None,
                            Some(Scale::Fractional(new_scale)),
                            None,
                        );

                        let rescale = scale / new_scale;
                        let output_location = output_location.to_f64();
                        let mut pointer_output_location =
                            self.pointer.current_location() - output_location;
                        pointer_output_location.x *= rescale;
                        pointer_output_location.y *= rescale;
                        let pointer_location = output_location + pointer_output_location;
                        crate::shell::fixup_positions(&mut self.workspaces, pointer_location);
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(pointer_location);
                        pointer.motion(
                            self,
                            under,
                            &smithay::input::pointer::MotionEvent {
                                location: pointer_location,
                                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                                time: 0,
                            },
                        );
                        pointer.frame(self);
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::ScaleDown => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| self.workspaces.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let (output_location, scale) = (
                            self.workspaces.output_geometry(&output).unwrap().loc,
                            output.current_scale().fractional_scale(),
                        );
                        let new_scale = f64::max(1.0, scale - 0.25);
                        output.change_current_state(
                            None,
                            None,
                            Some(Scale::Fractional(new_scale)),
                            None,
                        );

                        let rescale = scale / new_scale;
                        let output_location = output_location.to_f64();
                        let mut pointer_output_location =
                            self.pointer.current_location() - output_location;
                        pointer_output_location.x *= rescale;
                        pointer_output_location.y *= rescale;
                        let pointer_location = output_location + pointer_output_location;

                        crate::shell::fixup_positions(&mut self.workspaces, pointer_location);
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(pointer_location);
                        pointer.motion(
                            self,
                            under,
                            &smithay::input::pointer::MotionEvent {
                                location: pointer_location,
                                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                                time: 0,
                            },
                        );
                        pointer.frame(self);
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::RotateOutput => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .workspaces
                        .outputs()
                        .find(|o| self.workspaces.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let current_transform = output.current_transform();
                        let new_transform = match current_transform {
                            Transform::Normal => Transform::_90,
                            Transform::_90 => Transform::_180,
                            Transform::_180 => Transform::_270,
                            Transform::_270 => Transform::Normal,
                            _ => Transform::Normal,
                        };
                        output.change_current_state(None, Some(new_transform), None, None);
                        let current_location = self.pointer.current_location();
                        crate::shell::fixup_positions(&mut self.workspaces, current_location);
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::ApplicationSwitchNext => {
                    self.handle_app_switcher_next();
                }
                KeyAction::ApplicationSwitchPrev => {
                    self.handle_app_switcher_prev();
                }
                KeyAction::ApplicationSwitchNextWindow => {
                    self.handle_app_switcher_next_window();
                }
                KeyAction::ApplicationSwitchQuit => {
                    self.handle_app_switcher_quit();
                }
                KeyAction::ToggleMaximize => {
                    self.handle_toggle_maximize();
                }
                KeyAction::CloseWindow => {
                    self.handle_close_window();
                }
                KeyAction::ExposeShowDesktop => {
                    self.handle_expose_show_desktop();
                }
                KeyAction::ExposeShowAll => {
                    self.handle_expose_show_all();
                }
                KeyAction::WorkspaceNum(index) => {
                    self.handle_workspace_num(index);
                }
                action => match action {
                    KeyAction::None
                    | KeyAction::Quit
                    | KeyAction::Run(_)
                    | KeyAction::ToggleDecorations
                    | KeyAction::SceneSnapshot => self.process_common_key_action(action),

                    _ => unreachable!(),
                },
            },
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
            InputEvent::DeviceAdded { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    self.seat
                        .tablet_seat()
                        .add_tablet::<Self>(dh, &TabletDescriptor::from(&device));
                }
            }
            InputEvent::DeviceRemoved { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    let tablet_seat = self.seat.tablet_seat();

                    tablet_seat.remove_tablet(&TabletDescriptor::from(&device));

                    // If there are no tablets in seat we can remove all tools
                    if tablet_seat.count_tablets() == 0 {
                        tablet_seat.clear_tools();
                    }
                }
            }
            _ => {
                // other events are not handled (yet)
            }
        }
    }
}
