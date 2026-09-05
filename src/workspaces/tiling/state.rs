//! Per-workspace tiling state.
//!
//! Tiling is a property of one workspace on one output (see
//! `specs/tiling.md`), so this hangs off `WorkspaceView` rather than off the
//! compositor.

use smithay::reexports::wayland_server::backend::ObjectId;

use super::tree::{Axis, Tree};

/// Everything one workspace knows about its tiling.
#[derive(Debug, Default)]
pub struct TilingState {
    /// Is this workspace tiling? A floating workspace keeps an empty tree.
    pub enabled: bool,
    /// The tree of split containers and window leaves.
    pub tree: Tree<ObjectId>,
    /// The leaf insertions and directional commands act relative to.
    pub focused: Option<ObjectId>,
    /// An armed split axis: the next insertion splits the focused cell this
    /// way rather than following the cell's shape. Cleared by the insertion.
    pub preselect: Option<Axis>,
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

    /// Forget everything: leaving tiling mode empties the tree.
    pub fn clear(&mut self) {
        self.tree = Tree::default();
        self.focused = None;
        self.preselect = None;
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
