/// State for MenuBarNext component
#[derive(Clone, Debug)]
pub struct MenuBarState {
    /// Menu bar items (just labels for now)
    items: Vec<String>,
    /// Currently selected/active item index
    active_index: Option<usize>,
    /// Hovered item index
    hover_index: Option<usize>,
    /// Whether the menu bar has keyboard focus
    is_focused: bool,
}

impl MenuBarState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            active_index: None,
            hover_index: None,
            is_focused: false,
        }
    }

    // === Getters ===

    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    pub fn hover_index(&self) -> Option<usize> {
        self.hover_index
    }

    pub fn is_focused(&self) -> bool {
        self.is_focused
    }

    pub fn active_item(&self) -> Option<&str> {
        self.active_index
            .and_then(|idx| self.items.get(idx).map(|s| s.as_str()))
    }

    // === State Mutations ===

    pub fn add_item(&mut self, label: impl Into<String>) {
        self.items.push(label.into());
    }

    pub fn set_active(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if idx < self.items.len() {
                self.active_index = Some(idx);
            }
        } else {
            self.active_index = None;
        }
    }

    pub fn set_hover(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if idx < self.items.len() {
                self.hover_index = Some(idx);
            } else {
                self.hover_index = None;
            }
        } else {
            self.hover_index = None;
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }

    pub fn clear_active(&mut self) {
        self.active_index = None;
    }

    // === Navigation Logic ===

    /// Navigate to the next item (wraps around)
    pub fn navigate_next(&mut self) {
        if self.items.is_empty() {
            return;
        }

        self.active_index = Some(match self.active_index {
            Some(idx) => (idx + 1) % self.items.len(),
            None => 0,
        });
    }

    /// Navigate to the previous item (wraps around)
    pub fn navigate_previous(&mut self) {
        if self.items.is_empty() {
            return;
        }

        self.active_index = Some(match self.active_index {
            Some(idx) => {
                if idx == 0 {
                    self.items.len() - 1
                } else {
                    idx - 1
                }
            }
            None => self.items.len() - 1,
        });
    }

    /// Activate the item at the given index
    pub fn activate_at(&mut self, index: usize) -> bool {
        if index < self.items.len() {
            self.active_index = Some(index);
            true
        } else {
            false
        }
    }
}

impl Default for MenuBarState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let state = MenuBarState::new();
        assert_eq!(state.items().len(), 0);
        assert_eq!(state.active_index(), None);
        assert!(!state.is_focused());
    }

    #[test]
    fn test_add_items() {
        let mut state = MenuBarState::new();
        state.add_item("File");
        state.add_item("Edit");
        state.add_item("View");

        assert_eq!(state.items().len(), 3);
        assert_eq!(state.items()[0], "File");
        assert_eq!(state.items()[1], "Edit");
        assert_eq!(state.items()[2], "View");
    }

    #[test]
    fn test_navigation() {
        let mut state = MenuBarState::new();
        state.add_item("File");
        state.add_item("Edit");
        state.add_item("View");

        // Navigate next from None
        state.navigate_next();
        assert_eq!(state.active_index(), Some(0));

        // Navigate next
        state.navigate_next();
        assert_eq!(state.active_index(), Some(1));

        // Navigate next
        state.navigate_next();
        assert_eq!(state.active_index(), Some(2));

        // Wrap around
        state.navigate_next();
        assert_eq!(state.active_index(), Some(0));
    }

    #[test]
    fn test_navigation_previous() {
        let mut state = MenuBarState::new();
        state.add_item("File");
        state.add_item("Edit");
        state.add_item("View");

        // Navigate previous from None
        state.navigate_previous();
        assert_eq!(state.active_index(), Some(2));

        // Navigate previous
        state.navigate_previous();
        assert_eq!(state.active_index(), Some(1));

        // Navigate previous
        state.navigate_previous();
        assert_eq!(state.active_index(), Some(0));

        // Wrap around
        state.navigate_previous();
        assert_eq!(state.active_index(), Some(2));
    }

    #[test]
    fn test_activate_at() {
        let mut state = MenuBarState::new();
        state.add_item("File");
        state.add_item("Edit");

        assert!(state.activate_at(1));
        assert_eq!(state.active_index(), Some(1));

        // Out of bounds
        assert!(!state.activate_at(5));
        assert_eq!(state.active_index(), Some(1)); // Unchanged
    }
}
