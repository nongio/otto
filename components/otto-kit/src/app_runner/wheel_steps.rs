//! Wheel steps for `wl_pointer` v8 and later.
//!
//! From v8 a compositor reports a wheel as `axis_value120` (120 per notch,
//! less for a high-resolution wheel) and no longer sends `axis_discrete`.
//! Everything above the runner tells a wheel from a touchpad by
//! `AxisScroll::discrete`, so the runner fills it in from `value120` the way
//! the compositor does for an older client: it adds the fractions up and
//! reports a step once a whole notch has gone by, keeping the remainder until
//! the axis stops or the pointer enters another surface.

use smithay_client_toolkit::seat::pointer::{AxisScroll, PointerEvent, PointerEventKind};

#[derive(Default)]
pub(super) struct WheelSteps {
    horizontal: i32,
    vertical: i32,
}

impl WheelSteps {
    /// `events` with `discrete` set from `value120`, or `None` when no event
    /// changed (the frame carries no `value120`, or already has steps).
    pub(super) fn fill(&mut self, events: &[PointerEvent]) -> Option<Vec<PointerEvent>> {
        let mut filled: Option<Vec<PointerEvent>> = None;
        for (index, event) in events.iter().enumerate() {
            match &event.kind {
                PointerEventKind::Enter { .. } => *self = Self::default(),
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    let (mut new_horizontal, mut new_vertical) = (*horizontal, *vertical);
                    step(&mut self.horizontal, &mut new_horizontal);
                    step(&mut self.vertical, &mut new_vertical);
                    if (new_horizontal, new_vertical) == (*horizontal, *vertical) {
                        continue;
                    }
                    let events = filled.get_or_insert_with(|| events.to_vec());
                    if let PointerEventKind::Axis {
                        horizontal,
                        vertical,
                        ..
                    } = &mut events[index].kind
                    {
                        *horizontal = new_horizontal;
                        *vertical = new_vertical;
                    }
                }
                _ => {}
            }
        }
        filled
    }
}

fn needs_fill(axis: &AxisScroll) -> bool {
    axis.value120 != 0 && axis.discrete == 0
}

fn step(pending: &mut i32, axis: &mut AxisScroll) {
    if needs_fill(axis) {
        *pending += axis.value120;
        if pending.abs() >= 120 {
            axis.discrete = *pending / 120;
            *pending %= 120;
        }
    }
    if axis.stop {
        *pending = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(value120: i32) -> AxisScroll {
        AxisScroll {
            value120,
            ..AxisScroll::default()
        }
    }

    #[test]
    fn a_whole_notch_is_one_step() {
        let mut pending = 0;
        let mut a = axis(-120);
        step(&mut pending, &mut a);
        assert_eq!(a.discrete, -1);
        assert_eq!(pending, 0);
    }

    #[test]
    fn fractions_add_up_to_a_step() {
        let mut pending = 0;
        let steps: Vec<i32> = (0..4)
            .map(|_| {
                let mut a = axis(30);
                step(&mut pending, &mut a);
                a.discrete
            })
            .collect();
        assert_eq!(steps, [0, 0, 0, 1]);
    }

    #[test]
    fn a_compositor_step_is_kept() {
        let mut pending = 0;
        let mut a = AxisScroll {
            discrete: 2,
            value120: 240,
            ..AxisScroll::default()
        };
        step(&mut pending, &mut a);
        assert_eq!(a.discrete, 2);
        assert_eq!(pending, 0);
    }

    #[test]
    fn stop_drops_the_remainder() {
        let mut pending = 0;
        let mut a = axis(60);
        a.stop = true;
        step(&mut pending, &mut a);
        assert_eq!(a.discrete, 0);
        assert_eq!(pending, 0);
    }
}
