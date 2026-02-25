use crate::{components::menu_item::MenuItem, prelude::ContextMenuStyle};

/// State for ContextMenu
///
/// Pure state management - no rendering, no surface logic.
/// Handles menu items, selection, hover state, and submenu navigation.
#[derive(Clone, Debug, Hash)]
pub struct ContextMenuState {
    /// Menu items
    items: Vec<MenuItem>,

    /// Currently selected item index
    selected_index: Option<usize>,

    /// Selection at each depth level for submenu tree
    /// Index 0 = root menu, 1 = first submenu, etc.
    selections_by_depth: Vec<Option<usize>>,

    /// Which item has an open submenu at each depth
    open_submenu_by_depth: Vec<Option<usize>>,

    /// Current depth level (0 = root, 1 = first submenu level, etc.)
    depth: usize,

    /// Flag to request closing the menu
    should_close: bool,

    pub style: ContextMenuStyle,
}

impl ContextMenuState {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            selected_index: None,
            selections_by_depth: Vec::new(),
            open_submenu_by_depth: Vec::new(),
            depth: 0,
            should_close: false,
            style: ContextMenuStyle::default(),
        }
    }

    // === Getters ===

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn should_close(&self) -> bool {
        self.should_close
    }

    // === State Mutations ===

    pub fn set_items(&mut self, items: Vec<MenuItem>) {
        self.items = items;
    }

    pub fn select(&mut self, index: Option<usize>) {
        self.selected_index = index;
    }

    pub fn request_close(&mut self) {
        self.should_close = true;
    }

    pub fn reset_close_request(&mut self) {
        self.should_close = false;
    }

    // === Navigation Logic ===

    /// Select next non-separator item
    pub fn select_next(&mut self) {
        self.selected_index = Self::find_next_selectable(&self.items, self.selected_index, true);
    }

    /// Select previous non-separator item
    pub fn select_previous(&mut self) {
        self.selected_index = Self::find_next_selectable(&self.items, self.selected_index, false);
    }

    /// Find the next selectable (non-separator) item
    fn find_next_selectable(
        items: &[MenuItem],
        current: Option<usize>,
        forward: bool,
    ) -> Option<usize> {
        if items.is_empty() {
            return None;
        }

        let start = match current {
            Some(idx) if forward => (idx + 1) % items.len(),
            Some(idx) if !forward => {
                if idx == 0 {
                    items.len() - 1
                } else {
                    idx - 1
                }
            }
            None if forward => 0,
            None => items.len() - 1,
            _ => 0,
        };

        // Search for next non-separator item
        let mut idx = start;
        for _ in 0..items.len() {
            if !items[idx].is_separator() {
                return Some(idx);
            }
            idx = if forward {
                (idx + 1) % items.len()
            } else if idx == 0 {
                items.len() - 1
            } else {
                idx - 1
            };
        }

        None
    }

    // === Submenu Management ===

    /// Check if an item at the given index has a submenu
    pub fn has_submenu(&self, index: usize) -> bool {
        self.items
            .get(index)
            .map(|item| item.has_submenu())
            .unwrap_or(false)
    }

    /// Get submenu items for a given index
    pub fn get_submenu_items(&self, index: usize) -> Option<&[MenuItem]> {
        self.items.get(index).and_then(|item| item.submenu_items())
    }

    /// Check if a submenu is open at the given depth for the given item index
    pub fn is_submenu_open(&self, depth: usize, item_idx: usize) -> bool {
        self.open_submenu_by_depth.get(depth).and_then(|idx| *idx) == Some(item_idx)
    }

    /// Get selected index at a specific depth
    /// - depth 0: returns root selection (selected_index)
    /// - depth > 0: returns selection from selections_by_depth
    pub fn selected_at_depth(&self, depth: usize) -> Option<usize> {
        if depth == 0 {
            self.selected_index
        } else {
            self.selections_by_depth.get(depth - 1).and_then(|s| *s)
        }
    }

    /// Set selection at a specific depth
    pub fn select_at_depth(&mut self, depth: usize, index: Option<usize>) {
        if depth == 0 {
            self.selected_index = index;
        } else {
            // Ensure vec is large enough
            while self.selections_by_depth.len() < depth {
                self.selections_by_depth.push(None);
            }
            self.selections_by_depth[depth - 1] = index;
        }
    }

    /// Select next item at specific depth (or current depth if None)
    pub fn select_next_at_depth(&mut self, depth: Option<usize>) {
        let target_depth = depth.unwrap_or(self.depth);
        let items = self.items_at_depth(target_depth);
        let current = self.selected_at_depth(target_depth);
        let next = Self::find_next_selectable(items, current, true);
        self.select_at_depth(target_depth, next);
    }

    /// Select previous item at specific depth (or current depth if None)
    pub fn select_previous_at_depth(&mut self, depth: Option<usize>) {
        let target_depth = depth.unwrap_or(self.depth);
        let items = self.items_at_depth(target_depth);
        let current = self.selected_at_depth(target_depth);
        let prev = Self::find_next_selectable(items, current, false);
        self.select_at_depth(target_depth, prev);
    }

    /// Check if selected item at current depth has a submenu
    pub fn selected_has_submenu(&self, depth: Option<usize>) -> bool {
        let target_depth = depth.unwrap_or(self.depth);
        if let Some(idx) = self.selected_at_depth(target_depth) {
            let items = self.items_at_depth(target_depth);
            items
                .get(idx)
                .map(|item| item.has_submenu())
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Get selected item's label at current depth
    pub fn selected_label(&self, depth: Option<usize>) -> Option<&str> {
        let target_depth = depth.unwrap_or(self.depth);
        if let Some(idx) = self.selected_at_depth(target_depth) {
            let items = self.items_at_depth(target_depth);
            items.get(idx).and_then(|item| item.label())
        } else {
            None
        }
    }

    /// Get selected index (convenience for depth 0)
    pub fn selected_index(&self, maybe_depth: Option<usize>) -> Option<usize> {
        self.selected_at_depth(maybe_depth.unwrap_or(self.depth))
    }

    /// Mark a submenu as open at the given depth
    pub fn open_submenu(&mut self, depth: usize, item_idx: usize) {
        // Ensure vec is large enough
        while self.open_submenu_by_depth.len() <= depth {
            self.open_submenu_by_depth.push(None);
        }
        self.open_submenu_by_depth[depth] = Some(item_idx);

        // Clear any deeper submenus
        self.open_submenu_by_depth.truncate(depth + 1);
        self.depth = depth + 1;
    }

    /// Close submenus from the given depth onwards
    pub fn close_submenus_from(&mut self, depth: usize) {
        // Clear the submenu marker at this depth (nothing is open at this level anymore)
        if depth < self.open_submenu_by_depth.len() {
            self.open_submenu_by_depth[depth] = None;
        }

        // Close submenus AFTER this depth
        if depth + 1 < self.open_submenu_by_depth.len() {
            self.open_submenu_by_depth.truncate(depth + 1);
        }
        // Keep selections up to and including this depth
        if depth + 1 < self.selections_by_depth.len() {
            self.selections_by_depth.truncate(depth + 1);
        }
        self.depth = depth;
    }

    /// Close all submenus
    pub fn close_all_submenus(&mut self) {
        self.close_submenus_from(0);
    }

    // === Utility ===

    /// Get the label for a menu item (if it has one)
    pub fn get_item_label(&self, index: usize) -> Option<&str> {
        self.items.get(index).and_then(|item| item.label())
    }

    /// Get items for rendering at a specific depth level
    ///
    /// Recursively traverses the submenu tree to find items at the target depth.
    /// - depth 0: returns root items
    /// - depth 1: returns submenu items from the open submenu at depth 0
    /// - depth 2: returns submenu items from the open submenu at depth 1, etc.
    pub fn items_at_depth(&self, depth: usize) -> &[MenuItem] {
        if depth == 0 {
            return &self.items;
        }

        // Traverse the submenu chain from root to target depth
        let mut current_items: &[MenuItem] = &self.items;

        for d in 0..depth {
            // Get which item has an open submenu at this depth
            if let Some(Some(item_idx)) = self.open_submenu_by_depth.get(d) {
                // Get that item and its submenu
                if let Some(item) = current_items.get(*item_idx) {
                    if let Some(submenu_items) = item.submenu_items() {
                        current_items = submenu_items;
                    } else {
                        // No submenu at this level - return empty
                        return &[];
                    }
                } else {
                    return &[];
                }
            } else {
                // No open submenu at this depth
                return &[];
            }
        }

        current_items
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.selected_index = None;
        self.close_all_submenus();
        self.should_close = false;
    }
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::MenuItemKind;

    use super::*;

    fn create_test_items() -> Vec<MenuItem> {
        vec![
            MenuItem::new(MenuItemKind::Action {
                label: "Item 1".to_string(),
                shortcut: None,
                action_id: Some("action_1".to_string()),
            }),
            MenuItem::new(MenuItemKind::Separator),
            MenuItem::new(MenuItemKind::Action {
                label: "Item 2".to_string(),
                shortcut: None,
                action_id: Some("action_2".to_string()),
            }),
        ]
    }

    #[test]
    fn test_state_creation() {
        let items = create_test_items();
        let state = ContextMenuState::new(items.clone());

        assert_eq!(state.items().len(), 3);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn test_navigation_skips_separators() {
        let items = create_test_items();
        let mut state = ContextMenuState::new(items);

        state.select_next();
        assert_eq!(state.selected(), Some(0)); // First item

        state.select_next();
        assert_eq!(state.selected(), Some(2)); // Skips separator at index 1

        state.select_previous();
        assert_eq!(state.selected(), Some(0)); // Back to first, skipping separator
    }

    #[test]
    fn test_submenu_management() {
        let mut state = ContextMenuState::new(vec![]);

        state.open_submenu(0, 2);
        assert_eq!(state.depth(), 1);
        assert!(state.is_submenu_open(0, 2));

        state.close_all_submenus();
        assert_eq!(state.depth(), 0);
        assert!(!state.is_submenu_open(0, 2));
    }
}
