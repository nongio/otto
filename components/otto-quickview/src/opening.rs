//! The opening animation's geometry.
//!
//! The preview grows out of the file. `anchor` is the rect of the item the user
//! pressed space on, and the card animates from there to its resting rect —
//! which is what makes the preview feel attached to the file rather than
//! summoned on top of it.
//!
//! The *motion* is run by the compositor through surface-style transactions,
//! the same way otto-launcher's card animates, so it costs this process no
//! frames — which matters more here than for the launcher, because this process
//! is also supervising a decode. What lives in this module is the geometry the
//! transaction is given, and the curve, so both can be reasoned about and
//! tested without a display.

use std::time::Duration;

/// How long the card takes to arrive. It does not fade: the card is opaque
/// from its first frame and only its geometry animates, so what the eye
/// follows is one thing moving rather than a shape resolving out of nothing.
///
/// Overridable at run time — `OTTO_QUICKVIEW_OPEN_MS` — because the only way
/// to settle how an entrance feels is to watch it at several speeds.
pub fn geometry_in() -> Duration {
    static V: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_ms("OTTO_QUICKVIEW_OPEN_MS").unwrap_or(Duration::from_millis(300)))
}

fn env_ms(name: &str) -> Option<Duration> {
    let ms: u64 = std::env::var(name).ok()?.parse().ok()?;
    Some(Duration::from_millis(ms.clamp(1, 10_000)))
}

/// And how it leaves — quicker, and without a bounce. There is nothing to
/// settle into on the way out, and an exit that lingers reads as the card
/// being reluctant rather than as the card going home.
///
/// Overridable at run time — `OTTO_QUICKVIEW_CLOSE_MS`.
pub fn geometry_out() -> Duration {
    static V: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_ms("OTTO_QUICKVIEW_CLOSE_MS").unwrap_or(Duration::from_millis(180)))
}

/// How far the entrance overshoots. Deliberately smaller than the launcher's
/// 0.35: this surface is much larger, and the same overshoot on a large card
/// stops reading as life and starts reading as wobble.
///
/// Overridable at run time — `OTTO_QUICKVIEW_BOUNCE`. Zero is a clean ease
/// with no overshoot at all.
pub fn bounce() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("OTTO_QUICKVIEW_BOUNCE")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0.07f32)
            .clamp(0.0, 1.0)
    })
}

pub const IN_PLACE_SCALE: f32 = 0.96;

/// A rectangle in logical screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// An anchor is usable only if it has area. Callers are asked to send an
    /// empty rect rather than a stale one, so this is the documented signal for
    /// "there is nothing to open from", not a guess.
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}

/// The start of the entrance, expressed the way a surface-style transaction
/// wants it: a scale and a translation applied about the card's centre.
///
/// Scaling rather than animating width and height is what keeps this
/// compositor-side. The card's buffer is drawn once at its resting size and the
/// compositor transforms it — so the entrance never re-lays-out the content and
/// never asks the previewer for another frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entrance {
    pub scale_x: f32,
    pub scale_y: f32,
    /// Where the card's centre starts, relative to where it will rest.
    pub offset_x: f32,
    pub offset_y: f32,
}

/// Compute the entrance for a card resting at `resting`, opening from `anchor`.
///
/// With no usable anchor this degrades to the launcher's in-place swell rather
/// than to nothing: an invocation with no item still deserves an entrance.
pub fn entrance(anchor: Rect, resting: Rect) -> Entrance {
    if anchor.is_empty() || resting.width <= 0.0 || resting.height <= 0.0 {
        return Entrance {
            scale_x: IN_PLACE_SCALE,
            scale_y: IN_PLACE_SCALE,
            offset_x: 0.0,
            offset_y: 0.0,
        };
    }

    // One scale for both axes, from the anchor's larger relative dimension.
    // Scaling the axes independently would stretch the card's content on the
    // way in — a row is wide and short, and a card matching that aspect would
    // arrive squashed before snapping square.
    let scale = (anchor.width / resting.width)
        .max(anchor.height / resting.height)
        .clamp(0.04, 1.0);

    let (anchor_cx, anchor_cy) = anchor.center();
    let (resting_cx, resting_cy) = resting.center();

    Entrance {
        scale_x: scale,
        scale_y: scale,
        offset_x: anchor_cx - resting_cx,
        offset_y: anchor_cy - resting_cy,
    }
}

/// Interpolate the entrance for a filmstrip or a test. `t` runs 0 → 1.
///
/// The compositor runs the real curve; this reproduces it closely enough to
/// reason about and to look at.
pub fn sample(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let start = entrance(anchor, resting);
    place(start, resting, spring(t.clamp(0.0, 1.0), bounce()))
}

/// The same geometry, run backwards: the card returning to the item it came
/// out of. `t` runs 0 → 1, resting → anchor.
///
/// Not `sample` with a reversed `t`. The entrance overshoots, and an overshoot
/// played backwards puts the bounce at the *start* of the exit, which reads as
/// the card being knocked rather than dismissed. This eases instead, so the
/// card leaves the way it would if you had never let it settle.
pub fn sample_out(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let start = entrance(anchor, resting);
    place(start, resting, 1.0 - smoothstep(t.clamp(0.0, 1.0)))
}

/// Place the card partway along its path, where `eased` is 0 at the anchor and
/// 1 at rest. Shared so the way in and the way out cannot drift apart: only
/// the curve that feeds this differs between them.
fn place(start: Entrance, resting: Rect, eased: f32) -> Rect {
    let scale = start.scale_x + (1.0 - start.scale_x) * eased;
    let offset_x = start.offset_x * (1.0 - eased);
    let offset_y = start.offset_y * (1.0 - eased);

    let width = resting.width * scale;
    let height = resting.height * scale;
    let (cx, cy) = resting.center();
    Rect::new(
        cx + offset_x - width / 2.0,
        cy + offset_y - height / 2.0,
        width,
        height,
    )
}

/// Ease in and out, settling at both ends and overshooting at neither.
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A spring that settles at 1.0, overshooting by roughly `bounce`.
///
/// Critically shaped rather than physically simulated: it has to finish inside
/// the transaction's duration, so the bounce is a shape rather than a length.
fn spring(t: f32, bounce: f32) -> f32 {
    if t >= 1.0 {
        return 1.0;
    }
    let decay = (-7.0 * t).exp();
    let frequency = std::f32::consts::PI * (1.0 + bounce * 6.0);
    1.0 - decay * (frequency * t).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESTING: Rect = Rect {
        x: 400.0,
        y: 200.0,
        width: 800.0,
        height: 600.0,
    };

    #[test]
    fn opens_from_the_item_not_the_centre() {
        // A row near the top-left of a file list.
        let anchor = Rect::new(40.0, 120.0, 220.0, 24.0);
        let start = entrance(anchor, RESTING);
        // The card starts small, and off toward the item.
        assert!(start.scale_x < 0.3, "scale was {}", start.scale_x);
        assert!(start.offset_x < 0.0 && start.offset_y < 0.0);
    }

    /// The exit is the entrance's mirror in where it starts and ends, but not
    /// in how it gets there: it must not overshoot, or the card lurches
    /// outwards before going home.
    #[test]
    fn the_exit_returns_to_the_item_without_a_bounce() {
        let anchor = Rect::new(40.0, 120.0, 24.0, 24.0);

        let at_rest = sample_out(anchor, RESTING, 0.0);
        assert!((at_rest.width - RESTING.width).abs() < 1.0, "{at_rest:?}");

        let gone = sample_out(anchor, RESTING, 1.0);
        let (cx, cy) = gone.center();
        let (ax, ay) = anchor.center();
        assert!((cx - ax).abs() < 1.0 && (cy - ay).abs() < 1.0, "{gone:?}");
        assert!(gone.width < RESTING.width * 0.1, "{gone:?}");

        // Never larger than its resting size at any point along the way.
        for step in 0..=20 {
            let rect = sample_out(anchor, RESTING, step as f32 / 20.0);
            assert!(rect.width <= RESTING.width + 0.5, "at {step}: {rect:?}");
        }
    }

    #[test]
    fn no_anchor_falls_back_to_an_in_place_swell() {
        let start = entrance(Rect::new(0.0, 0.0, 0.0, 0.0), RESTING);
        assert_eq!(start.scale_x, IN_PLACE_SCALE);
        assert_eq!((start.offset_x, start.offset_y), (0.0, 0.0));
    }

    #[test]
    fn both_axes_scale_together_so_content_never_arrives_squashed() {
        // A wide, short row: independent axis scaling would stretch it.
        let start = entrance(Rect::new(0.0, 0.0, 400.0, 20.0), RESTING);
        assert_eq!(start.scale_x, start.scale_y);
    }

    #[test]
    fn the_card_ends_exactly_at_its_resting_rect() {
        let anchor = Rect::new(40.0, 120.0, 220.0, 24.0);
        let rect = sample(anchor, RESTING, 1.0);
        assert!((rect.x - RESTING.x).abs() < 0.01, "{rect:?}");
        assert!((rect.y - RESTING.y).abs() < 0.01, "{rect:?}");
        assert!((rect.width - RESTING.width).abs() < 0.01, "{rect:?}");
    }

    /// The card is legible for the whole entrance because it never fades —
    /// what moves is geometry alone, and at a third of the way in there is
    /// still geometry left to run.
    #[test]
    fn the_card_is_still_arriving_partway_through() {
        let anchor = Rect::new(40.0, 120.0, 220.0, 24.0);
        let rect = sample(anchor, RESTING, 0.35);
        assert!(
            rect.width < RESTING.width * 1.05,
            "still arriving at t=0.35"
        );
    }

    #[test]
    fn a_tiny_anchor_does_not_collapse_the_card_to_nothing() {
        // A 1×1 anchor would otherwise scale to ~0.001 and read as a flash.
        let start = entrance(Rect::new(10.0, 10.0, 1.0, 1.0), RESTING);
        assert!(start.scale_x >= 0.04);
    }
}
