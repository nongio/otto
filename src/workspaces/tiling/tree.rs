//! The tiling tree: a pure arena of split containers and window leaves.
//!
//! Nothing in this file knows about Smithay, lay-rs or pixels. A leaf is
//! keyed by an opaque id (`ObjectId` at the call site, a `u32` in the tests),
//! and every extent is a *share*: a fraction of the parent container's extent
//! along its axis. Shares within one container always sum to `1.0`, which is
//! what lets a tree be re-fitted to a different output, scale or rotation
//! without losing its shape (see `specs/tiling.md`).
//!
//! Invariants, restored after every mutation:
//!
//! * a container's shares sum to `1.0`;
//! * no container has fewer than two children — one left with a single child
//!   is dissolved into its parent;
//! * every node's `parent` back-pointer agrees with its parent's child list.

use std::fmt::Debug;
use std::hash::Hash;

/// Index into the tree's arena. Only meaningful for the tree that issued it.
pub type NodeId = usize;

/// Smallest and largest share a single child may hold, so a resize can never
/// squeeze a window out of existence.
pub const MIN_SHARE: f32 = 0.05;
pub const MAX_SHARE: f32 = 0.95;

/// The direction a container lays its children out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Children side by side, left to right.
    Row,
    /// Children stacked, top to bottom.
    Column,
}

impl Axis {
    /// The other axis.
    pub fn other(self) -> Axis {
        match self {
            Axis::Row => Axis::Column,
            Axis::Column => Axis::Row,
        }
    }
}

/// A direction for focus, movement and resize commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The container axis this direction travels along.
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Row,
            Direction::Up | Direction::Down => Axis::Column,
        }
    }

    /// Does this direction go towards *later* children of a container?
    pub fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

/// One child of a container: the node, and the fraction of the container's
/// extent it takes along the container's axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Child {
    pub node: NodeId,
    pub share: f32,
}

/// Identity of an empty slot, unique within one tree for as long as the slot
/// lives. Not a [`NodeId`]: arena slots are recycled, and a slot has to stay
/// recognisable across the relayouts a design-mode edit triggers.
pub type EmptyId = u32;

/// A node of the tree.
#[derive(Debug, Clone)]
pub enum Node<L> {
    /// A window.
    Leaf(L),
    /// A cell reserved by design mode with no window in it yet. It takes up
    /// space in the layout exactly like a leaf; the next window to map fills
    /// it (`specs/tiling.md`, *Design mode*).
    Empty(EmptyId),
    /// A split holding two or more children in order.
    Container { axis: Axis, children: Vec<Child> },
}

/// What occupies one cell of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell<L> {
    Window(L),
    Empty(EmptyId),
}

impl<L> Cell<L> {
    pub fn window(&self) -> Option<&L> {
        match self {
            Cell::Window(l) => Some(l),
            Cell::Empty(_) => None,
        }
    }

    pub fn is_empty_slot(&self) -> bool {
        matches!(self, Cell::Empty(_))
    }
}

#[derive(Debug, Clone)]
struct Slot<L> {
    node: Node<L>,
    parent: Option<NodeId>,
}

/// A tiling tree over leaves of type `L`.
#[derive(Debug, Clone)]
pub struct Tree<L> {
    slots: Vec<Option<Slot<L>>>,
    free: Vec<NodeId>,
    root: Option<NodeId>,
    /// Never reused, so an empty slot's identity outlives an arena slot.
    next_empty: EmptyId,
}

impl<L> Default for Tree<L> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            root: None,
            next_empty: 0,
        }
    }
}

impl<L: Clone + Eq + Hash + Debug> Tree<L> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The root node, or `None` for an empty tree.
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// The node at `id`, if it is still live.
    pub fn node(&self, id: NodeId) -> Option<&Node<L>> {
        self.slots.get(id).and_then(|s| s.as_ref()).map(|s| &s.node)
    }

    /// The container `id` belongs to, or `None` for the root.
    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        self.slots
            .get(id)
            .and_then(|s| s.as_ref())
            .and_then(|s| s.parent)
    }

    /// Every leaf, in layout order (left to right, top to bottom).
    pub fn leaves(&self) -> Vec<L> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.collect_leaves(root, &mut out);
        }
        out
    }

    /// Every leaf under `id`, in layout order. A leaf node yields itself.
    pub fn leaves_under(&self, id: NodeId) -> Vec<L> {
        let mut out = Vec::new();
        if self.node(id).is_some() {
            self.collect_leaves(id, &mut out);
        }
        out
    }

    /// The split axis of `id`, or `None` when it is a leaf.
    pub fn container_axis(&self, id: NodeId) -> Option<Axis> {
        self.axis_of(id)
    }

    /// The children of `id`, or an empty list when it is a leaf.
    pub fn children_of(&self, id: NodeId) -> Vec<Child> {
        match self.node(id) {
            Some(Node::Container { children, .. }) => children.clone(),
            _ => Vec::new(),
        }
    }

    /// Turn a container's split the other way (i3's `layout splith` /
    /// `splitv`). `false` when `id` is a leaf, or already lies that way.
    pub fn set_container_axis(&mut self, id: NodeId, axis: Axis) -> bool {
        match self.node_mut(id) {
            Some(Node::Container { axis: current, .. }) if *current != axis => {
                *current = axis;
                true
            }
            _ => false,
        }
    }

    /// Every window leaf's *node id*, in layout order. Empty slots are not
    /// leaves: nothing that acts on a window has anything to do there.
    pub fn leaf_nodes(&self) -> Vec<NodeId> {
        self.cell_nodes()
            .into_iter()
            .filter(|id| matches!(self.node(*id), Some(Node::Leaf(_))))
            .collect()
    }

    /// Every *cell* node id — windows and empty slots alike — in layout
    /// order. This is what takes up space, so it is what the layout counts.
    pub fn cell_nodes(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.collect_cell_nodes(root, &mut out);
        }
        out
    }

    /// What sits in `id`, when it is a cell rather than a container.
    pub fn cell(&self, id: NodeId) -> Option<Cell<L>> {
        match self.node(id)? {
            Node::Leaf(l) => Some(Cell::Window(l.clone())),
            Node::Empty(e) => Some(Cell::Empty(*e)),
            Node::Container { .. } => None,
        }
    }

    /// The number of cells: windows plus empty slots.
    pub fn len(&self) -> usize {
        self.cell_nodes().len()
    }

    /// The container axis of `id`, or `None` when it is a cell.
    pub fn axis(&self, id: NodeId) -> Option<Axis> {
        self.axis_of(id)
    }

    /// A container's children, in order. Empty for a cell.
    pub fn children(&self, id: NodeId) -> Vec<Child> {
        match self.node(id) {
            Some(Node::Container { children, .. }) => children.clone(),
            _ => Vec::new(),
        }
    }

    // ── Empty slots ──────────────────────────────────────────────────────

    /// Mint an identity for a new empty slot.
    pub fn new_empty_id(&mut self) -> EmptyId {
        let id = self.next_empty;
        self.next_empty = self.next_empty.wrapping_add(1);
        id
    }

    /// Every empty slot, in layout order.
    pub fn empty_slots(&self) -> Vec<EmptyId> {
        self.cell_nodes()
            .into_iter()
            .filter_map(|id| match self.node(id) {
                Some(Node::Empty(e)) => Some(*e),
                _ => None,
            })
            .collect()
    }

    /// The node holding empty slot `slot`.
    pub fn node_of_empty(&self, slot: EmptyId) -> Option<NodeId> {
        self.cell_nodes()
            .into_iter()
            .find(|id| matches!(self.node(*id), Some(Node::Empty(e)) if *e == slot))
    }

    /// The empty slot a newly mapped window should fill: `preferred` when it
    /// is still there, otherwise the first one in layout order.
    pub fn next_empty(&self, preferred: Option<EmptyId>) -> Option<EmptyId> {
        let slots = self.empty_slots();
        match preferred {
            Some(p) if slots.contains(&p) => Some(p),
            _ => slots.first().copied(),
        }
    }

    /// Put `leaf` into empty slot `slot`, keeping the slot's share.
    pub fn fill_empty(&mut self, slot: EmptyId, leaf: L) -> bool {
        let Some(node) = self.node_of_empty(slot) else {
            return false;
        };
        if let Some(n) = self.node_mut(node) {
            *n = Node::Leaf(leaf);
            return true;
        }
        false
    }

    /// Drop an empty slot, the way [`Tree::remove`] drops a window.
    pub fn remove_empty(&mut self, slot: EmptyId) -> bool {
        match self.node_of_empty(slot) {
            Some(node) => {
                self.remove_node(node);
                true
            }
            None => false,
        }
    }

    /// Replace the cell at `node` with a container of `axis` holding that cell
    /// and a fresh empty slot, half each. Returns the new slot's id.
    ///
    /// This is design mode's "split this cell": the window stays where it is
    /// and the space it gives up becomes a slot to drop something into.
    pub fn split_cell(&mut self, node: NodeId, axis: Axis) -> Option<EmptyId> {
        if !matches!(self.node(node), Some(Node::Leaf(_)) | Some(Node::Empty(_))) {
            return None;
        }
        let slot = self.new_empty_id();
        let parent = self.parent_of(node);
        let new_node = self.alloc(Node::Empty(slot), None);
        let container = self.alloc(
            Node::Container {
                axis,
                children: vec![
                    Child { node, share: 0.5 },
                    Child {
                        node: new_node,
                        share: 0.5,
                    },
                ],
            },
            parent,
        );
        self.set_parent(node, Some(container));
        self.set_parent(new_node, Some(container));
        self.replace_in_parent(node, container, parent);
        Some(slot)
    }

    /// Take one cell node out of the tree, restoring every invariant.
    pub fn remove_node(&mut self, node: NodeId) {
        match self.parent_of(node) {
            None => {
                if self.root == Some(node) {
                    self.root = None;
                }
                self.free_node(node);
            }
            Some(parent) => {
                self.detach_child(parent, node);
                self.free_node(node);
                self.dissolve_upwards(parent);
            }
        }
    }

    // ── Design-mode share editing ────────────────────────────────────────

    /// Set the shares of the two children either side of the bar at `index`
    /// in `container`, keeping their sum — and so every other child's share —
    /// exactly as it was.
    pub fn set_pair_shares(&mut self, container: NodeId, index: usize, first: f32) -> bool {
        let Some(Node::Container { children, .. }) = self.node_mut(container) else {
            return false;
        };
        if index + 1 >= children.len() {
            return false;
        }
        let pair = children[index].share + children[index + 1].share;
        let first = first.clamp(MIN_SHARE.min(pair), (pair - MIN_SHARE).max(0.0));
        if (children[index].share - first).abs() < f32::EPSILON {
            return false;
        }
        children[index].share = first;
        children[index + 1].share = pair - first;
        true
    }

    /// Replace the whole tree with `cells` laid out by `builder`. Used by the
    /// design-mode presets, which build a shape out of empty slots.
    pub fn set_root(&mut self, root: Option<NodeId>) {
        self.root = root;
        if let Some(root) = root {
            self.set_parent(root, None);
        }
    }

    /// Allocate a bare empty-slot node, unparented. Callers wire it up.
    pub fn alloc_empty(&mut self) -> NodeId {
        let slot = self.new_empty_id();
        self.alloc(Node::Empty(slot), None)
    }

    /// Allocate a container over `children`, equally shared, and adopt them.
    pub fn alloc_container(&mut self, axis: Axis, children: &[NodeId]) -> NodeId {
        let share = if children.is_empty() {
            1.0
        } else {
            1.0 / children.len() as f32
        };
        let kids: Vec<Child> = children
            .iter()
            .map(|node| Child { node: *node, share })
            .collect();
        let container = self.alloc(
            Node::Container {
                axis,
                children: kids,
            },
            None,
        );
        for child in children {
            self.set_parent(*child, Some(container));
        }
        container
    }

    /// Is `leaf` in this tree?
    pub fn contains(&self, leaf: &L) -> bool {
        self.node_of(leaf).is_some()
    }

    /// The node holding `leaf`.
    pub fn node_of(&self, leaf: &L) -> Option<NodeId> {
        self.slots
            .iter()
            .enumerate()
            .find_map(|(id, slot)| match slot {
                Some(Slot {
                    node: Node::Leaf(l),
                    ..
                }) if l == leaf => Some(id),
                _ => None,
            })
    }

    /// The container the leaf sits in, if it is not the lone root.
    pub fn container_of(&self, leaf: &L) -> Option<NodeId> {
        self.node_of(leaf).and_then(|n| self.parent_of(n))
    }

    // ── Insertion ────────────────────────────────────────────────────────

    /// Insert `new_leaf` next to `focused`, per the spec's insertion rule.
    ///
    /// * an empty tree gains `new_leaf` as its root;
    /// * with a `preselect` axis armed, the focused leaf is replaced by a new
    ///   container of that axis holding the old leaf and the new one, 50/50;
    /// * otherwise, when the focused leaf's parent already splits along the
    ///   shape of its cell (`cell_is_wide` ⇒ [`Axis::Row`]), the new leaf
    ///   becomes a sibling right after it, taking half of its share;
    /// * otherwise the focused leaf is split along that axis, 50/50.
    ///
    /// With no focused leaf the last leaf in layout order stands in for it.
    pub fn insert_next_to(
        &mut self,
        focused: Option<&L>,
        new_leaf: L,
        preselect: Option<Axis>,
        cell_is_wide: bool,
    ) -> NodeId {
        if self.root.is_none() {
            let n = self.alloc(Node::Leaf(new_leaf), None);
            self.root = Some(n);
            return n;
        }

        let target = focused
            .and_then(|f| self.node_of(f))
            .or_else(|| self.cell_nodes().last().copied());
        let Some(target) = target else {
            let n = self.alloc(Node::Leaf(new_leaf), None);
            self.root = Some(n);
            return n;
        };

        let shape_axis = if cell_is_wide {
            Axis::Row
        } else {
            Axis::Column
        };
        let axis = preselect.unwrap_or(shape_axis);

        // Without a pre-selection, a parent that already splits along the
        // shape's axis takes the new window as a plain sibling: that is what
        // keeps successive windows filling the screen in a spiral instead of
        // nesting a container per window.
        if preselect.is_none() {
            if let Some(parent) = self.parent_of(target) {
                if self.axis_of(parent) == Some(axis) {
                    return self.insert_sibling_after(parent, target, new_leaf);
                }
            }
        }

        self.split_leaf(target, axis, new_leaf)
    }

    /// Replace the leaf at `target` with a container of `axis` holding the old
    /// leaf and `new_leaf`, half each. Returns the new leaf's node.
    fn split_leaf(&mut self, target: NodeId, axis: Axis, new_leaf: L) -> NodeId {
        self.split_leaf_at(target, axis, new_leaf, true)
    }

    /// [`Self::split_leaf`] with the new leaf on the chosen side: `after`
    /// puts it right of a [`Axis::Row`] split, below a [`Axis::Column`] one.
    fn split_leaf_at(&mut self, target: NodeId, axis: Axis, new_leaf: L, after: bool) -> NodeId {
        let parent = self.parent_of(target);
        let new_node = self.alloc(Node::Leaf(new_leaf), None);
        let old = Child {
            node: target,
            share: 0.5,
        };
        let fresh = Child {
            node: new_node,
            share: 0.5,
        };
        let children = if after {
            vec![old, fresh]
        } else {
            vec![fresh, old]
        };
        let container = self.alloc(Node::Container { axis, children }, parent);
        self.set_parent(target, Some(container));
        self.set_parent(new_node, Some(container));
        self.replace_in_parent(target, container, parent);
        new_node
    }

    /// Add `new_leaf` right after `after` in `parent`, taking half of
    /// `after`'s share.
    fn insert_sibling_after(&mut self, parent: NodeId, after: NodeId, new_leaf: L) -> NodeId {
        self.insert_sibling_at(parent, after, new_leaf, true)
    }

    /// Add `new_leaf` beside `anchor` in `parent`, before or after it, taking
    /// half of `anchor`'s share.
    fn insert_sibling_at(
        &mut self,
        parent: NodeId,
        anchor: NodeId,
        new_leaf: L,
        after: bool,
    ) -> NodeId {
        let new_node = self.alloc(Node::Leaf(new_leaf), Some(parent));
        if let Some(Node::Container { children, .. }) = self.node_mut(parent) {
            if let Some(index) = children.iter().position(|c| c.node == anchor) {
                let half = children[index].share / 2.0;
                children[index].share = half;
                children.insert(
                    if after { index + 1 } else { index },
                    Child {
                        node: new_node,
                        share: half,
                    },
                );
            } else {
                children.push(Child {
                    node: new_node,
                    share: 0.0,
                });
            }
        }
        self.normalize(parent);
        new_node
    }

    /// Insert `new_leaf` beside the cell at `target`, on a named side.
    ///
    /// The pointer's answer to [`Self::insert_next_to`]: a drop names the side
    /// it landed on, so the window goes left of the tile it was dropped on the
    /// left half of, rather than always after it. A parent that already splits
    /// along `axis` takes the new leaf as a plain sibling; otherwise the
    /// target is split, the new leaf on the chosen side.
    pub fn insert_beside(
        &mut self,
        target: NodeId,
        new_leaf: L,
        axis: Axis,
        after: bool,
    ) -> Option<NodeId> {
        if self.node(target).is_none() {
            return None;
        }
        if let Some(parent) = self.parent_of(target) {
            if self.axis_of(parent) == Some(axis) {
                return Some(self.insert_sibling_at(parent, target, new_leaf, after));
            }
        }
        Some(self.split_leaf_at(target, axis, new_leaf, after))
    }

    /// Exchange two leaves' windows, cells and shares staying where they are.
    ///
    /// What a drop on the middle of a tile means: the two windows trade
    /// places and the layout does not otherwise move.
    pub fn swap_leaves(&mut self, a: &L, b: &L) -> bool {
        if a == b {
            return false;
        }
        let (Some(na), Some(nb)) = (self.node_of(a), self.node_of(b)) else {
            return false;
        };
        if let Some(Node::Leaf(leaf)) = self.node_mut(na) {
            *leaf = b.clone();
        }
        if let Some(Node::Leaf(leaf)) = self.node_mut(nb) {
            *leaf = a.clone();
        }
        true
    }

    /// Put `new_leaf` against the outside of the whole tree, on `axis` and at
    /// the near or far end of it.
    ///
    /// Where a drop that missed every tile lands. A root that already splits
    /// along `axis` gains one more child at that end, each child keeping its
    /// proportion of what is left; otherwise the root is wrapped in a new
    /// container and the two halve the area.
    pub fn insert_at_root_edge(&mut self, new_leaf: L, axis: Axis, after: bool) -> NodeId {
        let Some(root) = self.root else {
            let node = self.alloc(Node::Leaf(new_leaf), None);
            self.root = Some(node);
            return node;
        };
        if self.axis_of(root) == Some(axis) {
            let new_node = self.alloc(Node::Leaf(new_leaf), Some(root));
            if let Some(Node::Container { children, .. }) = self.node_mut(root) {
                let share = 1.0 / (children.len() + 1) as f32;
                for child in children.iter_mut() {
                    child.share *= 1.0 - share;
                }
                let at = if after { children.len() } else { 0 };
                children.insert(
                    at,
                    Child {
                        node: new_node,
                        share,
                    },
                );
            }
            self.normalize(root);
            return new_node;
        }

        let new_node = self.alloc(Node::Leaf(new_leaf), None);
        let old = Child {
            node: root,
            share: 0.5,
        };
        let fresh = Child {
            node: new_node,
            share: 0.5,
        };
        let children = if after {
            vec![old, fresh]
        } else {
            vec![fresh, old]
        };
        let container = self.alloc(Node::Container { axis, children }, None);
        self.set_parent(root, Some(container));
        self.set_parent(new_node, Some(container));
        self.root = Some(container);
        new_node
    }

    // ── Removal ──────────────────────────────────────────────────────────

    /// Drop `leaf` from the tree, giving its share to its siblings in
    /// proportion to theirs and dissolving any container left with one child.
    ///
    /// Returns `true` when the leaf was there.
    pub fn remove(&mut self, leaf: &L) -> bool {
        let Some(node) = self.node_of(leaf) else {
            return false;
        };
        self.remove_node(node);
        true
    }

    /// Take `child` out of `container`, spreading its share over what is left.
    fn detach_child(&mut self, container: NodeId, child: NodeId) {
        let Some(Node::Container { children, .. }) = self.node_mut(container) else {
            return;
        };
        let Some(index) = children.iter().position(|c| c.node == child) else {
            return;
        };
        let freed = children.remove(index).share;
        let rest: f32 = children.iter().map(|c| c.share).sum();
        if children.is_empty() {
            return;
        }
        if rest > f32::EPSILON {
            for c in children.iter_mut() {
                c.share += freed * (c.share / rest);
            }
        } else {
            let equal = 1.0 / children.len() as f32;
            for c in children.iter_mut() {
                c.share = equal;
            }
        }
        self.normalize(container);
    }

    /// Dissolve `container` into its parent while it holds a single child,
    /// walking upwards. Returns the node that ends up in `container`'s place.
    fn dissolve_upwards(&mut self, container: NodeId) -> NodeId {
        let mut current = container;
        // What now occupies the slot `container` held: itself when nothing
        // dissolved, otherwise the child that was promoted into its place.
        let mut anchor = container;
        loop {
            let single = match self.node(current) {
                Some(Node::Container { children, .. }) if children.len() == 1 => {
                    Some(children[0].node)
                }
                _ => None,
            };
            let Some(only) = single else {
                return anchor;
            };
            let parent = self.parent_of(current);
            self.set_parent(only, parent);
            self.replace_in_parent(current, only, parent);
            self.free_node(current);
            anchor = only;
            match parent {
                Some(p) => current = p,
                None => return anchor,
            }
        }
    }

    // ── Movement ─────────────────────────────────────────────────────────

    /// Move `leaf` one step in `dir`.
    ///
    /// Along its container's axis it swaps with the neighbouring sibling;
    /// at the edge, or across the axis, it leaves the container and is
    /// inserted into the grandparent at the corresponding side — splitting
    /// that ancestor (the root included) when the axis does not match.
    pub fn move_dir(&mut self, leaf: &L, dir: Direction) -> bool {
        let Some(node) = self.node_of(leaf) else {
            return false;
        };
        let Some(parent) = self.parent_of(node) else {
            // A lone root leaf has nowhere to go.
            return false;
        };

        if self.axis_of(parent) == Some(dir.axis()) {
            let (index, len) = {
                let Some(Node::Container { children, .. }) = self.node(parent) else {
                    return false;
                };
                let Some(index) = children.iter().position(|c| c.node == node) else {
                    return false;
                };
                (index, children.len())
            };
            let target = if dir.is_forward() {
                index + 1
            } else {
                index.wrapping_sub(1)
            };
            if target < len {
                if let Some(Node::Container { children, .. }) = self.node_mut(parent) {
                    // The whole entry swaps, so the share travels with the
                    // window rather than staying with the slot.
                    children.swap(index, target);
                }
                return true;
            }
        }

        self.move_out(node, dir)
    }

    /// Lift `node` out of its container and re-insert it one level up.
    fn move_out(&mut self, node: NodeId, dir: Direction) -> bool {
        let Some(parent) = self.parent_of(node) else {
            return false;
        };
        self.detach_child(parent, node);
        self.set_parent(node, None);
        let anchor = self.dissolve_upwards(parent);
        let grandparent = self.parent_of(anchor);

        match grandparent {
            Some(g) if self.axis_of(g) == Some(dir.axis()) => {
                let index = match self.node(g) {
                    Some(Node::Container { children, .. }) => {
                        children.iter().position(|c| c.node == anchor).unwrap_or(0)
                    }
                    _ => 0,
                };
                let at = if dir.is_forward() { index + 1 } else { index };
                self.set_parent(node, Some(g));
                if let Some(Node::Container { children, .. }) = self.node_mut(g) {
                    let at = at.min(children.len());
                    children.insert(at, Child { node, share: 0.0 });
                }
                // Equalising the container the window landed in is the simple
                // rule for this cut; a share that survives the trip through
                // two levels of nesting is a later refinement.
                self.equalize(g);
                true
            }
            other => {
                // Axis mismatch (or no grandparent at all): wrap the anchor —
                // the root, when it has no parent — in a new container of the
                // travelled axis and put the window on the matching side.
                let container = self.alloc(
                    Node::Container {
                        axis: dir.axis(),
                        children: Vec::new(),
                    },
                    other,
                );
                let children = if dir.is_forward() {
                    vec![
                        Child {
                            node: anchor,
                            share: 0.5,
                        },
                        Child { node, share: 0.5 },
                    ]
                } else {
                    vec![
                        Child { node, share: 0.5 },
                        Child {
                            node: anchor,
                            share: 0.5,
                        },
                    ]
                };
                if let Some(Node::Container { children: slot, .. }) = self.node_mut(container) {
                    *slot = children;
                }
                self.set_parent(anchor, Some(container));
                self.set_parent(node, Some(container));
                self.replace_in_parent(anchor, container, other);
                true
            }
        }
    }

    // ── Resize ───────────────────────────────────────────────────────────

    /// Grow `leaf`'s share along `axis` by `delta`, taking it from the next
    /// sibling (or the previous one when the leaf is last).
    ///
    /// The nearest ancestor container that splits along `axis` and has more
    /// than one child is the one that moves, so a resize command does
    /// something as long as more than one window is tiled. Shares are clamped
    /// to `[MIN_SHARE, MAX_SHARE]`.
    pub fn resize(&mut self, leaf: &L, axis: Axis, delta: f32) -> bool {
        let Some(node) = self.node_of(leaf) else {
            return false;
        };

        let mut child = node;
        let mut container = self.parent_of(node);
        while let Some(c) = container {
            let matches = matches!(
                self.node(c),
                Some(Node::Container { axis: a, children }) if *a == axis && children.len() > 1
            );
            if matches {
                break;
            }
            child = c;
            container = self.parent_of(c);
        }
        let Some(container) = container else {
            return false;
        };

        let Some(Node::Container { children, .. }) = self.node_mut(container) else {
            return false;
        };
        let Some(index) = children.iter().position(|c| c.node == child) else {
            return false;
        };
        let other = if index + 1 < children.len() {
            index + 1
        } else if index > 0 {
            index - 1
        } else {
            return false;
        };

        let mine = children[index].share;
        let theirs = children[other].share;
        let mut applied = mine.clamp(MIN_SHARE, MAX_SHARE) + delta;
        applied = applied.clamp(MIN_SHARE, MAX_SHARE) - mine;
        if theirs - applied < MIN_SHARE {
            applied = theirs - MIN_SHARE;
        }
        if applied.abs() < f32::EPSILON {
            return false;
        }
        children[index].share = mine + applied;
        children[other].share = theirs - applied;
        self.normalize(container);
        true
    }

    // ── Equalising ───────────────────────────────────────────────────────

    /// Give every child of `container` the same share.
    pub fn equalize(&mut self, container: NodeId) {
        if let Some(Node::Container { children, .. }) = self.node_mut(container) {
            if children.is_empty() {
                return;
            }
            let equal = 1.0 / children.len() as f32;
            for c in children.iter_mut() {
                c.share = equal;
            }
        }
    }

    /// Equalise the container `leaf` sits in.
    pub fn equalize_container_of(&mut self, leaf: &L) -> bool {
        match self.container_of(leaf) {
            Some(container) => {
                self.equalize(container);
                true
            }
            None => false,
        }
    }

    /// Equalise every container in the tree.
    pub fn equalize_all(&mut self) {
        let ids: Vec<NodeId> = (0..self.slots.len())
            .filter(|id| matches!(self.node(*id), Some(Node::Container { .. })))
            .collect();
        for id in ids {
            self.equalize(id);
        }
    }

    // ── Arena plumbing ───────────────────────────────────────────────────

    fn axis_of(&self, id: NodeId) -> Option<Axis> {
        match self.node(id) {
            Some(Node::Container { axis, .. }) => Some(*axis),
            _ => None,
        }
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node<L>> {
        self.slots
            .get_mut(id)
            .and_then(|s| s.as_mut())
            .map(|s| &mut s.node)
    }

    fn alloc(&mut self, node: Node<L>, parent: Option<NodeId>) -> NodeId {
        let slot = Slot { node, parent };
        match self.free.pop() {
            Some(id) => {
                self.slots[id] = Some(slot);
                id
            }
            None => {
                self.slots.push(Some(slot));
                self.slots.len() - 1
            }
        }
    }

    fn free_node(&mut self, id: NodeId) {
        if id < self.slots.len() && self.slots[id].is_some() {
            self.slots[id] = None;
            self.free.push(id);
        }
    }

    fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) {
        if let Some(Some(slot)) = self.slots.get_mut(id) {
            slot.parent = parent;
        }
    }

    /// Put `new` where `old` sat in `parent` (or make it the root), keeping
    /// the share `old` held.
    fn replace_in_parent(&mut self, old: NodeId, new: NodeId, parent: Option<NodeId>) {
        match parent {
            None => {
                self.root = Some(new);
                self.set_parent(new, None);
            }
            Some(p) => {
                if let Some(Node::Container { children, .. }) = self.node_mut(p) {
                    if let Some(entry) = children.iter_mut().find(|c| c.node == old) {
                        entry.node = new;
                    }
                }
                self.set_parent(new, Some(p));
            }
        }
    }

    /// Re-scale a container's shares so they sum to one.
    fn normalize(&mut self, container: NodeId) {
        if let Some(Node::Container { children, .. }) = self.node_mut(container) {
            if children.is_empty() {
                return;
            }
            let total: f32 = children.iter().map(|c| c.share).sum();
            if total.abs() < f32::EPSILON {
                let equal = 1.0 / children.len() as f32;
                for c in children.iter_mut() {
                    c.share = equal;
                }
            } else {
                for c in children.iter_mut() {
                    c.share /= total;
                }
            }
        }
    }

    fn collect_leaves(&self, id: NodeId, out: &mut Vec<L>) {
        match self.node(id) {
            Some(Node::Leaf(l)) => out.push(l.clone()),
            Some(Node::Container { children, .. }) => {
                let kids: Vec<NodeId> = children.iter().map(|c| c.node).collect();
                for k in kids {
                    self.collect_leaves(k, out);
                }
            }
            Some(Node::Empty(_)) | None => {}
        }
    }

    fn collect_cell_nodes(&self, id: NodeId, out: &mut Vec<NodeId>) {
        match self.node(id) {
            Some(Node::Leaf(_)) | Some(Node::Empty(_)) => out.push(id),
            Some(Node::Container { children, .. }) => {
                let kids: Vec<NodeId> = children.iter().map(|c| c.node).collect();
                for k in kids {
                    self.collect_cell_nodes(k, out);
                }
            }
            None => {}
        }
    }

    /// Panic unless every invariant holds. Test-only.
    #[cfg(test)]
    fn assert_invariants(&self) {
        let Some(root) = self.root else {
            return;
        };
        self.assert_node(root, None);
    }

    #[cfg(test)]
    fn assert_node(&self, id: NodeId, parent: Option<NodeId>) {
        assert_eq!(self.parent_of(id), parent, "parent back-pointer of {id}");
        if let Some(Node::Container { children, .. }) = self.node(id) {
            assert!(
                children.len() >= 2,
                "container {id} has {} children",
                children.len()
            );
            let total: f32 = children.iter().map(|c| c.share).sum();
            assert!(
                (total - 1.0).abs() < 1e-4,
                "container {id} shares sum to {total}"
            );
            for c in children {
                self.assert_node(c.node, Some(id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shares(tree: &Tree<u32>, container: NodeId) -> Vec<f32> {
        match tree.node(container) {
            Some(Node::Container { children, .. }) => children.iter().map(|c| c.share).collect(),
            _ => Vec::new(),
        }
    }

    fn wide(tree: &mut Tree<u32>, focused: Option<u32>, leaf: u32) {
        tree.insert_next_to(focused.as_ref(), leaf, None, true);
    }

    #[test]
    fn first_window_becomes_the_root() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        assert_eq!(tree.leaves(), vec![1]);
        assert!(tree.parent_of(tree.root().unwrap()).is_none());
        tree.assert_invariants();
    }

    #[test]
    fn a_wide_cell_splits_into_a_row() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        let root = tree.root().unwrap();
        assert!(matches!(
            tree.node(root),
            Some(Node::Container {
                axis: Axis::Row,
                ..
            })
        ));
        assert_eq!(shares(&tree, root), vec![0.5, 0.5]);
        assert_eq!(tree.leaves(), vec![1, 2]);
        tree.assert_invariants();
    }

    #[test]
    fn a_tall_cell_splits_into_a_column() {
        let mut tree = Tree::<u32>::new();
        tree.insert_next_to(None, 1, None, false);
        tree.insert_next_to(Some(&1), 2, None, false);
        let root = tree.root().unwrap();
        assert!(matches!(
            tree.node(root),
            Some(Node::Container {
                axis: Axis::Column,
                ..
            })
        ));
        tree.assert_invariants();
    }

    #[test]
    fn a_matching_parent_axis_takes_a_sibling_at_half_the_focused_share() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        // 2's parent is a Row and its cell is still wide: 3 becomes a sibling.
        wide(&mut tree, Some(2), 3);
        let root = tree.root().unwrap();
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        assert_eq!(shares(&tree, root), vec![0.5, 0.25, 0.25]);
        tree.assert_invariants();
    }

    #[test]
    fn a_mismatched_shape_splits_the_focused_cell() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        // 2's cell is now tall, so 3 splits it into a column instead.
        tree.insert_next_to(Some(&2), 3, None, false);
        let root = tree.root().unwrap();
        assert_eq!(shares(&tree, root), vec![0.5, 0.5]);
        let second = match tree.node(root) {
            Some(Node::Container { children, .. }) => children[1].node,
            _ => unreachable!(),
        };
        assert!(matches!(
            tree.node(second),
            Some(Node::Container {
                axis: Axis::Column,
                ..
            })
        ));
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        tree.assert_invariants();
    }

    #[test]
    fn a_preselect_splits_even_when_the_axis_matches() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        tree.insert_next_to(Some(&2), 3, Some(Axis::Column), true);
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        let root = tree.root().unwrap();
        assert_eq!(shares(&tree, root), vec![0.5, 0.5]);
        tree.assert_invariants();
    }

    #[test]
    fn removal_redistributes_proportionally() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        wide(&mut tree, Some(2), 3);
        // shares: 0.5, 0.25, 0.25 — dropping the first gives the rest half each
        // of what it held, in proportion: 0.25/0.5 of 0.5 to each.
        assert!(tree.remove(&1));
        let root = tree.root().unwrap();
        assert_eq!(tree.leaves(), vec![2, 3]);
        let s = shares(&tree, root);
        assert!((s[0] - 0.5).abs() < 1e-5, "{s:?}");
        assert!((s[1] - 0.5).abs() < 1e-5, "{s:?}");
        tree.assert_invariants();
    }

    #[test]
    fn a_container_left_with_one_child_dissolves() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        tree.insert_next_to(Some(&2), 3, None, false);
        // Dropping 3 leaves the column with one child: it must vanish.
        assert!(tree.remove(&3));
        let root = tree.root().unwrap();
        match tree.node(root) {
            Some(Node::Container { children, .. }) => {
                assert_eq!(children.len(), 2);
                assert!(children
                    .iter()
                    .all(|c| matches!(tree.node(c.node), Some(Node::Leaf(_)))));
            }
            other => panic!("unexpected root {other:?}"),
        }
        tree.assert_invariants();
    }

    #[test]
    fn removing_the_last_leaf_empties_the_tree() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        assert!(tree.remove(&1));
        assert!(tree.is_empty());
        assert_eq!(tree.leaves(), Vec::<u32>::new());
    }

    #[test]
    fn move_swaps_with_the_sibling_along_the_axis() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        assert!(tree.move_dir(&1, Direction::Right));
        assert_eq!(tree.leaves(), vec![2, 1]);
        tree.assert_invariants();
    }

    #[test]
    fn move_at_the_edge_splits_the_root_along_the_new_axis() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        wide(&mut tree, Some(2), 3);
        // 3 is last in a row; moving it down leaves the row and splits the
        // root into a column with the row on top.
        assert!(tree.move_dir(&3, Direction::Down));
        let root = tree.root().unwrap();
        assert!(matches!(
            tree.node(root),
            Some(Node::Container {
                axis: Axis::Column,
                ..
            })
        ));
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        tree.assert_invariants();
    }

    #[test]
    fn move_out_of_a_nested_container_joins_the_grandparent() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        tree.insert_next_to(Some(&2), 3, None, false); // column [2,3] on the right
                                                       // 3 is the bottom of the column; moving it right leaves the column and
                                                       // joins the root row after it. The column then dissolves.
        assert!(tree.move_dir(&3, Direction::Right));
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        let root = tree.root().unwrap();
        assert_eq!(shares(&tree, root).len(), 3);
        tree.assert_invariants();
    }

    #[test]
    fn a_lone_root_leaf_cannot_move() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        assert!(!tree.move_dir(&1, Direction::Left));
    }

    #[test]
    fn resize_takes_from_the_next_sibling_and_clamps() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        assert!(tree.resize(&1, Axis::Row, 0.1));
        let s = shares(&tree, tree.root().unwrap());
        assert!((s[0] - 0.6).abs() < 1e-5, "{s:?}");
        assert!((s[1] - 0.4).abs() < 1e-5, "{s:?}");

        // Grow past the clamp: the sibling never drops below MIN_SHARE.
        for _ in 0..20 {
            tree.resize(&1, Axis::Row, 0.1);
        }
        let s = shares(&tree, tree.root().unwrap());
        assert!(s[1] >= MIN_SHARE - 1e-5, "{s:?}");
        assert!(s[0] <= MAX_SHARE + 1e-5, "{s:?}");
        tree.assert_invariants();
    }

    #[test]
    fn resize_climbs_to_the_nearest_ancestor_on_that_axis() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        tree.insert_next_to(Some(&2), 3, None, false); // column [2,3]
                                                       // 3 has no row sibling — the resize has to move the root row's split.
        assert!(tree.resize(&3, Axis::Row, 0.1));
        let s = shares(&tree, tree.root().unwrap());
        assert!((s[0] - 0.4).abs() < 1e-5, "{s:?}");
        assert!((s[1] - 0.6).abs() < 1e-5, "{s:?}");
        tree.assert_invariants();
    }

    #[test]
    fn resize_on_a_lone_tile_does_nothing() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        assert!(!tree.resize(&1, Axis::Row, 0.1));
    }

    #[test]
    fn equalize_evens_out_a_container() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        wide(&mut tree, Some(2), 3);
        assert!(tree.equalize_container_of(&1));
        let s = shares(&tree, tree.root().unwrap());
        for share in &s {
            assert!((share - 1.0 / 3.0).abs() < 1e-5, "{s:?}");
        }
        tree.assert_invariants();
    }

    #[test]
    fn equalize_all_evens_out_every_container() {
        let mut tree = Tree::<u32>::new();
        wide(&mut tree, None, 1);
        wide(&mut tree, Some(1), 2);
        tree.insert_next_to(Some(&2), 3, None, false);
        tree.resize(&1, Axis::Row, 0.2);
        tree.equalize_all();
        let s = shares(&tree, tree.root().unwrap());
        assert!((s[0] - 0.5).abs() < 1e-5, "{s:?}");
        tree.assert_invariants();
    }
}
