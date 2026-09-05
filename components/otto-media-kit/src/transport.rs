//! The transport bar: play/pause, a scrubber, the clock, and mute.
//!
//! Canvas-pure, in the toolkit's draw/hit-test convention: [`layout`] places
//! the controls along the bottom of a rect, [`draw`] paints them from a
//! [`TransportState`], and [`TransportLayout::hit`] says what is under a
//! point. The host owns every piece of interaction state — whether a drag is
//! in progress, where it is — and passes it back in to draw.

use std::time::Duration;

use otto_kit::common::Renderable;
use otto_kit::components::label::Label;
use otto_kit::theme::Theme;
use otto_kit::typography::styles;
use skia_safe::{Canvas, Color, Contains, Paint, PathBuilder, Point, RRect, Rect};

/// The bar's height. Room for a control a finger can hit and a track a
/// pointer can.
pub const HEIGHT: f32 = 44.0;
const INSET: f32 = 12.0;
const BUTTON: f32 = 28.0;
const CLOCK_W: f32 = 52.0;
const TRACK_H: f32 = 4.0;
const KNOB: f32 = 12.0;

/// Where everything is.
#[derive(Debug, Clone, Copy)]
pub struct TransportLayout {
    pub bar: Rect,
    pub play: Rect,
    pub elapsed: Rect,
    /// The scrubber's track. The thumb rides along it; hits anywhere in the
    /// bar's height over it count as the track.
    pub track: Rect,
    pub remaining: Rect,
    pub mute: Rect,
}

/// What a point lands on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportHit {
    PlayPause,
    /// On the scrubber, at this fraction of the duration.
    Scrub(f32),
    Mute,
    /// On the bar, but on nothing in it.
    Bar,
}

/// What the bar shows.
#[derive(Debug, Clone, Copy)]
pub struct TransportState {
    pub playing: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub muted: bool,
    /// A drag in progress: the fraction under the pointer, drawn in place
    /// of the position.
    pub scrubbing: Option<f32>,
    /// Whether the bar is shown at all, 0 → 1. A host fades it out over a
    /// playing video the pointer has left.
    pub opacity: f32,
}

/// Lay the bar along the bottom of `bounds`.
pub fn layout(bounds: Rect) -> TransportLayout {
    let bar = Rect::from_ltrb(
        bounds.left,
        bounds.bottom - HEIGHT,
        bounds.right,
        bounds.bottom,
    );
    let cy = bar.center_y();
    let play = Rect::from_xywh(bar.left + INSET, cy - BUTTON / 2.0, BUTTON, BUTTON);
    let mute = Rect::from_xywh(
        bar.right - INSET - BUTTON,
        cy - BUTTON / 2.0,
        BUTTON,
        BUTTON,
    );
    let elapsed = Rect::from_xywh(play.right + 6.0, bar.top, CLOCK_W, HEIGHT);
    let remaining = Rect::from_xywh(mute.left - 6.0 - CLOCK_W, bar.top, CLOCK_W, HEIGHT);
    let track = Rect::from_ltrb(
        elapsed.right + 6.0,
        cy - TRACK_H / 2.0,
        (remaining.left - 6.0).max(elapsed.right + 6.0),
        cy + TRACK_H / 2.0,
    );
    TransportLayout {
        bar,
        play,
        elapsed,
        track,
        remaining,
        mute,
    }
}

impl TransportLayout {
    pub fn hit(&self, point: Point) -> Option<TransportHit> {
        if !self.bar.contains(point) {
            return None;
        }
        if self.play.with_outset((4.0, 4.0)).contains(point) {
            return Some(TransportHit::PlayPause);
        }
        if self.mute.with_outset((4.0, 4.0)).contains(point) {
            return Some(TransportHit::Mute);
        }
        let reach = Rect::from_ltrb(
            self.track.left,
            self.bar.top,
            self.track.right,
            self.bar.bottom,
        );
        if reach.contains(point) {
            return Some(TransportHit::Scrub(self.fraction_at(point.x)));
        }
        Some(TransportHit::Bar)
    }

    /// The fraction of the track `x` is at, clamped to it. Used while
    /// dragging, when the pointer may well have left the track.
    pub fn fraction_at(&self, x: f32) -> f32 {
        if self.track.width() <= 0.0 {
            return 0.0;
        }
        ((x - self.track.left) / self.track.width()).clamp(0.0, 1.0)
    }
}

/// Paint the bar.
pub fn draw(canvas: &Canvas, layout: &TransportLayout, state: &TransportState, theme: &Theme) {
    if state.opacity <= 0.0 {
        return;
    }
    let alpha = |color: Color| {
        Color::from_argb(
            (color.a() as f32 * state.opacity) as u8,
            color.r(),
            color.g(),
            color.b(),
        )
    };
    let mut paint = Paint::default();
    paint.set_anti_alias(true);

    // A dark scrim rather than the theme's material: the bar sits over
    // video, whose colours it cannot know, and white glyphs over a dark
    // band read on any of them.
    paint.set_color(alpha(Color::from_argb(0xA0, 0x10, 0x10, 0x12)));
    canvas.draw_rect(layout.bar, &paint);

    let ink = alpha(Color::WHITE);
    let dim = alpha(Color::from_argb(0x80, 0xFF, 0xFF, 0xFF));

    // Play / pause.
    paint.set_color(ink);
    if state.playing {
        let bar_w = 4.0;
        let gap = 4.0;
        let h = 14.0;
        let cx = layout.play.center_x();
        let cy = layout.play.center_y();
        for x in [cx - gap / 2.0 - bar_w, cx + gap / 2.0] {
            canvas.draw_round_rect(Rect::from_xywh(x, cy - h / 2.0, bar_w, h), 1.0, 1.0, &paint);
        }
    } else {
        let cx = layout.play.center_x() + 1.5;
        let cy = layout.play.center_y();
        let h = 15.0;
        let w = 13.0;
        let mut path = PathBuilder::new();
        path.move_to((cx - w / 2.0, cy - h / 2.0));
        path.line_to((cx + w / 2.0, cy));
        path.line_to((cx - w / 2.0, cy + h / 2.0));
        path.close();
        canvas.draw_path(&path.detach(), &paint);
    }

    // The clock either side of the track.
    let fraction = state.scrubbing.unwrap_or_else(|| match state.duration {
        Some(duration) if !duration.is_zero() => {
            (state.position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 0.0,
    });
    let shown = match (state.scrubbing, state.duration) {
        (Some(fraction), Some(duration)) => duration.mul_f32(fraction),
        _ => state.position,
    };
    Label::new(clock(shown))
        .with_style(styles::CAPTION_1)
        .with_color(ink)
        .centered_at(layout.elapsed.center_x(), layout.elapsed.center_y())
        .render(canvas);
    if let Some(duration) = state.duration {
        Label::new(format!("-{}", clock(duration.saturating_sub(shown))))
            .with_style(styles::CAPTION_1)
            .with_color(dim)
            .centered_at(layout.remaining.center_x(), layout.remaining.center_y())
            .render(canvas);
    }

    // The track, the part played, and the thumb.
    let radius = TRACK_H / 2.0;
    paint.set_color(dim);
    canvas.draw_rrect(RRect::new_rect_xy(layout.track, radius, radius), &paint);
    if layout.track.width() > 0.0 {
        let played = Rect::from_ltrb(
            layout.track.left,
            layout.track.top,
            layout.track.left + layout.track.width() * fraction,
            layout.track.bottom,
        );
        paint.set_color(alpha(theme.accent));
        canvas.draw_rrect(RRect::new_rect_xy(played, radius, radius), &paint);
        paint.set_color(ink);
        canvas.draw_circle((played.right, layout.track.center_y()), KNOB / 2.0, &paint);
    }

    // Mute: a speaker, with a stroke through it when muted.
    let m = layout.mute;
    let cx = m.center_x() - 3.0;
    let cy = m.center_y();
    paint.set_color(ink);
    let mut speaker = PathBuilder::new();
    speaker.move_to((cx - 7.0, cy - 3.5));
    speaker.line_to((cx - 3.0, cy - 3.5));
    speaker.line_to((cx + 2.0, cy - 8.0));
    speaker.line_to((cx + 2.0, cy + 8.0));
    speaker.line_to((cx - 3.0, cy + 3.5));
    speaker.line_to((cx - 7.0, cy + 3.5));
    speaker.close();
    canvas.draw_path(&speaker.detach(), &paint);
    let mut stroke = Paint::default();
    stroke.set_anti_alias(true);
    stroke.set_style(skia_safe::paint::Style::Stroke);
    stroke.set_stroke_width(2.0);
    stroke.set_stroke_cap(skia_safe::PaintCap::Round);
    if state.muted {
        stroke.set_color(ink);
        canvas.draw_line((cx + 5.0, cy - 4.0), (cx + 12.0, cy + 4.0), &stroke);
        canvas.draw_line((cx + 12.0, cy - 4.0), (cx + 5.0, cy + 4.0), &stroke);
    } else {
        stroke.set_color(ink);
        let mut arc = PathBuilder::new();
        arc.add_arc(Rect::from_xywh(cx - 1.0, cy - 5.0, 10.0, 10.0), -45.0, 90.0);
        canvas.draw_path(&arc.detach(), &stroke);
        stroke.set_color(dim);
        let mut outer = PathBuilder::new();
        outer.add_arc(Rect::from_xywh(cx - 3.0, cy - 9.0, 18.0, 18.0), -45.0, 90.0);
        canvas.draw_path(&outer.detach(), &stroke);
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_as_a_duration() {
        assert_eq!(clock(Duration::ZERO), "0:00");
        assert_eq!(clock(Duration::from_secs(61)), "1:01");
        assert_eq!(clock(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn the_track_answers_with_a_fraction() {
        let layout = layout(Rect::from_wh(600.0, 400.0));
        let mid = Point::new(layout.track.center_x(), layout.bar.center_y());
        match layout.hit(mid) {
            Some(TransportHit::Scrub(fraction)) => assert!((fraction - 0.5).abs() < 0.01),
            other => panic!("expected a scrub, got {other:?}"),
        }
        assert_eq!(layout.fraction_at(-100.0), 0.0);
        assert_eq!(layout.fraction_at(10_000.0), 1.0);
    }

    #[test]
    fn the_buttons_are_where_they_are_drawn() {
        let layout = layout(Rect::from_wh(600.0, 400.0));
        assert_eq!(
            layout.hit(Point::new(layout.play.center_x(), layout.play.center_y())),
            Some(TransportHit::PlayPause)
        );
        assert_eq!(
            layout.hit(Point::new(layout.mute.center_x(), layout.mute.center_y())),
            Some(TransportHit::Mute)
        );
        assert_eq!(layout.hit(Point::new(300.0, 10.0)), None);
    }
}
