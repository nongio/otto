//! What a drag and drop operation means to the side canvas, as plain
//! decisions the compositor state applies.
//!
//! A drag that rests near the right edge of an output brings the canvas out
//! a little, following the pointer, to say a drop can go there
//! ([`PeekFollow`]). Turning back left takes it in again to where it
//! started, and it stays there for the rest of the drag: the user does not
//! want it. Only a drag that rests close to the
//! edge opens it, so an item can take the drop ([`EdgeZone`], [`EdgeDwell`]). When the drag ends the canvas
//! goes again, unless it was open before the drag or the drop landed on an
//! item ([`hide_when_drag_ends`]). A press outside a shown canvas reaches
//! what is under it, and the canvas hides when that button comes up, unless
//! the press started a drag ([`hide_on_release`]).

// Rust guideline compliant 2026-02-21

use std::time::{Duration, Instant};

/// How much of the canvas width, measured in from the right edge of an
/// output, counts as near the edge for a drag: there the canvas peeks out
/// and follows the pointer.
///
/// Wider than the column, so the canvas starts to come out while the drag is
/// still on its way, and has time to open before the pointer reaches the
/// edge.
pub const EDGE_FRACTION: f64 = 1.5;

/// How much of the canvas width, measured in from the right edge of an
/// output, counts as close to the edge: there the canvas opens. Half the
/// column, so it opens before the pointer is pinned against the edge.
pub const OPEN_FRACTION: f64 = 0.5;

/// How far out a peeking canvas is when the pointer has just come near the
/// edge, from 0 (hidden) to 1 (shown).
pub const PEEK_MIN: f64 = 0.1;

/// How far out a peeking canvas is with the pointer close to the edge, just
/// before it opens. Far enough to see the items, not so far that they are
/// in the way of a drag meant for a window near the edge.
pub const PEEK_MAX: f64 = 0.4;

/// How far the pointer goes back left, in logical points, before a peeking
/// canvas takes it as turning away. Enough to ride out the drift of a
/// touchpad: turning away holds for the rest of the drag.
pub const RETREAT_POINTS: f64 = 24.0;

/// How long a drag rests near the edge before the canvas peeks out.
///
/// Long enough that a drag on its way to an output further right, or to a
/// window near the edge, passes through without opening it, short enough not
/// to feel like waiting.
pub const DWELL: Duration = Duration::from_millis(120);

/// How long a drag rests close to the edge before the canvas opens. Longer
/// than [`DWELL`]: opening covers what is under the column, so the drag has
/// to mean it.
pub const OPEN_DWELL: Duration = Duration::from_millis(200);

/// How often the pointer is looked at while a drag goes on.
pub const POLL: Duration = Duration::from_millis(50);

/// Where a pointer is relative to the right edge of an output, during a
/// drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeZone {
    /// Away from the edge.
    Away,
    /// Near the edge: the canvas peeks out.
    Near,
    /// Close to the edge: the canvas opens.
    Close,
}

/// The zone of a pointer at `x` on an output whose right edge is at
/// `output_right`, both in global logical points.
pub fn edge_zone(x: f64, output_right: f64, canvas_width: f64) -> EdgeZone {
    if x >= output_right - canvas_width * OPEN_FRACTION {
        EdgeZone::Close
    } else if x >= output_right - canvas_width * EDGE_FRACTION {
        EdgeZone::Near
    } else {
        EdgeZone::Away
    }
}

/// How far out a peeking canvas is as the pointer moves, and when it turns
/// back.
///
/// It grows from [`PEEK_MIN`] where the near strip starts to [`PEEK_MAX`]
/// where the close one does. The pointer going back left by
/// [`RETREAT_POINTS`] from the furthest right it has been is turning back.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PeekFollow {
    /// The furthest right the pointer has been during this peek.
    furthest: Option<f64>,
}

impl PeekFollow {
    /// The progress for a pointer at `x` on an output whose right edge is
    /// at `output_right`, and whether the pointer has turned back.
    pub fn observe(&mut self, x: f64, output_right: f64, canvas_width: f64) -> (f64, bool) {
        let near = output_right - canvas_width * EDGE_FRACTION;
        let close = output_right - canvas_width * OPEN_FRACTION;
        let furthest = self.furthest.map_or(x, |f| f.max(x));
        self.furthest = Some(furthest);
        let depth = ((x - near) / (close - near).max(1.0)).clamp(0.0, 1.0);
        (
            PEEK_MIN + (PEEK_MAX - PEEK_MIN) * depth,
            furthest - x >= RETREAT_POINTS,
        )
    }

    /// Forget the pointer, for a new peek.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
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
    /// the edge, when it has lasted `wait`. Leaving the edge starts over.
    pub fn observe(&mut self, at_edge: bool, now: Instant, wait: Duration) -> bool {
        if !at_edge {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if self.fired || now.duration_since(since) < wait {
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
    /// The pointer's rest near the edge, which makes the canvas peek.
    pub peek_dwell: EdgeDwell,
    /// The pointer's rest close to the edge, which opens the canvas.
    pub open_dwell: EdgeDwell,
    /// How far out the canvas is while it peeks.
    pub follow: PeekFollow,
    /// The pointer turned back while the canvas peeked: it neither follows,
    /// nor opens, nor peeks again until the drag ends.
    pub dismissed: bool,
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

    const RIGHT: f64 = 1920.0;
    const WIDTH: f64 = 400.0;
    const NEAR: f64 = RIGHT - WIDTH * EDGE_FRACTION;
    const CLOSE: f64 = RIGHT - WIDTH * OPEN_FRACTION;

    #[test]
    fn near_starts_before_the_column_and_close_is_its_outer_half() {
        assert_eq!(edge_zone(RIGHT, RIGHT, WIDTH), EdgeZone::Close);
        assert_eq!(edge_zone(CLOSE, RIGHT, WIDTH), EdgeZone::Close);
        assert_eq!(edge_zone(CLOSE - 1.0, RIGHT, WIDTH), EdgeZone::Near);
        assert_eq!(edge_zone(RIGHT - WIDTH, RIGHT, WIDTH), EdgeZone::Near);
        assert_eq!(edge_zone(NEAR, RIGHT, WIDTH), EdgeZone::Near);
        assert_eq!(edge_zone(NEAR - 1.0, RIGHT, WIDTH), EdgeZone::Away);
        assert_eq!(edge_zone(100.0, RIGHT, WIDTH), EdgeZone::Away);
    }

    #[test]
    fn the_peek_grows_from_min_to_max_across_the_near_strip() {
        let mut follow = PeekFollow::default();
        assert_eq!(follow.observe(NEAR, RIGHT, WIDTH), (PEEK_MIN, false));
        let (mid, _) = follow.observe((NEAR + CLOSE) / 2.0, RIGHT, WIDTH);
        assert!((mid - (PEEK_MIN + PEEK_MAX) / 2.0).abs() < 1e-9);
        assert_eq!(follow.observe(CLOSE, RIGHT, WIDTH), (PEEK_MAX, false));
        assert_eq!(follow.observe(RIGHT, RIGHT, WIDTH), (PEEK_MAX, false));
    }

    #[test]
    fn going_back_left_past_the_drift_is_turning_back() {
        let mut follow = PeekFollow::default();
        follow.observe(NEAR, RIGHT, WIDTH);
        follow.observe(CLOSE, RIGHT, WIDTH);
        let (_, turned) = follow.observe(CLOSE - RETREAT_POINTS / 2.0, RIGHT, WIDTH);
        assert!(!turned, "a little drift is not turning back");
        let (_, turned) = follow.observe(CLOSE - RETREAT_POINTS, RIGHT, WIDTH);
        assert!(turned);
        follow.reset();
        let (_, turned) = follow.observe(NEAR, RIGHT, WIDTH);
        assert!(!turned, "a new peek starts over");
    }

    #[test]
    fn a_rest_at_the_edge_fires_once_after_the_dwell() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        assert!(!dwell.observe(true, start, DWELL));
        assert!(!dwell.observe(true, start + DWELL / 2, DWELL));
        assert!(dwell.observe(true, start + DWELL, DWELL));
        assert!(
            !dwell.observe(true, start + DWELL * 3, DWELL),
            "only once per rest"
        );
    }

    #[test]
    fn leaving_the_edge_starts_over() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        dwell.observe(true, start, DWELL);
        dwell.observe(false, start + DWELL / 2, DWELL);
        assert!(!dwell.observe(true, start + DWELL, DWELL));
        assert!(dwell.observe(true, start + DWELL * 2, DWELL));
        dwell.observe(false, start + DWELL * 3, DWELL);
        dwell.observe(true, start + DWELL * 4, DWELL);
        assert!(
            dwell.observe(true, start + DWELL * 5, DWELL),
            "a new rest fires again"
        );
    }

    #[test]
    fn a_rearmed_rest_fires_again() {
        let start = Instant::now();
        let mut dwell = EdgeDwell::default();
        dwell.observe(true, start, DWELL);
        assert!(dwell.observe(true, start + DWELL, DWELL));
        dwell.rearm();
        assert!(dwell.observe(true, start + DWELL + POLL, DWELL));
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
