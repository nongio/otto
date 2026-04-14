#[cfg(feature = "udev")]
use smithay::{
    backend::input::{
        Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _,
        GestureSwipeUpdateEvent as _, InputBackend,
    },
    input::pointer::{
        GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
        GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent,
    },
    utils::SERIAL_COUNTER as SCOUNTER,
};

#[cfg(feature = "udev")]
impl crate::Otto<crate::udev::UdevData> {
    pub(crate) fn on_gesture_swipe_begin<B: InputBackend>(
        &mut self,
        evt: B::GestureSwipeBeginEvent,
    ) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();

        // 3-finger swipe: start detecting direction (but not if show desktop is active)
        let is_show_desktop_active = self.workspaces.get_show_desktop();
        if evt.fingers() == 3 && !self.is_pinching && !is_show_desktop_active {
            self.gesture_swipe_begin_3finger();
        }

        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    pub(crate) fn on_gesture_swipe_update<B: InputBackend>(
        &mut self,
        evt: B::GestureSwipeUpdateEvent,
    ) {
        let pointer = self.pointer.clone();
        let delta = evt.delta();

        match &mut self.swipe_gesture {
            crate::state::SwipeGestureState::Detecting { accumulated } => {
                accumulated.0 += delta.x;
                accumulated.1 += delta.y;

                let direction = crate::state::SwipeDirection::from_accumulated(
                    accumulated.0.abs(),
                    accumulated.1.abs(),
                );

                match direction {
                    crate::state::SwipeDirection::Horizontal(_) => {
                        // Detect the output under the pointer at gesture start
                        let pointer_loc = pointer.current_location();
                        let output_name = self
                            .workspaces
                            .output_under(pointer_loc)
                            .next()
                            .map(|o| o.name())
                            .or_else(|| self.workspaces.primary_output().map(|o| o.name()))
                            .unwrap_or_default();

                        self.swipe_gesture = crate::state::SwipeGestureState::WorkspaceSwitching {
                            velocity_samples: vec![delta.x],
                            output_name: output_name.clone(),
                        };
                        self.workspaces
                            .workspace_swipe_update(&output_name, delta.x as f32);
                    }
                    crate::state::SwipeDirection::Vertical(_) => {
                        self.dismiss_all_popups();

                        if !self.workspaces.get_show_all() {
                            self.workspaces.expose_gesture_start();
                        } else {
                            self.workspaces.expose_gesture_close_start();
                        }

                        self.swipe_gesture = crate::state::SwipeGestureState::Expose {
                            velocity_samples: vec![-delta.y],
                        };
                        // Apply the current frame's delta (not accumulated)
                        let expose_delta =
                            (-delta.y / crate::state::EXPOSE_DELTA_MULTIPLIER) as f32;
                        self.workspaces.expose_update(expose_delta);
                    }
                    crate::state::SwipeDirection::Undetermined => {}
                }
            }
            crate::state::SwipeGestureState::WorkspaceSwitching {
                velocity_samples,
                output_name,
            } => {
                velocity_samples.push(delta.x);
                if velocity_samples.len() > crate::state::VELOCITY_SAMPLE_COUNT {
                    velocity_samples.remove(0);
                }
                let name = output_name.clone();
                self.workspaces
                    .workspace_swipe_update(&name, delta.x as f32);
            }
            crate::state::SwipeGestureState::Expose { velocity_samples } => {
                // Collect velocity samples for momentum-based spring animation
                velocity_samples.push(-delta.y);
                if velocity_samples.len() > crate::state::VELOCITY_SAMPLE_COUNT {
                    velocity_samples.remove(0);
                }

                let expose_delta = (-delta.y / crate::state::EXPOSE_DELTA_MULTIPLIER) as f32;
                self.workspaces.expose_update(expose_delta);
            }
            crate::state::SwipeGestureState::Idle => {}
        }

        pointer.gesture_swipe_update(
            self,
            &GestureSwipeUpdateEvent {
                time: evt.time_msec(),
                delta,
            },
        );
    }

    pub(crate) fn on_gesture_swipe_end<B: InputBackend>(&mut self, evt: B::GestureSwipeEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();

        match std::mem::replace(
            &mut self.swipe_gesture,
            crate::state::SwipeGestureState::Idle,
        ) {
            crate::state::SwipeGestureState::Expose { velocity_samples } => {
                self.gesture_swipe_end_expose(velocity_samples);
            }
            crate::state::SwipeGestureState::WorkspaceSwitching {
                velocity_samples,
                output_name,
            } => {
                self.gesture_swipe_end_workspace(velocity_samples, output_name, evt.cancelled());
            }
            _ => {}
        }

        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    pub(crate) fn on_gesture_pinch_begin<B: InputBackend>(
        &mut self,
        evt: B::GesturePinchBeginEvent,
    ) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();

        // 4-finger pinch for show desktop (don't activate if we're in a swipe gesture or expose is active)
        let is_swiping = !matches!(self.swipe_gesture, crate::state::SwipeGestureState::Idle);
        let is_expose_active = self.workspaces.get_show_all();
        if evt.fingers() == 4 && !is_swiping && !is_expose_active {
            self.is_pinching = true;
            self.pinch_last_scale = 1.0; // Reset to baseline
            self.workspaces.reset_show_desktop_gesture();
        }

        pointer.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    pub(crate) fn on_gesture_pinch_update<B: InputBackend>(
        &mut self,
        evt: B::GesturePinchUpdateEvent,
    ) {
        let pointer = self.pointer.clone();

        if self.is_pinching {
            // Scale > 1.0 = pinch out (spread fingers) = show desktop (positive delta)
            // Scale < 1.0 = pinch in (close fingers) = hide desktop (negative delta)
            let current_scale = evt.scale() as f32;
            let last_scale = self.pinch_last_scale as f32;

            // Calculate the change in scale since last event
            let scale_delta = current_scale - last_scale;

            // Pinching out (positive delta) should show desktop (positive)
            // Amplify the gesture for better sensitivity (reduced from 5.0 to 2.5)
            let delta = scale_delta * 1.5;

            self.pinch_last_scale = current_scale as f64;
            self.workspaces.expose_show_desktop(delta, false);
        }

        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent {
                time: evt.time_msec(),
                delta: evt.delta(),
                scale: evt.scale(),
                rotation: evt.rotation(),
            },
        );
    }

    pub(crate) fn on_gesture_pinch_end<B: InputBackend>(&mut self, evt: B::GesturePinchEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();

        if self.is_pinching {
            self.workspaces.expose_show_desktop(0.0, true);
            self.is_pinching = false;
        }
        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    pub(crate) fn on_gesture_hold_begin<B: InputBackend>(&mut self, evt: B::GestureHoldBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_begin(
            self,
            &GestureHoldBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    pub(crate) fn on_gesture_hold_end<B: InputBackend>(&mut self, evt: B::GestureHoldEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_end(
            self,
            &GestureHoldEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }
}

// ── Headless / test helpers ──────────────────────────────────────────
// These bypass InputBackend events and manipulate gesture state directly.
// Available for any backend (not just udev).
impl<B: crate::state::Backend> crate::Otto<B> {
    /// Simulate a 3-finger swipe begin gesture.
    pub fn gesture_swipe_begin_3finger(&mut self) {
        self.swipe_gesture = crate::state::SwipeGestureState::Detecting {
            accumulated: (0.0, 0.0),
        };
    }

    /// Simulate a swipe gesture update with raw deltas (no InputBackend needed).
    pub fn gesture_swipe_update(&mut self, dx: f64, dy: f64) {
        let delta = smithay::utils::Point::<f64, smithay::utils::Logical>::from((dx, dy));

        match &mut self.swipe_gesture {
            crate::state::SwipeGestureState::Detecting { accumulated } => {
                accumulated.0 += delta.x;
                accumulated.1 += delta.y;

                let direction = crate::state::SwipeDirection::from_accumulated(
                    accumulated.0.abs(),
                    accumulated.1.abs(),
                );

                match direction {
                    crate::state::SwipeDirection::Horizontal(_) => {
                        let pointer_loc = self.pointer.current_location();
                        let output_name = self
                            .workspaces
                            .output_under(pointer_loc)
                            .next()
                            .map(|o| o.name())
                            .or_else(|| self.workspaces.primary_output().map(|o| o.name()))
                            .unwrap_or_default();

                        self.swipe_gesture = crate::state::SwipeGestureState::WorkspaceSwitching {
                            velocity_samples: vec![delta.x],
                            output_name: output_name.clone(),
                        };
                        self.workspaces
                            .workspace_swipe_update(&output_name, delta.x as f32);
                    }
                    crate::state::SwipeDirection::Vertical(_) => {
                        self.dismiss_all_popups();
                        if !self.workspaces.get_show_all() {
                            self.workspaces.expose_gesture_start();
                        } else {
                            self.workspaces.expose_gesture_close_start();
                        }
                        self.swipe_gesture = crate::state::SwipeGestureState::Expose {
                            velocity_samples: vec![-delta.y],
                        };
                        let expose_delta =
                            (-delta.y / crate::state::EXPOSE_DELTA_MULTIPLIER) as f32;
                        self.workspaces.expose_update(expose_delta);
                    }
                    crate::state::SwipeDirection::Undetermined => {}
                }
            }
            crate::state::SwipeGestureState::WorkspaceSwitching {
                velocity_samples,
                output_name,
            } => {
                velocity_samples.push(delta.x);
                if velocity_samples.len() > crate::state::VELOCITY_SAMPLE_COUNT {
                    velocity_samples.remove(0);
                }
                let name = output_name.clone();
                self.workspaces
                    .workspace_swipe_update(&name, delta.x as f32);
            }
            crate::state::SwipeGestureState::Expose { velocity_samples } => {
                velocity_samples.push(-delta.y);
                if velocity_samples.len() > crate::state::VELOCITY_SAMPLE_COUNT {
                    velocity_samples.remove(0);
                }
                let expose_delta = (-delta.y / crate::state::EXPOSE_DELTA_MULTIPLIER) as f32;
                self.workspaces.expose_update(expose_delta);
            }
            crate::state::SwipeGestureState::Idle => {}
        }
    }

    fn gesture_swipe_end_expose(&mut self, velocity_samples: Vec<f64>) {
        let velocity = if velocity_samples.is_empty() {
            0.0
        } else {
            velocity_samples.iter().sum::<f64>() / velocity_samples.len() as f64
        };
        self.expose_end_with_velocity_and_focus_top(velocity as f32);
    }

    fn gesture_swipe_end_workspace(
        &mut self,
        velocity_samples: Vec<f64>,
        output_name: String,
        cancelled: bool,
    ) {
        let velocity = if !cancelled && !velocity_samples.is_empty() {
            velocity_samples.iter().sum::<f64>() / velocity_samples.len() as f64
        } else {
            0.0
        };
        let target_index = self
            .workspaces
            .workspace_swipe_end(&output_name, velocity as f32);
        self.focus_top_window_or_clear(target_index);
    }

    /// Simulate ending a swipe gesture (no InputBackend needed).
    pub fn gesture_swipe_end(&mut self, cancelled: bool) {
        match std::mem::replace(
            &mut self.swipe_gesture,
            crate::state::SwipeGestureState::Idle,
        ) {
            crate::state::SwipeGestureState::Expose { velocity_samples } => {
                self.gesture_swipe_end_expose(velocity_samples);
            }
            crate::state::SwipeGestureState::WorkspaceSwitching {
                velocity_samples,
                output_name,
            } => {
                self.gesture_swipe_end_workspace(velocity_samples, output_name, cancelled);
            }
            _ => {}
        }
    }

    /// Simulate a 4-finger pinch begin (no InputBackend needed).
    pub fn gesture_pinch_begin_4finger(&mut self) {
        let is_swiping = !matches!(self.swipe_gesture, crate::state::SwipeGestureState::Idle);
        let is_expose_active = self.workspaces.get_show_all();
        if !is_swiping && !is_expose_active {
            self.is_pinching = true;
            self.pinch_last_scale = 1.0;
            self.workspaces.reset_show_desktop_gesture();
        }
    }

    /// Simulate a pinch gesture update with a scale value (no InputBackend needed).
    pub fn gesture_pinch_update(&mut self, scale: f64) {
        if self.is_pinching {
            let current_scale = scale as f32;
            let last_scale = self.pinch_last_scale as f32;
            let scale_delta = current_scale - last_scale;
            let delta = scale_delta * 1.5;
            self.pinch_last_scale = scale;
            self.workspaces.expose_show_desktop(delta, false);
        }
    }

    /// Simulate ending a pinch gesture (no InputBackend needed).
    pub fn gesture_pinch_end(&mut self) {
        if self.is_pinching {
            self.workspaces.expose_show_desktop(0.0, true);
            self.is_pinching = false;
        }
    }
}

#[cfg(all(test, feature = "udev"))]
mod tests {

    #[test]
    fn test_gesture_swipe_velocity_calculation() {
        // Test velocity averaging
        let samples = [100.0, 200.0, 300.0];
        let avg = samples.iter().sum::<f64>() / samples.len() as f64;
        assert_eq!(avg, 200.0);
    }

    #[test]
    fn test_pinch_scale_delta() {
        let current = 1.5_f32;
        let last = 1.0_f32;
        let delta = current - last;
        assert_eq!(delta, 0.5);
    }
}
