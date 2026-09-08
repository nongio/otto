//! Swiping sideways through the category panes.
//!
//! A pane fills the card, so panning is paging rather than scrolling: the
//! panes follow the fingers while the gesture lasts and then settle onto one
//! of them, which is how Otto's workspace swipe behaves and what a full-width
//! pane wants. Free scrolling would leave two half panes on screen with
//! nothing to say which one is meant.
//!
//! The settle decision is the workspace swipe's: a flick past a speed
//! threshold moves one pane in the direction it was thrown, whatever distance
//! it covered, and anything slower goes to whichever pane is nearest.
//!
//! Every gesture has an end. A touchpad reports one when the fingers lift, but
//! not every source does, and a gesture that never ended would leave the
//! picker animating — and so repainting — for as long as it was open. So a
//! gesture with no deltas for [`GESTURE_TIMEOUT`] is finished as if the
//! fingers had lifted.

use std::time::{Duration, Instant};

/// A gesture is over once this long has passed with nothing from it, whether
/// or not the source said so.
pub const GESTURE_TIMEOUT: Duration = Duration::from_millis(120);

/// How long the settle takes.
const SETTLE: Duration = Duration::from_millis(340);

/// How far the panes travel for a given swipe, as a multiple of the raw
/// touchpad delta.
///
/// Deliberately lower than the scroll speed the rest of the desktop uses
/// (`otto_kit::components::scroll::wheel_scale`, 9): that number is tuned for
/// moving content past a viewport, and a pane fills the whole card, so the
/// same gesture that scrolls a list comfortably would fly across several
/// categories.
///
/// 2.5 was settled by swiping it on a touchpad, not derived from anything, so
/// it is a judgement rather than a number to be tidied towards the scroll
/// speed later. Override with `OTTO_EMOJI_PAN_SPEED` to taste.
pub fn pan_speed() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("OTTO_EMOJI_PAN_SPEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.5_f32)
            .clamp(0.1, 50.0)
    })
}

/// How fast a swipe has to be moving, in panes per second, to count as a
/// flick and carry to the next pane rather than settling on the nearest.
///
/// Measured in panes rather than points so it does not have to be retuned
/// whenever [`pan_speed`] or the card's width changes.
fn flick_panes_per_second() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("OTTO_EMOJI_FLICK")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.1_f32)
            .max(0.05)
    })
}

/// How much of a drag past the first or last pane is actually followed. The
/// resistance is what says "there is nothing that way" without a hard stop.
const RUBBER_BAND: f32 = 0.35;

/// The panes' horizontal position, and the gesture or settle moving it.
#[derive(Debug)]
pub struct Pan {
    /// Where the panes are, in content points: `pane_index * pane_width`.
    offset: f32,
    /// How far the panes can travel.
    max: f32,
    /// One pane's width, which is also the paging step.
    page: f32,
    gesture: Option<Gesture>,
    settle: Option<Settle>,
}

#[derive(Debug)]
struct Gesture {
    /// When the last delta arrived, so a gesture whose end is never announced
    /// can still be finished.
    last: Instant,
    /// Points per second, smoothed — a single stuttering frame should not
    /// decide where the swipe lands.
    velocity: f32,
}

#[derive(Debug)]
struct Settle {
    from: f32,
    to: f32,
    started: Instant,
}

impl Default for Pan {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Pan {
    pub fn new(page: f32) -> Self {
        Self {
            offset: 0.0,
            max: 0.0,
            page: page.max(1.0),
            gesture: None,
            settle: None,
        }
    }

    /// How wide one pane is, and how far the panes can travel in total.
    pub fn set_extent(&mut self, page: f32, max: f32) {
        self.page = page.max(1.0);
        self.max = max.max(0.0);
        if self.gesture.is_none() && self.settle.is_none() {
            self.offset = self.offset.clamp(0.0, self.max);
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// The pane the panes have come to rest on.
    pub fn pane(&self) -> usize {
        (self.offset / self.page).round().max(0.0) as usize
    }

    /// Whether anything is still moving, and the picker should keep painting.
    pub fn is_busy(&self) -> bool {
        self.gesture.is_some() || self.settle.is_some()
    }

    /// Go to a pane at once, with no animation — a tab click, a keystroke.
    pub fn jump_to_pane(&mut self, pane: usize) {
        self.gesture = None;
        self.settle = None;
        self.offset = (pane as f32 * self.page).clamp(0.0, self.max);
    }

    /// Settle onto a pane, animated.
    pub fn glide_to_pane(&mut self, pane: usize, now: Instant) {
        let to = (pane as f32 * self.page).clamp(0.0, self.max);
        self.gesture = None;
        if (to - self.offset).abs() < 0.5 {
            self.offset = to;
            self.settle = None;
            return;
        }
        self.settle = Some(Settle {
            from: self.offset,
            to,
            started: now,
        });
    }

    /// A swipe moved by `dx` points. Positive moves the panes left, revealing
    /// the next one, the way a positive scroll moves content up.
    pub fn drag(&mut self, dx: f32, now: Instant) {
        self.settle = None;
        let moved = self.offset + dx;
        // Past either end the panes follow at a fraction of the movement, so
        // the edge is felt rather than hit.
        self.offset = if moved < 0.0 {
            moved * RUBBER_BAND
        } else if moved > self.max {
            self.max + (moved - self.max) * RUBBER_BAND
        } else {
            moved
        };

        let velocity = match &self.gesture {
            Some(gesture) => {
                let elapsed = now.duration_since(gesture.last).as_secs_f32();
                if elapsed > 0.0005 {
                    // Smoothed, so one long frame cannot decide the landing.
                    gesture.velocity * 0.6 + (dx / elapsed) * 0.4
                } else {
                    gesture.velocity
                }
            }
            None => 0.0,
        };
        self.gesture = Some(Gesture {
            last: now,
            velocity,
        });
    }

    /// The fingers lifted. Settle onto a pane.
    pub fn release(&mut self, now: Instant) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        let panes = if self.page > 0.0 {
            (self.max / self.page).round() as usize
        } else {
            0
        };
        let resting = (self.offset / self.page).round().clamp(0.0, panes as f32) as usize;

        let flick = flick_panes_per_second() * self.page;
        let target = if gesture.velocity.abs() > flick {
            // Thrown: one pane the way it was thrown, from wherever it started
            // — not wherever the throw happened to reach.
            let from = (self.offset / self.page).clamp(0.0, panes as f32);
            if gesture.velocity > 0.0 {
                (from.floor() as usize + 1).min(panes)
            } else {
                (from.ceil() as usize).saturating_sub(1)
            }
        } else {
            resting
        };
        self.glide_to_pane(target, now);
    }

    /// Advance the settle, and end a gesture whose source stopped talking.
    /// Returns whether anything moved and the grid needs redrawing.
    pub fn tick(&mut self, now: Instant) -> bool {
        if let Some(gesture) = &self.gesture {
            if now.duration_since(gesture.last) >= GESTURE_TIMEOUT {
                self.release(now);
                return true;
            }
        }

        let Some(settle) = &self.settle else {
            return false;
        };
        let elapsed = now.duration_since(settle.started).as_secs_f32();
        let progress = (elapsed / SETTLE.as_secs_f32()).clamp(0.0, 1.0);
        // Ease out: quick away from the finger, gentle into place.
        let eased = 1.0 - (1.0 - progress).powi(3);
        self.offset = settle.from + (settle.to - settle.from) * eased;
        if progress >= 1.0 {
            self.offset = settle.to;
            self.settle = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: f32 = 100.0;

    fn pan() -> Pan {
        let mut pan = Pan::new(PAGE);
        // Four panes: 0, 100, 200, 300.
        pan.set_extent(PAGE, PAGE * 3.0);
        pan
    }

    #[test]
    fn a_slow_drag_settles_on_the_nearest_pane() {
        let mut pan = pan();
        let now = Instant::now();
        // Most of the way to the second pane, slowly.
        for step in 0..6 {
            pan.drag(10.0, now + Duration::from_millis(step * 50));
        }
        pan.release(now + Duration::from_millis(300));
        // Run the settle out.
        for step in 1..40 {
            pan.tick(now + Duration::from_millis(300 + step * 20));
        }
        assert_eq!(pan.pane(), 1);
        assert_eq!(pan.offset(), PAGE);
        assert!(!pan.is_busy());
    }

    #[test]
    fn a_short_drag_falls_back_to_where_it_started() {
        let mut pan = pan();
        let now = Instant::now();
        // A nudge, nowhere near half way, and slow.
        for step in 0..3 {
            pan.drag(4.0, now + Duration::from_millis(step * 60));
        }
        pan.release(now + Duration::from_millis(200));
        for step in 1..40 {
            pan.tick(now + Duration::from_millis(200 + step * 20));
        }
        assert_eq!(pan.pane(), 0, "it should fall back, not creep forward");
        assert_eq!(pan.offset(), 0.0);
    }

    #[test]
    fn a_flick_moves_one_pane_however_short_it_was() {
        let mut pan = pan();
        let now = Instant::now();
        // Barely any distance, but fast: 8 points in 5ms is 1600 points/s.
        pan.drag(8.0, now);
        pan.drag(8.0, now + Duration::from_millis(5));
        pan.drag(8.0, now + Duration::from_millis(10));
        pan.release(now + Duration::from_millis(12));
        for step in 1..40 {
            pan.tick(now + Duration::from_millis(12 + step * 20));
        }
        assert_eq!(pan.pane(), 1, "a flick should carry to the next pane");
    }

    #[test]
    fn a_flick_backwards_goes_back() {
        let mut pan = pan();
        pan.jump_to_pane(2);
        let now = Instant::now();
        for step in 0..3 {
            pan.drag(-8.0, now + Duration::from_millis(step * 5));
        }
        pan.release(now + Duration::from_millis(12));
        for step in 1..40 {
            pan.tick(now + Duration::from_millis(12 + step * 20));
        }
        assert_eq!(pan.pane(), 1);
    }

    #[test]
    fn the_ends_resist_and_come_back() {
        let mut pan = pan();
        let now = Instant::now();
        pan.drag(-100.0, now);
        assert!(
            pan.offset() > -100.0 && pan.offset() < 0.0,
            "the pull past the start should be damped, got {}",
            pan.offset()
        );
        pan.release(now + Duration::from_millis(20));
        for step in 1..40 {
            pan.tick(now + Duration::from_millis(20 + step * 20));
        }
        assert_eq!(pan.offset(), 0.0, "and spring back onto the first pane");
    }

    /// The one that matters for not freezing the desktop: a source that never
    /// says the fingers lifted must not leave the pan running for ever.
    #[test]
    fn a_gesture_that_is_never_ended_finishes_itself() {
        let mut pan = pan();
        let now = Instant::now();
        pan.drag(30.0, now);
        assert!(pan.is_busy());

        // Nothing more arrives. Well before the timeout it is still going.
        pan.tick(now + Duration::from_millis(50));
        assert!(pan.is_busy());

        // Past it, the gesture ends itself and the settle runs out.
        pan.tick(now + GESTURE_TIMEOUT);
        for step in 1..40 {
            pan.tick(now + GESTURE_TIMEOUT + Duration::from_millis(step * 20));
        }
        assert!(
            !pan.is_busy(),
            "an abandoned gesture must not keep the picker painting"
        );
    }

    #[test]
    fn jumping_is_immediate_and_clamped() {
        let mut pan = pan();
        pan.jump_to_pane(2);
        assert_eq!(pan.offset(), 200.0);
        assert!(!pan.is_busy());
        pan.jump_to_pane(99);
        assert_eq!(pan.offset(), PAGE * 3.0, "clamped to the last pane");
    }
}
