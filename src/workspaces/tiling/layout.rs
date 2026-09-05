//! Resolving a [`Tree`] to rectangles, and directional neighbour lookup.
//!
//! Pure: rectangles are plain `i32` logical pixels, and the same input always
//! produces the same output. This is the authority on "what is this window's
//! rectangle" — the client configure, hit-testing and the tests all read it
//! from here.

use std::fmt::Debug;
use std::hash::Hash;

use super::tree::{Axis, Direction, Node, NodeId, Tree};

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Is this cell wider than it is tall? Decides which way an insertion
    /// splits it (see `specs/tiling.md`, *Insertion*).
    pub fn is_wide(&self) -> bool {
        self.w >= self.h
    }

    fn inset(self, by: i32) -> Rect {
        Rect {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2 * by).max(0),
            h: (self.h - 2 * by).max(0),
        }
    }
}

/// Gap configuration for one workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gaps {
    /// Between two siblings.
    pub inner: i32,
    /// Between the tree and the edge of the usable area.
    pub outer: i32,
    /// A lone tile drops the gaps entirely.
    pub smart: bool,
}

/// Resolve `tree` against `area`, in layout order.
///
/// The outer gap insets `area`; the inner gap sits between siblings. With
/// `gaps.smart` a lone tile fills `area` outright — no outer gap and, having
/// no siblings, no inner one either.
pub fn resolve<L: Clone + Eq + Hash + Debug>(
    tree: &Tree<L>,
    area: Rect,
    gaps: Gaps,
) -> Vec<(L, Rect)> {
    let Some(root) = tree.root() else {
        return Vec::new();
    };
    let lone = tree.len() <= 1;
    let outer = if gaps.smart && lone { 0 } else { gaps.outer };
    let inner = if gaps.smart && lone { 0 } else { gaps.inner };

    let mut out = Vec::new();
    place(tree, root, area.inset(outer), inner, &mut out);
    out
}

fn place<L: Clone + Eq + Hash + Debug>(
    tree: &Tree<L>,
    node: NodeId,
    rect: Rect,
    inner: i32,
    out: &mut Vec<(L, Rect)>,
) {
    match tree.node(node) {
        Some(Node::Leaf(leaf)) => out.push((leaf.clone(), rect)),
        Some(Node::Container { axis, children }) => {
            let n = children.len();
            if n == 0 {
                return;
            }
            let extent = match axis {
                Axis::Row => rect.w,
                Axis::Column => rect.h,
            };
            // The gaps come off the top, so the children divide what is left.
            let usable = (extent - inner * (n as i32 - 1)).max(0);
            let mut offset = 0;
            let mut used = 0;
            for (i, child) in children.iter().enumerate() {
                // The last child takes whatever rounding left over, so the
                // children fill the container exactly.
                let size = if i + 1 == n {
                    usable - used
                } else {
                    (child.share * usable as f32).round() as i32
                };
                let size = size.max(0);
                let child_rect = match axis {
                    Axis::Row => Rect {
                        x: rect.x + offset,
                        y: rect.y,
                        w: size,
                        h: rect.h,
                    },
                    Axis::Column => Rect {
                        x: rect.x,
                        y: rect.y + offset,
                        w: rect.w,
                        h: size,
                    },
                };
                place(tree, child.node, child_rect, inner, out);
                used += size;
                offset += size + inner;
            }
        }
        None => {}
    }
}

/// The leaf `from` moves focus to when the user asks for `dir`.
///
/// Candidates are the cells that lie that way and overlap `from`'s span on
/// the perpendicular axis; the nearest edge wins, ties broken by the smaller
/// perpendicular offset. Focus never wraps.
pub fn neighbour<L: Clone + Eq>(rects: &[(L, Rect)], from: &L, dir: Direction) -> Option<L> {
    let origin = rects.iter().find(|(l, _)| l == from)?.1;

    let mut best: Option<(L, i32, i32)> = None;
    for (leaf, rect) in rects {
        if leaf == from {
            continue;
        }
        let (ahead, distance, perpendicular) = match dir {
            Direction::Left => (
                rect.x + rect.w <= origin.x,
                origin.x - (rect.x + rect.w),
                (rect.y - origin.y).abs(),
            ),
            Direction::Right => (
                rect.x >= origin.x + origin.w,
                rect.x - (origin.x + origin.w),
                (rect.y - origin.y).abs(),
            ),
            Direction::Up => (
                rect.y + rect.h <= origin.y,
                origin.y - (rect.y + rect.h),
                (rect.x - origin.x).abs(),
            ),
            Direction::Down => (
                rect.y >= origin.y + origin.h,
                rect.y - (origin.y + origin.h),
                (rect.x - origin.x).abs(),
            ),
        };
        if !ahead {
            continue;
        }
        let overlaps = match dir.axis() {
            // Horizontal travel: the candidate has to share some rows.
            Axis::Row => rect.y < origin.y + origin.h && origin.y < rect.y + rect.h,
            Axis::Column => rect.x < origin.x + origin.w && origin.x < rect.x + rect.w,
        };
        if !overlaps {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, d, p)) => distance < *d || (distance == *d && perpendicular < *p),
        };
        if better {
            best = Some((leaf.clone(), distance, perpendicular));
        }
    }
    best.map(|(leaf, _, _)| leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_of_three() -> Tree<u32> {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.insert_next_to(Some(&2), 3, None, true);
        tree.equalize_all();
        tree
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 30,
        w: 1000,
        h: 700,
    };

    #[test]
    fn a_lone_tile_fills_the_area_inside_the_outer_gap() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        let gaps = Gaps {
            inner: 8,
            outer: 8,
            smart: false,
        };
        let rects = resolve(&tree, AREA, gaps);
        assert_eq!(rects, vec![(1, Rect::new(8, 38, 984, 684))]);
    }

    #[test]
    fn smart_gaps_drop_every_gap_for_a_lone_tile() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        let gaps = Gaps {
            inner: 8,
            outer: 8,
            smart: true,
        };
        let rects = resolve(&tree, AREA, gaps);
        assert_eq!(rects, vec![(1, AREA)]);
    }

    #[test]
    fn two_tiles_split_the_area_with_an_inner_gap() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        let gaps = Gaps {
            inner: 10,
            outer: 0,
            smart: true,
        };
        let rects = resolve(&tree, AREA, gaps);
        assert_eq!(
            rects,
            vec![
                (1, Rect::new(0, 30, 495, 700)),
                (2, Rect::new(505, 30, 495, 700)),
            ]
        );
    }

    #[test]
    fn the_last_child_absorbs_the_rounding_remainder() {
        let tree = row_of_three();
        let gaps = Gaps {
            inner: 0,
            outer: 0,
            smart: false,
        };
        // 1000 / 3 does not divide: the cells must still add up to 1000.
        let rects = resolve(&tree, AREA, gaps);
        let total: i32 = rects.iter().map(|(_, r)| r.w).sum();
        assert_eq!(total, 1000);
        assert_eq!(rects[0].1.x, 0);
        assert_eq!(rects[2].1.x + rects[2].1.w, 1000);
    }

    #[test]
    fn gaps_do_not_change_the_total_extent() {
        let tree = row_of_three();
        let gaps = Gaps {
            inner: 7,
            outer: 5,
            smart: false,
        };
        let rects = resolve(&tree, AREA, gaps);
        assert_eq!(rects[0].1.x, 5);
        assert_eq!(rects[2].1.x + rects[2].1.w, 995);
        for w in rects.windows(2) {
            assert_eq!(w[1].1.x - (w[0].1.x + w[0].1.w), 7);
        }
    }

    #[test]
    fn an_empty_tree_resolves_to_nothing() {
        let tree = Tree::<u32>::new();
        assert!(resolve(&tree, AREA, Gaps::default()).is_empty());
    }

    #[test]
    fn neighbour_picks_the_nearest_cell_in_that_direction() {
        let tree = row_of_three();
        let rects = resolve(&tree, AREA, Gaps::default());
        assert_eq!(neighbour(&rects, &1, Direction::Right), Some(2));
        assert_eq!(neighbour(&rects, &2, Direction::Right), Some(3));
        assert_eq!(neighbour(&rects, &3, Direction::Right), None);
        assert_eq!(neighbour(&rects, &2, Direction::Left), Some(1));
        assert_eq!(neighbour(&rects, &1, Direction::Up), None);
    }

    #[test]
    fn neighbour_needs_an_overlap_on_the_perpendicular_axis() {
        // Left column, right column split into a top and a bottom cell.
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, true);
        tree.insert_next_to(Some(&1), 2, None, true);
        tree.insert_next_to(Some(&2), 3, None, false);
        let rects = resolve(&tree, AREA, Gaps::default());
        // From the left cell, right lands on the top-right one: they share the
        // upper rows, and it is the first candidate found at that distance.
        assert_eq!(neighbour(&rects, &1, Direction::Right), Some(2));
        // From the bottom-right cell, up is the top-right one, not the left.
        assert_eq!(neighbour(&rects, &3, Direction::Up), Some(2));
        assert_eq!(neighbour(&rects, &3, Direction::Left), Some(1));
        assert_eq!(neighbour(&rects, &2, Direction::Down), Some(3));
    }
}
