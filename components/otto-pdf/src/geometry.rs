//! Turning a page's own bottom-up coordinates into the shown page's and back.
//!
//! A page's user space has y going up from wherever its boxes say. The page
//! as shown is its visible box (crop box within media box), turned clockwise
//! by `/Rotate`, measured from the top-left corner with y going down.

// Rust guideline compliant 2026-02-21

use crate::{Page, Rect};

/// Where a page's visible box sits in its user space, and how it is turned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PageGeometry {
    /// The visible box, in user space: `bottom < top`.
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
    /// Quarter turns clockwise, 0–3.
    pub quarter_turns: u8,
}

impl PageGeometry {
    /// A geometry from the visible box's corners (in any order) and `/Rotate`
    /// in degrees; a rotation that isn't a multiple of 90 counts as none.
    pub fn new(box_corners: (f32, f32, f32, f32), rotate_degrees: i32) -> Self {
        let (x0, y0, x1, y1) = box_corners;
        let quarter_turns = if rotate_degrees % 90 == 0 {
            (rotate_degrees / 90).rem_euclid(4) as u8
        } else {
            0
        };
        Self {
            left: x0.min(x1),
            bottom: y0.min(y1),
            right: x0.max(x1),
            top: y0.max(y1),
            quarter_turns,
        }
    }

    fn box_width(self) -> f32 {
        self.right - self.left
    }

    fn box_height(self) -> f32 {
        self.top - self.bottom
    }

    /// The page's size as shown.
    pub fn page(self) -> Page {
        if self.quarter_turns % 2 == 1 {
            Page {
                width: self.box_height(),
                height: self.box_width(),
            }
        } else {
            Page {
                width: self.box_width(),
                height: self.box_height(),
            }
        }
    }

    /// A user-space point on the shown page.
    pub fn to_shown(self, (x, y): (f32, f32)) -> (f32, f32) {
        // Unturned, from the box's top-left corner, y down.
        let (ux, uy) = (x - self.left, self.top - y);
        let (w, h) = (self.box_width(), self.box_height());
        match self.quarter_turns {
            1 => (h - uy, ux),
            2 => (w - ux, h - uy),
            3 => (uy, w - ux),
            _ => (ux, uy),
        }
    }

    /// A shown point in user space.
    pub fn to_user(self, (sx, sy): (f32, f32)) -> (f32, f32) {
        let (w, h) = (self.box_width(), self.box_height());
        let (ux, uy) = match self.quarter_turns {
            1 => (sy, h - sx),
            2 => (w - sx, h - sy),
            3 => (w - sy, sx),
            _ => (sx, sy),
        };
        (ux + self.left, self.top - uy)
    }

    /// A user-space rect, as `(left, bottom, right, top)` in any order, on
    /// the shown page.
    pub fn rect_to_shown(self, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> Rect {
        let a = self.to_shown((x0, y0));
        let b = self.to_shown((x1, y1));
        Rect::new(a.0, a.1, b.0, b.1)
    }

    /// A shown rect in user space, as `(left, bottom, right, top)`.
    pub fn rect_to_user(self, rect: Rect) -> (f32, f32, f32, f32) {
        let a = self.to_user((rect.left, rect.top));
        let b = self.to_user((rect.right, rect.bottom));
        (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }

    /// Where each shown corner of a quad comes from among the user-space
    /// corners, both in the order top-left, top-right, bottom-left,
    /// bottom-right.
    fn quad_order(self) -> [usize; 4] {
        match self.quarter_turns {
            // Turned clockwise, the user's bottom-left becomes the top-left.
            1 => [2, 0, 3, 1],
            2 => [3, 2, 1, 0],
            3 => [1, 3, 0, 2],
            _ => [0, 1, 2, 3],
        }
    }

    /// A user-space quad (top-left, top-right, bottom-left, bottom-right in
    /// user space) on the shown page, in the same corner order as shown.
    pub fn quad_to_shown(self, quad: [(f32, f32); 4]) -> [(f32, f32); 4] {
        self.quad_order().map(|from| self.to_shown(quad[from]))
    }

    /// A shown quad in user space, corners reordered so they are top-left,
    /// top-right, bottom-left, bottom-right in user space.
    pub fn quad_to_user(self, quad: [(f32, f32); 4]) -> [(f32, f32); 4] {
        let mut user = [(0.0, 0.0); 4];
        for (shown, from) in self.quad_order().into_iter().enumerate() {
            user[from] = self.to_user(quad[shown]);
        }
        user
    }

    /// The affine matrix `(a, b, c, d, e, f)` that maps the unit square onto
    /// a shown rect so that what is up in the unit square is up as shown:
    /// how an image is placed upright on a turned page.
    pub fn upright_matrix(self, rect: Rect) -> [f32; 6] {
        let bottom_left = self.to_user((rect.left, rect.bottom));
        let bottom_right = self.to_user((rect.right, rect.bottom));
        let top_left = self.to_user((rect.left, rect.top));
        [
            bottom_right.0 - bottom_left.0,
            bottom_right.1 - bottom_left.1,
            top_left.0 - bottom_left.0,
            top_left.1 - bottom_left.1,
            bottom_left.0,
            bottom_left.1,
        ]
    }
}

/// The smallest rect holding every point, grown by `margin` on each side.
pub(crate) fn bounds(points: impl IntoIterator<Item = (f32, f32)>, margin: f32) -> Option<Rect> {
    let mut points = points.into_iter();
    let first = points.next()?;
    let (mut left, mut top, mut right, mut bottom) = (first.0, first.1, first.0, first.1);
    for (x, y) in points {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(Rect {
        left: left - margin,
        top: top - margin,
        right: right + margin,
        bottom: bottom + margin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    /// A 200×100 box whose bottom-left is (10, 20), as a crop box might be.
    fn cropped(degrees: i32) -> PageGeometry {
        PageGeometry::new((10.0, 20.0, 210.0, 120.0), degrees)
    }

    #[test]
    fn unturned_page_flips_y_from_the_box_top() {
        let g = cropped(0);
        assert_eq!(
            g.page(),
            Page {
                width: 200.0,
                height: 100.0
            }
        );
        assert!(close(g.to_shown((10.0, 120.0)), (0.0, 0.0)));
        assert!(close(g.to_shown((210.0, 20.0)), (200.0, 100.0)));
        assert!(close(g.to_shown((60.0, 100.0)), (50.0, 20.0)));
    }

    #[test]
    fn quarter_turn_puts_the_box_top_left_at_the_shown_top_right() {
        let g = cropped(90);
        assert_eq!(
            g.page(),
            Page {
                width: 100.0,
                height: 200.0
            }
        );
        // The box's top-left corner is shown at the top right.
        assert!(close(g.to_shown((10.0, 120.0)), (100.0, 0.0)));
        // Its bottom-left at the top left.
        assert!(close(g.to_shown((10.0, 20.0)), (0.0, 0.0)));
        // Its top-right at the bottom right.
        assert!(close(g.to_shown((210.0, 120.0)), (100.0, 200.0)));
    }

    #[test]
    fn half_turn_mirrors_both_axes() {
        let g = cropped(180);
        assert_eq!(
            g.page(),
            Page {
                width: 200.0,
                height: 100.0
            }
        );
        assert!(close(g.to_shown((10.0, 120.0)), (200.0, 100.0)));
        assert!(close(g.to_shown((210.0, 20.0)), (0.0, 0.0)));
    }

    #[test]
    fn three_quarter_turn_puts_the_box_top_left_at_the_shown_bottom_left() {
        let g = cropped(270);
        assert_eq!(
            g.page(),
            Page {
                width: 100.0,
                height: 200.0
            }
        );
        assert!(close(g.to_shown((10.0, 120.0)), (0.0, 200.0)));
        assert!(close(g.to_shown((210.0, 120.0)), (0.0, 0.0)));
    }

    #[test]
    fn rotation_normalises_negative_and_large_angles() {
        assert_eq!(cropped(-90).quarter_turns, 3);
        assert_eq!(cropped(450).quarter_turns, 1);
        assert_eq!(cropped(45).quarter_turns, 0);
    }

    #[test]
    fn every_turn_round_trips() {
        for degrees in [0, 90, 180, 270] {
            let g = cropped(degrees);
            for point in [(10.0, 20.0), (37.5, 99.0), (210.0, 120.0), (-5.0, 300.0)] {
                let back = g.to_user(g.to_shown(point));
                assert!(
                    close(back, point),
                    "{degrees}°: {point:?} came back {back:?}"
                );
            }
        }
    }

    #[test]
    fn rects_round_trip_and_normalise() {
        for degrees in [0, 90, 180, 270] {
            let g = cropped(degrees);
            let user = (30.0, 40.0, 80.0, 60.0);
            let shown = g.rect_to_shown(user);
            assert!(shown.left <= shown.right && shown.top <= shown.bottom);
            let back = g.rect_to_user(shown);
            assert!(
                close((back.0, back.1), (user.0, user.1)),
                "{degrees}°: {back:?}"
            );
            assert!(
                close((back.2, back.3), (user.2, user.3)),
                "{degrees}°: {back:?}"
            );
        }
    }

    #[test]
    fn quads_keep_corner_order_on_the_shown_page() {
        // A line of text in user space: TL, TR, BL, BR.
        let quad = [(30.0, 60.0), (80.0, 60.0), (30.0, 40.0), (80.0, 40.0)];
        for degrees in [0, 90, 180, 270] {
            let g = cropped(degrees);
            let [tl, tr, bl, br] = g.quad_to_shown(quad);
            assert!(tl.0 < tr.0 && bl.0 < br.0, "{degrees}°: left before right");
            assert!(tl.1 < bl.1 && tr.1 < br.1, "{degrees}°: top above bottom");
            let back = g.quad_to_user([tl, tr, bl, br]);
            for (got, want) in back.iter().zip(quad) {
                assert!(close(*got, want), "{degrees}°: {back:?}");
            }
        }
    }

    #[test]
    fn upright_matrix_maps_the_unit_square_onto_the_rect_upright() {
        let rect = Rect::new(20.0, 10.0, 60.0, 30.0);
        for degrees in [0, 90, 180, 270] {
            let g = cropped(degrees);
            let [a, b, c, d, e, f] = g.upright_matrix(rect);
            let apply = |(u, v): (f32, f32)| g.to_shown((a * u + c * v + e, b * u + d * v + f));
            assert!(
                close(apply((0.0, 1.0)), (20.0, 10.0)),
                "{degrees}°: image top-left"
            );
            assert!(
                close(apply((1.0, 1.0)), (60.0, 10.0)),
                "{degrees}°: image top-right"
            );
            assert!(
                close(apply((0.0, 0.0)), (20.0, 30.0)),
                "{degrees}°: image bottom-left"
            );
        }
    }

    #[test]
    fn bounds_grow_by_the_margin() {
        let rect = bounds([(1.0, 5.0), (4.0, 2.0)], 1.0).unwrap();
        assert_eq!(rect, Rect::new(0.0, 1.0, 5.0, 6.0));
        assert!(bounds(std::iter::empty(), 1.0).is_none());
    }
}
