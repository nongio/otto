//! Per-workspace tiling state.
//!
//! Tiling is a property of one workspace on one output (see
//! `specs/tiling.md`), so this hangs off `WorkspaceView` rather than off the
//! compositor.

use smithay::reexports::wayland_server::backend::ObjectId;

use super::design::UndoStack;
use super::tree::{Axis, EmptyId, NodeId, Tree};

/// A bar or corner drag design mode is in the middle of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignDrag {
    /// The split the pointer is moving, as `(container, index)` — stable
    /// across the relayouts the drag itself causes.
    pub bar: (NodeId, usize),
    /// The second split, when a corner is being dragged.
    pub corner: Option<(NodeId, usize)>,
}

/// Everything design mode adds to a workspace's tiling state.
///
/// Kept beside the tree rather than inside it so the tree stays pure: design
/// mode is a way of editing a tree, not a property of one.
#[derive(Debug, Default)]
pub struct TilingDesignState {
    /// Is the pane grid up on this workspace?
    pub active: bool,
    /// The empty slot the next window fills, and whose pane carries the
    /// accent border while nothing is focused.
    pub focused_empty: Option<EmptyId>,
    /// The drag in flight, if any.
    pub drag: Option<DesignDrag>,
    /// Tree snapshots, one per edit.
    pub undo: UndoStack<ObjectId>,
}

impl TilingDesignState {
    /// Leave design mode, forgetting the drag but keeping the undo stack —
    /// the session's undo history outlives one visit to the editor.
    pub fn leave(&mut self) {
        self.active = false;
        self.drag = None;
        self.focused_empty = None;
    }
}

/// Everything one workspace knows about its tiling.
#[derive(Debug, Default)]
pub struct TilingState {
    /// Is this workspace tiling? A floating workspace keeps an empty tree.
    pub enabled: bool,
    /// The tree of split containers and window leaves.
    pub tree: Tree<ObjectId>,
    /// The leaf insertions and directional commands act relative to.
    pub focused: Option<ObjectId>,
    /// Something removed a window from the tree and the workspace's windows
    /// have not been moved into their new cells yet. The removal happens
    /// wherever the window went away — unmap, minimize, a workspace move —
    /// and only the compositor can run the relayout, so it rides on this flag
    /// and is picked up on the next event-loop iteration.
    pub dirty: bool,
    /// An armed split axis: the next insertion splits the focused cell this
    /// way rather than following the cell's shape. Cleared by the insertion.
    pub preselect: Option<Axis>,
    /// The pane grid, its drag and its undo stack.
    pub design: TilingDesignState,
}

impl TilingState {
    /// Arm a split along `axis`, or disarm it when the same axis is asked for
    /// twice (the spec's "pressing the command again disarms it").
    pub fn set_preselect(&mut self, axis: Axis) {
        self.preselect = if self.preselect == Some(axis) {
            None
        } else {
            Some(axis)
        };
    }

    /// Consume the armed split, if any.
    pub fn take_preselect(&mut self) -> Option<Axis> {
        self.preselect.take()
    }

    /// Forget everything: leaving tiling mode empties the tree, and with no
    /// tree there is nothing for design mode to edit.
    pub fn clear(&mut self) {
        self.tree = Tree::default();
        self.focused = None;
        self.preselect = None;
        self.dirty = false;
        self.design = TilingDesignState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_preselect_disarms() {
        let mut state = TilingState::default();
        state.set_preselect(Axis::Row);
        assert_eq!(state.preselect, Some(Axis::Row));
        state.set_preselect(Axis::Row);
        assert_eq!(state.preselect, None);
        state.set_preselect(Axis::Column);
        assert_eq!(state.preselect, Some(Axis::Column));
        assert_eq!(state.take_preselect(), Some(Axis::Column));
        assert_eq!(state.preselect, None);
    }
}
