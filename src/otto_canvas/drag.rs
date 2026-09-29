//! What a drag and drop operation means to the side canvas, as plain
//! decisions the compositor state applies.
//!
//! A drag that rests at the right edge of an output opens the canvas, so an
//! item can take the drop ([`EdgeDwell`]). When the drag ends the canvas
//! goes again, unless it was open before the drag or the drop landed on an
//! item ([`hide_when_drag_ends`]). A press outside a shown canvas reaches
//! what is under it, and the canvas hides when that button comes up, unless
//! the press started a drag ([`hide_on_release`]).

// Rust guideline compliant 2026-02-21

use std::time::{Duration, Instant};

/// How much of the canvas width, measured in from the right edge of an
/// output, counts as the edge for a drag.
///
/// A strip rather than the last point: a drag on a touchpad drifts, and the
/// canvas should answer a drag taken over to the right side, not only one
/// pinned against the edge. The whole strip the column slides into counts.
pub const EDGE_FRACTION: f64 = 1.0;

/// How long a drag rests at the edge before the canvas opens.
///
/// Long enough that a drag on its way to an output further right, or to a
/// window near the edge, passes through without opening it, short enough not
/// to feel like waiting.
pub const DWELL: Duration = Duration::from_millis(250);

/// How often the pointer is looked at while a drag goes on.
pub const POLL: Duration = Duration::from_millis(50);

/// Whether a pointer at `x` is at the right edge of an output whose right
/// edge is at `output_right`, both in global logical points.
pub fn at_right_edge(x: f64, output_right: f64, canvas_width: f64) -> bool {
    x >= output_right - canvas_width * EDGE_FRACTION
}

/// Times how long the pointer has rested at the edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeDwell {
    /// When the pointer reached the edge, while it stays there.
    since: Option<Instant>,
    /// The current rest has already opened the canvas.
    fired: bool,
}

impl EdgeDwell {
    /// Record where the pointer is at `now`. Returns true once per rest at
    /// the edge, when it has lasted [`DWELL`]. Leaving the edge starts over.
    pub fn observe(&mut self, at_edge: bool, now: Instant) -> bool {
        if !at_edge {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if self.fired || now.duration_since(since) < DWELL {
            return false;
        }
        self.fired = true;
        true
    }

    /// Allow the current rest to fire again, for a canvas that could not
    /// open when it first fired.
    pub fn rearm(&mut self) {
        self.fired = false;
    }
}

/// One drag and drop operation, from start to drop or cancel.
#[derive(Debug, Clone, Default)]
pub struct DragSession {
    /// The mime types the drag's data is offered as.
    pub mime_types: Vec<String>,
    /// The drag opened the canvas, which was hidden when it started.
    pub opened_canvas: bool,
    /// The pointer's rest at the edge.
    pub dwell: EdgeDwell,
}

/// Whether the canvas hides as a drag ends: only when the drag opened it
/// and the drop did not land on one of its items.
pub fn hide_when_drag_ends(opened_canvas: bool, landed_on_item: bool) -> bool {
    opened_canvas && !landed_on_item
}

/// A button pressed outside a shown canvas, waiting for its release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressOutside {
    pub button: u32,
    /// A drag and drop operation started while the button was down.
    pub dragged: bool,
}

/// Whether the canvas hides when the button pressed outside it comes up:
/// a plain click outside does, a press that started a drag does not, nor
/// one released over the canvas.
pub fn hide_on_release(press: PressOutside, released_inside: bool) -> bool {
    !press.dragged && !released_inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_is_the_last_points_of_the_output() {
        assert!(at_right_edge(1920.0, 1920.0, 400.0));
        assert!(at_right_edge(1520.0, 1920.0, 400.0));
        assert!(!at_right_edge(1519.0, 1920.0, 400.0));
        assert!(!at_right_edge(100.0, 1920.0, 400.0));
    }

    #[test]
    fn a_rest_at_the_edge_fires_once_after_the_dwell() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        assert!(!dwell.observe(true, start));
        assert!(!dwell.observe(true, start + DWELL / 2));
        assert!(dwell.observe(true, start + DWELL));
        assert!(
            !dwell.observe(true, start + DWELL * 3),
            "only once per rest"
        );
    }

    #[test]
    fn leaving_the_edge_starts_over() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        dwell.observe(true, start);
        dwell.observe(false, start + DWELL / 2);
        assert!(!dwell.observe(true, start + DWELL));
        assert!(dwell.observe(true, start + DWELL * 2));
        dwell.observe(false, start + DWELL * 3);
        dwell.observe(true, start + DWELL * 4);
        assert!(
            dwell.observe(true, start + DWELL * 5),
            "a new rest fires again"
        );
    }

    #[test]
    fn a_rearmed_rest_fires_again() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        dwell.observe(true, start);
        assert!(dwell.observe(true, start + DWELL));
        dwell.rearm();
        assert!(dwell.observe(true, start + DWELL + POLL));
    }

    #[test]
    fn only_a_canvas_the_drag_opened_hides_and_only_when_no_item_took_the_drop() {
        assert!(hide_when_drag_ends(true, false));
        assert!(!hide_when_drag_ends(true, true));
        assert!(!hide_when_drag_ends(false, false));
        assert!(!hide_when_drag_ends(false, true));
    }

    #[test]
    fn a_plain_click_outside_hides_and_a_drag_does_not() {
        let click = PressOutside {
            button: 0x110,
            dragged: false,
        };
        let drag = PressOutside {
            dragged: true,
            ..click
        };
        assert!(hide_on_release(click, false));
        assert!(!hide_on_release(click, true));
        assert!(!hide_on_release(drag, false));
        assert!(!hide_on_release(drag, true));
    }
}
