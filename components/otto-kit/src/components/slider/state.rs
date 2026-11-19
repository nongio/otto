use skia_safe::Rect;

use super::slider::{hit_test_knob, hit_test_track, value_at};

/// What the host should do after a pointer event reaches [`SliderDrag`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SliderResponse {
    /// The event was not for this slider — let the host handle it.
    Ignored,
    /// The value changed: the host should update its model and redraw.
    Changed(f32),
    /// No value change, but the interaction state did (drag started or
    /// ended) — redraw so the knob's pressed look stays in sync.
    Redraw,
}

/// Tracks a pointer drag on a slider: press on the knob or on the track to
/// jump, drag, release. The value itself stays with the host — this only
/// knows whether a drag is in progress and turns pointer positions into the
/// values [`super::slider::value_at`] maps them to.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SliderDrag {
    dragging: bool,
}

impl SliderDrag {
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Pointer pressed at `(x, y)`. Starts a drag on a hit against the knob
    /// *or* anywhere on the track — a track click jumps straight to that
    /// value, the same click-to-jump behaviour as a native slider.
    #[allow(clippy::too_many_arguments)]
    pub fn on_pointer_down(
        &mut self,
        rect: Rect,
        min: f32,
        max: f32,
        step: Option<f32>,
        value: f32,
        x: f32,
        y: f32,
    ) -> SliderResponse {
        if !hit_test_knob(rect, value, min, max, x, y) && !hit_test_track(rect, x, y) {
            return SliderResponse::Ignored;
        }
        self.dragging = true;
        let new_value = value_at(rect, min, max, step, x);
        if new_value != value {
            SliderResponse::Changed(new_value)
        } else {
            SliderResponse::Redraw
        }
    }

    /// Pointer moved to `x` while the button is held.
    pub fn on_pointer_drag(
        &mut self,
        rect: Rect,
        min: f32,
        max: f32,
        step: Option<f32>,
        value: f32,
        x: f32,
    ) -> SliderResponse {
        if !self.dragging {
            return SliderResponse::Ignored;
        }
        let new_value = value_at(rect, min, max, step, x);
        if new_value != value {
            SliderResponse::Changed(new_value)
        } else {
            SliderResponse::Ignored
        }
    }

    /// Pointer released.
    pub fn on_pointer_up(&mut self) -> SliderResponse {
        if self.dragging {
            self.dragging = false;
            SliderResponse::Redraw
        } else {
            SliderResponse::Ignored
        }
    }
}

/// How far a touchpad travels for one step, in the pointer's units.
const STEP_DISTANCE: f64 = 15.0;

/// Wheel notches and touchpad travel, counted out in whole steps: one per
/// notch, one per [`STEP_DISTANCE`] of a two-finger scroll, with what is
/// left kept for the next event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollSteps {
    /// Wheel travel not yet a step, in 120ths of a notch.
    wheel: i32,
    /// Touchpad travel not yet a step.
    travel: f64,
}

impl ScrollSteps {
    /// Nothing counted yet.
    pub const fn new() -> Self {
        Self {
            wheel: 0,
            travel: 0.0,
        }
    }

    /// Add one axis event: `value120` from a wheel (120 a notch), else
    /// `absolute` travel. Returns the whole steps it completed, positive
    /// down, as the axis runs.
    pub fn add(&mut self, value120: i32, absolute: f64) -> i32 {
        if value120 != 0 {
            self.travel = 0.0;
            self.wheel += value120;
            let steps = self.wheel / 120;
            self.wheel -= steps * 120;
            return steps;
        }
        self.travel += absolute;
        let steps = (self.travel / STEP_DISTANCE).trunc();
        self.travel -= steps * STEP_DISTANCE;
        steps as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notch_is_a_step_and_a_swipe_is_counted_out() {
        let mut steps = ScrollSteps::default();
        assert_eq!(steps.add(120, 15.0), 1);
        assert_eq!(steps.add(60, 7.5), 0);
        assert_eq!(steps.add(60, 7.5), 1);
        assert_eq!(steps.add(-240, -30.0), -2);
        assert_eq!(steps.add(0, 10.0), 0);
        assert_eq!(steps.add(0, 10.0), 1);
        assert_eq!(steps.add(0, -50.0), -3);
    }

    fn rect() -> Rect {
        Rect::from_xywh(0.0, 0.0, 160.0, 24.0)
    }

    #[test]
    fn press_on_track_jumps_and_starts_dragging() {
        let mut drag = SliderDrag::default();
        let r = rect();
        let response =
            drag.on_pointer_down(r, 0.0, 100.0, None, 0.0, r.left + r.width(), r.center_y());
        assert_eq!(response, SliderResponse::Changed(100.0));
        assert!(drag.is_dragging());
    }

    #[test]
    fn press_off_the_control_is_ignored() {
        let mut drag = SliderDrag::default();
        let r = rect();
        let response = drag.on_pointer_down(r, 0.0, 100.0, None, 0.0, r.left, r.bottom + 50.0);
        assert_eq!(response, SliderResponse::Ignored);
        assert!(!drag.is_dragging());
    }

    #[test]
    fn drag_without_a_prior_press_is_ignored() {
        let mut drag = SliderDrag::default();
        let r = rect();
        assert_eq!(
            drag.on_pointer_drag(r, 0.0, 100.0, None, 0.0, r.right),
            SliderResponse::Ignored
        );
    }

    #[test]
    fn drag_then_release_reports_changes_then_a_final_redraw() {
        let mut drag = SliderDrag::default();
        let r = rect();
        drag.on_pointer_down(r, 0.0, 100.0, None, 0.0, r.left, r.center_y());
        let response = drag.on_pointer_drag(r, 0.0, 100.0, None, 0.0, r.right);
        assert_eq!(response, SliderResponse::Changed(100.0));
        assert_eq!(drag.on_pointer_up(), SliderResponse::Redraw);
        assert!(!drag.is_dragging());
        assert_eq!(drag.on_pointer_up(), SliderResponse::Ignored);
    }

    #[test]
    fn stepped_drag_snaps() {
        let mut drag = SliderDrag::default();
        let r = rect();
        drag.on_pointer_down(r, 0.0, 100.0, Some(25.0), 0.0, r.left, r.center_y());
        let x = r.left + r.width() * 0.55;
        let response = drag.on_pointer_drag(r, 0.0, 100.0, Some(25.0), 0.0, x);
        assert_eq!(response, SliderResponse::Changed(50.0));
    }
}
