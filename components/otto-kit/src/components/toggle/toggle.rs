use std::time::{Duration, Instant};

use skia_safe::{BlurStyle, Canvas, Color, MaskFilter, Paint, PaintStyle, Point, RRect, Rect};

use crate::theme::Theme;

/// Size the visual design was tuned at. Callers may draw at other sizes —
/// the geometry below is all relative to `rect` — but this is the reference.
pub const WIDTH: f32 = 40.0;
pub const HEIGHT: f32 = 24.0;

/// Gap between the track's inner edge and the knob.
const KNOB_INSET: f32 = 2.0;

/// Interaction state, set by the caller from pointer tracking. Hover and
/// press need to read as visually distinct from normal, so a form gives back
/// some feedback before the value actually changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleInteraction {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

/// The knob's resting fraction along the track for a snapped (non-animating)
/// value: 0.0 sits at the off end, 1.0 at the on end. A caller that wants the
/// flip to animate drives [`Flip`] instead and passes its in-between fraction
/// to [`draw`] each frame; this is only the resting point.
pub fn knob_fraction_for(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

/// How long a flip takes. Short enough that the switch still feels like it
/// answers the click, long enough that the knob is seen to travel.
pub const FLIP_DURATION: Duration = Duration::from_millis(180);

/// One flip in progress: the knob sliding from where it was to where the new
/// value puts it, with the track colour crossing over with it.
///
/// A caller keeps one per switch that is currently moving, asks it for
/// [`fraction`](Flip::fraction) on every frame it paints, and drops it once
/// [`is_running`](Flip::is_running) goes false. Starting a flip from the
/// current fraction rather than from the old value is what makes a switch
/// clicked twice in quick succession turn back from where it had got to,
/// instead of jumping to the end first.
#[derive(Debug, Clone, Copy)]
pub struct Flip {
    from: f32,
    to: f32,
    started: Instant,
    duration: Duration,
}

impl Flip {
    /// Start a flip towards `on`, leaving from `current` — pass the fraction
    /// on screen right now, which is [`knob_fraction_for`] for a switch at
    /// rest or a running flip's own [`fraction`](Flip::fraction).
    pub fn start(current: f32, on: bool) -> Self {
        let to = knob_fraction_for(on);
        let from = current.clamp(0.0, 1.0);
        Self {
            from,
            to,
            started: Instant::now(),
            // A flip that only has part of the track left to cover takes
            // proportionally less time, so its speed matches a full one's.
            duration: FLIP_DURATION.mul_f32((to - from).abs().max(0.05)),
        }
    }

    /// Where the knob is now, eased so it starts and settles softly.
    pub fn fraction(&self) -> f32 {
        let t = if self.duration.is_zero() {
            1.0
        } else {
            (self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
        };
        // Smoothstep: zero velocity at both ends, no overshoot to undo.
        let eased = t * t * (3.0 - 2.0 * t);
        self.from + (self.to - self.from) * eased
    }

    /// Still moving? Once this is false the caller can stop asking for frames
    /// and go back to [`knob_fraction_for`].
    pub fn is_running(&self) -> bool {
        self.started.elapsed() < self.duration
    }
}

/// Is `(x, y)` within the control? Reads the same `rect` [`draw`] paints
/// into, so hit-testing cannot drift from what is on screen.
pub fn hit_test(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

/// Draw the switch into `rect`.
///
/// `knob_fraction` is where the knob sits along the track: 0.0 off, 1.0 on,
/// anything between for a flip in progress. It is the *only* input for the
/// value, colour included — the track crosses from the neutral fill to the
/// accent as the knob travels, so a caller that animates the fraction gets
/// the colour animated with it and one that does not gets a clean snap.
/// Pass [`knob_fraction_for`] for a plain switch, or [`Flip::fraction`] while
/// a flip is running.
pub fn draw(
    canvas: &Canvas,
    rect: Rect,
    knob_fraction: f32,
    interaction: ToggleInteraction,
    theme: &Theme,
) {
    let disabled = interaction == ToggleInteraction::Disabled;
    let knob_fraction = knob_fraction.clamp(0.0, 1.0);

    let mut track_color = lerp_color(theme.fill_secondary, theme.accent, knob_fraction);
    track_color = match interaction {
        ToggleInteraction::Hovered => lighten(track_color),
        ToggleInteraction::Pressed => darken(track_color),
        _ => track_color,
    };
    if disabled {
        track_color = scale_alpha(track_color, 0.5);
    }

    let track = RRect::new_rect_xy(rect, rect.height() / 2.0, rect.height() / 2.0);
    canvas.draw_rrect(track, &fill(track_color));

    let travel = rect.width() - rect.height();
    let knob_cx = rect.left + rect.height() / 2.0 + travel * knob_fraction;
    let knob_cy = rect.center_y();

    let base_r = rect.height() / 2.0 - KNOB_INSET;
    // A held-down knob reads as pressed by shrinking a hair, the way a real
    // switch gives under a finger — changing its colour would be confused
    // with the on/off state itself.
    let knob_r = if interaction == ToggleInteraction::Pressed {
        base_r * 0.92
    } else {
        base_r
    };

    if !disabled {
        let mut shadow = fill(Color::from_argb(0x33, 0, 0, 0));
        shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 1.5, false));
        canvas.draw_circle(Point::new(knob_cx, knob_cy + 0.5), knob_r, &shadow);
    }

    let knob_color = if disabled {
        scale_alpha(Color::WHITE, 0.6)
    } else {
        Color::WHITE
    };
    canvas.draw_circle(Point::new(knob_cx, knob_cy), knob_r, &fill(knob_color));

    if interaction == ToggleInteraction::Hovered {
        // A thin ring around the knob — the same restrained hover treatment
        // used elsewhere in the toolkit, not a colour change.
        canvas.draw_circle(
            Point::new(knob_cx, knob_cy),
            knob_r + 1.0,
            &stroke(theme.fill_primary, 1.0),
        );
    }
}

fn fill(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    paint
}

fn stroke(color: Color, width: f32) -> Paint {
    let mut paint = fill(color);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width);
    paint
}

fn scale_alpha(color: Color, factor: f32) -> Color {
    let a = (color.a() as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color::from_argb(a, color.r(), color.g(), color.b())
}

fn lighten(color: Color) -> Color {
    mix_toward(color, 255, 0.12)
}

fn darken(color: Color) -> Color {
    mix_toward(color, 0, 0.12)
}

/// Blend `a` into `b`, premultiplied-free: both channels and alpha move, so
/// a translucent off-fill crossing to an opaque accent does not darken
/// through the middle of the flip.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::from_argb(
        mix(a.a(), b.a()),
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
    )
}

fn mix_toward(color: Color, target: u8, t: f32) -> Color {
    let mix = |c: u8| (c as f32 + (target as f32 - c as f32) * t).round() as u8;
    Color::from_argb(color.a(), mix(color.r()), mix(color.g()), mix(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_matches_the_drawn_rect() {
        let rect = Rect::from_xywh(10.0, 10.0, WIDTH, HEIGHT);
        assert!(hit_test(rect, 15.0, 15.0));
        assert!(!hit_test(rect, 5.0, 15.0));
        assert!(!hit_test(rect, 15.0, 100.0));
    }

    #[test]
    fn knob_fraction_matches_on_off() {
        assert_eq!(knob_fraction_for(true), 1.0);
        assert_eq!(knob_fraction_for(false), 0.0);
    }

    #[test]
    fn a_flip_leaves_from_where_the_knob_already_is() {
        // Reversed mid-travel: it starts from 0.4, not from the old end.
        let flip = Flip::start(0.4, false);
        assert!(flip.fraction() <= 0.4);
        assert!(flip.fraction() >= 0.0);
    }

    #[test]
    fn a_flip_ends_on_the_value_it_was_started_for() {
        let flip = Flip::start(0.0, true);
        std::thread::sleep(FLIP_DURATION + Duration::from_millis(20));
        assert!(!flip.is_running());
        assert_eq!(flip.fraction(), 1.0);
    }

    #[test]
    fn the_track_reads_neutral_off_and_accent_on() {
        let theme = Theme::light();
        assert_eq!(
            lerp_color(theme.fill_secondary, theme.accent, 0.0),
            theme.fill_secondary
        );
        assert_eq!(
            lerp_color(theme.fill_secondary, theme.accent, 1.0),
            theme.accent
        );
    }
}
