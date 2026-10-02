//! Two-finger edge swipe that drives the side canvas.
//!
//! A two-finger scroll whose fingers land at the right edge of the touchpad
//! and move left reveals the canvas; while it is shown, a two-finger scroll
//! to the right that starts away from that edge closes it. libinput reports
//! neither finger positions nor where a scroll began, so the touch origins
//! come from the kernel's multitouch slot state (see [`evdev`]) and the
//! decision is made here, in a state machine that knows nothing about
//! devices or the compositor:
//!
//! - [`TouchOrigins`] remembers where each touch was first seen.
//! - [`EdgeZone`] says whether those origins count as "at the right edge".
//! - [`EdgeSwipe`] turns the scroll stream into pass, hold or claim verdicts.
//!
//! The udev glue lives in [`dispatch`].

// Rust guideline compliant 2026-02-21

pub mod dispatch;
pub mod evdev;

/// Width of the right-edge zone the rightmost finger must land in.
///
/// Touchpads are 70 to 150 mm wide; 12 mm is about a fingertip, so a swipe
/// has to start on the rim rather than anywhere on the right half. Widening
/// it makes ordinary horizontal scrolls near the edge open the canvas.
pub const EDGE_ZONE_MM: f64 = 12.0;

/// How far from the right edge the second finger may land.
///
/// Two fingers side by side sit 15 to 25 mm apart, so the companion finger
/// of an edge swipe is never itself on the rim.
pub const COMPANION_ZONE_MM: f64 = 40.0;

/// Edge zone as a fraction of the axis range, for pads without a resolution.
pub const EDGE_ZONE_FRACTION: f64 = 0.10;

/// Companion zone as a fraction of the axis range, for pads without a
/// resolution.
pub const COMPANION_ZONE_FRACTION: f64 = 0.35;

/// Upper bound on how far a touch origin is pushed back to undo the travel
/// that happened before the touch was first sampled.
///
/// libinput's scroll units only approximate millimetres, so the correction
/// is capped to keep a wrong estimate from inventing an edge start.
pub const MAX_COMPENSATION_MM: f64 = 15.0;

/// Finger travel, in logical points, after which a held scroll is judged.
///
/// Long enough for the direction to be clear, short enough that a claimed
/// swipe does not visibly lag the fingers.
pub const DECIDE_DISTANCE_POINTS: f64 = 8.0;

/// Longest a scroll is held while undecided, in seconds.
///
/// A slow drag that has not covered [`DECIDE_DISTANCE_POINTS`] by then is
/// released to the client rather than kept waiting.
pub const DECIDE_TIMEOUT_S: f64 = 0.15;

/// How much more horizontal than vertical a swipe must be to be claimed.
pub const HORIZONTAL_DOMINANCE: f64 = 2.0;

/// Window over which the release velocity is averaged, in seconds.
///
/// Fingers that pause before lifting end with no samples in the window and
/// settle without a fling.
pub const VELOCITY_WINDOW_S: f64 = 0.1;

/// A touch currently on the pad, as read from one multitouch slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    /// Kernel tracking id; a new one is assigned every time a finger lands.
    pub tracking_id: i32,
    /// Horizontal position in device units.
    pub x: i32,
}

/// The horizontal axis of a touchpad, as reported by `EVIOCGABS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeZone {
    /// Smallest reported x.
    pub min: i32,
    /// Largest reported x.
    pub max: i32,
    /// Units per millimetre; 0 when the device does not say.
    pub resolution: i32,
}

impl EdgeZone {
    /// Device units covered by `mm`, or by `fraction` of the range when the
    /// resolution is unknown.
    fn units(&self, mm: f64, fraction: f64) -> f64 {
        if self.resolution > 0 {
            mm * f64::from(self.resolution)
        } else {
            f64::from(self.max - self.min) * fraction
        }
    }

    /// Converts millimetres to device units; 0 when the resolution is unknown.
    pub fn mm_to_units(&self, mm: f64) -> f64 {
        mm * f64::from(self.resolution.max(0))
    }

    /// Whether `x` lies within `width` device units of the right edge.
    fn near_right(&self, x: i32, width: f64) -> bool {
        f64::from(self.max) - f64::from(x) <= width
    }

    /// Whether the touches with these origins make an edge swipe.
    ///
    /// The rightmost origin has to be in the [`EDGE_ZONE_MM`] rim and the
    /// next one within [`COMPANION_ZONE_MM`]; further touches (a resting
    /// thumb) are ignored. Fewer than two origins never qualify.
    pub fn is_edge_start(&self, origins: impl IntoIterator<Item = i32>) -> bool {
        let (mut first, mut second) = (None::<i32>, None::<i32>);
        for x in origins {
            match first {
                Some(f) if x <= f => {
                    if second.is_none_or(|s| x > s) {
                        second = Some(x);
                    }
                }
                _ => {
                    second = first;
                    first = Some(x);
                }
            }
        }
        let (Some(first), Some(second)) = (first, second) else {
            return false;
        };
        self.near_right(first, self.units(EDGE_ZONE_MM, EDGE_ZONE_FRACTION))
            && self.near_right(
                second,
                self.units(COMPANION_ZONE_MM, COMPANION_ZONE_FRACTION),
            )
    }
}

/// Where each touch on the pad was first seen.
#[derive(Debug, Default)]
pub struct TouchOrigins {
    /// `(tracking_id, origin_x)` for every touch in the latest sample.
    origins: Vec<(i32, i32)>,
}

impl TouchOrigins {
    /// Folds in a fresh slot sample.
    ///
    /// Touches that are gone are forgotten. A touch seen for the first time
    /// gets `x - travel_units` as its origin: `travel_units` is how far the
    /// fingers have moved (positive = right) since the gesture began, which
    /// undoes the motion libinput needed before it reported anything. The
    /// origin is clamped to `zone`.
    pub fn observe(&mut self, touches: &[Touch], travel_units: f64, zone: &EdgeZone) {
        self.origins
            .retain(|(id, _)| touches.iter().any(|t| t.tracking_id == *id));
        for touch in touches {
            if self.origins.iter().any(|(id, _)| *id == touch.tracking_id) {
                continue;
            }
            let origin = (f64::from(touch.x) - travel_units)
                .round()
                .clamp(f64::from(zone.min), f64::from(zone.max));
            // Clamped into an i32 range above, so the cast cannot truncate.
            #[expect(clippy::cast_possible_truncation, reason = "clamped to i32 bounds")]
            self.origins.push((touch.tracking_id, origin as i32));
        }
    }

    /// Origins of the touches currently on the pad.
    pub fn xs(&self) -> impl Iterator<Item = i32> + '_ {
        self.origins.iter().map(|(_, x)| *x)
    }

    /// Forgets every touch, e.g. when the device goes away.
    pub fn clear(&mut self) {
        self.origins.clear();
    }
}

/// One two-finger scroll event, in physical finger terms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    /// Horizontal finger travel in logical points, positive to the right,
    /// whatever the natural-scroll setting.
    pub dx: f64,
    /// Vertical finger travel in logical points, positive downwards.
    pub dy: f64,
    /// Event time in seconds.
    pub time_s: f64,
    /// The fingers lifted: every axis in the event reported zero.
    pub stop: bool,
}

/// What the canvas and the touches look like when a scroll begins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Some client has put a surface in the canvas.
    pub available: bool,
    /// The canvas is shown or on its way in.
    pub shown: bool,
    /// The touches started at the right edge of the pad.
    pub from_edge: bool,
}

/// What to do with a scroll event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// Deliver the event to clients as usual.
    Pass,
    /// Keep the event back until the swipe is judged.
    Hold,
    /// Deliver the held events, then this one.
    Release,
    /// Drop the held events and this one; start driving the canvas.
    Begin {
        /// Travel so far in points, positive towards revealing.
        delta: f64,
    },
    /// Swallow the event and move the canvas.
    Update {
        /// Travel in points, positive towards revealing.
        delta: f64,
    },
    /// Swallow the event; the fingers lifted and the canvas settles.
    End {
        /// Release speed in points per second, positive towards revealing.
        velocity: f64,
    },
}

/// Which way a claimed swipe was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    /// Leftward from the edge, canvas hidden.
    Reveal,
    /// Rightward from elsewhere, canvas shown.
    Dismiss,
}

#[derive(Debug, Default)]
enum Phase {
    /// No scroll in progress.
    #[default]
    Idle,
    /// A scroll that could be a swipe; its events are held.
    Deciding {
        intent: Intent,
        start_s: f64,
        dx: f64,
        dy: f64,
    },
    /// A scroll that belongs to clients until the fingers lift.
    Passing,
    /// A scroll that drives the canvas until the fingers lift.
    Claimed,
}

/// The edge swipe decision state machine.
///
/// Fed one [`Scroll`] per two-finger scroll event, it answers with a
/// [`Verdict`]. A scroll is judged once: at its first event the [`Context`]
/// picks a candidate intent (or none, and every event passes straight
/// through), then the first [`DECIDE_DISTANCE_POINTS`] of travel decide
/// between claiming it and handing it back.
#[derive(Debug, Default)]
pub struct EdgeSwipe {
    phase: Phase,
    /// `(time_s, delta)` of the claimed updates within the velocity window.
    samples: Vec<(f64, f64)>,
}

impl EdgeSwipe {
    /// Whether no scroll is in progress, i.e. the next event starts one.
    pub fn is_idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    /// Whether the current scroll is driving the canvas.
    pub fn is_claimed(&self) -> bool {
        matches!(self.phase, Phase::Claimed)
    }

    /// Whether events are being held while the scroll is judged.
    pub fn is_deciding(&self) -> bool {
        matches!(self.phase, Phase::Deciding { .. })
    }

    /// Abandons the current scroll.
    ///
    /// Returns whether the canvas was being driven, in which case the
    /// caller ends the canvas gesture as cancelled. Held events, if any,
    /// belong to the caller to release.
    pub fn reset(&mut self) -> bool {
        let claimed = self.is_claimed();
        self.phase = Phase::Idle;
        self.samples.clear();
        claimed
    }

    /// Judges one scroll event.
    pub fn on_scroll(&mut self, scroll: Scroll, ctx: Context) -> Verdict {
        match self.phase {
            Phase::Idle => {
                if scroll.stop {
                    return Verdict::Pass;
                }
                let intent = if ctx.shown {
                    (!ctx.from_edge).then_some(Intent::Dismiss)
                } else {
                    (ctx.available && ctx.from_edge).then_some(Intent::Reveal)
                };
                let Some(intent) = intent else {
                    self.phase = Phase::Passing;
                    return Verdict::Pass;
                };
                self.phase = Phase::Deciding {
                    intent,
                    start_s: scroll.time_s,
                    dx: 0.0,
                    dy: 0.0,
                };
                self.decide(scroll)
            }
            Phase::Deciding { .. } => self.decide(scroll),
            Phase::Passing => {
                if scroll.stop {
                    self.phase = Phase::Idle;
                }
                Verdict::Pass
            }
            Phase::Claimed => {
                if scroll.stop {
                    let velocity = self.velocity(scroll.time_s);
                    self.reset();
                    return Verdict::End { velocity };
                }
                let delta = -scroll.dx;
                self.push_sample(scroll.time_s, delta);
                Verdict::Update { delta }
            }
        }
    }

    /// Accumulates travel while deciding and judges once there is enough.
    fn decide(&mut self, scroll: Scroll) -> Verdict {
        let Phase::Deciding {
            intent,
            start_s,
            ref mut dx,
            ref mut dy,
        } = self.phase
        else {
            return Verdict::Pass;
        };
        if scroll.stop {
            self.phase = Phase::Idle;
            return Verdict::Release;
        }
        *dx += scroll.dx;
        *dy += scroll.dy;
        let (dx, dy) = (*dx, *dy);

        if dx.hypot(dy) < DECIDE_DISTANCE_POINTS {
            if scroll.time_s - start_s > DECIDE_TIMEOUT_S {
                self.phase = Phase::Passing;
                return Verdict::Release;
            }
            return Verdict::Hold;
        }

        let horizontal = dx.abs() >= dy.abs() * HORIZONTAL_DOMINANCE;
        let direction_ok = match intent {
            Intent::Reveal => dx < 0.0,
            Intent::Dismiss => dx > 0.0,
        };
        if horizontal && direction_ok {
            self.phase = Phase::Claimed;
            self.samples.clear();
            let delta = -dx;
            self.push_sample(scroll.time_s, delta);
            Verdict::Begin { delta }
        } else {
            self.phase = Phase::Passing;
            Verdict::Release
        }
    }

    fn push_sample(&mut self, time_s: f64, delta: f64) {
        self.samples
            .retain(|(t, _)| time_s - *t <= VELOCITY_WINDOW_S);
        self.samples.push((time_s, delta));
    }

    /// Average speed over the [`VELOCITY_WINDOW_S`] before `now_s`.
    fn velocity(&self, now_s: f64) -> f64 {
        let travel: f64 = self
            .samples
            .iter()
            .filter(|(t, _)| now_s - *t <= VELOCITY_WINDOW_S)
            .map(|(_, d)| d)
            .sum();
        travel / VELOCITY_WINDOW_S
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 mm wide pad at 10 units/mm.
    const PAD: EdgeZone = EdgeZone {
        min: 0,
        max: 1000,
        resolution: 10,
    };

    fn scroll(t: f64, dx: f64, dy: f64) -> Scroll {
        Scroll {
            dx,
            dy,
            time_s: t,
            stop: false,
        }
    }

    fn stop(t: f64) -> Scroll {
        Scroll {
            dx: 0.0,
            dy: 0.0,
            time_s: t,
            stop: true,
        }
    }

    const REVEAL: Context = Context {
        available: true,
        shown: false,
        from_edge: true,
    };

    #[test]
    fn edge_start_needs_rightmost_on_the_rim_and_a_companion() {
        // Rim at >= 880, companion zone at >= 600.
        assert!(PAD.is_edge_start([950, 750]));
        assert!(PAD.is_edge_start([750, 950]));
        assert!(!PAD.is_edge_start([850, 750]), "rightmost off the rim");
        assert!(!PAD.is_edge_start([950, 500]), "companion too far in");
        assert!(!PAD.is_edge_start([950]), "one finger");
        assert!(!PAD.is_edge_start([]), "no fingers");
        // A resting thumb far left does not spoil the swipe.
        assert!(PAD.is_edge_start([100, 950, 750]));
    }

    #[test]
    fn edge_start_falls_back_to_a_fraction_without_resolution() {
        let pad = EdgeZone {
            resolution: 0,
            ..PAD
        };
        // 10 % rim = 900, 35 % companion = 650.
        assert!(pad.is_edge_start([920, 700]));
        assert!(!pad.is_edge_start([880, 700]));
        assert!(!pad.is_edge_start([920, 600]));
    }

    #[test]
    fn origins_are_first_sightings_compensated_for_travel() {
        let mut origins = TouchOrigins::default();
        let a = Touch {
            tracking_id: 1,
            x: 800,
        };
        let b = Touch {
            tracking_id: 2,
            x: 700,
        };
        // Fingers already moved 150 units left before the first sample.
        origins.observe(&[a, b], -150.0, &PAD);
        assert_eq!(origins.xs().collect::<Vec<_>>(), [950, 850]);
        assert!(PAD.is_edge_start(origins.xs()));

        // Later samples do not move the origins.
        let a2 = Touch { x: 400, ..a };
        let b2 = Touch { x: 300, ..b };
        origins.observe(&[a2, b2], -550.0, &PAD);
        assert_eq!(origins.xs().collect::<Vec<_>>(), [950, 850]);

        // A lifted touch is forgotten; a new one gets its own origin.
        let c = Touch {
            tracking_id: 3,
            x: 200,
        };
        origins.observe(&[a2, c], 0.0, &PAD);
        assert_eq!(origins.xs().collect::<Vec<_>>(), [950, 200]);
    }

    #[test]
    fn compensation_is_clamped_to_the_pad() {
        let mut origins = TouchOrigins::default();
        origins.observe(
            &[Touch {
                tracking_id: 1,
                x: 990,
            }],
            -100.0,
            &PAD,
        );
        assert_eq!(origins.xs().collect::<Vec<_>>(), [1000]);
    }

    #[test]
    fn leftward_edge_scroll_is_claimed() {
        let mut m = EdgeSwipe::default();
        assert_eq!(m.on_scroll(scroll(0.0, -3.0, 0.0), REVEAL), Verdict::Hold);
        assert_eq!(
            m.on_scroll(scroll(0.01, -6.0, 0.5), REVEAL),
            Verdict::Begin { delta: 9.0 }
        );
        assert_eq!(
            m.on_scroll(scroll(0.02, -10.0, 0.0), REVEAL),
            Verdict::Update { delta: 10.0 }
        );
        let Verdict::End { velocity } = m.on_scroll(stop(0.03), REVEAL) else {
            panic!("expected End");
        };
        assert!(velocity > 0.0, "velocity towards revealing: {velocity}");
        assert!(m.is_idle());
    }

    #[test]
    fn non_edge_scroll_passes_without_holding() {
        let mut m = EdgeSwipe::default();
        let ctx = Context {
            from_edge: false,
            ..REVEAL
        };
        assert_eq!(m.on_scroll(scroll(0.0, -3.0, 0.0), ctx), Verdict::Pass);
        // Even if the next event would look like an edge swipe.
        assert_eq!(m.on_scroll(scroll(0.01, -30.0, 0.0), REVEAL), Verdict::Pass);
        assert_eq!(m.on_scroll(stop(0.02), REVEAL), Verdict::Pass);
        assert!(m.is_idle());
    }

    #[test]
    fn edge_scroll_without_a_canvas_passes() {
        let mut m = EdgeSwipe::default();
        let ctx = Context {
            available: false,
            ..REVEAL
        };
        assert_eq!(m.on_scroll(scroll(0.0, -20.0, 0.0), ctx), Verdict::Pass);
    }

    #[test]
    fn edge_scroll_the_wrong_way_is_released() {
        let mut m = EdgeSwipe::default();
        assert_eq!(m.on_scroll(scroll(0.0, 3.0, 0.0), REVEAL), Verdict::Hold);
        assert_eq!(
            m.on_scroll(scroll(0.01, 6.0, 0.0), REVEAL),
            Verdict::Release
        );
        assert_eq!(m.on_scroll(scroll(0.02, -40.0, 0.0), REVEAL), Verdict::Pass);
        assert_eq!(m.on_scroll(stop(0.03), REVEAL), Verdict::Pass);
        assert!(m.is_idle());
    }

    #[test]
    fn vertical_edge_scroll_is_released() {
        let mut m = EdgeSwipe::default();
        assert_eq!(
            m.on_scroll(scroll(0.0, -5.0, 10.0), REVEAL),
            Verdict::Release
        );
    }

    #[test]
    fn short_edge_tap_scroll_is_released_on_lift() {
        let mut m = EdgeSwipe::default();
        assert_eq!(m.on_scroll(scroll(0.0, -2.0, 0.0), REVEAL), Verdict::Hold);
        assert_eq!(m.on_scroll(stop(0.01), REVEAL), Verdict::Release);
        assert!(m.is_idle());
    }

    #[test]
    fn slow_edge_scroll_is_released_after_the_timeout() {
        let mut m = EdgeSwipe::default();
        assert_eq!(m.on_scroll(scroll(0.0, -1.0, 0.0), REVEAL), Verdict::Hold);
        assert_eq!(m.on_scroll(scroll(0.1, -1.0, 0.0), REVEAL), Verdict::Hold);
        assert_eq!(
            m.on_scroll(scroll(0.2, -1.0, 0.0), REVEAL),
            Verdict::Release
        );
        assert_eq!(m.on_scroll(scroll(0.3, -20.0, 0.0), REVEAL), Verdict::Pass);
    }

    #[test]
    fn rightward_scroll_dismisses_a_shown_canvas() {
        let mut m = EdgeSwipe::default();
        let ctx = Context {
            available: true,
            shown: true,
            from_edge: false,
        };
        assert_eq!(
            m.on_scroll(scroll(0.0, 12.0, 0.0), ctx),
            Verdict::Begin { delta: -12.0 }
        );
        assert_eq!(
            m.on_scroll(scroll(0.01, 5.0, 0.0), ctx),
            Verdict::Update { delta: -5.0 }
        );
        let Verdict::End { velocity } = m.on_scroll(stop(0.02), ctx) else {
            panic!("expected End");
        };
        assert!(velocity < 0.0, "velocity towards hiding: {velocity}");
    }

    #[test]
    fn leftward_scroll_on_a_shown_canvas_passes() {
        let mut m = EdgeSwipe::default();
        let ctx = Context {
            available: true,
            shown: true,
            from_edge: false,
        };
        assert_eq!(m.on_scroll(scroll(0.0, -12.0, 0.0), ctx), Verdict::Release);
        let mut m = EdgeSwipe::default();
        let ctx = Context {
            from_edge: true,
            ..ctx
        };
        assert_eq!(m.on_scroll(scroll(0.0, 12.0, 0.0), ctx), Verdict::Pass);
    }

    #[test]
    fn natural_scroll_does_not_change_the_verdict() {
        // The glue flips libinput's inverted values back to finger travel,
        // so the same finger motion yields the same `Scroll` either way.
        let finger = |natural: bool, raw_dx: f64| if natural { -raw_dx } else { raw_dx };
        for natural in [false, true] {
            let raw = if natural { 12.0 } else { -12.0 };
            let mut m = EdgeSwipe::default();
            let v = m.on_scroll(scroll(0.0, finger(natural, raw), 0.0), REVEAL);
            assert_eq!(v, Verdict::Begin { delta: 12.0 }, "natural = {natural}");
        }
    }

    #[test]
    fn pause_before_lift_settles_without_fling() {
        let mut m = EdgeSwipe::default();
        assert!(matches!(
            m.on_scroll(scroll(0.0, -20.0, 0.0), REVEAL),
            Verdict::Begin { .. }
        ));
        m.on_scroll(scroll(0.01, -20.0, 0.0), REVEAL);
        assert_eq!(
            m.on_scroll(stop(0.5), REVEAL),
            Verdict::End { velocity: 0.0 }
        );
    }

    #[test]
    fn reset_reports_a_claimed_swipe() {
        let mut m = EdgeSwipe::default();
        m.on_scroll(scroll(0.0, -20.0, 0.0), REVEAL);
        assert!(m.reset());
        assert!(m.is_idle());
        assert!(!m.reset());
    }
}
