use crate::{focus::PointerFocusTarget, shell::FullscreenSurface, state::Backend, Otto};
use smithay::{
    backend::input::{
        self, Axis, AxisSource, ButtonState, Event, InputBackend, PointerAxisEvent,
        PointerButtonEvent,
    },
    desktop::{layer_map_for_output, WindowSurfaceType},
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    reexports::wayland_server::{protocol::wl_pointer, Resource},
    utils::{IsAlive, Logical, Point, Serial, SERIAL_COUNTER as SCOUNTER},
    wayland::{input_method::InputMethodSeat, shell::wlr_layer::Layer as WlrLayer},
};

#[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
use smithay::backend::input::AbsolutePositionEvent;

#[cfg(any(feature = "winit", feature = "x11"))]
use smithay::output::Output;

#[cfg(feature = "udev")]
use smithay::{
    backend::input::PointerMotionEvent,
    input::pointer::RelativeMotionEvent,
    wayland::{
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        seat::WaylandFocus,
    },
};

use crate::config::Config;

impl<BackendData: Backend> Otto<BackendData> {
    pub(crate) fn on_pointer_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
        let serial = SCOUNTER.next_serial();
        let button = evt.button_code();

        let state = wl_pointer::ButtonState::from(evt.state());

        if !self.workspaces.get_show_all() && wl_pointer::ButtonState::Pressed == state {
            self.focus_window_under_cursor(serial);
        }
        let pointer = self.pointer.clone();
        let button_state = state.try_into().unwrap();
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: button_state,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
        match button_state {
            ButtonState::Pressed => {
                self.layers_engine.pointer_button_down();
            }
            ButtonState::Released => {
                self.layers_engine.pointer_button_up();
            }
        }
    }

    /// Update the focus on the topmost surface under the cursor in the current workspace
    /// The window is raised and the keyboard focus is set to the window.
    pub(crate) fn focus_window_under_cursor(&mut self, serial: Serial) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let input_method = self.seat.input_method();
        // change the keyboard focus unless the pointer or keyboard is grabbed
        // We test for any matching surface type here but always use the root
        // (in case of a window the toplevel) surface for the focus.
        // So for example if a user clicks on a subsurface or popup the toplevel
        // will receive the keyboard focus. Directly assigning the focus to the
        // matching surface leads to issues with clients dismissing popups and
        // subsurface menus (for example firefox-wayland).
        // see here for a discussion about that issue:
        // https://gitlab.freedesktop.org/wayland/wayland/-/issues/294
        if !self.pointer.is_grabbed() && (!keyboard.is_grabbed() || input_method.keyboard_grabbed())
        {
            let output = self
                .workspaces
                .output_under(self.pointer.current_location())
                .next()
                .cloned();
            if let Some(output) = output.as_ref() {
                let output_geo = self.workspaces.output_geometry(output).unwrap();
                if let Some(window) = output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                {
                    if let Some((_, _)) = window.surface_under::<BackendData>(
                        self.pointer.current_location() - output_geo.loc.to_f64(),
                        WindowSurfaceType::ALL,
                    ) {
                        #[cfg(feature = "xwayland")]
                        if let crate::shell::window::WindowSurface::X11(surf) =
                            window.underlying_surface()
                        {
                            self.xwm.as_mut().unwrap().raise_window(surf).unwrap();
                        }
                        keyboard.set_focus(self, Some(window.into()), serial);
                        return;
                    }
                }

                // Check if an overlay/top layer surface should receive keyboard focus
                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Overlay, self.pointer.current_location())
                    .or_else(|| layers.layer_under(WlrLayer::Top, self.pointer.current_location()))
                {
                    if layer.can_receive_keyboard_focus() {
                        if let Some((_, _)) = layer.surface_under(
                            self.pointer.current_location()
                                - output_geo.loc.to_f64()
                                - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                            return;
                        }
                    }
                }
            }
            let scale = output
                .as_ref()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0);
            let position = self.pointer.current_location();
            let scaled_position = position.to_physical(scale);
            if !self
                .workspaces
                .is_cursor_over_dock(scaled_position.x as f32, scaled_position.y as f32)
            {
                let window_under = self
                    .workspaces
                    .element_under(position)
                    .map(|(w, p)| (w.clone(), p));

                if let Some((window, _)) = window_under {
                    if let Some(id) = window.wl_surface().as_ref().map(|s| s.id()) {
                        if let Some(w) = self.workspaces.get_window_for_surface(&id) {
                            if w.is_minimised() {
                                return;
                            }
                            if w.is_fullscreen() {
                                return;
                            }
                            self.workspaces.focus_app_with_window(&id);
                            keyboard.set_focus(self, Some(window.into()), serial);
                            self.workspaces.update_workspace_model();
                        }
                    }

                    #[cfg(feature = "xwayland")]
                    if let crate::shell::window::WindowSurface::X11(surf) =
                        &window.underlying_surface()
                    {
                        self.xwm.as_mut().unwrap().raise_window(surf).unwrap();
                    }
                }
            }

            // Check if a bottom/background layer surface should receive keyboard focus
            if let Some(output) = output.as_ref() {
                let output_geo = self.workspaces.output_geometry(output).unwrap();
                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Bottom, self.pointer.current_location())
                    .or_else(|| {
                        layers.layer_under(WlrLayer::Background, self.pointer.current_location())
                    })
                {
                    if layer.can_receive_keyboard_focus() {
                        if let Some((_, _)) = layer.surface_under(
                            self.pointer.current_location()
                                - output_geo.loc.to_f64()
                                - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                        }
                    }
                }
            }
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget<BackendData>, Point<f64, Logical>)> {
        let output = self.workspaces.outputs().find(|o| {
            let geometry = self.workspaces.output_geometry(o).unwrap();
            geometry.contains(pos.to_i32_round())
        })?;
        let output_geo = self.workspaces.output_geometry(output).unwrap();
        let layers = layer_map_for_output(output);
        let scale = output.current_scale().fractional_scale();
        let physical_pos = pos.to_physical(scale);
        let mut under = None;

        // App switcher check
        if self.workspaces.app_switcher.alive() {
            let focus = self.workspaces.app_switcher.as_ref().clone().into();
            return Some((focus, (0.0, 0.0).into()));
        }

        // Workspace selector
        if self.workspaces.get_show_all() {
            let focus = self
                .workspaces
                .workspace_selector_view
                .as_ref()
                .clone()
                .into();

            let layer = self.workspaces.workspace_selector_view.layer.clone();

            if layer.cointains_point((physical_pos.x as f32, physical_pos.y as f32)) {
                let position = self
                    .workspaces
                    .workspace_selector_view
                    .layer
                    .render_position();
                return Some((focus, (position.x as f64, position.y as f64).into()));
            }
        }
        // Window selector check
        if self.workspaces.get_show_all() {
            let workspace = self.workspaces.get_current_workspace();
            let focus = workspace.window_selector_view.as_ref().clone().into();
            let position = workspace.window_selector_view.layer.render_position();

            return Some((focus, (position.x as f64, position.y as f64).into()));
        }

        if let Some(layer) = layers
            .layer_under(WlrLayer::Overlay, pos)
            .or_else(|| layers.layer_under(WlrLayer::Top, pos))
        {
            let layer_loc = layers.layer_geometry(layer).unwrap().loc;
            under = Some((layer.clone().into(), output_geo.loc + layer_loc));
        }
        // Check dock
        else if self
            .workspaces
            .is_cursor_over_dock(physical_pos.x as f32, physical_pos.y as f32)
        {
            // Dock
            under = Some((self.workspaces.dock.as_ref().clone().into(), (0, 0).into()));
        } else if let Some((focus, location)) =
            self.workspaces
                .element_under(pos)
                .and_then(|(window, loc)| {
                    if let Some(id) = window.wl_surface().as_ref().map(|s| s.id()) {
                        if let Some(w) = self.workspaces.get_window_for_surface(&id) {
                            if w.is_minimised() {
                                return None;
                            }
                        }
                    }
                    window
                        .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surf_loc)| (surface, surf_loc + loc))
                })
        {
            under = Some((focus, location));
        } else if let Some(layer) = layers
            .layer_under(WlrLayer::Bottom, pos)
            .or_else(|| layers.layer_under(WlrLayer::Background, pos))
        {
            let layer_loc = layers.layer_geometry(layer).unwrap().loc;
            under = Some((layer.clone().into(), output_geo.loc + layer_loc));
        };
        under.map(|(s, l)| (s, l.to_f64()))
    }

    pub(crate) fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        let horizontal_amount = evt.amount(input::Axis::Horizontal).unwrap_or_else(|| {
            evt.amount_v120(input::Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
        });
        let vertical_amount = evt
            .amount(input::Axis::Vertical)
            .unwrap_or_else(|| evt.amount_v120(input::Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
        let horizontal_amount_discrete = evt.amount_v120(input::Axis::Horizontal);
        let vertical_amount_discrete = evt.amount_v120(input::Axis::Vertical);

        {
            let mut frame = AxisFrame::new(evt.time_msec()).source(evt.source());
            if horizontal_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Horizontal, evt.relative_direction(Axis::Horizontal));
                frame = frame.value(Axis::Horizontal, horizontal_amount);
                if let Some(discrete) = horizontal_amount_discrete {
                    frame = frame.v120(Axis::Horizontal, discrete as i32);
                }
            }
            if vertical_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Vertical, evt.relative_direction(Axis::Vertical));
                frame = frame.value(Axis::Vertical, vertical_amount);
                if let Some(discrete) = vertical_amount_discrete {
                    frame = frame.v120(Axis::Vertical, discrete as i32);
                }
            }
            if evt.source() == AxisSource::Finger {
                if evt.amount(Axis::Horizontal) == Some(0.0) {
                    frame = frame.stop(Axis::Horizontal);
                }
                if evt.amount(Axis::Vertical) == Some(0.0) {
                    frame = frame.stop(Axis::Vertical);
                }
            }
            let pointer = self.pointer.clone();
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }
}

#[cfg(any(feature = "winit", feature = "x11"))]
impl<Backend: crate::state::Backend> Otto<Backend> {
    pub(crate) fn on_pointer_move_absolute_windowed<B: InputBackend>(
        &mut self,
        evt: B::PointerMotionAbsoluteEvent,
        output: &Output,
    ) {
        let output_geo = self.workspaces.output_geometry(output).unwrap();

        let pos = evt.position_transformed(output_geo.size) + output_geo.loc.to_f64();
        let serial = SCOUNTER.next_serial();

        let under = self.surface_under(pos);
        let pointer = self.pointer.clone();

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        let scale = output.current_scale().fractional_scale();
        let pos = pos.to_physical(scale);
        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);
    }
}

#[cfg(feature = "udev")]
impl crate::Otto<crate::udev::UdevData> {
    pub(crate) fn on_pointer_move<B: InputBackend>(
        &mut self,
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        evt: B::PointerMotionEvent,
    ) {
        let mut pointer_location = self.pointer.current_location();
        let current_scale = self
            .workspaces
            .outputs()
            .find(|o| {
                self.workspaces
                    .output_geometry(o)
                    .map(|geo| geo.contains(pointer_location.to_i32_round()))
                    .unwrap_or(false)
            })
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0);
        let logical_delta = {
            let p = evt.delta();
            Point::from((p.x / current_scale, p.y / current_scale))
        };
        let logical_delta_unaccel = {
            let p = evt.delta_unaccel();
            Point::from((p.x / current_scale, p.y / current_scale))
        };
        let serial = SCOUNTER.next_serial();

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);

        let mut pointer_locked = false;
        let mut pointer_confined = false;
        let mut confine_region = None;
        if let Some((surface, surface_loc)) = under
            .as_ref()
            .and_then(|(target, l)| Some((target.wl_surface()?, l)))
        {
            with_pointer_constraint(&surface, &pointer, |constraint| match constraint {
                Some(constraint) if constraint.is_active() => {
                    // Constraint does not apply if not within region
                    if !constraint.region().is_none_or(|x| {
                        x.contains((pointer_location - *surface_loc).to_i32_round())
                    }) {
                        return;
                    }
                    match &*constraint {
                        PointerConstraint::Locked(_locked) => {
                            pointer_locked = true;
                        }
                        PointerConstraint::Confined(confine) => {
                            pointer_confined = true;
                            confine_region = confine.region().cloned();
                        }
                    }
                }
                _ => {}
            });
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta: logical_delta,
                delta_unaccel: logical_delta_unaccel,
                utime: evt.time(),
            },
        );

        // If pointer is locked, only emit relative motion
        if pointer_locked {
            pointer.frame(self);
            return;
        }

        pointer_location += logical_delta;

        // clamp to screen limits
        // this event is never generated by winit
        pointer_location = self.clamp_coords(pointer_location);

        let new_under = self.surface_under(pointer_location);

        // If confined, don't move pointer if it would go outside surface or region
        if pointer_confined {
            if let Some((surface, surface_loc)) = &under {
                if new_under.as_ref().and_then(|(under, _)| under.wl_surface())
                    != surface.wl_surface()
                {
                    pointer.frame(self);
                    return;
                }
                if let Some(region) = confine_region {
                    if !region.contains((pointer_location - *surface_loc).to_i32_round()) {
                        pointer.frame(self);
                        return;
                    }
                }
            }
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        let scale = Config::with(|c| c.screen_scale);
        let pos = pointer_location.to_physical(scale);

        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);

        // Schedule a redraw to update the cursor position
        self.schedule_event_loop_dispatch();

        // If pointer is now in a constraint region, activate it
        if let Some((under, surface_location)) =
            new_under.and_then(|(target, loc)| Some((target.wl_surface()?.into_owned(), loc)))
        {
            with_pointer_constraint(&under, &pointer, |constraint| match constraint {
                Some(constraint) if !constraint.is_active() => {
                    let point = (pointer_location - surface_location).to_i32_round();
                    if constraint
                        .region()
                        .is_none_or(|region| region.contains(point))
                    {
                        constraint.activate();
                    }
                }
                _ => {}
            });
        }
    }

    pub(crate) fn on_pointer_move_absolute<B: InputBackend>(
        &mut self,
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        evt: B::PointerMotionAbsoluteEvent,
    ) {
        let serial = SCOUNTER.next_serial();
        let max_x = self.workspaces.outputs().fold(0, |acc, o| {
            acc + self.workspaces.output_geometry(o).unwrap().size.w
        });

        let max_h_output = self
            .workspaces
            .outputs()
            .max_by_key(|o| self.workspaces.output_geometry(o).unwrap().size.h)
            .unwrap()
            .clone();

        let max_y = self
            .workspaces
            .output_geometry(&max_h_output)
            .unwrap()
            .size
            .h;

        let mut pointer_location = (evt.x_transformed(max_x), evt.y_transformed(max_y)).into();

        // clamp to screen limits
        pointer_location = self.clamp_coords(pointer_location);

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        let scale = Config::with(|c| c.screen_scale);
        let pos = pointer_location.to_physical(scale);

        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);

        // Schedule a redraw to update the cursor position
        self.schedule_event_loop_dispatch();
    }

    pub(crate) fn clamp_coords(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.workspaces.outputs().next().is_none() {
            return pos;
        }

        let (pos_x, pos_y) = pos.into();
        let max_x = self.workspaces.outputs().fold(0, |acc, o| {
            acc + self.workspaces.output_geometry(o).unwrap().size.w
        });
        let clamped_x = pos_x.clamp(0.0, max_x as f64);
        let max_y = self
            .workspaces
            .outputs()
            .find(|o| {
                let geo = self.workspaces.output_geometry(o).unwrap();
                geo.contains((clamped_x as i32, 0))
            })
            .map(|o| self.workspaces.output_geometry(o).unwrap().size.h);

        if let Some(max_y) = max_y {
            let clamped_y = pos_y.clamp(0.0, max_y as f64);
            (clamped_x, clamped_y).into()
        } else {
            (clamped_x, pos_y).into()
        }
    }
}
