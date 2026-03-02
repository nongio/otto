use std::hash::{Hash, Hasher};

use layers::engine::NodeRef;

use crate::workspaces::Application;

#[derive(Debug, Clone, Default)]
pub struct AppSwitcherModel {
    pub apps: Vec<Application>,
    pub current_app: usize,
    pub width: i32,
    /// NodeRef for each app's icon_stack layer in the dock, parallel to `apps`.
    pub icon_stacks: Vec<Option<NodeRef>>,
}

impl Hash for AppSwitcherModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.apps.hash(state);
        self.current_app.hash(state);
        self.width.hash(state);
        for node in &self.icon_stacks {
            node.map(|n| n.0).hash(state);
        }
    }
}

impl AppSwitcherModel {
    pub fn new() -> Self {
        Default::default()
    }
}
