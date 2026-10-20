//! Where a surface shows through a window, as `wl_subsurface` and
//! `wp_viewport` can say it.
//!
//! A compositor does not clip a subsurface to its parent, so a pane clips its
//! children itself: each one is cropped to the part of it the pane shows, with
//! a viewport source rect over its buffer and a destination the size of that
//! part, and placed where that part sits. A surface the window does not reach
//! at all is unmapped instead, since a viewport cannot be empty.
//!
//! The arithmetic is here, apart from any surface, so it can be tested.
//! Subsurface positions and viewport destinations are whole points; the
//! fraction a scroll stands at goes into the source rect, which is exact.

use skia_safe::{Rect, Size};

use crate::surfaces::SubsurfaceSurface;

/// One surface's share of a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Crop {
    /// Where the shown part sits in the parent, in whole points.
    pub position: (i32, i32),
    /// The part of the surface shown, in the surface's own points.
    pub source: Rect,
    /// The size it is shown at, in whole points.
    pub destination: (i32, i32),
}

impl Crop {
    /// Whether this shows the whole surface at its own size, which needs no
    /// viewport at all.
    pub(crate) fn is_whole(&self, content: Size) -> bool {
        self.source == Rect::from_size(content)
            && self.destination == (content.width as i32, content.height as i32)
    }
}

/// The part of a surface showing through `window`.
///
/// `content` is the surface's size in its own points: what its buffer holds.
/// `extent` is where the whole of it would be drawn in the parent, which may
/// stretch it (a one-pixel fill) or put it at a fraction of a point (a band
/// mid-scroll). `window` is what the parent shows, in the same coordinates.
/// `scale` is the output's: an unstretched source starts on a physical pixel,
/// so the buffer lands on the screen without being resampled.
///
/// `None` when the window shows none of it.
pub(crate) fn crop(content: Size, extent: Rect, window: Rect, scale: f32) -> Option<Crop> {
    if content.is_empty() || extent.is_empty() {
        return None;
    }
    let mut shown = extent;
    if !shown.intersect(window) {
        return None;
    }
    let left = shown.left.round();
    let top = shown.top.round();
    let right = shown.right.round();
    let bottom = shown.bottom.round();
    if right <= left || bottom <= top {
        return None;
    }

    // From the parent back into the surface's own points.
    let ratio_x = content.width / extent.width();
    let ratio_y = content.height / extent.height();
    let unstretched = ratio_x == 1.0 && ratio_y == 1.0;
    let snap = |points: f32| {
        if unstretched && scale > 0.0 {
            (points * scale).round() / scale
        } else {
            points
        }
    };
    let source_left = snap((left - extent.left) * ratio_x).clamp(0.0, content.width);
    let source_top = snap((top - extent.top) * ratio_y).clamp(0.0, content.height);
    let source_right = (source_left + (right - left) * ratio_x).min(content.width);
    let source_bottom = (source_top + (bottom - top) * ratio_y).min(content.height);
    if source_right <= source_left || source_bottom <= source_top {
        return None;
    }
    let (source_left, source_right) = wire_span(source_left, source_right, content.width);
    let (source_top, source_bottom) = wire_span(source_top, source_bottom, content.height);

    Some(Crop {
        position: (left as i32, top as i32),
        source: Rect::from_ltrb(source_left, source_top, source_right, source_bottom),
        destination: ((right - left) as i32, (bottom - top) as i32),
    })
}

/// The narrowest source span the protocol can carry, with room to spare.
///
/// A viewport source goes out as `wl_fixed`, in 1/256 of a point, truncated,
/// and a compositor ends the client for a source of zero width or height. A
/// sliver of a stretched surface gets that thin: one point of a one-point fill
/// spread over 360 is 1/360 of its pixel. Two units leave a margin for the
/// rounding of `f32` subtraction.
const MIN_SOURCE_SPAN: f32 = 2.0 / 256.0;

/// `start..end` widened inward to [`MIN_SOURCE_SPAN`], staying in `0..len`.
///
/// Only a stretched surface's source gets this thin, and there a sliver of a
/// buffer point looks the same as a little more of it.
fn wire_span(start: f32, end: f32, len: f32) -> (f32, f32) {
    if end - start >= MIN_SOURCE_SPAN {
        return (start, end);
    }
    let start = (end - MIN_SOURCE_SPAN).max(0.0);
    (start, (start + MIN_SOURCE_SPAN).min(len))
}

/// What [`show`] asks of a surface: a viewport over its buffer, a place in
/// its parent, and leaving the screen. [`SubsurfaceSurface`] is the one that
/// reaches the compositor.
pub(crate) trait Placeable {
    /// See [`SubsurfaceSurface::set_view`].
    fn set_view(&mut self, view: Option<(Rect, (i32, i32))>) -> bool;
    /// See [`SubsurfaceSurface::set_position`].
    fn set_position(&self, x: i32, y: i32);
    /// See [`SubsurfaceSurface::unmap`].
    fn unmap(&self);
}

impl Placeable for SubsurfaceSurface {
    fn set_view(&mut self, view: Option<(Rect, (i32, i32))>) -> bool {
        SubsurfaceSurface::set_view(self, view)
    }

    fn set_position(&self, x: i32, y: i32) {
        SubsurfaceSurface::set_position(self, x, y);
    }

    fn unmap(&self) {
        SubsurfaceSurface::unmap(self);
    }
}

/// Crop `surface` to `window`, put it where the crop says, and say what of it
/// shows.
///
/// `content`, `extent`, `window` and `scale` are [`crop`]'s; `parent_origin`
/// is where the surface's parent starts, in the coordinates of `extent` and
/// `window`. A surface the window does not reach is unmapped, and `None`
/// comes back; a caller showing it again later has to draw it, since only a
/// buffer maps it. Otherwise the viewport and the position are sent, and the
/// caller commits the surface (a paint or a bare commit) and then its parent.
///
/// Both are sent before that commit. The protocol makes a position the
/// parent's state, but Smithay compositors, Otto among them, take it with the
/// surface's own next commit: a position sent after the paint that maps a
/// surface leaves it where it was until it next changes.
///
/// Without `wp_viewporter` nothing is cropped: the crop that comes back is
/// the whole surface, where it stands.
pub(crate) fn show(
    surface: &mut impl Placeable,
    content: Size,
    extent: Rect,
    window: Rect,
    scale: f32,
    parent_origin: (i32, i32),
) -> Option<Crop> {
    let Some(mut shown) = crop(content, extent, window, scale) else {
        surface.unmap();
        return None;
    };
    let view = (!shown.is_whole(content)).then_some((shown.source, shown.destination));
    if !surface.set_view(view) {
        shown = Crop {
            position: (extent.left.round() as i32, extent.top.round() as i32),
            source: Rect::from_size(content),
            destination: (content.width as i32, content.height as i32),
        };
    }
    surface.set_position(
        shown.position.0 - parent_origin.0,
        shown.position.1 - parent_origin.1,
    );
    Some(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 200.0,
        bottom: 300.0,
    };

    #[test]
    fn a_surface_inside_the_window_is_shown_whole() {
        let content = Size::new(100.0, 50.0);
        let crop = crop(
            content,
            Rect::from_xywh(10.0, 20.0, 100.0, 50.0),
            WINDOW,
            1.0,
        )
        .unwrap();
        assert_eq!(crop.position, (10, 20));
        assert!(crop.is_whole(content));
    }

    #[test]
    fn a_band_longer_than_the_window_shows_only_the_window() {
        // 600 points of band, scrolled 150 points in.
        let content = Size::new(200.0, 600.0);
        let crop = crop(
            content,
            Rect::from_xywh(0.0, -150.0, 200.0, 600.0),
            WINDOW,
            1.0,
        )
        .unwrap();
        assert_eq!(crop.position, (0, 0));
        assert_eq!(crop.destination, (200, 300));
        assert_eq!(crop.source, Rect::from_xywh(0.0, 150.0, 200.0, 300.0));
    }

    #[test]
    fn a_fractional_scroll_goes_into_the_source_on_the_pixel_grid() {
        let content = Size::new(200.0, 600.0);
        let crop = crop(
            content,
            Rect::from_xywh(0.0, -150.3, 200.0, 600.0),
            WINDOW,
            1.25,
        )
        .unwrap();
        assert_eq!(crop.position, (0, 0));
        assert_eq!(crop.destination, (200, 300));
        // 150.3 points is 187.875 pixels at 1.25: the nearest pixel is 188.
        assert_eq!(crop.source.top, 188.0 / 1.25);
    }

    #[test]
    fn a_band_ending_inside_the_window_stops_where_it_ends() {
        // The content runs out 100 points into the pane.
        let content = Size::new(200.0, 400.0);
        let crop = crop(
            content,
            Rect::from_xywh(0.0, -300.0, 200.0, 400.0),
            WINDOW,
            1.0,
        )
        .unwrap();
        assert_eq!(crop.position, (0, 0));
        assert_eq!(crop.destination, (200, 100));
        assert_eq!(crop.source, Rect::from_xywh(0.0, 300.0, 200.0, 100.0));
    }

    #[test]
    fn a_stretched_pixel_is_cropped_in_proportion() {
        // A one-point fill stretched over 100 by 400, half of it in the window.
        let content = Size::new(1.0, 1.0);
        let crop = crop(
            content,
            Rect::from_xywh(50.0, 100.0, 100.0, 400.0),
            WINDOW,
            2.0,
        )
        .unwrap();
        assert_eq!(crop.position, (50, 100));
        assert_eq!(crop.destination, (100, 200));
        assert_eq!(crop.source, Rect::from_xywh(0.0, 0.0, 1.0, 0.5));
    }

    #[test]
    fn a_surface_outside_the_window_is_not_shown() {
        let content = Size::new(100.0, 100.0);
        assert_eq!(
            crop(
                content,
                Rect::from_xywh(250.0, 0.0, 100.0, 100.0),
                WINDOW,
                1.0
            ),
            None
        );
        // Less than half a point inside rounds to nothing.
        assert_eq!(
            crop(
                content,
                Rect::from_xywh(199.6, 0.0, 100.0, 100.0),
                WINDOW,
                1.0
            ),
            None
        );
    }

    #[test]
    fn the_source_never_reaches_past_the_buffer() {
        // A column partly left of the window, at a fraction of a point.
        let content = Size::new(220.0, 300.0);
        let crop = crop(
            content,
            Rect::from_xywh(-30.4, 0.0, 220.0, 300.0),
            WINDOW,
            1.0,
        )
        .unwrap();
        assert!(crop.source.left >= 0.0);
        assert!(crop.source.right <= content.width);
        assert_eq!(crop.position, (0, 0));
        assert_eq!(crop.destination, (190, 300));
    }

    #[test]
    fn an_empty_surface_is_not_shown() {
        assert_eq!(
            crop(
                Size::new(0.0, 10.0),
                Rect::from_xywh(0.0, 0.0, 10.0, 10.0),
                WINDOW,
                1.0
            ),
            None
        );
        assert_eq!(
            crop(
                Size::new(10.0, 10.0),
                Rect::from_xywh(0.0, 0.0, 0.0, 10.0),
                WINDOW,
                1.0
            ),
            None
        );
    }

    #[test]
    fn a_sliver_of_a_stretched_fill_still_reaches_the_wire() {
        // One point of a one-point fill spread over 360: its source is
        // 1/360 of the pixel, which wl_fixed would send as zero.
        let wire = |points: f32| (f64::from(points) * 256.0) as i32;
        let crop = crop(
            Size::new(1.0, 1.0),
            Rect::from_xywh(-359.4995, 0.0, 360.0, 300.0),
            Rect::from_xywh(0.0, 0.0, 360.0, 300.0),
            1.0,
        )
        .unwrap();
        assert_eq!(crop.destination, (1, 300));
        assert!(wire(crop.source.width()) > 0, "{:?}", crop.source);
        assert!(wire(crop.source.height()) > 0, "{:?}", crop.source);
        assert!(crop.source.left >= 0.0 && crop.source.right <= 1.0);
    }

    /// A surface on a compositor that takes a subsurface's position with the
    /// surface's own commit, as Smithay does.
    #[derive(Default)]
    struct Surface {
        sent: std::cell::Cell<Option<(i32, i32)>>,
        placed: Option<(i32, i32)>,
    }

    impl Surface {
        fn commit(&mut self) {
            if let Some(position) = self.sent.take() {
                self.placed = Some(position);
            }
        }
    }

    impl Placeable for Surface {
        fn set_view(&mut self, _view: Option<(Rect, (i32, i32))>) -> bool {
            true
        }

        fn set_position(&self, x: i32, y: i32) {
            self.sent.set(Some((x, y)));
        }

        fn unmap(&self) {}
    }

    #[test]
    fn a_shown_surface_lands_with_its_own_next_commit() {
        // A highlight in a pane whose clip starts 30 points into the window:
        // its paint is the commit that maps it, and nothing commits it again
        // until the selection moves.
        let mut surface = Surface::default();
        let shown = show(
            &mut surface,
            Size::new(1.0, 1.0),
            Rect::from_xywh(46.0, 6.0, 40.0, 40.0),
            WINDOW,
            2.0,
            (30, 0),
        )
        .unwrap();
        surface.commit();
        assert_eq!(shown.position, (46, 6));
        assert_eq!(surface.placed, Some((16, 6)));
    }
}
