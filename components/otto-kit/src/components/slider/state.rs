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

#[cfg(test)]
mod tests {
    use super::*;

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
