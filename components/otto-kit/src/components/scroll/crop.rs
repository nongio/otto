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

    Some(Crop {
        position: (left as i32, top as i32),
        source: Rect::from_ltrb(source_left, source_top, source_right, source_bottom),
        destination: ((right - left) as i32, (bottom - top) as i32),
    })
}

/// Crop `surface` to `window` and say what of it shows, and where.
///
/// The arguments are [`crop`]'s. A surface the window does not reach is
/// unmapped, and `None` comes back; a caller showing it again later has to
/// draw it, since only a buffer maps it. Otherwise the viewport is set for
/// the surface's next commit and the crop comes back, for the caller to place
/// the surface at its position. Without `wp_viewporter` nothing is cropped:
/// the crop that comes back is the whole surface, where it stands.
pub(crate) fn show(
    surface: &mut SubsurfaceSurface,
    content: Size,
    extent: Rect,
    window: Rect,
    scale: f32,
) -> Option<Crop> {
    let Some(shown) = crop(content, extent, window, scale) else {
        surface.unmap();
        return None;
    };
    let view = (!shown.is_whole(content)).then_some((shown.source, shown.destination));
    if surface.set_view(view) {
        return Some(shown);
    }
    Some(Crop {
        position: (extent.left.round() as i32, extent.top.round() as i32),
        source: Rect::from_size(content),
        destination: (content.width as i32, content.height as i32),
    })
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
}
