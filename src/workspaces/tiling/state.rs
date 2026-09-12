//! Per-workspace tiling state.
//!
//! Tiling is a property of one workspace on one output (see
//! `specs/tiling.md`), so this hangs off `WorkspaceView` rather than off the
//! compositor.

use smithay::reexports::wayland_server::backend::ObjectId;

use super::design::UndoStack;
use super::layout::Gaps;
use super::tree::{Axis, EmptyId, NodeId, Tree};
use crate::config::TilingConfig;

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
    /// Windows coming back from the dock that have not rejoined the tree
    /// yet. Unminimize runs inside `Workspaces`, after the workspace-switch
    /// animation when the window lives elsewhere, and only the compositor can
    /// insert a leaf, so the id waits here and the relayout flush that reads
    /// `dirty` adopts it next to the focused leaf (`specs/tiling.md`,
    /// *Restoring*).
    pub returning: Vec<ObjectId>,
    /// The window on this workspace's *floating* layer that held keyboard
    /// focus last — where `focus mode_toggle` lands when it comes up out of
    /// the tree (`specs/tiling.md`, *Stacking*). `focused` is the tiled
    /// layer's half of the same memory.
    pub floating_focused: Option<ObjectId>,
    /// An armed split axis: the next insertion splits the focused cell this
    /// way rather than following the cell's shape. Cleared by the insertion.
    pub preselect: Option<Axis>,
    /// The pane grid, its drag and its undo stack.
    pub design: TilingDesignState,
    /// This workspace's own gaps, when `gaps inner|outer <n> current` set
    /// them. `None` means the `[tiling]` defaults apply.
    ///
    /// `smart_gaps` is not part of an override: it is a global preference
    /// about how a *lone* tile looks, not a measurement of this workspace.
    pub gaps: Option<Gaps>,
    /// `focus parent` walked focus up to a container, and this is it.
    ///
    /// The focused *leaf* is still remembered in `focused`: it is where
    /// `focus child` comes back down to, and what every command that needs a
    /// window still acts on. Only the commands that read the tree's shape —
    /// `layout`, and one day a container move — look here first
    /// (`docs/developer/tiling-plan.md`, *Command language*).
    pub focused_container: Option<NodeId>,
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

    /// The gaps this workspace actually lays out with: its own override if it
    /// has one, else the `[tiling]` defaults. Every caller of
    /// [`super::layout::resolve`] goes through here, so an override cannot be
    /// honoured on one path and missed on another.
    pub fn effective_gaps(&self, config: &TilingConfig) -> Gaps {
        match self.gaps {
            Some(gaps) => Gaps {
                smart: config.smart_gaps,
                ..gaps
            },
            None => config.gaps(),
        }
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
        self.floating_focused = None;
        self.preselect = None;
        self.focused_container = None;
        self.dirty = false;
        self.returning.clear();
        self.design = TilingDesignState::default();
    }

    // ── Container focus ──────────────────────────────────────────────────

    /// The node commands act on: the container `focus parent` walked up to,
    /// else the focused leaf's node.
    pub fn focused_node(&self) -> Option<NodeId> {
        if let Some(container) = self.focused_container {
            if self.tree.node(container).is_some() {
                return Some(container);
            }
        }
        self.focused.as_ref().and_then(|id| self.tree.node_of(id))
    }

    /// Walk focus one level up the tree (i3's `focus parent`).
    ///
    /// `false` at the root: focus never leaves the workspace.
    pub fn focus_parent(&mut self) -> bool {
        let Some(from) = self.focused_node() else {
            return false;
        };
        match self.tree.parent_of(from) {
            Some(parent) => {
                self.focused_container = Some(parent);
                true
            }
            None => false,
        }
    }

    /// Walk focus one level back down, towards the leaf it came from (i3's
    /// `focus child`).
    ///
    /// The remembered leaf is the path: focus descends into whichever child
    /// holds it, and the container focus is cleared once the walk is back on
    /// a window. With no remembered leaf the first child stands in.
    pub fn focus_child(&mut self) -> bool {
        let Some(container) = self.focused_container else {
            return false;
        };
        let leaf_node = self.focused.as_ref().and_then(|id| self.tree.node_of(id));
        let children = self.tree.children_of(container);
        let next = leaf_node
            .and_then(|leaf| {
                children
                    .iter()
                    .map(|c| c.node)
                    .find(|child| self.contains_node(*child, leaf))
            })
            .or_else(|| children.first().map(|c| c.node));
        let Some(next) = next else {
            return false;
        };
        if self.tree.container_axis(next).is_some() {
            self.focused_container = Some(next);
        } else {
            // Landing on a leaf ends the walk: commands act on the window again.
            self.focused_container = None;
            if let Some(leaf) = self.tree.leaves_under(next).into_iter().next() {
                self.focused = Some(leaf);
            }
        }
        true
    }

    /// Keyboard focus landed on a window: the container walk is over.
    pub fn focus_leaf(&mut self, leaf: ObjectId) {
        self.focused = Some(leaf);
        self.focused_container = None;
    }

    /// Is `node` at or under `ancestor`?
    fn contains_node(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(id) = at {
            if id == ancestor {
                return true;
            }
            at = self.tree.parent_of(id);
        }
        false
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

    #[test]
    fn a_workspace_without_an_override_uses_the_tiling_defaults() {
        let config = TilingConfig {
            inner_gap: 8,
            outer_gap: 12,
            smart_gaps: true,
            ..TilingConfig::default()
        };
        let state = TilingState::default();
        assert_eq!(
            state.effective_gaps(&config),
            Gaps {
                inner: 8,
                outer: 12,
                smart: true
            }
        );
    }

    #[test]
    fn an_override_replaces_both_gaps_but_not_smart_gaps() {
        let config = TilingConfig {
            inner_gap: 8,
            outer_gap: 12,
            smart_gaps: true,
            ..TilingConfig::default()
        };
        // `smart` is global, so whatever was stored on the override loses to
        // the configured value.
        let state = TilingState {
            gaps: Some(Gaps {
                inner: 0,
                outer: 4,
                smart: false,
            }),
            ..TilingState::default()
        };
        assert_eq!(
            state.effective_gaps(&config),
            Gaps {
                inner: 0,
                outer: 4,
                smart: true
            }
        );
    }

    #[test]
    fn leaving_tiling_mode_keeps_the_gap_override() {
        let mut state = TilingState {
            gaps: Some(Gaps {
                inner: 2,
                outer: 2,
                smart: false,
            }),
            ..TilingState::default()
        };
        state.clear();
        assert!(
            state.gaps.is_some(),
            "the override belongs to the workspace, not to its tree"
        );
    }
}
