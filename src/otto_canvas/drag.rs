//! What a drag and drop operation means to the side canvas, as plain
//! decisions the compositor state applies.
//!
//! As soon as a drag starts the canvas peeks out a little, to say a drop can
//! go there. From the middle of the output on, it follows the pointer
//! further out the further right the pointer goes ([`PeekFollow`]). Turning
//! back left takes it in to where it started, and it stops following: the
//! user does not want it. Heading right again nudges it out, and going on
//! makes it follow once more. Reaching the last stretch of the output opens
//! it either way, so an item can take the drop ([`in_open_zone`],
//! [`EdgeDwell`]). When the drag ends the canvas goes again, unless it was
//! open before the drag or the drop landed on an item
//! ([`hide_when_drag_ends`]). A press outside a shown canvas reaches what is
//! under it, and the canvas hides when that button comes up, unless the
//! press started a drag ([`hide_on_release`]).

// Rust guideline compliant 2026-02-21

use std::time::{Duration, Instant};

/// Where across an output, as a fraction of its width from the left, the
/// peeking canvas starts to follow the pointer.
pub const FOLLOW_FROM: f64 = 0.5;

/// How much of an output's width, measured in from its right edge, opens
/// the canvas.
pub const OPEN_FRACTION: f64 = 0.1;

/// How much of the column is on screen while the canvas peeks during a
/// drag, from 0 (none) to 1 (all of it, where it rests when shown).
pub const PEEK_MIN: f64 = 0.1;

/// How much of the column is on screen when the pointer reaches the stretch
/// that opens it: all of it, so a drag heading there finds the canvas out.
pub const PEEK_MAX: f64 = 1.0;

/// How far the pointer goes back left, as a fraction of the output's width,
/// before a following canvas takes it as turning away; and how far right
/// again, after that, before it follows once more. Well past the drift of a
/// touchpad or a hand correcting its aim.
pub const TURN_FRACTION: f64 = 0.08;

/// How much further out than [`PEEK_MIN`] a turned-away canvas comes as the
/// pointer heads right again, at most, just before it follows once more: a
/// hint that going on brings it back.
pub const RESUME_NUDGE: f64 = 0.05;

/// How long a drag rests in the stretch that opens the canvas before it
/// does. Short, but enough that a drag on its way to an output further
/// right passes through.
pub const OPEN_DWELL: Duration = Duration::from_millis(150);

/// How often the pointer is looked at while a drag goes on.
pub const POLL: Duration = Duration::from_millis(50);

/// Whether a pointer at `x` is in the stretch that opens the canvas, on an
/// output whose left edge is at `output_left` and `output_width` wide, all
/// in global logical points.
pub fn in_open_zone(x: f64, output_left: f64, output_width: f64) -> bool {
    x >= output_left + output_width * (1.0 - OPEN_FRACTION)
}

/// What a look at the pointer changed about following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowChange {
    /// Nothing: the canvas goes on as it was.
    None,
    /// The pointer turned back: the canvas goes in to [`PEEK_MIN`] and stops
    /// following.
    TurnedAway,
    /// The pointer headed right again after turning back: the canvas follows
    /// once more.
    Resumed,
}

/// How far out a peeking canvas is as the pointer moves.
///
/// [`PEEK_MIN`] left of the middle of the output, growing from there to
/// [`PEEK_MAX`] where the stretch that opens the canvas starts. The pointer
/// going back left by [`TURN_FRACTION`] of the output from the furthest right
/// it has been, past the middle, turns it away: it stays near [`PEEK_MIN`],
/// nudged out by up to [`RESUME_NUDGE`] as the pointer heads right again,
/// and follows once more when the pointer has gone as far right as it went
/// back.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PeekFollow {
    /// The furthest right the pointer has been while followed.
    furthest: Option<f64>,
    /// The furthest left the pointer has been since it turned away, while
    /// turned away.
    lowest: Option<f64>,
}

impl PeekFollow {
    /// How much of the column is on screen for a pointer at `x` on an
    /// output whose left edge is at `output_left` and `output_width` wide,
    /// and what changed.
    pub fn observe(&mut self, x: f64, output_left: f64, output_width: f64) -> (f64, FollowChange) {
        let from = output_left + output_width * FOLLOW_FROM;
        let to = output_left + output_width * (1.0 - OPEN_FRACTION);
        let turn = output_width * TURN_FRACTION;
        let depth = ((x - from) / (to - from).max(1.0)).clamp(0.0, 1.0);
        let following = PEEK_MIN + (PEEK_MAX - PEEK_MIN) * depth;

        if let Some(lowest) = self.lowest {
            let lowest = lowest.min(x);
            let back = x - lowest;
            if back >= turn && following > PEEK_MIN {
                self.lowest = None;
                self.furthest = Some(x);
                return (following, FollowChange::Resumed);
            }
            self.lowest = Some(lowest);
            let nudge = PEEK_MIN + RESUME_NUDGE * (back / turn).clamp(0.0, 1.0);
            return (nudge, FollowChange::None);
        }

        let furthest = self.furthest.map_or(x, |f| f.max(x));
        if furthest > from && furthest - x >= turn {
            self.furthest = None;
            self.lowest = Some(x);
            return (PEEK_MIN, FollowChange::TurnedAway);
        }
        self.furthest = Some(furthest);
        (following, FollowChange::None)
    }

    /// Whether the pointer has turned away and not headed back since.
    pub fn turned_away(&self) -> bool {
        self.lowest.is_some()
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
    /// The pointer's rest in the stretch that opens the canvas.
    pub open_dwell: EdgeDwell,
    /// How far out the canvas is while it peeks.
    pub follow: PeekFollow,
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

    const DWELL: Duration = OPEN_DWELL;

    const LEFT: f64 = 1920.0;
    const WIDTH: f64 = 1440.0;
    const FROM: f64 = LEFT + WIDTH * FOLLOW_FROM;
    const OPEN: f64 = LEFT + WIDTH * (1.0 - OPEN_FRACTION);

    #[test]
    fn the_last_tenth_of_the_output_opens_the_canvas() {
        assert!(in_open_zone(LEFT + WIDTH, LEFT, WIDTH));
        assert!(in_open_zone(OPEN, LEFT, WIDTH));
        assert!(!in_open_zone(OPEN - 1.0, LEFT, WIDTH));
        assert!(!in_open_zone(LEFT, LEFT, WIDTH));
    }

    const TURN: f64 = WIDTH * TURN_FRACTION;

    #[test]
    fn the_peek_follows_from_the_middle_to_the_opening_stretch() {
        let mut follow = PeekFollow::default();
        let none = FollowChange::None;
        assert_eq!(follow.observe(LEFT, LEFT, WIDTH), (PEEK_MIN, none));
        assert_eq!(follow.observe(FROM, LEFT, WIDTH), (PEEK_MIN, none));
        let (mid, _) = follow.observe((FROM + OPEN) / 2.0, LEFT, WIDTH);
        assert!((mid - (PEEK_MIN + PEEK_MAX) / 2.0).abs() < 1e-9);
        assert_eq!(follow.observe(OPEN, LEFT, WIDTH), (PEEK_MAX, none));
        assert_eq!(follow.observe(LEFT + WIDTH, LEFT, WIDTH), (PEEK_MAX, none));
    }

    #[test]
    fn going_back_left_while_following_turns_it_away() {
        let mut follow = PeekFollow::default();
        follow.observe(OPEN - 10.0, LEFT, WIDTH);
        let (_, change) = follow.observe(OPEN - 10.0 - TURN / 2.0, LEFT, WIDTH);
        assert_eq!(
            change,
            FollowChange::None,
            "correcting aim is not turning back"
        );
        assert_eq!(
            follow.observe(OPEN - 11.0 - TURN, LEFT, WIDTH),
            (PEEK_MIN, FollowChange::TurnedAway)
        );
        assert!(follow.turned_away());
    }

    #[test]
    fn heading_right_again_nudges_then_follows_once_more() {
        let mut follow = PeekFollow::default();
        let low = OPEN - 11.0 - TURN;
        follow.observe(OPEN - 10.0, LEFT, WIDTH);
        follow.observe(low, LEFT, WIDTH);
        let (nudged, change) = follow.observe(low + TURN / 2.0, LEFT, WIDTH);
        assert_eq!(change, FollowChange::None);
        assert!(nudged > PEEK_MIN && nudged <= PEEK_MIN + RESUME_NUDGE);
        let (back, change) = follow.observe(low + TURN + 1.0, LEFT, WIDTH);
        assert_eq!(change, FollowChange::Resumed);
        assert!(back > PEEK_MIN + RESUME_NUDGE);
        assert!(!follow.turned_away());
    }

    #[test]
    fn going_left_before_the_middle_is_not_turning_back() {
        let mut follow = PeekFollow::default();
        follow.observe(FROM - 10.0, LEFT, WIDTH);
        let (_, change) = follow.observe(LEFT, LEFT, WIDTH);
        assert_eq!(change, FollowChange::None);
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
