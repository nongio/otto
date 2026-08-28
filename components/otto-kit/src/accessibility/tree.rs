//! Describing a window to an assistive technology.
//!
//! Kit widgets are drawn, not retained: a button is a few Skia calls inside a
//! layer that also holds twenty other buttons, so there is no object graph to
//! walk and nothing an adapter could infer. The accessible tree is therefore
//! declared, in the same pass that draws — which also means it cannot drift out
//! of date, because a frame that does not describe a control does not draw one
//! either.
//!
//! Node identity comes from [`crate::focus::FocusId`], the same id the keyboard
//! uses. The tree and the focus ring then agree by construction rather than by
//! being kept in step.

use accesskit::{Node, NodeId, Rect, Role, Tree, TreeId, TreeUpdate};

use crate::focus::FocusId;

/// The window node's id. Fixed: every kit window has exactly one root, and an
/// assistive technology needs it to be stable across updates.
pub const ROOT: NodeId = NodeId(1);

/// Builds one window's accessible tree.
///
/// Nodes are added inside their parent through [`A11yTree::group`], so the
/// nesting in the code is the nesting in the tree:
///
/// ```no_run
/// # use otto_kit::accessibility::A11yTree;
/// # use otto_kit::focus::FocusId;
/// # use accesskit::{Role, Action};
/// # let mut tree = A11yTree::new("Settings");
/// tree.group(FocusId::new("sidebar"), Role::List, |tree| {
///     tree.node(FocusId::new("row-appearance"), Role::ListItem, |node| {
///         node.set_label("Appearance");
///         node.add_action(Action::Click);
///     });
/// });
/// ```
pub struct A11yTree {
    nodes: Vec<(NodeId, Node)>,
    /// The chain of open groups, innermost last. The root is always at the
    /// bottom, so there is always somewhere to put a node.
    open: Vec<(NodeId, Node)>,
    focus: NodeId,
    /// Every id already in the tree, so none can be added twice. See
    /// [`A11yTree::push`].
    seen: std::collections::HashSet<NodeId>,
    /// Whether the application named the focused node itself.
    focus_is_explicit: bool,
}

impl A11yTree {
    /// Starts a tree for a window titled `title`.
    pub fn new(title: impl Into<String>) -> Self {
        let mut root = Node::new(Role::Window);
        root.set_label(title.into());

        Self {
            nodes: Vec::new(),
            open: vec![(ROOT, root)],
            focus: ROOT,
            seen: std::collections::HashSet::from([ROOT]),
            focus_is_explicit: false,
        }
    }

    /// Says where the window is and how big it is, in points. Assistive
    /// technologies use it to place a highlight and to hit-test.
    pub fn set_window_bounds(&mut self, width: f32, height: f32) {
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        self.open[0].1.set_bounds(bounds);
    }

    /// The control the keyboard is on. Anything else reported as focused would
    /// make a screen reader announce a control the user cannot type into.
    pub fn set_focus(&mut self, id: FocusId) {
        self.focus = node_id(id);
        self.focus_is_explicit = true;
    }

    /// The focus to report if the application does not say otherwise.
    ///
    /// The run loop fills this in from the surface's focus ring, which is right
    /// for a window whose controls and whose keyboard stops are the same list.
    /// Where they are not — a list that is one stop but many nodes — the
    /// application says which node the focus is really on, and that wins.
    pub(crate) fn set_default_focus(&mut self, id: FocusId) {
        if !self.focus_is_explicit {
            self.focus = node_id(id);
        }
    }

    /// Adds a leaf: a button, a label, a row.
    pub fn node(&mut self, id: FocusId, role: Role, build: impl FnOnce(&mut Node)) {
        let mut node = Node::new(role);
        build(&mut node);
        self.push(node_id(id), node);
    }

    /// Adds a node that other nodes go inside — a list, a toolbar, a pane.
    pub fn group(&mut self, id: FocusId, role: Role, build: impl FnOnce(&mut Self)) {
        self.group_with(id, role, |_| {}, build)
    }

    /// [`A11yTree::group`], for a group that needs properties of its own.
    pub fn group_with(
        &mut self,
        id: FocusId,
        role: Role,
        describe: impl FnOnce(&mut Node),
        build: impl FnOnce(&mut Self),
    ) {
        let mut node = Node::new(role);
        describe(&mut node);

        let group_id = node_id(id);
        if !self.seen.insert(group_id) {
            debug_assert!(false, "a group was declared twice: {group_id:?}");
            return;
        }

        self.open.push((group_id, node));
        build(self);
        let (id, node) = self.open.pop().expect("group left the root open");
        self.seen.remove(&id);
        self.push(id, node);
    }

    /// Files a finished node under whichever group is open.
    fn push(&mut self, id: NodeId, node: Node) {
        // A node may appear once in a tree, and AccessKit panics on a repeat —
        // which would take the application down, from nothing worse than the
        // same id declared twice in a loop. The first one wins; the second is
        // dropped, so a mistake costs an unannounced control rather than a
        // crash. `debug_assert` so it is still loud in development.
        if !self.seen.insert(id) {
            debug_assert!(false, "a control was declared twice: {id:?}");
            return;
        }

        self.open
            .last_mut()
            .expect("no open node to add to")
            .1
            .push_child(id);
        self.nodes.push((id, node));
    }

    /// Closes the tree into the update an adapter takes.
    pub fn finish(mut self) -> TreeUpdate {
        // Only the root may still be open; anything else is a `group` whose
        // closure escaped, which cannot happen through the public API.
        debug_assert_eq!(self.open.len(), 1);
        let (root_id, root) = self.open.pop().expect("the root was closed");
        self.nodes.push((root_id, root));

        let focus = if self.nodes.iter().any(|(id, _)| *id == self.focus) {
            self.focus
        } else {
            // A focus pointing at nothing is an error in the protocol, not a
            // detail: report the window itself.
            ROOT
        };

        TreeUpdate {
            nodes: self.nodes,
            tree: Some(Tree {
                root: root_id,
                toolkit_name: Some("otto-kit".into()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }
}

/// The accessible node id of a focusable control.
///
/// Offset past [`ROOT`] so no control can collide with the window node.
pub fn node_id(id: FocusId) -> NodeId {
    NodeId(id.raw() | 1 << 63)
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::Action;

    #[test]
    fn a_flat_window_has_its_controls_as_children() {
        let mut tree = A11yTree::new("Settings");
        tree.node(FocusId::new("ok"), Role::Button, |node| {
            node.set_label("OK");
            node.add_action(Action::Click);
        });
        tree.set_focus(FocusId::new("ok"));

        let update = tree.finish();
        let root = update
            .nodes
            .iter()
            .find(|(id, _)| *id == ROOT)
            .expect("no root");
        assert_eq!(root.1.children(), &[node_id(FocusId::new("ok"))]);
        assert_eq!(update.focus, node_id(FocusId::new("ok")));
        assert_eq!(update.tree.as_ref().unwrap().root, ROOT);
    }

    #[test]
    fn groups_nest_the_way_the_code_does() {
        let mut tree = A11yTree::new("Files");
        tree.group(FocusId::new("list"), Role::List, |tree| {
            tree.node(FocusId::new("row-0"), Role::ListItem, |node| {
                node.set_label("Documents");
            });
        });

        let update = tree.finish();
        let list = update
            .nodes
            .iter()
            .find(|(id, _)| *id == node_id(FocusId::new("list")))
            .expect("no list");
        assert_eq!(list.1.children(), &[node_id(FocusId::new("row-0"))]);

        let root = update.nodes.iter().find(|(id, _)| *id == ROOT).unwrap();
        assert_eq!(root.1.children(), &[node_id(FocusId::new("list"))]);
    }

    #[test]
    fn a_focus_on_nothing_falls_back_to_the_window() {
        let mut tree = A11yTree::new("Settings");
        tree.node(FocusId::new("ok"), Role::Button, |_| {});
        tree.set_focus(FocusId::new("gone"));

        assert_eq!(tree.finish().focus, ROOT);
    }

    #[test]
    fn no_control_can_collide_with_the_window_node() {
        // The window's id is reserved, whatever a control is called.
        for key in ["", "ok", "window", "1"] {
            assert_ne!(node_id(FocusId::new(key)), ROOT);
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    /// A window whose keyboard stops and whose nodes are the same list: the
    /// ring is the answer.
    #[test]
    fn the_ring_answers_for_a_window_that_says_nothing() {
        let mut tree = A11yTree::new("Files");
        tree.node(FocusId::new("ok"), Role::Button, |node| {
            node.set_label("OK")
        });
        tree.set_default_focus(FocusId::new("ok"));

        assert_eq!(tree.finish().focus, node_id(FocusId::new("ok")));
    }

    /// A list that is one keyboard stop but many nodes: the application names
    /// the row, and the ring's own id — which is not in the tree — must not
    /// replace it.
    #[test]
    fn an_application_that_names_the_focus_keeps_it() {
        let mut tree = A11yTree::new("Settings");
        tree.node(FocusId::new("row-1"), Role::ListItem, |node| {
            node.set_label("Displays")
        });
        tree.set_focus(FocusId::new("row-1"));
        tree.set_default_focus(FocusId::new("the-whole-sidebar"));

        assert_eq!(tree.finish().focus, node_id(FocusId::new("row-1")));
    }
}

#[cfg(test)]
mod repeat_tests {
    use super::*;

    /// A repeated id crashes AccessKit, and with it the application. Declaring
    /// one twice — the same id inside a loop — must cost a missing control and
    /// nothing more. Release behaviour: `debug_assert` fires in development.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "the debug_assert fires first, by design")]
    fn a_control_declared_twice_appears_once() {
        let mut tree = A11yTree::new("Files");
        tree.node(FocusId::new("row"), Role::Button, |node| {
            node.set_label("A")
        });
        tree.node(FocusId::new("row"), Role::Button, |node| {
            node.set_label("B")
        });

        let update = tree.finish();
        let mut ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }

    /// Nesting is not repetition: a group and its children are distinct nodes,
    /// and closing a group must not leave its id looking used twice.
    #[test]
    fn nesting_is_not_a_repeat() {
        let mut tree = A11yTree::new("Files");
        tree.group(FocusId::new("list"), Role::List, |tree| {
            tree.node(FocusId::new("row-0"), Role::ListItem, |n| n.set_label("A"));
            tree.node(FocusId::new("row-1"), Role::ListItem, |n| n.set_label("B"));
        });

        let update = tree.finish();
        let list = update
            .nodes
            .iter()
            .find(|(id, _)| *id == node_id(FocusId::new("list")))
            .expect("the group is in the tree");
        assert_eq!(list.1.children().len(), 2);

        let mut ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }
}
