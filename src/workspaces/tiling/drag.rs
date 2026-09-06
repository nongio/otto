//! What a pointer drag on a tiled workspace means, in arithmetic.
//!
//! Two questions, both pure — rectangles, shares and tree nodes, no lay-rs and
//! no Smithay, so `cargo test --lib tiling` covers them:
//!
//! * a titlebar drag hovering a tile: *which slot would this land in* —
//!   [`drop_side`] and [`preview_rect`];
//! * an edge drag on a tile: *which split is this edge* — [`split_for_edge`],
//!   with [`min_extent`] for the size the neighbours refuse to go below.
//!
//! The compositor half — detaching the leaf, the overlay, the grabs — is in
//! `src/shell/tiling_drag.rs`.

use std::fmt::Debug;
use std::hash::Hash;

use super::layout::Rect;
use super::tree::{Axis, Child, Node, NodeId, Tree};

/// How much of a tile, measured from its centre, means "swap with this
/// window" rather than "go beside it": the central 30 % on both axes.
pub const CENTRE_FRACTION: f64 = 0.3;

/// Where a drop on a tile puts the dragged window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropSide {
    Left,
    Right,
    Top,
    Bottom,
    /// The pointer is near the tile's middle: swap the two windows.
    Centre,
}

impl DropSide {
    /// The split this side asks for: the axis it divides, and whether the
    /// dragged window goes after the tile along it.
    ///
    /// `None` for [`DropSide::Centre`], which is a swap rather than an insert.
    pub fn axis(self) -> Option<(Axis, bool)> {
        match self {
            DropSide::Left => Some((Axis::Row, false)),
            DropSide::Right => Some((Axis::Row, true)),
            DropSide::Top => Some((Axis::Column, false)),
            DropSide::Bottom => Some((Axis::Column, true)),
            DropSide::Centre => None,
        }
    }
}

/// Which half of `rect` the pointer is asking for.
///
/// Within the central [`CENTRE_FRACTION`] of both axes the answer is
/// [`DropSide::Centre`]. Otherwise the tile is halved the way it is shaped —
/// a wide tile left/right, a tall one top/bottom — and the pointer picks the
/// half it is nearer to.
pub fn drop_side(rect: Rect, x: f64, y: f64) -> DropSide {
    let centre_x = rect.x as f64 + rect.w as f64 / 2.0;
    let centre_y = rect.y as f64 + rect.h as f64 / 2.0;
    let half = CENTRE_FRACTION / 2.0;
    if (x - centre_x).abs() <= rect.w as f64 * half && (y - centre_y).abs() <= rect.h as f64 * half
    {
        return DropSide::Centre;
    }
    if rect.is_wide() {
        if x < centre_x {
            DropSide::Left
        } else {
            DropSide::Right
        }
    } else if y < centre_y {
        DropSide::Top
    } else {
        DropSide::Bottom
    }
}

/// The rectangle the slot overlay draws for a drop on `side` of `rect`: the
/// half that would be taken, or the whole tile for a swap.
pub fn preview_rect(rect: Rect, side: DropSide) -> Rect {
    let half_w = rect.w / 2;
    let half_h = rect.h / 2;
    match side {
        DropSide::Centre => rect,
        DropSide::Left => Rect::new(rect.x, rect.y, half_w, rect.h),
        DropSide::Right => Rect::new(rect.x + rect.w - half_w, rect.y, half_w, rect.h),
        DropSide::Top => Rect::new(rect.x, rect.y, rect.w, half_h),
        DropSide::Bottom => Rect::new(rect.x, rect.y + rect.h - half_h, rect.w, half_h),
    }
}

/// Does `rect` hold the point?
pub fn contains(rect: Rect, x: f64, y: f64) -> bool {
    x >= rect.x as f64
        && y >= rect.y as f64
        && x < (rect.x + rect.w) as f64
        && y < (rect.y + rect.h) as f64
}

/// The cell under the pointer, and the side of it the drop asks for.
pub fn drop_target(cells: &[(NodeId, Rect)], x: f64, y: f64) -> Option<(NodeId, DropSide)> {
    cells
        .iter()
        .find(|(_, rect)| contains(*rect, x, y))
        .map(|(node, rect)| (*node, drop_side(*rect, x, y)))
}

/// The edge of `area` a point that missed every tile is nearest to, as the
/// axis it would split and which end of it the window goes on.
///
/// A drop with no tile under it still has to land somewhere; the plan's answer
/// is the floating layer, which does not exist yet, so it goes against the
/// outside of the tree on the side it was let go.
pub fn nearest_root_edge(area: Rect, x: f64, y: f64) -> (Axis, bool) {
    let left = (x - area.x as f64).max(0.0);
    let right = ((area.x + area.w) as f64 - x).max(0.0);
    let top = (y - area.y as f64).max(0.0);
    let bottom = ((area.y + area.h) as f64 - y).max(0.0);
    let nearest = left.min(right).min(top).min(bottom);
    if nearest == left {
        (Axis::Row, false)
    } else if nearest == right {
        (Axis::Row, true)
    } else if nearest == top {
        (Axis::Column, false)
    } else {
        (Axis::Column, true)
    }
}

/// The split a window's edge drags, as `(container, bar index)`.
///
/// The nearest ancestor container that splits along `axis` and in which the
/// leaf's own subtree has a neighbour on that side. An edge with nothing
/// beyond it — the outside of the tree, against the usable area — has no
/// split and yields `None`, which is why dragging the screen edge of a tile
/// does nothing.
pub fn split_for_edge<L: Clone + Eq + Hash + Debug>(
    tree: &Tree<L>,
    leaf: &L,
    axis: Axis,
    forward: bool,
) -> Option<(NodeId, usize)> {
    let mut child = tree.node_of(leaf)?;
    while let Some(parent) = tree.parent_of(child) {
        if tree.axis(parent) == Some(axis) {
            let children = tree.children(parent);
            if let Some(index) = children.iter().position(|c| c.node == child) {
                if forward && index + 1 < children.len() {
                    return Some((parent, index));
                }
                if !forward && index > 0 {
                    return Some((parent, index - 1));
                }
            }
        }
        child = parent;
    }
    None
}

/// The smallest `node`'s subtree can be made along `axis`, in logical pixels.
///
/// `leaf_min` answers for one window. Children of a container that splits
/// along `axis` sit end to end, so their minimums add up; children of one
/// that splits the other way share the extent, so the largest wins.
pub fn min_extent<L: Clone + Eq + Hash + Debug>(
    tree: &Tree<L>,
    node: NodeId,
    axis: Axis,
    leaf_min: &dyn Fn(&L) -> i32,
) -> i32 {
    match tree.node(node) {
        Some(Node::Leaf(leaf)) => leaf_min(leaf),
        Some(Node::Empty(_)) => 0,
        Some(Node::Container {
            axis: container_axis,
            children,
        }) => {
            let extents = children
                .iter()
                .map(|Child { node, .. }| min_extent(tree, *node, axis, leaf_min));
            if *container_axis == axis {
                extents.sum()
            } else {
                extents.max().unwrap_or(0)
            }
        }
        None => 0,
    }
}

/// Hold a bar's first share inside what the two subtrees either side of it
/// can actually be shrunk to.
///
/// `pair` is the two shares' sum, `pair_px` the pixels they fill between
/// them, and the two minimums are in the same pixels. A pair with no room for
/// both minimums is split in proportion to them rather than refusing to move.
pub fn clamp_share_to_minimums(
    share: f32,
    pair: f32,
    pair_px: f32,
    min_first_px: f32,
    min_second_px: f32,
) -> f32 {
    if pair <= 0.0 || pair_px <= 0.0 {
        return share;
    }
    let low = pair * (min_first_px / pair_px).clamp(0.0, 1.0);
    let high = pair - pair * (min_second_px / pair_px).clamp(0.0, 1.0);
    if low > high {
        // Not enough room for both: give each side its share of the demand.
        let total = min_first_px + min_second_px;
        if total <= 0.0 {
            return pair / 2.0;
        }
        return pair * (min_first_px / total);
    }
    share.clamp(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Which side of the hovered tile ───────────────────────────────────

    #[test]
    fn a_wide_tile_is_halved_left_and_right() {
        let rect = Rect::new(0, 0, 400, 200);
        assert_eq!(drop_side(rect, 20.0, 100.0), DropSide::Left);
        assert_eq!(drop_side(rect, 380.0, 100.0), DropSide::Right);
    }

    #[test]
    fn a_tall_tile_is_halved_top_and_bottom() {
        let rect = Rect::new(0, 0, 200, 400);
        assert_eq!(drop_side(rect, 100.0, 20.0), DropSide::Top);
        assert_eq!(drop_side(rect, 100.0, 380.0), DropSide::Bottom);
    }

    #[test]
    fn the_middle_of_a_tile_is_a_swap() {
        let rect = Rect::new(0, 0, 400, 200);
        assert_eq!(drop_side(rect, 200.0, 100.0), DropSide::Centre);
        // The band is 30% of each axis, centred: ±60px across, ±30 down.
        assert_eq!(drop_side(rect, 259.0, 129.0), DropSide::Centre);
        // Outside it on either axis, the halves take over again.
        assert_eq!(drop_side(rect, 261.0, 100.0), DropSide::Right);
        assert_eq!(drop_side(rect, 200.0, 131.0), DropSide::Right);
    }

    #[test]
    fn the_side_is_measured_in_the_tiles_own_frame() {
        let rect = Rect::new(1000, 500, 400, 200);
        assert_eq!(drop_side(rect, 1020.0, 600.0), DropSide::Left);
        assert_eq!(drop_side(rect, 1380.0, 600.0), DropSide::Right);
        assert_eq!(drop_side(rect, 1200.0, 600.0), DropSide::Centre);
    }

    #[test]
    fn a_preview_is_the_half_that_would_be_taken() {
        let rect = Rect::new(100, 50, 400, 200);
        assert_eq!(
            preview_rect(rect, DropSide::Left),
            Rect::new(100, 50, 200, 200)
        );
        assert_eq!(
            preview_rect(rect, DropSide::Right),
            Rect::new(300, 50, 200, 200)
        );
        assert_eq!(
            preview_rect(rect, DropSide::Top),
            Rect::new(100, 50, 400, 100)
        );
        assert_eq!(
            preview_rect(rect, DropSide::Bottom),
            Rect::new(100, 150, 400, 100)
        );
        assert_eq!(preview_rect(rect, DropSide::Centre), rect);
    }

    #[test]
    fn the_hovered_cell_is_the_one_the_pointer_is_in() {
        let cells = vec![
            (0, Rect::new(0, 0, 200, 200)),
            (1, Rect::new(200, 0, 200, 200)),
        ];
        assert_eq!(drop_target(&cells, 10.0, 10.0), Some((0, DropSide::Left)));
        assert_eq!(drop_target(&cells, 390.0, 10.0), Some((1, DropSide::Right)));
        assert_eq!(drop_target(&cells, 500.0, 10.0), None);
    }

    #[test]
    fn a_miss_lands_on_the_edge_it_was_nearest() {
        let area = Rect::new(0, 0, 1000, 600);
        assert_eq!(nearest_root_edge(area, 5.0, 300.0), (Axis::Row, false));
        assert_eq!(nearest_root_edge(area, 995.0, 300.0), (Axis::Row, true));
        assert_eq!(nearest_root_edge(area, 500.0, 2.0), (Axis::Column, false));
        assert_eq!(nearest_root_edge(area, 500.0, 598.0), (Axis::Column, true));
    }

    // ── Which split an edge is ───────────────────────────────────────────

    /// `a | b` — one row of two.
    fn pair() -> Tree<u32> {
        let mut tree = Tree::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree
    }

    #[test]
    fn an_inner_edge_finds_the_split_it_sits_on() {
        let tree = pair();
        let container = tree.container_of(&1).expect("a row");
        assert_eq!(
            split_for_edge(&tree, &1, Axis::Row, true),
            Some((container, 0)),
            "a's right edge is the row's only bar"
        );
        assert_eq!(
            split_for_edge(&tree, &2, Axis::Row, false),
            Some((container, 0)),
            "and so is b's left edge"
        );
    }

    #[test]
    fn an_outer_edge_has_no_split() {
        let tree = pair();
        assert_eq!(split_for_edge(&tree, &1, Axis::Row, false), None);
        assert_eq!(split_for_edge(&tree, &2, Axis::Row, true), None);
        // Nothing splits the other way at all.
        assert_eq!(split_for_edge(&tree, &1, Axis::Column, true), None);
        assert_eq!(split_for_edge(&tree, &1, Axis::Column, false), None);
    }

    #[test]
    fn a_lone_tile_has_no_split_on_any_edge() {
        let mut tree = Tree::new();
        tree.insert_next_to(None, 1, None, true);
        for (axis, forward) in [
            (Axis::Row, true),
            (Axis::Row, false),
            (Axis::Column, true),
            (Axis::Column, false),
        ] {
            assert_eq!(split_for_edge(&tree, &1, axis, forward), None);
        }
    }

    #[test]
    fn a_nested_edge_climbs_to_the_ancestor_that_splits_that_way() {
        // A row of [a, column[b, c]]: c's left edge is the outer row's bar,
        // because its own column does not split left-to-right.
        let mut tree = Tree::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.insert_next_to(Some(&2), 3, Some(Axis::Column), true);

        let outer = tree.container_of(&1).expect("the outer row");
        let inner = tree.container_of(&3).expect("the inner column");
        assert_ne!(outer, inner);

        assert_eq!(
            split_for_edge(&tree, &3, Axis::Row, false),
            Some((outer, 0)),
            "c's left edge is the row's bar, one level up"
        );
        assert_eq!(
            split_for_edge(&tree, &3, Axis::Column, false),
            Some((inner, 0)),
            "and its top edge is its own column's"
        );
        assert_eq!(
            split_for_edge(&tree, &3, Axis::Row, true),
            None,
            "its right edge is the outside of the tree"
        );
    }

    // ── Minimum sizes ────────────────────────────────────────────────────

    #[test]
    fn minimums_add_along_the_axis_and_max_across_it() {
        let mut tree = Tree::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.insert_next_to(Some(&2), 3, Some(Axis::Column), true);
        let root = tree.root().expect("a root");
        let mins = |leaf: &u32| match leaf {
            1 => 100,
            2 => 50,
            _ => 70,
        };
        // Left to right the row adds a to the column, and the column's own
        // children — stacked — only have to fit the widest of them.
        assert_eq!(min_extent(&tree, root, Axis::Row, &mins), 100 + 70);
        // Top to bottom the row's children overlap, and the tallest is the
        // column, which stacks b on c.
        assert_eq!(min_extent(&tree, root, Axis::Column, &mins), 50 + 70);
    }

    #[test]
    fn a_share_is_held_off_both_minimums() {
        // 400px between them, each side refusing to go under 100px: the
        // boundary may travel between a quarter and three quarters.
        let clamped = |share: f32| clamp_share_to_minimums(share, 1.0, 400.0, 100.0, 100.0);
        assert!((clamped(0.5) - 0.5).abs() < 1e-6);
        assert!((clamped(0.05) - 0.25).abs() < 1e-6);
        assert!((clamped(0.95) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_pair_too_small_for_both_minimums_splits_in_proportion() {
        let out = clamp_share_to_minimums(0.9, 1.0, 100.0, 100.0, 100.0);
        assert!((out - 0.5).abs() < 1e-6, "{out}");
    }
}
