//! Design mode's arithmetic: where the handles are, what a drag does to the
//! two shares either side of one, the starting-point presets, and the undo
//! stack.
//!
//! Pure, like [`super::tree`] and [`super::layout`] — rectangles and shares,
//! no lay-rs and no Smithay — so `cargo test --lib tiling` covers all of it.
//! The panes, the cursor and the pointer grab live in
//! `src/workspaces/tiling_design.rs`.

use std::fmt::Debug;
use std::hash::Hash;

use super::layout::{resolve_nodes, Gaps, Rect};
use super::tree::{Axis, NodeId, Tree, MIN_SHARE};

/// How thick a bar handle is, at minimum, in logical pixels. A workspace with
/// `inner_gap = 0` still has to offer something to grab
/// (`docs/developer/tiling-plan.md`, *Design mode*: "large enough to hit
/// without aiming").
pub const MIN_HANDLE: i32 = 8;

/// How near a snap target a drag has to come, in logical pixels, before it
/// jumps there. `Shift` bypasses it.
pub const SNAP_PX: f32 = 6.0;

/// The fractions of a pair a bar snaps to: halves, thirds and quarters.
pub const SNAP_TARGETS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];

/// How many tree snapshots design mode keeps to undo.
pub const UNDO_DEPTH: usize = 32;

/// One draggable split between two siblings.
///
/// Identified by `(container, index)` rather than by position: a relayout mid
/// drag moves the rectangle but not the split it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarHandle {
    /// The container whose children the bar divides.
    pub container: NodeId,
    /// The bar sits between children `index` and `index + 1`.
    pub index: usize,
    /// The container's axis: a [`Axis::Row`] container has vertical bars that
    /// are dragged left and right.
    pub axis: Axis,
    /// Where to draw and hit-test it, in logical pixels.
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
    /// Does this bar move left/right (rather than up/down)?
    pub fn is_vertical_bar(&self) -> bool {
        self.axis == Axis::Row
    }

    /// The pointer coordinate that drives this bar.
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

/// Where cells meet at a point: one split running one way, and every split
/// running the other way that ends on it.
///
/// Bars in a tree layout never *cross* — a row's column split stops at the
/// row's edge — so a corner is a junction, and dragging it moves the split it
/// sits on plus every split that lands there. On a 2×2 grid that is the row
/// split and both columns' splits, which is what makes the corner behave like
/// the grid corner it looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct CornerHandle {
    /// Splits driven by the pointer's x: bars of [`Axis::Row`] containers.
    pub along_x: Vec<BarHandle>,
    /// Splits driven by the pointer's y: bars of [`Axis::Column`] containers.
    pub along_y: Vec<BarHandle>,
    /// Hit and draw rect, in logical pixels.
    pub rect: Rect,
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

/// Every bar handle in `tree`, resolved against `area`.
///
/// A bar spans the shared edge of two siblings and sits in the gap between
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

/// Every corner in a set of bars.
///
/// One per horizontal split per cluster of vertical splits that end on it:
/// the vertical bars are clustered by overlapping x, so the two column splits
/// of a 2×2 grid — which line up — make a single corner rather than two
/// stacked on the same pixel.
pub fn corner_handles(bars: &[BarHandle]) -> Vec<CornerHandle> {
    let mut out = Vec::new();
    for along_y in bars.iter().filter(|b| b.axis == Axis::Column) {
        let mut adjacent: Vec<BarHandle> = bars
            .iter()
            .filter(|b| b.axis == Axis::Row && meets(b.rect, along_y.rect))
            .copied()
            .collect();
        adjacent.sort_by_key(|b| b.rect.x);

        // Cluster by overlapping x: bars that line up are one corner.
        let mut cluster: Vec<BarHandle> = Vec::new();
        let mut flush = |cluster: &mut Vec<BarHandle>, out: &mut Vec<CornerHandle>| {
            if cluster.is_empty() {
                return;
            }
            let x = cluster.iter().map(|b| b.rect.x).max().unwrap_or(0);
            let right = cluster
                .iter()
                .map(|b| b.rect.x + b.rect.w)
                .min()
                .unwrap_or(x);
            out.push(CornerHandle {
                rect: Rect::new(x, along_y.rect.y, (right - x).max(1), along_y.rect.h),
                along_x: std::mem::take(cluster),
                along_y: vec![*along_y],
            });
        };
        for bar in adjacent {
            let overlaps = cluster
                .iter()
                .any(|c| bar.rect.x < c.rect.x + c.rect.w && c.rect.x < bar.rect.x + bar.rect.w);
            if cluster.is_empty() || overlaps {
                cluster.push(bar);
            } else {
                flush(&mut cluster, &mut out);
                cluster.push(bar);
            }
        }
        flush(&mut cluster, &mut out);
    }
    out
}

/// Do these two rectangles meet — overlap, or touch within a handle's width?
/// Bars that share a junction end on each other rather than overlapping.
fn meets(a: Rect, b: Rect) -> bool {
    let tol = MIN_HANDLE;
    a.x < b.x + b.w + tol && b.x < a.x + a.w + tol && a.y < b.y + b.h + tol && b.y < a.y + a.h + tol
}

/// Does `rect` contain the logical point `(x, y)`?
pub fn contains(rect: Rect, x: f64, y: f64) -> bool {
    x >= rect.x as f64
        && x < (rect.x + rect.w) as f64
        && y >= rect.y as f64
        && y < (rect.y + rect.h) as f64
}

/// A shape design mode offers on an empty tiling workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    TwoColumns,
    ThreeColumns,
    /// One wide cell on the left, two stacked on the right.
    MainAndStack,
    Grid,
}

impl Preset {
    /// Every preset, in the order the row of buttons shows them.
    pub const ALL: [Preset; 4] = [
        Preset::TwoColumns,
        Preset::ThreeColumns,
        Preset::MainAndStack,
        Preset::Grid,
    ];

    /// The name the config and the headless harness use.
    pub fn name(self) -> &'static str {
        match self {
            Preset::TwoColumns => "two-columns",
            Preset::ThreeColumns => "three-columns",
            Preset::MainAndStack => "main-and-stack",
            Preset::Grid => "grid",
        }
    }

    pub fn parse(name: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.name() == name)
    }

    /// How many cells the shape has.
    pub fn slots(self) -> usize {
        match self {
            Preset::TwoColumns => 2,
            Preset::ThreeColumns => 3,
            Preset::MainAndStack => 3,
            Preset::Grid => 4,
        }
    }
}

/// Build `preset` as a tree of empty slots, replacing whatever `tree` held.
pub fn apply_preset<L: Clone + Eq + Hash + Debug>(tree: &mut Tree<L>, preset: Preset) {
    let mut fresh = Tree::<L>::new();
    let root = match preset {
        Preset::TwoColumns => {
            let cells = [fresh.alloc_empty(), fresh.alloc_empty()];
            fresh.alloc_container(Axis::Row, &cells)
        }
        Preset::ThreeColumns => {
            let cells = [
                fresh.alloc_empty(),
                fresh.alloc_empty(),
                fresh.alloc_empty(),
            ];
            fresh.alloc_container(Axis::Row, &cells)
        }
        Preset::MainAndStack => {
            let main = fresh.alloc_empty();
            let stacked = [fresh.alloc_empty(), fresh.alloc_empty()];
            let stack = fresh.alloc_container(Axis::Column, &stacked);
            fresh.alloc_container(Axis::Row, &[main, stack])
        }
        Preset::Grid => {
            let top = [fresh.alloc_empty(), fresh.alloc_empty()];
            let top_row = fresh.alloc_container(Axis::Row, &top);
            let bottom = [fresh.alloc_empty(), fresh.alloc_empty()];
            let bottom_row = fresh.alloc_container(Axis::Row, &bottom);
            fresh.alloc_container(Axis::Column, &[top_row, bottom_row])
        }
    };
    fresh.set_root(Some(root));
    *tree = fresh;
}

/// A bounded stack of tree snapshots.
///
/// Every structural or share edit made in design mode pushes one before it
/// changes anything, so `TilingUndo` pops the tree the workspace had before
/// the last edit (`specs/tiling.md`, *Design mode*: "every structural edit in
/// design mode goes on the session undo stack").
#[derive(Debug)]
pub struct UndoStack<L> {
    entries: Vec<Tree<L>>,
    depth: usize,
}

impl<L> Default for UndoStack<L> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            depth: UNDO_DEPTH,
        }
    }
}

impl<L: Clone> UndoStack<L> {
    pub fn push(&mut self, tree: &Tree<L>) {
        if self.entries.len() == self.depth {
            self.entries.remove(0);
        }
        self.entries.push(tree.clone());
    }

    pub fn pop(&mut self) -> Option<Tree<L>> {
        self.entries.pop()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
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

    // ── Handles ──────────────────────────────────────────────────────────

    #[test]
    fn two_columns_have_one_bar_in_their_gap() {
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
    fn a_gapless_split_still_offers_something_to_grab() {
        let tree = two_columns();
        let bars = bar_handles(&tree, AREA, gaps(0));
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].rect.w, MIN_HANDLE);
        // Centred on the split at 500.
        assert_eq!(bars[0].rect.x, 500 - MIN_HANDLE / 2);
    }

    #[test]
    fn a_grid_has_three_bars_and_two_corners() {
        let mut tree = Tree::<u32>::new();
        apply_preset(&mut tree, Preset::Grid);
        let bars = bar_handles(&tree, AREA, gaps(8));
        // One bar per row, plus the one between the rows.
        assert_eq!(bars.len(), 3);
        let corners = corner_handles(&bars);
        // One junction, driving all three splits: the row split and the
        // column split of each row.
        assert_eq!(corners.len(), 1);
        assert_eq!(corners[0].along_x.len(), 2);
        assert_eq!(corners[0].along_y.len(), 1);
    }

    #[test]
    fn two_columns_have_no_corner_to_drag() {
        let tree = two_columns();
        let bars = bar_handles(&tree, AREA, gaps(8));
        assert!(corner_handles(&bars).is_empty());
    }

    #[test]
    fn a_bar_drag_moves_only_its_two_neighbours() {
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

    // ── Presets ──────────────────────────────────────────────────────────

    #[test]
    fn every_preset_builds_its_slots_and_nothing_else() {
        for preset in Preset::ALL {
            let mut tree = Tree::<u32>::new();
            apply_preset(&mut tree, preset);
            assert_eq!(
                tree.empty_slots().len(),
                preset.slots(),
                "{} slots",
                preset.name()
            );
            assert!(tree.leaves().is_empty(), "{}", preset.name());
            assert_eq!(tree.len(), preset.slots());
        }
    }

    #[test]
    fn main_and_stack_puts_two_cells_beside_one() {
        let mut tree = Tree::<u32>::new();
        apply_preset(&mut tree, Preset::MainAndStack);
        let cells = super::super::layout::resolve_cells(&tree, AREA, gaps(0));
        assert_eq!(cells.len(), 3);
        // Main on the left, full height.
        assert_eq!(cells[0].1, Rect::new(0, 0, 500, 600));
        assert_eq!(cells[1].1, Rect::new(500, 0, 500, 300));
        assert_eq!(cells[2].1, Rect::new(500, 300, 500, 300));
    }

    #[test]
    fn preset_names_round_trip() {
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.name()), Some(preset));
        }
        assert_eq!(Preset::parse("spiral"), None);
    }

    // ── Empty slots ──────────────────────────────────────────────────────

    #[test]
    fn splitting_a_cell_leaves_an_empty_slot_beside_it() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        let node = tree.node_of(&1).unwrap();
        let slot = tree.split_cell(node, Axis::Row).unwrap();
        assert_eq!(tree.empty_slots(), vec![slot]);
        assert_eq!(tree.leaves(), vec![1]);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn the_next_window_fills_the_focused_empty_slot() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        let node = tree.node_of(&1).unwrap();
        let first = tree.split_cell(node, Axis::Row).unwrap();
        let node = tree.node_of(&1).unwrap();
        let second = tree.split_cell(node, Axis::Column).unwrap();
        // Focus on the later slot: that is the one that fills.
        assert_eq!(tree.next_empty(Some(second)), Some(second));
        assert!(tree.fill_empty(second, 2));
        assert_eq!(tree.empty_slots(), vec![first]);
        assert_eq!(tree.len(), 3);
        // With no focused slot, the first in layout order fills.
        assert_eq!(tree.next_empty(None), Some(first));
        // And a slot that is gone falls back the same way.
        assert_eq!(tree.next_empty(Some(second)), Some(first));
    }

    #[test]
    fn closing_an_empty_slot_gives_its_space_back() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        let node = tree.node_of(&1).unwrap();
        let slot = tree.split_cell(node, Axis::Row).unwrap();
        assert!(tree.remove_empty(slot));
        assert!(tree.empty_slots().is_empty());
        // The container it left behind dissolved: the window is the root again.
        assert_eq!(tree.root(), tree.node_of(&1));
        let cells = super::super::layout::resolve_cells(&tree, AREA, gaps(0));
        assert_eq!(cells[0].1, AREA);
    }

    #[test]
    fn an_empty_workspace_can_be_laid_out_then_populated() {
        let mut tree = Tree::<u32>::new();
        apply_preset(&mut tree, Preset::TwoColumns);
        let slots = tree.empty_slots();
        assert!(tree.fill_empty(slots[0], 1));
        assert!(tree.fill_empty(slots[1], 2));
        assert!(tree.empty_slots().is_empty());
        let cells = super::super::layout::resolve_cells(&tree, AREA, gaps(0));
        assert_eq!(cells[0].1, Rect::new(0, 0, 500, 600));
        assert_eq!(cells[1].1, Rect::new(500, 0, 500, 600));
    }

    // ── Undo ─────────────────────────────────────────────────────────────

    #[test]
    fn undo_returns_the_tree_as_it_was() {
        let mut tree = two_columns();
        let mut undo = UndoStack::<u32>::default();
        undo.push(&tree);
        let node = tree.node_of(&1).unwrap();
        tree.split_cell(node, Axis::Column);
        assert_eq!(tree.len(), 3);
        let restored = undo.pop().expect("one edit to undo");
        assert_eq!(restored.len(), 2);
        assert!(undo.is_empty());
        assert!(undo.pop().is_none());
    }

    #[test]
    fn the_undo_stack_is_bounded() {
        let tree = two_columns();
        let mut undo = UndoStack::<u32>::default();
        for _ in 0..UNDO_DEPTH + 10 {
            undo.push(&tree);
        }
        assert_eq!(undo.len(), UNDO_DEPTH);
        undo.clear();
        assert!(undo.is_empty());
    }
}
