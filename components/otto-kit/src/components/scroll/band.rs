//! Bookkeeping for compositor-side scroll cropping.
//!
//! A scrollable pane is a fixed parent surface that clips its children, and
//! the scrolled content lives in a child subsurface whose buffer is taller
//! than the viewport — a *band* of content. Scrolling within that band costs
//! nothing but a subsurface position change: no repaint, no new buffer, no
//! round trip. The client only paints again when the scroll gets close enough
//! to an edge of the band that the next few frames could run off it.
//!
//! This module is only the decision logic for that: where the band sits in
//! content space, whether it still covers what is about to be visible, and
//! where the child surface belongs for a given scroll offset. It knows
//! nothing about Wayland, buffers or drawing.
//!
//! Everything here is in the same one-dimensional content space as
//! [`ScrollState`](super::ScrollState): `0` is the top of the content and
//! `content_height` is its bottom, measured in points.

use skia_safe::Rect;

/// How much extra content to render beyond the viewport, per side, as a
/// fraction of the viewport height. At `0.5` a band is twice the viewport
/// tall: one viewport of visible content plus half a viewport of slack above
/// and below.
///
/// This is the whole memory-versus-refill trade. A bigger ratio means the
/// scroll can travel further before it runs out of rendered content, so
/// refills — a client repaint and a new buffer attach — become rarer, but
/// every scrollable pane permanently holds a proportionally larger buffer,
/// and each refill costs proportionally more to paint. Half a viewport per
/// side is the point where an ordinary wheel notch or a short drag never
/// refills at all, while the buffer stays small enough that having several
/// scroll panes open at once is unremarkable.
const OVERDRAW_RATIO: f32 = 0.5;
/// Floor on the total overdraw, in points, whatever the viewport's size.
///
/// Sizing the slack purely as a fraction of the viewport falls apart on a
/// small pane: a 200pt viewport would carry 100pt of slack per side, which a
/// fling crosses in under a tenth of a second, and the band ends up refilling
/// several times a second. A fling is measured in hundreds of points however
/// tall the pane is, so the slack has an absolute floor too. The cost is
/// buffer memory — this is the knob to turn if that ever matters.
const MIN_OVERDRAW: f32 = 600.0;

/// How much rendered content must remain beyond the viewport edge before a
/// refill is requested, as a fraction of the viewport height.
///
/// The point of a margin is that a refill is not instantaneous: the client
/// has to be woken, paint the new band, and commit it, and the compositor has
/// to present the result. Waiting until the band edge is actually exposed
/// would show a blank strip for every one of those frames. Refilling while
/// there is still a margin of painted content ahead buys that time.
const REFILL_MARGIN_RATIO: f32 = 0.25;

/// Floor on the refill margin, in points, so small panes still get a margin
/// measured against real scroll speed rather than against their own size.
///
/// The scroll view coasts at up to ~5000 pt/s. A repaint-and-commit round
/// trip is realistically about four frames at 60 Hz, or ~67 ms, during which
/// content at that speed travels ~335 points. 340 covers that, and covers it
/// with room to spare on any real fling, since a fling is at its peak only at
/// the instant the finger leaves and decays from there.
const MIN_REFILL_MARGIN: f32 = 340.0;

/// Ceiling on the refill margin, as a fraction of the overdraw a band has on
/// one side when it is built at rest.
///
/// The margin cannot exceed what the geometry can supply — a band that fails
/// its own coverage check the moment it is built would refill on every frame
/// forever. It has to stay comfortably *under* that, too: the difference
/// between the slack a band is built with and the margin at which it gives up
/// is precisely the distance the scroll may travel for free, so a margin
/// pressed right up against the slack would refill on the first pixel of
/// movement. Six tenths leaves the other four as free travel.
///
/// On a short pane this cap wins over [`MIN_REFILL_MARGIN`], and it should:
/// no amount of wishing produces protection a small buffer does not contain.
/// Such a pane refills more often, which is the correct trade — its buffer is
/// cheap to repaint.
const MAX_MARGIN_SHARE: f32 = 0.6;

/// The largest share of the total overdraw that may be spent ahead of the
/// scroll. `0.9` leaves a tenth behind even at full speed.
///
/// Nothing is gained by going to a full `1.0`: a fling can be interrupted and
/// reversed at any moment, and a band with literally nothing behind it would
/// expose blank content on the first backwards pixel.
const MAX_LEAD_SHARE: f32 = 0.9;

/// The speed at which the directional bias reaches [`MAX_LEAD_SHARE`], in
/// points per second. Below it the bias ramps linearly from an even split.
///
/// Chosen well under the scroll view's 5000 pt/s ceiling so that any gesture
/// which reads as a fling rather than a drag is already fully biased.
const BIAS_FULL_SPEED: f32 = 2000.0;

/// Above this speed, in points per second, the scroll counts as travelling
/// and only the leading edge of the band is checked for coverage.
///
/// A trailing edge is moving away from the viewport; it cannot be exposed
/// while the motion continues, and refilling for it would throw away the
/// bias that the direction of travel just earned. When the fling decays past
/// this threshold the next check looks at both edges again and recentres.
const TRAVELLING_SPEED: f32 = 60.0;

/// What the band has to cover: where the content is scrolled to, how much of
/// it is visible, how much there is in total, and how fast it is moving.
///
/// `velocity` is in points per second and positive means scrolling *down* —
/// the same sign convention as a scroll offset increasing, so at positive
/// velocity the bottom of the band is the leading edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandView {
    /// Content coordinate at the top of the viewport. May sit outside
    /// `0..=content_height - viewport_height` during a rubber-band overscroll.
    pub offset: f32,
    pub viewport_height: f32,
    pub content_height: f32,
    /// Points per second, positive scrolling down.
    pub velocity: f32,
}

impl BandView {
    pub fn new(offset: f32, viewport_height: f32, content_height: f32, velocity: f32) -> Self {
        Self {
            offset,
            viewport_height,
            content_height,
            velocity,
        }
    }

    fn viewport_height(&self) -> f32 {
        self.viewport_height.max(0.0)
    }

    fn content_height(&self) -> f32 {
        self.content_height.max(0.0)
    }

    /// The viewport top used for coverage decisions, pulled back inside the
    /// content.
    ///
    /// During an overscroll the offset runs past an end, but there is no
    /// content out there to render — the gap is the rubber band, and it is
    /// meant to be empty. Judging coverage against the clamped position keeps
    /// a bounce from demanding a band that cannot exist.
    fn clamped_offset(&self) -> f32 {
        let max = (self.content_height() - self.viewport_height()).max(0.0);
        self.offset.clamp(0.0, max)
    }

    /// Total slack to distribute around the viewport.
    fn overdraw(&self) -> f32 {
        (2.0 * OVERDRAW_RATIO * self.viewport_height()).max(MIN_OVERDRAW)
    }

    /// How much of [`Self::overdraw`] is spent ahead of the scroll, ramping
    /// from an even split at rest to [`MAX_LEAD_SHARE`] at
    /// [`BIAS_FULL_SPEED`].
    fn lead_share(&self) -> f32 {
        let t = (self.velocity.abs() / BIAS_FULL_SPEED).min(1.0);
        0.5 + (MAX_LEAD_SHARE - 0.5) * t
    }

    /// Rendered content required beyond each viewport edge before a refill.
    fn refill_margin(&self) -> f32 {
        let vh = self.viewport_height();
        // Derived from the same slack the band is actually built with, floor
        // included — computing it from the ratio again would leave the margin
        // and the band disagreeing about how much room there is.
        let at_rest_slack = self.overdraw() / 2.0;
        (vh * REFILL_MARGIN_RATIO)
            .max(MIN_REFILL_MARGIN)
            .min(at_rest_slack * MAX_MARGIN_SHARE)
    }
}

/// A rendered band of content: the slice of content space that currently
/// exists in the child subsurface's buffer.
///
/// Only the vertical extent is modelled. The band's width is always the pane
/// width and is decided by layout, not by scrolling, so carrying it here
/// would be a field that every caller sets to the same thing and no rule in
/// this module ever reads. Callers that want a rect for the buffer ask for
/// one with [`Band::rect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    origin: f32,
    height: f32,
}

impl Band {
    pub fn new(origin: f32, height: f32) -> Self {
        Self {
            origin,
            height: height.max(0.0),
        }
    }

    /// A band holding nothing — the state before the first paint.
    pub fn empty() -> Self {
        Self {
            origin: 0.0,
            height: 0.0,
        }
    }

    /// Content coordinate of the band's first rendered row.
    pub fn origin(&self) -> f32 {
        self.origin
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Content coordinate just past the band's last rendered row.
    pub fn end(&self) -> f32 {
        self.origin + self.height
    }

    pub fn is_empty(&self) -> bool {
        self.height <= 0.0
    }

    /// The band as a rect in content space, for sizing and positioning the
    /// buffer that backs it.
    pub fn rect(&self, x: f32, width: f32) -> Rect {
        Rect::from_xywh(x, self.origin, width, self.height)
    }

    /// Where the child subsurface's top belongs, relative to the top of the
    /// parent's clip box, for a given scroll offset.
    ///
    /// This is the entire cost of scrolling within a band: the content does
    /// not move inside the buffer, the buffer moves under the clip.
    ///
    /// During a rubber-band overscroll the offset is allowed outside
    /// `0..=content_height - viewport_height`, so this can put the surface
    /// partly or even entirely outside the clip box. That is correct and is
    /// exactly what makes the overscroll visible: the content slides away
    /// from the edge it was pulled past and the empty space behind it is the
    /// rubber band.
    pub fn surface_top(&self, offset: f32) -> f32 {
        self.origin - offset
    }

    /// The band that should be rendered for this view right now, sized to the
    /// viewport plus [`OVERDRAW_RATIO`] on each side, biased in the direction
    /// of travel and clamped to the content.
    pub fn for_view(view: &BandView) -> Self {
        let vh = view.viewport_height();
        let ch = view.content_height();

        // Content that fits within one band is simply rendered whole: there
        // is nothing to scroll off, and a band that already holds everything
        // can never need refilling.
        let height = (vh + view.overdraw()).min(ch);
        if height >= ch {
            return Self::new(0.0, ch);
        }

        let overdraw = view.overdraw();
        let ahead = overdraw * view.lead_share();
        let behind = overdraw - ahead;

        let offset = view.clamped_offset();
        // Positive velocity scrolls down, so the bottom is the leading edge
        // and the slack above the viewport is the part left behind.
        let origin = if view.velocity >= 0.0 {
            offset - behind
        } else {
            offset - ahead
        };

        Self::new(origin.clamp(0.0, ch - height), height)
    }

    /// Whether this band still covers the viewport with enough margin that a
    /// refill can wait.
    ///
    /// A side pinned against a content end needs no margin — there is no
    /// content out there to have rendered — so the requirement on each side
    /// is the smaller of [`BandView::refill_margin`] and whatever content
    /// actually remains on that side.
    pub fn covers(&self, view: &BandView) -> bool {
        let vh = view.viewport_height();
        let ch = view.content_height();
        if self.origin <= 0.0 && self.end() >= ch {
            return true;
        }

        let top = view.clamped_offset();
        let bottom = top + vh;
        let margin = view.refill_margin();

        let above_ok = (top - self.origin) >= margin.min(top);
        let below_ok = (self.end() - bottom) >= margin.min((ch - bottom).max(0.0));

        if view.velocity > TRAVELLING_SPEED {
            below_ok
        } else if view.velocity < -TRAVELLING_SPEED {
            above_ok
        } else {
            above_ok && below_ok
        }
    }

    /// The band to render next, or `None` when this one still suffices.
    ///
    /// The intended use is one call per frame while a scroll is live: `None`
    /// on nearly every frame, and on the rare `Some` the client repaints its
    /// content into a buffer covering the returned band and attaches it.
    pub fn refill(&self, view: &BandView) -> Option<Self> {
        if !self.is_empty() && self.covers(view) {
            return None;
        }
        Some(Self::for_view(view))
    }
}

impl Default for Band {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: f32 = 600.0;
    const CONTENT: f32 = 10_000.0;

    /// A view onto tall content, scrolled to `offset`, at rest.
    fn at(offset: f32) -> BandView {
        BandView::new(offset, VIEWPORT, CONTENT, 0.0)
    }

    fn moving(offset: f32, velocity: f32) -> BandView {
        BandView::new(offset, VIEWPORT, CONTENT, velocity)
    }

    /// The bias is computed through a ratio, so exact equality on its results
    /// is a test of the f32 rounding rather than of the rule.
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.01, "{a} != {b}");
    }

    #[test]
    fn a_band_is_the_viewport_plus_overdraw_on_each_side() {
        let band = Band::for_view(&at(2000.0));
        assert_eq!(band.height(), VIEWPORT * (1.0 + 2.0 * OVERDRAW_RATIO));
        // At rest the slack is split evenly.
        assert_eq!(band.origin(), 2000.0 - VIEWPORT * OVERDRAW_RATIO);
    }

    #[test]
    fn scrolling_inside_the_band_needs_no_refill() {
        let band = Band::for_view(&at(2000.0));
        // A wheel notch or two, well short of the margin.
        assert!(band.refill(&at(2020.0)).is_none());
        assert!(band.refill(&at(1980.0)).is_none());
        assert!(band.refill(&at(2000.0)).is_none());
    }

    #[test]
    fn approaching_the_margin_triggers_a_refill() {
        let band = Band::for_view(&at(2000.0));
        let margin = at(2000.0).refill_margin();
        // Rendered content below the viewport is band.end() - (offset + vh);
        // push the offset until that dips under the margin.
        let last_ok = band.end() - VIEWPORT - margin;
        assert!(band.refill(&at(last_ok)).is_none());
        let next = band
            .refill(&at(last_ok + 1.0))
            .expect("crossing the margin must refill");
        assert!(next != band);
        // And the refill happened while there was still painted content
        // beyond the viewport — nothing blank was ever exposed.
        assert!(band.end() > last_ok + 1.0 + VIEWPORT);
    }

    #[test]
    fn an_empty_band_always_refills() {
        assert!(Band::empty().refill(&at(0.0)).is_some());
    }

    #[test]
    fn a_fresh_band_never_immediately_asks_for_another() {
        // At rest, mid-content, at both ends, and mid-fling: building a band
        // and then testing it against the very view it was built for must
        // never loop.
        for view in [
            at(0.0),
            at(2000.0),
            at(CONTENT - VIEWPORT),
            moving(2000.0, 5000.0),
            moving(2000.0, -5000.0),
            BandView::new(100.0, 120.0, CONTENT, 0.0),
        ] {
            let band = Band::for_view(&view);
            assert!(
                band.refill(&view).is_none(),
                "band {band:?} refilled itself for {view:?}"
            );
        }
    }

    #[test]
    fn a_downward_fling_puts_more_of_the_band_below_the_viewport() {
        let view = moving(2000.0, BIAS_FULL_SPEED);
        let band = Band::for_view(&view);
        let above = view.offset - band.origin();
        let below = band.end() - (view.offset + VIEWPORT);
        assert!(below > above, "below {below} should exceed above {above}");
        close(below, band.height() - VIEWPORT - above);
        close(above, view.overdraw() * (1.0 - MAX_LEAD_SHARE));
    }

    #[test]
    fn an_upward_fling_puts_more_of_the_band_above_the_viewport() {
        let view = moving(2000.0, -BIAS_FULL_SPEED);
        let band = Band::for_view(&view);
        let above = view.offset - band.origin();
        let below = band.end() - (view.offset + VIEWPORT);
        assert!(above > below, "above {above} should exceed below {below}");
        close(below, view.overdraw() * (1.0 - MAX_LEAD_SHARE));
    }

    #[test]
    fn the_bias_ramps_with_speed() {
        let above = |v: f32| {
            let view = moving(2000.0, v);
            view.offset - Band::for_view(&view).origin()
        };
        // Faster downward travel leaves progressively less behind.
        assert!(above(0.0) > above(BIAS_FULL_SPEED / 2.0));
        assert!(above(BIAS_FULL_SPEED / 2.0) > above(BIAS_FULL_SPEED));
        // And it saturates rather than inverting at absurd speeds.
        assert_eq!(above(BIAS_FULL_SPEED), above(5000.0));
    }

    #[test]
    fn the_band_never_extends_past_the_content_ends() {
        // Hard against the top, flinging up: the bias wants slack above 0.
        let band = Band::for_view(&moving(0.0, -5000.0));
        assert_eq!(band.origin(), 0.0);
        assert!(band.end() <= CONTENT);

        // Hard against the bottom, flinging down.
        let band = Band::for_view(&moving(CONTENT - VIEWPORT, 5000.0));
        assert_eq!(band.end(), CONTENT);
        assert!(band.origin() >= 0.0);
    }

    #[test]
    fn content_shorter_than_a_band_is_one_band_covering_everything() {
        // Taller than the viewport but shorter than viewport + overdraw.
        let short = VIEWPORT * 1.5;
        let view = BandView::new(200.0, VIEWPORT, short, 3000.0);
        let band = Band::for_view(&view);
        assert_eq!(band.origin(), 0.0);
        assert_eq!(band.height(), short);
        // Nothing can ever scroll off it, at any offset or speed.
        for offset in [0.0, 100.0, short - VIEWPORT] {
            assert!(band
                .refill(&BandView::new(offset, VIEWPORT, short, 5000.0))
                .is_none());
        }

        // And content shorter than the viewport itself.
        let tiny = BandView::new(0.0, VIEWPORT, 40.0, 0.0);
        let band = Band::for_view(&tiny);
        assert_eq!(band.origin(), 0.0);
        assert_eq!(band.height(), 40.0);
    }

    #[test]
    fn surface_top_is_exact_at_the_band_origin() {
        let band = Band::new(1700.0, 1200.0);
        assert_eq!(band.surface_top(1700.0), 0.0);
        // Scrolling down by n moves the surface up by exactly n.
        assert_eq!(band.surface_top(1750.0), -50.0);
        assert_eq!(band.surface_top(1650.0), 50.0);
    }

    #[test]
    fn overscroll_pushes_the_surface_outside_the_clip_box() {
        // Pulled past the top: the band starts at content 0, and a negative
        // offset slides it down, opening the rubber band above it.
        let band = Band::for_view(&at(0.0));
        assert_eq!(band.origin(), 0.0);
        assert_eq!(band.surface_top(-80.0), 80.0);

        // Pulled past the bottom: the band's tail rises above the clip box's
        // bottom edge, opening the rubber band below it.
        let bottom = CONTENT - VIEWPORT;
        let band = Band::for_view(&at(bottom));
        let top = band.surface_top(bottom + 80.0);
        assert_eq!(top + band.height(), band.end() - bottom - 80.0);
        assert!(top + band.height() < VIEWPORT);
    }

    #[test]
    fn an_overscroll_at_a_content_end_does_not_demand_a_refill() {
        // There is no content past the end to have rendered, so a bounce must
        // not spin the client on repaints it cannot satisfy.
        let band = Band::for_view(&at(0.0));
        assert!(band
            .refill(&BandView::new(-150.0, VIEWPORT, CONTENT, -4000.0))
            .is_none());

        let bottom = CONTENT - VIEWPORT;
        let band = Band::for_view(&at(bottom));
        assert!(band
            .refill(&BandView::new(bottom + 150.0, VIEWPORT, CONTENT, 4000.0))
            .is_none());
    }

    #[test]
    fn a_trailing_edge_is_ignored_while_flinging_and_checked_once_stopped() {
        // Build a band biased hard downwards, then ask about it at the same
        // place. While the fling continues the thin slack above is fine.
        let fling = moving(2000.0, 4000.0);
        let band = Band::for_view(&fling);
        assert!(band.refill(&fling).is_none());
        // Once it settles, the same band is judged on both edges and the
        // starved trailing side earns a recentring refill.
        let stopped = at(2000.0);
        let next = band.refill(&stopped).expect("a settled band recentres");
        assert!(next.origin() < band.origin());
    }

    #[test]
    fn a_refill_at_full_speed_covers_several_frames_of_travel() {
        // Assumption under test: the scroll view caps a fling at 5000 pt/s,
        // and a repaint-and-commit round trip costs about four frames at
        // 60 Hz. The new band must hold more than the 5000 * 4/60 ≈ 333 pt
        // the content covers in that time — and it must do so ahead of the
        // viewport, where the blank would otherwise appear.
        const MAX_FLING: f32 = 5000.0;
        const ROUND_TRIP: f32 = 4.0 / 60.0;
        let travel = MAX_FLING * ROUND_TRIP;

        let view = moving(2000.0, MAX_FLING);
        let band = Band::for_view(&view);
        let ahead = band.end() - (view.offset + VIEWPORT);
        assert!(
            ahead >= travel,
            "only {ahead} pt rendered ahead, need {travel}"
        );

        // The same for an upward fling.
        let view = moving(2000.0, -MAX_FLING);
        let band = Band::for_view(&view);
        let ahead = view.offset - band.origin();
        assert!(
            ahead >= travel,
            "only {ahead} pt rendered ahead, need {travel}"
        );

        // On a pane large enough for MIN_REFILL_MARGIN to fit inside the
        // overdraw, the margin alone — the content still painted ahead at the
        // instant the refill is asked for — already covers the round trip, so
        // the refill lands before the edge is anywhere near exposed.
        let tall = BandView::new(2000.0, 1200.0, CONTENT, MAX_FLING);
        assert!(
            tall.refill_margin() >= travel,
            "margin {} does not cover {travel}",
            tall.refill_margin()
        );
    }

    #[test]
    fn the_band_rect_carries_the_layout_width() {
        let band = Band::new(100.0, 900.0);
        assert_eq!(
            band.rect(12.0, 340.0),
            Rect::from_xywh(12.0, 100.0, 340.0, 900.0)
        );
    }
}
