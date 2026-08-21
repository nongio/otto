use std::time::Instant;

use skia_safe::{Canvas, Contains, Point, Rect};

use crate::theme::Theme;

use super::renderer::ScrollRenderer;
use super::state::{Axis, ScrollState};

/// What fraction of its velocity a fling keeps after one second of coasting.
///
/// Lower stops sooner: the whole glide covers `velocity / -ln(DECAY)` points,
/// so 0.08 coasts for `v/2.5` and 0.02 for `v/3.9` — about a third shorter.
/// Override at runtime with `OTTO_SCROLL_DECAY` to find a feel without a
/// rebuild; read once per process, so it costs nothing per frame.
fn momentum_decay() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        env_f32("OTTO_SCROLL_DECAY")
            .unwrap_or(0.006)
            .clamp(0.0001, 0.99)
    })
}

/// Read a tuning override, ignoring anything that is not a number.
fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.parse().ok()
}
/// A fling slower than this has visually stopped, so drop it rather than
/// creeping a fraction of a pixel per frame forever.
///
/// Raising it chops the tail off the glide instead of reshaping the curve:
/// where `decay` decides how quickly speed bleeds away, this decides how much
/// speed is still worth showing. High values read as a firm stop rather than
/// a fade. Override with `OTTO_SCROLL_MIN_VELOCITY`.
fn min_velocity() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        env_f32("OTTO_SCROLL_MIN_VELOCITY")
            .unwrap_or(120.0)
            .max(0.0)
    })
}
/// Ceiling on the velocity a gesture can hand to the momentum phase, in
/// points per second — one very short, very fast frame should not launch the
/// content across a thousand rows.
///
/// This is what decides how far a *throw* goes: the glide covers
/// `velocity / -ln(decay)`, so once a flick saturates this ceiling, every
/// flick coasts the same distance however hard it was thrown, and the decay
/// setting only scales that one fixed number. Override with
/// `OTTO_SCROLL_MAX_VELOCITY`.
fn max_velocity() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        env_f32("OTTO_SCROLL_MAX_VELOCITY")
            .unwrap_or(2200.0)
            .max(1.0)
    })
}
/// Length of the sliding window a gesture's speed is averaged over, in
/// seconds. Long enough to smooth out one stuttering frame, short enough that
/// what gets thrown is how the gesture ended, not how it started.
const VELOCITY_WINDOW: f32 = 0.09;
/// A gesture slower than this at the moment it ends is a placement, not a
/// throw: the content stays where it was let go.
const FLING_MIN_VELOCITY: f32 = 60.0;
/// A gesture measured over less time than this has not been observed long
/// enough to have a speed.
const MIN_MEASURED_TIME: f32 = 0.008;
/// A gesture whose deltas stop arriving for this long is abandoned — a
/// fallback for sources that never report the lift. Deliberately long: a
/// finger resting on the touchpad sends nothing, and it is still holding the
/// content where it put it, rubber band and all. Nothing is thrown when this
/// fires; only an actual lift throws.
const GESTURE_TIMEOUT: f32 = 0.5;

/// Largest timestep [`ScrollView::advance`] will integrate in one go.
///
/// A ceiling on how far one call can move the content, so a host that was
/// blocked — a stall, a suspend, a debugger — resumes where it left off
/// rather than teleporting. Set well outside the range of an ordinary slow
/// frame: anything inside that range turns late frames into lost travel.
const MAX_STEP: f32 = 0.25;

/// Spring pulling an overscrolled content back to its end. Softer springs
/// stretch further for the same impact and take longer to come home, which is
/// what reads as bounce.
const SPRING_STIFFNESS: f32 = 140.0;
/// Damping for [`SPRING_STIFFNESS`], deliberately under critical (ratio ~0.68)
/// so the return is quick and elastic rather than a slow asymptotic creep. It
/// cannot wobble: the bounce stops the moment the content reaches its end.
const SPRING_DAMPING: f32 = 16.0;
/// Fixed integration step for the spring, so the bounce looks the same
/// whether the host is painting at 60 or 144 Hz.
const SPRING_STEP: f32 = 1.0 / 240.0;
/// Below this overscroll and speed the bounce has arrived; snap and stop.
const SETTLE_DISTANCE: f32 = 0.35;
const SETTLE_VELOCITY: f32 = 30.0;
/// How much speed survives a fling running into an end. The rest is absorbed,
/// which is what keeps a fast fling from pulling the content halfway off the
/// viewport before the spring catches it.
const BOUNCE_RETENTION: f32 = 0.45;
/// Hard cap on the speed handed to the bounce, so the overshoot stays in the
/// tens of points however fast the fling was.
const MAX_BOUNCE_VELOCITY: f32 = 1600.0;
/// How far past the end a fling lands before the spring takes over. Just
/// enough to count as overscrolled and clear [`SETTLE_DISTANCE`].
const BOUNCE_NUDGE: f32 = 0.5;
/// How far past an end the content stretches, as a fraction of the viewport,
/// before the hand has to work `e` times as hard for the next point of it.
/// See [`band`]: this is the decay scale of the resistance, not a wall — the
/// stretch has no hard limit, it just gets exponentially more expensive.
const RUBBER_SCALE_RATIO: f32 = 0.03;

/// How far the content moves per point of scroll reported by the
/// compositor. Wayland axis values for a touchpad are the finger's own
/// movement in logical points, which reads as sluggish for a scroll — a
/// gesture should cover more ground than the finger does.
///
/// Public because a scroll view is not the only thing a two-finger gesture
/// moves: anything else that turns an axis delta into travel has to apply the
/// same factor, or the same gesture covers different ground depending on what
/// is under it.
///
/// Override with `OTTO_SCROLL_SPEED`.
pub fn wheel_scale() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_f32("OTTO_SCROLL_SPEED").unwrap_or(9.0).max(0.0))
}

/// How long the scrollbar stays up after the last scroll before fading out.
const SCROLLBAR_HOLD: f32 = 0.8;
const SCROLLBAR_FADE_IN: f32 = 0.10;
const SCROLLBAR_FADE_OUT: f32 = 0.35;
/// Time for the bar to widen (or narrow again) under the pointer.
const SCROLLBAR_EXPAND: f32 = 0.12;

/// A scroll view: clips content to a viewport, offsets it by a scroll
/// position, and draws a scrollbar over it. Vertical by default;
/// [`Self::horizontal`] scrolls sideways instead, with identical feel.
///
/// Beyond tracking the offset the widget owns the *feel* of scrolling:
///
/// - **Momentum** — deltas fed to [`Self::on_wheel`] (or a content drag) are
///   sampled per frame into a velocity, which keeps coasting under friction
///   once the gesture stops.
/// - **Rubber banding** — past either end the content still moves, but with
///   rising resistance, and a spring pulls it back when let go. A fling that
///   runs into an end bounces off it.
/// - **Overlay scrollbar** — the bar fades in while scrolling, fades out
///   when idle, and widens under the pointer.
///
/// All three are advanced by [`Self::advance`] (or [`Self::tick`]): a host
/// calls it once per frame and asks for another frame while
/// [`Self::is_animating`] is true. A host that never ticks still gets a
/// plain, immediate scroll view — every event method applies its delta
/// straight away.
///
/// Input plumbing beyond that stays with the host, which feeds the widget
/// events already translated into its own coordinate space.
#[derive(Debug, Clone)]
pub struct ScrollView {
    pub state: ScrollState,
    /// The pointer's position along the scrolling axis, and the offset it
    /// started from, captured when a thumb drag began — `None` when no drag
    /// is in progress.
    drag_origin: Option<(f32, f32)>,
    /// The last pointer position along the axis of an in-progress content
    /// drag (dragging the content itself, rather than the scrollbar).
    content_drag: Option<f32>,
    /// Is the pointer resting over the scrollbar's gutter?
    hovering: bool,
    /// Multiplier applied to incoming wheel deltas.
    wheel_scale: f32,
    /// Points per second the content is coasting at; positive scrolls down.
    velocity: f32,
    /// Delta applied by input since the last [`Self::advance`], which that
    /// call folds into the gesture's speed estimate.
    pending_delta: f32,
    /// The gesture in progress, if any. While one is running the input owns
    /// the offset outright — no momentum, no spring — and this accumulates
    /// the speed to throw with when it ends.
    gesture: Option<Gesture>,
    /// Seconds since the last scroll activity, driving the scrollbar fade.
    idle: f32,
    /// When [`Self::tick`] last ran, so it can derive its own delta time.
    last_tick: Option<Instant>,
}

impl ScrollView {
    /// A vertical scroll view — the common case.
    pub fn new(viewport: Rect) -> Self {
        Self::on_axis(Axis::Vertical, viewport)
    }

    /// A scroll view that scrolls left and right, with its bar along the
    /// viewport's bottom edge. Positive offsets move the content left, the
    /// way positive offsets move a vertical view's content up.
    pub fn horizontal(viewport: Rect) -> Self {
        Self::on_axis(Axis::Horizontal, viewport)
    }

    pub fn on_axis(axis: Axis, viewport: Rect) -> Self {
        Self {
            state: ScrollState::on_axis(axis, viewport),
            drag_origin: None,
            content_drag: None,
            hovering: false,
            wheel_scale: wheel_scale(),
            velocity: 0.0,
            pending_delta: 0.0,
            gesture: None,
            idle: f32::MAX,
            last_tick: None,
        }
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.state.set_viewport(viewport);
    }

    pub fn set_content_length(&mut self, content_length: f32) {
        self.state.set_content_length(content_length);
    }

    pub fn axis(&self) -> Axis {
        self.state.axis()
    }

    /// Set the offset directly, clamped to range, dropping any momentum or
    /// bounce first: a programmatic move and a fling are two authorities on
    /// one offset, and the caller asking for a position wins. Returns whether
    /// the offset changed.
    pub fn scroll_to(&mut self, offset: f32) -> bool {
        self.stop();
        self.state.set_offset(offset)
    }

    pub fn offset(&self) -> f32 {
        self.state.offset()
    }

    /// Points per second the content is coasting at; positive scrolls down.
    /// Zero while a gesture is driving it, since input owns the offset then.
    /// A surface-backed view reads this to spend its overdraw ahead of the
    /// scroll rather than evenly around it.
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// How far the content moves per point of scroll the host reports.
    /// Defaults to [`wheel_scale`]; set it to `1.0` to track the finger
    /// exactly, or higher for a longer reach per gesture. Applies to wheel
    /// input only — dragging the content or the thumb is always one to one,
    /// since those follow the pointer.
    pub fn set_wheel_scale(&mut self, scale: f32) {
        self.wheel_scale = scale.max(0.0);
    }

    // === Events ===

    /// Wheel/trackpad delta along the scroll axis, in points. Positive
    /// scrolls down (right, on a horizontal view). Returns whether the offset
    /// changed and a redraw is needed.
    ///
    /// This is the continuous (touchpad, kinetic) source: the delta feeds the
    /// momentum sampler, so letting go mid-flick keeps the content gliding.
    /// For a notched mouse wheel use [`Self::on_wheel_discrete`], which moves
    /// the same distance without launching a fling per click.
    pub fn on_wheel(&mut self, delta: f32) -> bool {
        self.wake();
        self.begin_gesture();
        self.pending_delta += delta * self.wheel_scale;
        self.apply_delta(delta, self.wheel_scale, true)
    }

    /// The scroll gesture ended — the fingers left the touchpad. A host knows
    /// this from an axis-stop event; call it and whatever speed the gesture
    /// was carrying becomes a fling, and anything pulled past an end springs
    /// back.
    ///
    /// Optional: a source that never reports the lift is treated as finished
    /// once its deltas stop arriving.
    pub fn on_wheel_end(&mut self) {
        self.end_gesture();
    }

    /// A notched mouse wheel step, in points. Scrolls immediately, with no
    /// momentum and no rubber banding — a wheel click is a discrete
    /// instruction, not a gesture with a speed.
    pub fn on_wheel_discrete(&mut self, delta: f32) -> bool {
        self.wake();
        self.velocity = 0.0;
        self.apply_delta(delta, self.wheel_scale, false)
    }

    /// Pointer press at canvas-local `(x, y)`. Returns whether it landed on
    /// the scrollbar thumb and started a drag; a host should only treat the
    /// press as "handled by the scrollbar" when this is `true`.
    ///
    /// A press anywhere in the viewport also catches an in-flight fling, the
    /// way putting a finger down stops a spinning wheel.
    pub fn on_pointer_down(&mut self, x: f32, y: f32) -> bool {
        if self.state.viewport().contains(Point::new(x, y)) {
            self.velocity = 0.0;
            self.pending_delta = 0.0;
            self.gesture = None;
        }
        self.wake();
        if ScrollRenderer::hit_test_thumb(&self.state, x, y) {
            self.drag_origin = Some((self.axis().coord(x, y), self.state.offset()));
            true
        } else {
            false
        }
    }

    /// Pointer moved to canvas-local `(x, y)` while a thumb drag may be in
    /// progress. A no-op (returns `false`) unless [`Self::on_pointer_down`]
    /// most recently reported a hit. Returns whether the offset changed.
    pub fn on_pointer_drag(&mut self, x: f32, y: f32) -> bool {
        let Some((start, start_offset)) = self.drag_origin else {
            return false;
        };
        self.wake();
        // Dragging the thumb by one pixel of track should move the content
        // by `max_offset / travel` pixels — the thumb travels a shorter
        // distance than the content does whenever it is shorter than the
        // track, which is exactly when there is anything to scroll.
        let axis = self.axis();
        let track_len = self.state.viewport_length();
        let thumb_len = ScrollRenderer::thumb_rect(&self.state)
            .map(|r| axis.length(r))
            .unwrap_or(track_len);
        let travel = (track_len - thumb_len).max(1.0);
        let ratio = self.state.max_offset() / travel;
        self.state
            .set_offset(start_offset + (axis.coord(x, y) - start) * ratio)
    }

    /// Pointer moved to canvas-local `(x, y)` with no button held. Tracks
    /// whether it is over the scrollbar, which keeps the bar up and widens
    /// it. Returns whether anything about the presentation changed and a
    /// redraw is needed.
    pub fn on_pointer_move(&mut self, x: f32, y: f32) -> bool {
        let over = ScrollRenderer::hit_test_gutter(&self.state, x, y);
        let changed = over != self.hovering;
        self.hovering = over;
        if over {
            self.idle = 0.0;
        }
        changed
    }

    /// The pointer left the surface: nothing is hovered any more.
    pub fn on_pointer_leave(&mut self) {
        self.hovering = false;
    }

    pub fn on_pointer_up(&mut self) {
        self.drag_origin = None;
        // Releasing a content drag hands whatever speed the drag was carrying
        // to the momentum phase.
        if self.content_drag.take().is_some() {
            self.end_gesture();
        }
    }

    /// Begin dragging the content itself from canvas-local `(x, y)` (touch,
    /// or a press-and-drag pan). Ended by [`Self::on_pointer_up`], which lets
    /// the gesture's speed become a fling.
    pub fn on_content_drag_start(&mut self, x: f32, y: f32) {
        self.velocity = 0.0;
        self.pending_delta = 0.0;
        self.content_drag = Some(self.axis().coord(x, y));
        self.begin_gesture();
        self.wake();
    }

    /// Continue a content drag at canvas-local `(x, y)`. Dragging down moves
    /// the content down, i.e. scrolls up. Returns whether the offset changed.
    pub fn on_content_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(last) = self.content_drag else {
            return false;
        };
        let position = self.axis().coord(x, y);
        self.content_drag = Some(position);
        self.wake();
        self.begin_gesture();
        let delta = last - position;
        self.pending_delta += delta;
        // A content drag follows the pointer one to one, so there is nothing
        // to amplify — and nothing to un-amplify at the ends either.
        self.apply_delta(delta, 1.0, true)
    }

    /// Drop any momentum and any bounce, leaving the content where it is
    /// (still inside its range).
    pub fn stop(&mut self) {
        self.velocity = 0.0;
        self.pending_delta = 0.0;
        self.gesture = None;
        self.state.set_offset(self.state.offset());
    }

    // === Animation ===

    /// Is there momentum, a bounce, or a scrollbar fade still to run? While
    /// this is true a host should keep asking for frames and calling
    /// [`Self::advance`].
    pub fn is_animating(&self) -> bool {
        // The last two clauses are what makes a *fade in* possible: right
        // after a click or a hover there is no motion and nothing drawn yet,
        // so without them the host would stop ticking and the bar would never
        // come up.
        self.velocity != 0.0
            || self.state.overscroll() != 0.0
            || self.pending_delta != 0.0
            || self.state.scrollbar_opacity() > 0.0
            || self.state.scrollbar_expansion() > 0.0
            || self.gesture.is_some()
            || self.idle < SCROLLBAR_HOLD
            || self.hovering
            || self.drag_origin.is_some()
            || self.content_drag.is_some()
    }

    /// Advance momentum, bounce and scrollbar fade by `dt` seconds. Returns
    /// whether anything moved and a redraw is needed.
    pub fn advance(&mut self, dt: f32) -> bool {
        // A host that was blocked for a second should not teleport the
        // content: cap the step at a stall, not at a slow frame.
        //
        // This used to cap at 50 ms, which sounds like "one very bad frame"
        // and is not: a client whose frames arrive at 20-70 ms — an ordinary
        // range under load — was having its timestep truncated on a fifth of
        // them. A truncated step advances the content by less than the time
        // that actually passed, so the fling silently falls behind itself and
        // then carries on from the deficit, which reads as a hitch on exactly
        // the frames that were already late. The guard is meant for a host
        // that stopped ticking altogether, so it belongs out where only a
        // real stall reaches it.
        let dt = dt.clamp(0.0, MAX_STEP);
        if dt <= 0.0 {
            return false;
        }
        let moved = self.advance_motion(dt);
        let faded = self.advance_scrollbar(dt);
        moved || faded
    }

    /// [`Self::advance`] with the delta time derived from a wall clock —
    /// what a host that just paints on frame callbacks wants.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|last| now.duration_since(last).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_tick = Some(now);
        self.advance(dt)
    }

    fn advance_motion(&mut self, dt: f32) -> bool {
        if self.drag_origin.is_some() {
            // A scrollbar drag is absolute positioning, not a throw.
            self.velocity = 0.0;
            self.pending_delta = 0.0;
            return false;
        }

        // While a gesture is running it owns the offset outright: no friction
        // pulling ahead of the finger, no spring fighting the rubber band.
        // All the tick does is keep the speed estimate current.
        if let Some(mut gesture) = self.gesture.take() {
            gesture.sample(self.pending_delta, dt);
            self.pending_delta = 0.0;
            // A source that reports no lift (and a stream that simply dries
            // up) is finished once its deltas stop arriving.
            if gesture.since_input >= GESTURE_TIMEOUT {
                // Gone quiet without ever saying it ended: let the content
                // go (so an overscroll can spring back), but throw nothing —
                // whatever speed it once had is long stale.
                self.gesture = None;
                self.velocity = 0.0;
                self.pending_delta = 0.0;
            } else {
                self.gesture = Some(gesture);
            }
            return false;
        }

        let over = self.state.overscroll();
        if over != 0.0 {
            self.advance_spring(dt, over)
        } else if self.velocity != 0.0 {
            self.advance_momentum(dt)
        } else {
            false
        }
    }

    /// Start (or continue) the gesture the incoming deltas belong to.
    fn begin_gesture(&mut self) {
        if self.gesture.is_none() {
            // Taking over from a coast keeps the throw-on-a-throw feel:
            // flicking again while the content is still gliding adds to it.
            self.gesture = Some(Gesture::new(self.velocity));
            self.velocity = 0.0;
        }
    }

    /// End the gesture, if one is running, turning its speed into a fling.
    fn end_gesture(&mut self) {
        if let Some(gesture) = self.gesture.take() {
            self.finish(gesture);
        }
    }

    /// Hand a lifted gesture's speed to the momentum phase — unless it was
    /// barely moving, in which case the content stops where it was let go.
    fn finish(&mut self, gesture: Gesture) {
        // Fold in the sliver of movement that arrived after the last tick.
        let pending = std::mem::take(&mut self.pending_delta);
        let measured = gesture.speed(pending);
        self.velocity = if measured.abs() < FLING_MIN_VELOCITY {
            // Too slow to be a throw. It also stops whatever was coasting:
            // taking hold of a gliding view and putting it down is how you
            // stop it.
            0.0
        } else if measured.signum() == signum(gesture.inherited) {
            // Same direction as the glide it interrupted, so it adds — this is
            // what makes flicking repeatedly build speed.
            (measured + gesture.inherited).clamp(-max_velocity(), max_velocity())
        } else {
            measured.clamp(-max_velocity(), max_velocity())
        };
        self.gesture = None;
    }

    /// Coast under friction: velocity decays exponentially, and the distance
    /// travelled is that decay's integral, so the result does not depend on
    /// how the frames happened to land.
    fn advance_momentum(&mut self, dt: f32) -> bool {
        let k = -momentum_decay().ln();
        let decayed = self.velocity * (-k * dt).exp();
        let travel = (self.velocity - decayed) / k;
        self.velocity = if decayed.abs() < min_velocity() {
            0.0
        } else {
            decayed
        };

        let max = self.state.max_offset();
        let target = self.state.offset() + travel;
        if target < 0.0 || target > max {
            // Ran into an end: absorb most of the speed, and hand the rest to
            // the spring by landing just past the end — how far the bounce
            // then reaches is that retained speed, not this frame's overshoot,
            // which is what keeps the rebound the same at any frame rate.
            let bound = if target < 0.0 { 0.0 } else { max };
            let direction = if target < 0.0 { -1.0 } else { 1.0 };
            self.velocity =
                (self.velocity * BOUNCE_RETENTION).clamp(-MAX_BOUNCE_VELOCITY, MAX_BOUNCE_VELOCITY);
            self.state
                .set_offset_overscrolled(bound + direction * BOUNCE_NUDGE)
        } else {
            self.state.set_offset_overscrolled(target)
        }
    }

    /// Pull the content back to the end it is hanging off, carrying whatever
    /// velocity it arrived with.
    fn advance_spring(&mut self, dt: f32, over: f32) -> bool {
        let bound = if over < 0.0 {
            0.0
        } else {
            self.state.max_offset()
        };
        let mut x = over;
        let mut v = self.velocity;
        let mut t = 0.0;
        while t < dt {
            let h = (dt - t).min(SPRING_STEP);
            v += (-SPRING_STIFFNESS * x - SPRING_DAMPING * v) * h;
            let next = x + v * h;
            // Crossing the end means the bounce is over. Stop dead: an
            // under-damped spring still carries speed at that point, and
            // leaving it in would hand the momentum phase a shove inwards —
            // the content would rebound off the end and sail back into the
            // list instead of resting against it.
            if next * x < 0.0 {
                x = 0.0;
                v = 0.0;
                break;
            }
            x = next;
            t += h;
        }

        if x.abs() <= SETTLE_DISTANCE && v.abs() <= SETTLE_VELOCITY {
            x = 0.0;
            v = 0.0;
        }
        self.velocity = v;
        self.state.set_offset_overscrolled(bound + x)
    }

    /// Fade the overlay scrollbar in while scrolling and out once idle, and
    /// widen it while the pointer is on it.
    fn advance_scrollbar(&mut self, dt: f32) -> bool {
        let dragging = self.drag_origin.is_some() || self.content_drag.is_some();
        let moving = self.velocity != 0.0 || self.state.overscroll() != 0.0;
        if dragging || self.hovering || moving {
            self.idle = 0.0;
        } else {
            self.idle += dt;
        }

        let held = dragging || self.hovering;
        let opacity = self.state.scrollbar_opacity();
        let opacity_target = if self.state.scrollable() && (held || self.idle < SCROLLBAR_HOLD) {
            1.0
        } else {
            0.0
        };
        let fade = if opacity_target > opacity {
            SCROLLBAR_FADE_IN
        } else {
            SCROLLBAR_FADE_OUT
        };
        let next_opacity = approach(opacity, opacity_target, dt / fade);

        let expansion = self.state.scrollbar_expansion();
        let expansion_target =
            if self.state.scrollable() && (self.hovering || self.drag_origin.is_some()) {
                1.0
            } else {
                0.0
            };
        let next_expansion = approach(expansion, expansion_target, dt / SCROLLBAR_EXPAND);

        self.state.set_scrollbar_opacity(next_opacity);
        self.state.set_scrollbar_expansion(next_expansion);
        next_opacity != opacity || next_expansion != expansion
    }

    /// Apply an input delta, resisting the part of it that pulls the content
    /// past an end when `rubber` is set.
    /// Apply an input movement of `delta` points, amplified by `scale` while
    /// it travels inside the content and resisted once it pulls past an end.
    ///
    /// The amplification deliberately stops at the ends. Past an end the
    /// gesture is no longer scrolling, it is stretching, and a stretch should
    /// answer to the hand: with the amplified delta going into the rubber band
    /// a flick covers the whole stretch in two or three events and the content
    /// pins to the asymptote at once, which reads as a wall rather than as
    /// resistance.
    ///
    /// The stretch is tracked as the *pull* behind it — the un-resisted hand
    /// travel that produced it — so releasing is the exact inverse of pulling
    /// and the content follows the finger home without a jump.
    ///
    /// Content that fits the viewport has nowhere to travel, but it still
    /// stretches: a gesture over a short list pulls it off its end and it
    /// springs back, which is how a scroll view says "that is all of it"
    /// rather than saying nothing at all. Only the un-resisted paths — a
    /// notched wheel click — stay dead there.
    fn apply_delta(&mut self, delta: f32, scale: f32, rubber: bool) -> bool {
        let max = self.state.max_offset();
        let offset = self.state.offset();
        if !rubber {
            if !self.state.scrollable() {
                return false;
            }
            return self.state.set_offset(offset + delta * scale);
        }

        let scale_pts = (self.state.viewport_length() * RUBBER_SCALE_RATIO).max(1.0);
        let inside_before = offset.clamp(0.0, max);
        let over_before = offset - inside_before;
        let mut pull = unband(over_before.abs(), scale_pts) * signum(over_before);
        let mut delta = delta;

        // Movement back towards the content releases the stretch first, one to
        // one with the hand — only what is left over travels inside.
        if pull != 0.0 && delta != 0.0 && (delta < 0.0) != (pull < 0.0) {
            let spent = delta.abs().min(pull.abs());
            pull -= spent * signum(pull);
            delta -= spent * signum(delta);
        }

        let (landing, pulled) = if pull != 0.0 {
            // Still hanging off an end, so everything left stretches further.
            (if pull < 0.0 { 0.0 } else { max }, pull + delta)
        } else {
            let travelled = inside_before + delta * scale;
            let landing = travelled.clamp(0.0, max);
            // Whatever the amplified move could not spend inside the content,
            // converted back into the hand's own units.
            (landing, (travelled - landing) / scale.max(f32::EPSILON))
        };

        self.state
            .set_offset_overscrolled(landing + band(pulled.abs(), scale_pts) * signum(pulled))
    }

    /// Any interaction brings the scrollbar back up.
    fn wake(&mut self) {
        self.idle = 0.0;
    }

    /// Is `(x, y)`, in canvas-local space, over the scrollbar thumb?
    pub fn hit_test_thumb(&self, x: f32, y: f32) -> bool {
        ScrollRenderer::hit_test_thumb(&self.state, x, y)
    }

    /// Map a viewport-space point into content-space, so a caller can
    /// hit-test its own content while scrolled.
    pub fn viewport_to_content(&self, x: f32, y: f32) -> (f32, f32) {
        ScrollRenderer::viewport_to_content(&self.state, x, y)
    }

    // === Drawing ===

    /// Draw the content and the scrollbar. `content` is handed the band of
    /// content-local space the viewport is showing — see
    /// [`ScrollRenderer::draw`] for what it may and may not do with it.
    pub fn render(&self, canvas: &Canvas, theme: &Theme, content: impl FnOnce(&Canvas, Rect)) {
        ScrollRenderer::draw(canvas, &self.state, theme, content);
    }
}

/// A scroll gesture in progress, tracking how fast it is moving.
///
/// Speed is an exponentially weighted average over [`VELOCITY_WINDOW`]:
/// distance and elapsed time are accumulated and both decayed at the same
/// rate, so their ratio is the recent speed however irregularly the deltas
/// and the ticks arrive. A touchpad reporting one point every four
/// milliseconds and a mouse reporting fifteen points at a time land on the
/// same number.
#[derive(Debug, Clone, Copy)]
struct Gesture {
    /// Decayed sum of the distance scrolled.
    distance: f32,
    /// Decayed sum of the time that distance took.
    elapsed: f32,
    /// Seconds since the last delta arrived, for spotting a gesture that
    /// ended without saying so.
    since_input: f32,
    /// Speed the content was already coasting at when this gesture took over.
    inherited: f32,
}

impl Gesture {
    /// Start a gesture over a content that is still coasting at
    /// `inherited_velocity`. The window itself starts empty — it measures this
    /// gesture and nothing else — and the inherited speed is kept aside to be
    /// *added* on release, so flicking again mid-glide accelerates the content
    /// the way a second push on a spinning wheel does. Averaging the two
    /// instead would make a second identical flick change nothing at all.
    fn new(inherited_velocity: f32) -> Self {
        Self {
            distance: 0.0,
            elapsed: 0.0,
            since_input: 0.0,
            inherited: inherited_velocity,
        }
    }

    /// Fold one tick's worth of movement into the window.
    ///
    /// Every tick counts, including the ones that brought no movement. It is
    /// tempting to skip those so a pause cannot dilute the throw, but the host
    /// ticks far more often than a touchpad reports — a spinning event loop
    /// samples several times between events — so dropping the empty ticks
    /// drops most of the elapsed time and the measured speed comes out several
    /// times too high. A held finger is meant to decay the speed; that is what
    /// the window is for.
    fn sample(&mut self, delta: f32, dt: f32) {
        if delta == 0.0 {
            self.since_input += dt;
        } else {
            self.since_input = 0.0;
        }
        let decay = (-dt / VELOCITY_WINDOW).exp();
        self.distance = self.distance * decay + delta;
        self.elapsed = self.elapsed * decay + dt;
    }

    /// The gesture's speed in points per second, including `trailing` points
    /// that arrived since the last sample. A gesture the host never ticked
    /// has no measured duration and so no speed — better to place the content
    /// than to throw it at a number made up from one event.
    ///
    /// A gesture that goes still before it ends decays to nothing on its own,
    /// because the window keeps accumulating time while no distance arrives —
    /// holding the content and then letting go leaves it where it was held.
    fn speed(&self, trailing: f32) -> f32 {
        if self.elapsed < MIN_MEASURED_TIME {
            return 0.0;
        }
        (self.distance + trailing) / self.elapsed
    }
}

/// `f32::signum` reports `1.0` for a positive zero, which would turn "no
/// stretch" into "stretched by nothing in the positive direction". Scroll
/// geometry needs zero to stay zero.
fn signum(v: f32) -> f32 {
    if v == 0.0 {
        0.0
    } else {
        v.signum()
    }
}

/// Squash a raw pull of `pull` points past an end into the overscroll
/// distance actually shown.
///
/// The rule the curve comes from is about the *hand*, not about the distance:
/// **each further point of pull moves the content less than the last, by a
/// factor that decays exponentially in how far it is already out.** At an
/// overscroll of `s` the content tracks the hand at `e^(-s / scale)` — one to
/// one at the end itself, 37% of the hand once `scale` out, 5% at three times
/// that. Integrating that rule gives the closed form below.
///
/// ```text
/// pull   0.5    1     2     4     8    (in units of `scale`)
/// shown  0.41  0.69  1.10  1.61  2.20
/// ```
///
/// So `scale` is not a wall the content approaches — it is the distance over
/// which the resistance grows by a factor of e. The stretch stays open-ended:
/// there is always a little more to give, but it costs exponentially more
/// hand travel to take, which is what a rubber band does and what pulling
/// against an asymptote never quite feels like. Reaching three times `scale`
/// takes nineteen times it in finger travel, so in practice the stretch is
/// bounded by the size of the gesture a hand can make.
fn band(pull: f32, scale: f32) -> f32 {
    scale * (pull / scale).ln_1p()
}

/// The inverse of [`band`]: the pull that would show an overscroll of `shown`.
///
/// The exponent is capped before it is taken, so a `shown` that arrives from
/// somewhere other than a gesture — a stale layout, a bounce caught in
/// flight — cannot overflow to infinity and poison the offset.
fn unband(shown: f32, scale: f32) -> f32 {
    scale * ((shown / scale).min(20.0).exp() - 1.0)
}

/// Move `current` towards `target` by `step`, landing exactly on it. Linear,
/// so a fade takes the time its constant says it does rather than trailing an
/// exponential tail that never quite arrives.
fn approach(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else {
        (current - step).max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ScrollView {
        let mut v = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        v.set_content_length(1000.0);
        // One point in, one point moved: the tests are about the physics, not
        // about how far a gesture is amplified.
        v.set_wheel_scale(1.0);
        v
    }

    /// A horizontal view of the same proportions, so its physics can be
    /// compared against [`view`]'s directly.
    fn horizontal_view() -> ScrollView {
        let mut v = ScrollView::horizontal(Rect::from_xywh(0.0, 0.0, 200.0, 100.0));
        v.set_content_length(1000.0);
        v.set_wheel_scale(1.0);
        v
    }

    #[test]
    fn a_horizontal_view_flings_and_bounces_like_a_vertical_one() {
        let mut sideways = horizontal_view();
        let mut down = view();
        for _ in 0..6 {
            sideways.on_wheel(20.0);
            down.on_wheel(20.0);
            sideways.advance(1.0 / 60.0);
            down.advance(1.0 / 60.0);
        }
        sideways.on_wheel_end();
        down.on_wheel_end();
        run(&mut sideways, 60);
        run(&mut down, 60);
        assert_eq!(sideways.offset(), down.offset());
        assert!(sideways.offset() > 120.0, "it should have coasted on");

        // And it stretches past the start the same way, dragging content
        // rightwards by x rather than downwards by y.
        sideways.scroll_to(0.0);
        sideways.on_content_drag_start(0.0, 0.0);
        sideways.on_content_drag(120.0, 0.0);
        assert!(sideways.state.overscroll() < 0.0);
        sideways.on_pointer_up();
        run(&mut sideways, 120);
        assert_eq!(sideways.offset(), 0.0);
    }

    #[test]
    fn scroll_to_drops_a_fling_in_flight() {
        let mut v = view();
        for _ in 0..6 {
            v.on_wheel(20.0);
            v.advance(1.0 / 60.0);
        }
        v.on_wheel_end();
        run(&mut v, 2);
        assert!(v.velocity() != 0.0, "should be coasting");

        v.scroll_to(500.0);
        assert_eq!(v.offset(), 500.0);
        assert_eq!(v.velocity(), 0.0);
        run(&mut v, 20);
        assert_eq!(v.offset(), 500.0, "nothing should have carried it on");
    }

    #[test]
    fn wheel_deltas_are_amplified_by_the_scale() {
        let mut v = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        v.set_content_length(1000.0);
        v.on_wheel(10.0);
        assert_eq!(v.offset(), 10.0 * wheel_scale());

        v.set_wheel_scale(1.0);
        v.on_wheel(10.0);
        assert_eq!(v.offset(), 10.0 * wheel_scale() + 10.0);
    }

    /// Run `frames` 60 Hz steps.
    fn run(v: &mut ScrollView, frames: usize) {
        for _ in 0..frames {
            v.advance(1.0 / 60.0);
        }
    }

    #[test]
    fn wheel_scrolls_and_a_huge_delta_lands_at_the_end() {
        let mut v = view();
        assert!(v.on_wheel(50.0));
        assert_eq!(v.offset(), 50.0);
        // A wild delta rubber-bands rather than clamping dead, but it cannot
        // pull the content further out than the resistance allows.
        v.on_wheel(-9999.0);
        assert!(v.offset() < 0.0);
        assert!(v.state.overscroll() > -v.state.viewport().height());
        // And it is back at the top once the bounce finishes.
        run(&mut v, 120);
        assert_eq!(v.offset(), 0.0);
    }

    #[test]
    fn press_off_the_thumb_does_not_start_a_drag() {
        let mut v = view();
        assert!(!v.on_pointer_down(0.0, 0.0));
        assert!(!v.on_pointer_drag(0.0, 50.0));
    }

    #[test]
    fn dragging_the_thumb_to_the_bottom_reaches_max_offset() {
        let mut v = view();
        let thumb = ScrollRenderer::thumb_rect(&v.state).unwrap();
        assert!(v.on_pointer_down(thumb.center_x(), thumb.center_y()));
        v.on_pointer_drag(thumb.center_x(), thumb.center_y() + 200.0);
        assert_eq!(v.offset(), v.state.max_offset());
        v.on_pointer_up();
        // Release stops the drag from tracking further motion.
        let before = v.offset();
        v.on_pointer_drag(0.0, 0.0);
        assert_eq!(v.offset(), before);
    }

    #[test]
    fn viewport_round_trips_through_offset() {
        let mut v = view();
        v.on_wheel(30.0);
        assert_eq!(v.viewport_to_content(10.0, 10.0), (10.0, 40.0));
    }

    #[test]
    fn a_flick_keeps_gliding_after_the_gesture_stops() {
        let mut v = view();
        // Three frames of a fast flick, sampled at 60 Hz.
        for _ in 0..3 {
            v.on_wheel(20.0);
            v.advance(1.0 / 60.0);
        }
        v.on_wheel_end();
        let released_at = v.offset();
        assert!(v.is_animating());

        run(&mut v, 5);
        let glided = v.offset();
        assert!(
            glided > released_at,
            "momentum should carry past {released_at}, got {glided}"
        );

        // And it comes to rest on its own rather than coasting forever.
        run(&mut v, 240);
        assert_eq!(v.velocity, 0.0);
        let resting = v.offset();
        run(&mut v, 10);
        assert_eq!(v.offset(), resting);
    }

    #[test]
    fn an_interaction_alone_is_enough_to_keep_a_host_ticking() {
        // A host only ticks while `is_animating`, so anything that should
        // bring the bar up has to report as animating before it has moved or
        // drawn anything — otherwise the fade never starts.
        let mut v = view();
        assert!(!v.is_animating());
        v.on_wheel_discrete(10.0);
        assert!(v.is_animating());
        run(&mut v, 10);
        assert_eq!(v.state.scrollbar_opacity(), 1.0);

        let mut hovered = view();
        assert!(!hovered.is_animating());
        let thumb = ScrollRenderer::thumb_rect(&hovered.state).unwrap();
        hovered.on_pointer_move(thumb.center_x(), thumb.center_y());
        assert!(hovered.is_animating());
    }

    /// Feed the event pattern a real touchpad produces through Otto: about a
    /// point of scroll every four milliseconds, with the host ticking in
    /// between.
    fn touchpad_flick(v: &mut ScrollView, points_per_event: f32, events: usize) {
        for _ in 0..events {
            v.on_wheel(points_per_event);
            v.advance(0.004);
        }
        v.on_wheel_end();
    }

    #[test]
    fn a_touchpads_fine_grained_deltas_still_throw_the_content() {
        let mut v = view();
        touchpad_flick(&mut v, 1.2, 40);
        let released_at = v.offset();
        run(&mut v, 90);
        let glide = v.offset() - released_at;
        // Expressed against the tuning rather than as a magic number: a fling
        // released at `v` coasts until it drops below the cutoff, covering
        // `(v - cutoff) / k`. Asserting a fixed distance would make this test
        // a tripwire for every change of feel, when what it is actually here
        // to catch is a touchpad's small deltas failing to throw at all.
        let expected = (300.0 - min_velocity()) / -momentum_decay().ln();
        assert!(
            glide > expected * 0.7,
            "a 300pt/s flick should coast most of the {expected:.0}pt its speed \
             and friction allow, only got {glide} (released at {released_at})"
        );
    }

    #[test]
    fn a_slow_drag_does_not_throw_at_all() {
        let mut v = view();
        // Same events, ten times slower: a placement, not a flick.
        for _ in 0..20 {
            v.on_wheel(0.2);
            v.advance(0.02);
        }
        v.on_wheel_end();
        let released_at = v.offset();
        run(&mut v, 60);
        assert_eq!(v.offset(), released_at);
    }

    #[test]
    fn the_rubber_band_holds_while_the_gesture_keeps_pulling() {
        let mut v = view();
        // At the top, pulling further up a point at a time. The spring must
        // not fight the finger — each event has to leave the content further
        // out than the last, even with ticks in between.
        let mut previous = 0.0;
        for _ in 0..20 {
            v.on_wheel(-1.0);
            v.advance(0.004);
            let over = v.state.overscroll();
            assert!(
                over < previous,
                "overscroll went back from {previous} to {over} mid-gesture"
            );
            previous = over;
        }
        // Only on release does it spring home.
        v.on_wheel_end();
        run(&mut v, 90);
        assert_eq!(v.offset(), 0.0);
    }

    #[test]
    fn a_finger_resting_on_the_touchpad_throws_nothing() {
        let mut v = view();
        for _ in 0..10 {
            v.on_wheel(2.0);
            v.advance(0.004);
        }
        // The finger stops moving but stays down: no events, and no lift.
        let held_at = v.offset();
        run(&mut v, 20);
        assert_eq!(
            v.offset(),
            held_at,
            "the content should stay where it was held"
        );

        // Lifting after that pause throws next to nothing: the speed it was
        // moving at before it stopped is stale.
        v.on_wheel_end();
        let released_at = v.offset();
        run(&mut v, 60);
        assert!(
            (v.offset() - released_at).abs() < 8.0,
            "a stale gesture should not throw: moved {}",
            v.offset() - released_at
        );
    }

    #[test]
    fn a_gesture_that_never_ends_is_eventually_abandoned() {
        let mut v = view();
        // Pulled past the top, then silence — no lift is ever reported.
        v.on_wheel(-20.0);
        v.advance(0.004);
        assert!(v.state.overscroll() < 0.0);
        // The pull holds for as long as the gesture might still be live.
        run(&mut v, 20);
        assert!(v.state.overscroll() < 0.0);
        // Then it is abandoned, and springs home rather than staying stuck.
        run(&mut v, 120);
        assert_eq!(v.offset(), 0.0);
    }

    #[test]
    fn a_hosts_idle_ticks_do_not_inflate_the_measured_speed() {
        // A real host ticks far more often than the touchpad reports: the
        // event loop spins every half millisecond while deltas arrive every
        // four. If the empty ticks were left out of the window, most of the
        // elapsed time would go missing and a gentle scroll would be thrown at
        // the speed ceiling.
        let mut v = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 400.0));
        v.set_content_length(3000.0);
        v.set_wheel_scale(1.0);
        for _ in 0..60 {
            v.on_wheel(1.2);
            for _ in 0..8 {
                v.advance(0.0005);
            }
        }
        v.on_wheel_end();
        // 1.2pt every 4ms is 300pt/s. Allow for the window's smoothing, but
        // nothing like the 5000pt/s ceiling.
        assert!(
            (200.0..500.0).contains(&v.velocity),
            "measured {} for a 300pt/s gesture",
            v.velocity
        );
    }

    #[test]
    fn a_second_flick_builds_on_the_glide() {
        let mut v = view();
        touchpad_flick(&mut v, 1.2, 40);
        let first = v.velocity;
        assert!(first > 0.0);

        // Mid-glide, flick again the same way.
        run(&mut v, 6);
        touchpad_flick(&mut v, 1.2, 40);
        assert!(
            v.velocity > first * 1.5,
            "a second flick should add to the glide: {first} then {}",
            v.velocity
        );

        // Flicking the other way instead replaces it rather than cancelling
        // out to something in between.
        run(&mut v, 6);
        touchpad_flick(&mut v, -1.2, 40);
        assert!(v.velocity < 0.0, "reversing wins outright: {}", v.velocity);
    }

    #[test]
    fn taking_hold_of_a_glide_and_placing_it_stops_it() {
        let mut v = view();
        touchpad_flick(&mut v, 1.2, 40);
        run(&mut v, 6);
        // A slow gesture: too slow to throw, so it takes the glide with it.
        for _ in 0..10 {
            v.on_wheel(0.1);
            v.advance(0.02);
        }
        v.on_wheel_end();
        assert_eq!(v.velocity, 0.0);
        let placed = v.offset();
        run(&mut v, 30);
        assert_eq!(v.offset(), placed);
    }

    #[test]
    fn the_stretch_answers_to_the_hand_not_the_amplified_delta() {
        let mut amplified = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        amplified.set_content_length(1000.0);
        amplified.set_wheel_scale(3.0);
        let mut plain = view();

        // The same hand movement past the top must stretch the same distance
        // whatever the scroll amplification is.
        for _ in 0..10 {
            amplified.on_wheel(-2.0);
            plain.on_wheel(-2.0);
        }
        assert!(
            (amplified.state.overscroll() - plain.state.overscroll()).abs() < 0.01,
            "amplified {} vs plain {}",
            amplified.state.overscroll(),
            plain.state.overscroll()
        );

        // And inside the content it is amplified as usual.
        amplified.on_wheel_end();
        run(&mut amplified, 120);
        amplified.on_wheel(10.0);
        assert_eq!(amplified.offset(), 30.0);
    }

    #[test]
    fn releasing_a_stretch_follows_the_hand_back_without_a_jump() {
        let mut v = view();
        v.set_wheel_scale(3.0);
        for _ in 0..10 {
            v.on_wheel(-2.0);
        }
        let stretched = v.state.overscroll();
        assert!(stretched < 0.0);

        // Coming back releases the stretch before the content moves at all.
        v.on_wheel(2.0);
        let released = v.state.overscroll();
        assert!(
            released > stretched && released < 0.0,
            "should still be stretched, just less: {stretched} -> {released}"
        );
        assert_eq!(v.offset(), released, "the content itself has not moved yet");
    }

    #[test]
    fn a_discrete_wheel_click_does_not_fling() {
        let mut v = view();
        v.on_wheel_discrete(60.0);
        assert_eq!(v.offset(), 60.0);
        run(&mut v, 30);
        assert_eq!(v.offset(), 60.0);
    }

    #[test]
    fn dragging_past_the_top_rubber_bands_and_springs_back() {
        let mut v = view();
        v.on_content_drag_start(0.0, 100.0);
        // Pull 120 points of content down past the top edge.
        v.on_content_drag(0.0, 220.0);
        let over = v.state.overscroll();
        assert!(over < 0.0, "should hang past the top, got {over}");
        assert!(
            over > -120.0,
            "overscroll should be resisted, got {over} for a 120pt pull"
        );

        v.on_pointer_up();
        run(&mut v, 120);
        assert_eq!(v.state.overscroll(), 0.0);
        assert_eq!(v.offset(), 0.0);
    }

    #[test]
    fn resistance_grows_the_further_the_content_is_pulled() {
        let mut v = view();
        v.on_content_drag_start(0.0, 0.0);
        v.on_content_drag(0.0, 40.0);
        let first = v.state.overscroll().abs();
        v.on_content_drag(0.0, 80.0);
        let second = v.state.overscroll().abs() - first;
        assert!(
            second < first,
            "the same 40pt pull should move less when already {first} out"
        );
    }

    #[test]
    fn the_first_of_a_pull_is_nearly_free_and_the_last_of_it_is_not() {
        // The shape of the resistance, not just its direction: a stretch that
        // resists evenly from the first pixel reads as a snag, and one that
        // never firms up reads as slack. The content should follow the hand
        // at the start and run into a knee later.
        let scale = 100.0;
        let early = band(10.0, scale);
        assert!(
            early > 9.0,
            "the first 10pt against a 100pt scale should be nearly free, moved {early}"
        );

        // The rule the curve exists to express: how much a further point of
        // pull is worth decays exponentially in how far the content is
        // already out. One `scale` out, the hand buys 1/e of what it bought
        // at the end itself; two out, 1/e² — measured here as the marginal
        // movement per point of pull.
        let marginal = |shown: f32| {
            let pull = unband(shown, scale);
            (band(pull + 0.01, scale) - band(pull, scale)) / 0.01
        };
        for (out, expected) in [
            (0.0, 1.0),
            (scale, 1.0 / 1f32.exp()),
            (2.0 * scale, 1.0 / 2f32.exp()),
        ] {
            let got = marginal(out);
            assert!(
                (got - expected).abs() < 0.02,
                "at {out}pt out the hand should buy {expected} per point, bought {got}"
            );
        }

        // No wall: there is always more to give, it just costs more to take.
        assert!(band(100.0 * scale, scale) > band(10.0 * scale, scale));

        // And the whole curve inverts, which is what lets a release follow the
        // hand home rather than jumping.
        for pull in [1.0, 25.0, 100.0, 400.0] {
            let round_trip = unband(band(pull, scale), scale);
            assert!(
                (round_trip - pull).abs() < 0.01 * pull.max(1.0),
                "{pull} round-tripped to {round_trip}"
            );
        }
    }

    #[test]
    fn a_fling_into_the_bottom_bounces_and_settles_at_the_end() {
        let mut v = view();
        for _ in 0..6 {
            v.on_wheel(120.0);
            v.advance(1.0 / 60.0);
        }
        v.on_wheel_end();
        // Somewhere in the coast it overshoots the end.
        let mut bounced = false;
        for _ in 0..120 {
            v.advance(1.0 / 60.0);
            if v.state.overscroll() > 0.0 {
                bounced = true;
            }
        }
        assert!(bounced, "a fast fling should bounce off the bottom");
        assert_eq!(v.offset(), v.state.max_offset());
        assert_eq!(v.state.overscroll(), 0.0);
    }

    #[test]
    fn content_that_fits_still_stretches_and_springs_back() {
        // Nowhere to scroll, but a gesture must still be answered: the list
        // pulls off its end and comes back, rather than sitting there dead.
        let mut v = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        v.set_content_length(120.0);
        assert!(v.on_wheel(80.0));
        let over = v.state.overscroll();
        assert!(over > 0.0, "should hang past the end, got {over}");
        assert!(over < 80.0, "and be resisted, got {over} for an 80pt pull");

        // Either way, and home again on release with no bar ever drawn.
        v.on_wheel(-160.0);
        assert!(v.state.overscroll() < 0.0);
        v.on_wheel_end();
        run(&mut v, 120);
        assert_eq!(v.offset(), 0.0);
        assert_eq!(v.state.scrollbar_opacity(), 0.0);
    }

    #[test]
    fn a_wheel_click_on_content_that_fits_does_nothing() {
        // A notched click is a discrete instruction, not a gesture: with
        // nowhere to go there is nothing to show.
        let mut v = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        v.set_content_length(120.0);
        assert!(!v.on_wheel_discrete(80.0));
        assert_eq!(v.offset(), 0.0);
        run(&mut v, 30);
        assert_eq!(v.offset(), 0.0);
    }

    #[test]
    fn the_scrollbar_fades_in_while_scrolling_and_out_when_idle() {
        let mut v = view();
        assert_eq!(v.state.scrollbar_opacity(), 0.0);
        // A discrete click, so the bar's timing is not extended by a glide.
        v.on_wheel_discrete(20.0);
        run(&mut v, 10);
        assert_eq!(v.state.scrollbar_opacity(), 1.0);

        // Idle long enough for the hold to expire and the fade to finish.
        run(&mut v, 90);
        assert_eq!(v.state.scrollbar_opacity(), 0.0);
        assert!(!v.is_animating());
    }

    #[test]
    fn hovering_the_gutter_keeps_the_bar_up_and_widens_it() {
        let mut v = view();
        let thumb = ScrollRenderer::thumb_rect(&v.state).unwrap();
        assert!(v.on_pointer_move(thumb.center_x(), thumb.center_y()));
        run(&mut v, 60);
        assert_eq!(v.state.scrollbar_opacity(), 1.0);
        assert_eq!(v.state.scrollbar_expansion(), 1.0);

        assert!(v.on_pointer_move(0.0, 0.0));
        run(&mut v, 120);
        assert_eq!(v.state.scrollbar_expansion(), 0.0);
        assert_eq!(v.state.scrollbar_opacity(), 0.0);
    }

    #[test]
    fn a_press_catches_an_in_flight_fling() {
        let mut v = view();
        for _ in 0..3 {
            v.on_wheel(30.0);
            v.advance(1.0 / 60.0);
        }
        v.on_wheel_end();
        run(&mut v, 2);
        v.on_pointer_down(10.0, 10.0);
        let caught = v.offset();
        run(&mut v, 30);
        assert_eq!(v.offset(), caught);
    }
}

#[cfg(test)]
mod step_tests {
    use super::*;

    fn flung() -> ScrollView {
        let mut view = ScrollView::new(Rect::from_xywh(0.0, 0.0, 100.0, 400.0));
        view.state.set_content_length(500_000.0);
        view.velocity = 3000.0;
        view
    }

    /// Coasting is exponential decay integrated in closed form, so the same
    /// elapsed time must cover the same distance however it is chopped up.
    /// This only holds while `advance` integrates the time it is given — the
    /// moment it truncates its own step, a host with slow frames quietly
    /// loses travel that a host with fast frames keeps.
    #[test]
    fn one_slow_frame_travels_as_far_as_the_fast_frames_it_replaces() {
        let mut slow = flung();
        slow.advance(0.07);

        let mut fast = flung();
        for _ in 0..7 {
            fast.advance(0.01);
        }

        let diff = (slow.offset() - fast.offset()).abs();
        assert!(
            diff < 1.0,
            "70ms in one step travelled {:.1} but in seven steps {:.1} \
             — advance() is truncating an ordinary slow frame",
            slow.offset(),
            fast.offset()
        );
    }

    /// The guard still has to exist: a host that stopped ticking entirely
    /// must not resume by teleporting the content wherever the elapsed
    /// wall-clock time would put it.
    #[test]
    fn a_real_stall_is_still_capped() {
        let mut stalled = flung();
        stalled.advance(30.0);

        let mut capped = flung();
        capped.advance(MAX_STEP);

        assert_eq!(
            stalled.offset(),
            capped.offset(),
            "a 30 second gap should advance no further than MAX_STEP"
        );
    }
}
