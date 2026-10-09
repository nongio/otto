/// Icon data for a menu bar item
#[derive(Clone, Debug)]
pub enum MenuBarIcon {
    /// Icon loaded from raw BGRA8888 pixel data (e.g. from D-Bus StatusNotifierItem)
    Pixmap {
        data: Vec<u8>,
        width: i32,
        height: i32,
    },
    /// Icon name to resolve from the current icon theme
    Named(String),
    /// Icon loaded from a file path (SVG or raster)
    File(String),
}

/// A single item in the menu bar (icon and/or label)
#[derive(Clone, Debug)]
pub struct MenuBarItem {
    pub label: Option<String>,
    pub icon: Option<MenuBarIcon>,
}

/// State for MenuBarNext component
#[derive(Clone, Debug)]
pub struct MenuBarState {
    items: Vec<MenuBarItem>,
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

    pub fn items(&self) -> &[MenuBarItem] {
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

    pub fn active_item(&self) -> Option<&MenuBarItem> {
        self.active_index.and_then(|idx| self.items.get(idx))
    }

    /// Get the label of the active item (if it has one)
    pub fn active_label(&self) -> Option<&str> {
        self.active_item().and_then(|item| item.label.as_deref())
    }

    // === State Mutations ===

    /// Add a text-only item
    pub fn add_item(&mut self, label: impl Into<String>) {
        self.items.push(MenuBarItem {
            label: Some(label.into()),
            icon: None,
        });
    }

    /// Add an icon-only item
    pub fn add_icon_item(&mut self, icon: MenuBarIcon) {
        self.items.push(MenuBarItem {
            label: None,
            icon: Some(icon),
        });
    }

    /// Add an item with both icon and label
    pub fn add_icon_label_item(&mut self, icon: MenuBarIcon, label: impl Into<String>) {
        self.items.push(MenuBarItem {
            label: Some(label.into()),
            icon: Some(icon),
        });
    }

    /// Add a fully constructed MenuBarItem
    pub fn add(&mut self, item: MenuBarItem) {
        self.items.push(item);
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
        assert_eq!(state.items()[0].label.as_deref(), Some("File"));
        assert_eq!(state.items()[1].label.as_deref(), Some("Edit"));
        assert_eq!(state.items()[2].label.as_deref(), Some("View"));
    }
}
