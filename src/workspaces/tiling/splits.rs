//! Split arithmetic: where the draggable boundary between two siblings is,
//! and what a pointer on it does to the two shares either side.
//!
//! Pure, like [`super::tree`] and [`super::layout`] — rectangles and shares,
//! no lay-rs and no Smithay — so `cargo test --lib tiling` covers all of it.
//! The compositor half, which turns a window-edge drag into share edits, is
//! in `src/shell/tiling_drag.rs`.

use std::fmt::Debug;
use std::hash::Hash;

use super::layout::{resolve_nodes, Gaps, Rect};
use super::tree::{Axis, NodeId, Tree, MIN_SHARE};

/// How thick a split's grab strip is, at minimum, in logical pixels. A
/// workspace with `inner_gap = 0` still has a boundary wide enough to reason
/// about.
pub const MIN_HANDLE: i32 = 8;

/// How near a snap target a drag has to come, in logical pixels, before it
/// jumps there. `Shift` bypasses it.
pub const SNAP_PX: f32 = 6.0;

/// The fractions of a pair a boundary snaps to: halves, thirds and quarters.
pub const SNAP_TARGETS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];

/// One draggable split between two siblings.
///
/// Identified by `(container, index)` rather than by position: a relayout mid
/// drag moves the rectangle but not the split it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarHandle {
    /// The container whose children the split divides.
    pub container: NodeId,
    /// The split sits between children `index` and `index + 1`.
    pub index: usize,
    /// The container's axis: a [`Axis::Row`] container has vertical splits
    /// that are dragged left and right.
    pub axis: Axis,
    /// Where the boundary is, in logical pixels.
    pub rect: Rect,
    /// Start of the first of the two children along `axis`.
    pub pair_start: i32,
    /// Pixels the two children occupy between them, gap excluded.
    pub pair_px: i32,
    /// The inner gap between them, in pixels.
    pub gap: i32,
    /// The two children's shares, as they stand.
    pub shares: (f32, f32),
}

impl BarHandle {
    /// The pointer coordinate that drives this split.
    pub fn pointer_along(&self, x: f64, y: f64) -> f64 {
        match self.axis {
            Axis::Row => x,
            Axis::Column => y,
        }
    }

    /// The first child's share the pointer at `along` asks for.
    pub fn share_at(&self, along: f64, snap: bool) -> f32 {
        let pair = self.shares.0 + self.shares.1;
        let offset = along as f32 - self.pair_start as f32 - self.gap as f32 / 2.0;
        share_from_offset(pair, self.pair_px as f32, offset, snap)
    }
}

/// The first of two shares a pointer offset asks for.
///
/// `pair` is what the two children hold between them and is preserved
/// exactly, so no other child of the container moves. `offset_px` is measured
/// from the start of the first child; `pair_px` is the pixels the two of them
/// fill. With `snap` the boundary jumps to a half, third or quarter of the
/// pair when it comes within [`SNAP_PX`] of one.
pub fn share_from_offset(pair: f32, pair_px: f32, offset_px: f32, snap: bool) -> f32 {
    if pair <= 0.0 || pair_px <= 0.0 {
        return pair.max(0.0) / 2.0;
    }
    let mut ratio = (offset_px / pair_px).clamp(0.0, 1.0);
    if snap {
        if let Some(target) = SNAP_TARGETS
            .iter()
            .copied()
            .filter(|t| ((t - ratio) * pair_px).abs() <= SNAP_PX)
            .min_by(|a, b| {
                (a - ratio)
                    .abs()
                    .partial_cmp(&(b - ratio).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            ratio = target;
        }
    }
    // Neither side may be squeezed out of existence, and a pair too small to
    // hold two minimums is simply halved.
    let floor = MIN_SHARE.min(pair / 2.0);
    (pair * ratio).clamp(floor, pair - floor)
}

/// Every split in `tree`, resolved against `area`.
///
/// A split spans the shared edge of two siblings and sits in the gap between
/// them, widened to [`MIN_HANDLE`] around its centre when the gap is thinner
/// than that — including when it is zero.
pub fn bar_handles<L: Clone + Eq + Hash + Debug>(
    tree: &Tree<L>,
    area: Rect,
    gaps: Gaps,
) -> Vec<BarHandle> {
    let nodes = resolve_nodes(tree, area, gaps);
    let rect_of = |id: NodeId| nodes.iter().find(|(n, _)| *n == id).map(|(_, r)| *r);

    let mut out = Vec::new();
    for (node, container_rect) in nodes.iter() {
        let Some(axis) = tree.axis(*node) else {
            continue;
        };
        let children = tree.children(*node);
        for index in 0..children.len().saturating_sub(1) {
            let (Some(first), Some(second)) = (
                rect_of(children[index].node),
                rect_of(children[index + 1].node),
            ) else {
                continue;
            };
            let (pair_start, first_end, second_start, pair_end) = match axis {
                Axis::Row => (first.x, first.x + first.w, second.x, second.x + second.w),
                Axis::Column => (first.y, first.y + first.h, second.y, second.y + second.h),
            };
            let gap = (second_start - first_end).max(0);
            let thickness = gap.max(MIN_HANDLE);
            let centre = (first_end + second_start) / 2;
            let start = centre - thickness / 2;
            let rect = match axis {
                Axis::Row => Rect::new(start, container_rect.y, thickness, container_rect.h),
                Axis::Column => Rect::new(container_rect.x, start, container_rect.w, thickness),
            };
            out.push(BarHandle {
                container: *node,
                index,
                axis,
                rect,
                pair_start,
                pair_px: (pair_end - pair_start - gap).max(1),
                gap,
                shares: (children[index].share, children[index + 1].share),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 1000,
        h: 600,
    };

    fn gaps(inner: i32) -> Gaps {
        Gaps {
            inner,
            outer: 0,
            smart: false,
        }
    }

    fn two_columns() -> Tree<u32> {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.equalize_all();
        tree
    }

    // ── Share from the pointer ───────────────────────────────────────────

    #[test]
    fn the_boundary_follows_the_pointer_across_the_pair() {
        // A pair holding 0.6 of its container across 600px.
        assert!((share_from_offset(0.6, 600.0, 300.0, false) - 0.3).abs() < 1e-4);
        assert!((share_from_offset(0.6, 600.0, 150.0, false) - 0.15).abs() < 1e-4);
    }

    #[test]
    fn the_pair_total_is_preserved_whatever_the_pointer_does() {
        for offset in [-200.0, 0.0, 137.0, 599.0, 5000.0] {
            let first = share_from_offset(0.6, 600.0, offset, false);
            assert!(first > 0.0 && first < 0.6, "{first} out of the pair");
        }
    }

    #[test]
    fn neither_side_can_be_squeezed_out() {
        assert!((share_from_offset(1.0, 1000.0, -50.0, false) - MIN_SHARE).abs() < 1e-4);
        assert!((share_from_offset(1.0, 1000.0, 2000.0, false) - (1.0 - MIN_SHARE)).abs() < 1e-4);
    }

    #[test]
    fn a_near_miss_snaps_to_the_half() {
        // 497px of 1000 is 3px shy of the half: inside the 6px threshold.
        let snapped = share_from_offset(1.0, 1000.0, 497.0, true);
        assert!((snapped - 0.5).abs() < 1e-4, "{snapped}");
        // …and does not without snapping.
        let free = share_from_offset(1.0, 1000.0, 497.0, false);
        assert!((free - 0.497).abs() < 1e-4, "{free}");
    }

    #[test]
    fn thirds_and_quarters_snap_too() {
        assert!((share_from_offset(1.0, 1200.0, 401.0, true) - 1.0 / 3.0).abs() < 1e-4);
        assert!((share_from_offset(1.0, 1200.0, 299.0, true) - 0.25).abs() < 1e-4);
        assert!((share_from_offset(1.0, 1200.0, 902.0, true) - 0.75).abs() < 1e-4);
    }

    #[test]
    fn a_pointer_far_from_every_target_does_not_snap() {
        let free = share_from_offset(1.0, 1000.0, 420.0, true);
        assert!((free - 0.42).abs() < 1e-4, "{free}");
    }

    // ── Boundaries ───────────────────────────────────────────────────────

    #[test]
    fn two_columns_have_one_split_in_their_gap() {
        let tree = two_columns();
        let bars = bar_handles(&tree, AREA, gaps(10));
        assert_eq!(bars.len(), 1);
        let bar = bars[0];
        assert_eq!(bar.axis, Axis::Row);
        assert_eq!(bar.index, 0);
        // 1000px, 10px gap: cells of 495, so the gap runs 495..505.
        assert_eq!(bar.rect, Rect::new(495, 0, 10, 600));
        assert_eq!(bar.pair_start, 0);
        assert_eq!(bar.pair_px, 990);
        assert_eq!(bar.gap, 10);
    }

    #[test]
    fn a_gapless_split_still_has_a_strip_to_grab() {
        let tree = two_columns();
        let bars = bar_handles(&tree, AREA, gaps(0));
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].rect.w, MIN_HANDLE);
        // Centred on the split at 500.
        assert_eq!(bars[0].rect.x, 500 - MIN_HANDLE / 2);
    }

    #[test]
    fn a_grid_has_a_split_per_row_plus_one_between_them() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.insert_next_to(Some(&2), 3, Some(Axis::Column), false);
        tree.insert_next_to(Some(&1), 4, Some(Axis::Column), false);
        let bars = bar_handles(&tree, AREA, gaps(8));
        assert_eq!(bars.len(), 3);
        assert_eq!(bars.iter().filter(|b| b.axis == Axis::Column).count(), 2);
    }

    #[test]
    fn a_split_drag_moves_only_its_two_neighbours() {
        let mut tree = Tree::<u32>::new();
        for leaf in 1..=3 {
            tree.insert_next_to(None, leaf, None, true);
        }
        tree.equalize_all();
        let bars = bar_handles(&tree, AREA, gaps(0));
        let bar = bars.iter().find(|b| b.index == 0).copied().unwrap();
        let first = bar.share_at(100.0, false);
        assert!(tree.set_pair_shares(bar.container, bar.index, first));
        let children = tree.children(bar.container);
        assert_eq!(children.len(), 3);
        // The third child is untouched, and the shares still sum to one.
        assert!((children[2].share - 1.0 / 3.0).abs() < 1e-4);
        let total: f32 = children.iter().map(|c| c.share).sum();
        assert!((total - 1.0).abs() < 1e-4);
    }
}
