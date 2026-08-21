use skia_safe::Rect;

/// Which way a scroll view scrolls.
///
/// Everything about scrolling is one-dimensional — an offset, a velocity, a
/// spring — so the axis only decides which side of the viewport is its
/// *length*, which pointer coordinate moves along it, and which edge the
/// scrollbar hugs. Both variants behave identically otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Scrolls up and down; the scrollbar sits on the viewport's right edge.
    #[default]
    Vertical,
    /// Scrolls left and right; the scrollbar sits along the bottom edge.
    Horizontal,
}

impl Axis {
    /// The viewport's extent along this axis.
    pub fn length(self, rect: Rect) -> f32 {
        match self {
            Axis::Vertical => rect.height(),
            Axis::Horizontal => rect.width(),
        }
    }

    /// The component of a point that moves along this axis.
    pub fn coord(self, x: f32, y: f32) -> f32 {
        match self {
            Axis::Vertical => y,
            Axis::Horizontal => x,
        }
    }
}

/// Model for a scroll view: the viewport it is clipped to, the length of the
/// content painted inside it along the scrolling [`Axis`], and how far that
/// content is scrolled.
///
/// `offset` is normally kept inside `[0, max_offset()]` — [`Self::set_offset`]
/// and every layout setter re-clamp, so a host can feed it raw wheel deltas
/// or a stale content height without producing a scroll position that points
/// at nothing. The one exception is [`Self::set_offset_overscrolled`], which
/// lets [`ScrollView`](super::ScrollView) pull the content past an end while
/// a rubber-band bounce is in flight.
///
/// The scrollbar's own presentation — how faded in it is, how far it has
/// widened under the pointer — lives here too, because
/// [`ScrollRenderer`](super::ScrollRenderer) is stateless and draws from a
/// state alone. Those two values are animated by
/// [`ScrollView::advance`](super::ScrollView::advance).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollState {
    axis: Axis,
    viewport: Rect,
    content_length: f32,
    offset: f32,
    scrollbar_opacity: f32,
    scrollbar_expansion: f32,
}

impl ScrollState {
    /// A vertical scroll state — the common case.
    pub fn new(viewport: Rect) -> Self {
        Self::on_axis(Axis::Vertical, viewport)
    }

    pub fn on_axis(axis: Axis, viewport: Rect) -> Self {
        Self {
            axis,
            viewport,
            content_length: 0.0,
            offset: 0.0,
            scrollbar_opacity: 0.0,
            scrollbar_expansion: 0.0,
        }
    }

    pub fn axis(&self) -> Axis {
        self.axis
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// The viewport's extent along the scrolling axis — its height for a
    /// vertical view, its width for a horizontal one.
    pub fn viewport_length(&self) -> f32 {
        self.axis.length(self.viewport)
    }

    /// Resize/reposition the viewport (e.g. a window resize). Re-clamps the
    /// offset since a taller viewport can make more of the tail unreachable.
    ///
    /// A no-op when the viewport has not actually moved. Hosts re-assert
    /// their layout every frame, and re-clamping on each of those would erase
    /// a rubber band as fast as a gesture could build one.
    pub fn set_viewport(&mut self, viewport: Rect) {
        if viewport == self.viewport {
            return;
        }
        self.viewport = viewport;
        self.clamp();
    }

    pub fn content_length(&self) -> f32 {
        self.content_length
    }

    /// Set the content's total extent along the scrolling axis, e.g.
    /// recomputed from the same layout code that draws it. Re-clamps the
    /// offset.
    ///
    /// A no-op when the length is unchanged, for the same reason
    /// [`Self::set_viewport`] is.
    pub fn set_content_length(&mut self, content_length: f32) {
        let content_length = content_length.max(0.0);
        if content_length == self.content_length {
            return;
        }
        self.content_length = content_length;
        self.clamp();
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// The furthest the content can be scrolled: zero once it fits the
    /// viewport.
    pub fn max_offset(&self) -> f32 {
        (self.content_length - self.viewport_length()).max(0.0)
    }

    /// There is more content than the viewport can show, so a scrollbar has
    /// something to represent.
    pub fn scrollable(&self) -> bool {
        self.max_offset() > 0.0
    }

    /// Set the offset directly, clamped to range. Returns whether it
    /// actually changed, so a host knows whether a redraw is needed.
    pub fn set_offset(&mut self, offset: f32) -> bool {
        self.set_offset_overscrolled(offset.clamp(0.0, self.max_offset()))
    }

    /// Set the offset without clamping it to the ends, so the content can be
    /// pulled past its top or bottom while a rubber-band bounce is running.
    /// Only [`ScrollView`](super::ScrollView)'s physics should use this —
    /// nothing else guarantees the offset comes back into range.
    pub fn set_offset_overscrolled(&mut self, offset: f32) -> bool {
        let changed = offset != self.offset;
        self.offset = offset;
        changed
    }

    /// Scroll by a relative amount (wheel delta, drag delta). Returns
    /// whether the offset changed.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        self.set_offset(self.offset + delta)
    }

    /// How far the content is pulled past an end, signed: negative before the
    /// start, positive past the end, zero in the ordinary case.
    pub fn overscroll(&self) -> f32 {
        if self.offset < 0.0 {
            self.offset
        } else {
            (self.offset - self.max_offset()).max(0.0)
        }
    }

    /// How faded in the scrollbar is, `0.0`–`1.0`. Overlay scrollbars are
    /// hidden until something scrolls and fade back out once it stops.
    pub fn scrollbar_opacity(&self) -> f32 {
        self.scrollbar_opacity
    }

    pub fn set_scrollbar_opacity(&mut self, opacity: f32) {
        self.scrollbar_opacity = opacity.clamp(0.0, 1.0);
    }

    /// How far the scrollbar has widened towards its grabbable size,
    /// `0.0`–`1.0`: `0` while it is a thin indicator, `1` once the pointer is
    /// over it or dragging it.
    pub fn scrollbar_expansion(&self) -> f32 {
        self.scrollbar_expansion
    }

    pub fn set_scrollbar_expansion(&mut self, expansion: f32) {
        self.scrollbar_expansion = expansion.clamp(0.0, 1.0);
    }

    fn clamp(&mut self) {
        self.offset = self.offset.clamp(0.0, self.max_offset());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ScrollState {
        let mut s = ScrollState::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        s.set_content_length(500.0);
        s
    }

    #[test]
    fn offset_clamps_to_max() {
        let mut s = state();
        assert_eq!(s.max_offset(), 300.0);
        assert!(s.set_offset(9999.0));
        assert_eq!(s.offset(), 300.0);
        assert!(s.set_offset(-50.0));
        assert_eq!(s.offset(), 0.0);
    }

    #[test]
    fn content_shorter_than_viewport_is_not_scrollable() {
        let mut s = ScrollState::new(Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        s.set_content_length(50.0);
        assert!(!s.scrollable());
        assert_eq!(s.max_offset(), 0.0);
    }

    #[test]
    fn shrinking_content_reclamps_a_scrolled_offset() {
        let mut s = state();
        s.set_offset(300.0);
        s.set_content_length(220.0);
        assert_eq!(s.max_offset(), 20.0);
        assert_eq!(s.offset(), 20.0);
    }

    #[test]
    fn set_offset_reports_whether_it_changed() {
        let mut s = state();
        assert!(s.set_offset(100.0));
        assert!(!s.set_offset(100.0));
    }

    #[test]
    fn overscroll_is_signed_and_zero_in_range() {
        let mut s = state();
        assert_eq!(s.overscroll(), 0.0);
        s.set_offset_overscrolled(-20.0);
        assert_eq!(s.overscroll(), -20.0);
        s.set_offset_overscrolled(340.0);
        assert_eq!(s.overscroll(), 40.0);
    }

    #[test]
    fn re_asserting_the_same_layout_leaves_an_overscroll_alone() {
        // Hosts set the viewport and content height every frame from the
        // same measurements; that must not disturb a bounce in progress.
        let mut s = state();
        s.set_offset_overscrolled(-25.0);
        s.set_viewport(s.viewport());
        s.set_content_length(s.content_length());
        assert_eq!(s.offset(), -25.0);
    }

    #[test]
    fn a_horizontal_state_measures_its_viewport_across() {
        let mut s = ScrollState::on_axis(Axis::Horizontal, Rect::from_xywh(0.0, 0.0, 100.0, 200.0));
        s.set_content_length(400.0);
        assert_eq!(s.viewport_length(), 100.0);
        assert_eq!(s.max_offset(), 300.0);
    }

    #[test]
    fn a_layout_change_pulls_an_overscrolled_offset_back_in() {
        let mut s = state();
        s.set_offset_overscrolled(-40.0);
        s.set_viewport(Rect::from_xywh(0.0, 0.0, 100.0, 180.0));
        assert_eq!(s.offset(), 0.0);
    }
}
