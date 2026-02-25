use std::hash::Hash;

/// Type of menu item
#[derive(Clone)]
pub enum MenuItemKind {
    Action {
        label: String,
        action_id: Option<String>,
        shortcut: Option<String>,
    },
    Submenu {
        label: String,
        items: Vec<MenuItem>,
    },
    Separator,
}

impl std::fmt::Debug for MenuItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action {
                label, shortcut, ..
            } => f
                .debug_struct("Action")
                .field("label", label)
                .field("shortcut", shortcut)
                .finish(),
            Self::Submenu { label, items } => f
                .debug_struct("Submenu")
                .field("label", label)
                .field("items", items)
                .finish(),
            Self::Separator => write!(f, "Separator"),
        }
    }
}

/// Visual interaction state of a menu item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualState {
    Normal,
    Hovered,
    Disabled,
}

/// Pure state/data for a MenuItem
///
/// No rendering, no I/O, just data.
#[derive(Debug, Clone)]
pub struct MenuItem {
    /// The kind of menu item
    pub kind: MenuItemKind,

    /// Visual state
    pub visual_state: VisualState,

    /// Whether this item is enabled
    pub enabled: bool,

    /// Height in logical pixels (computed based on kind)
    pub height: f32,
}
impl Hash for MenuItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash all fields except callback (which can't be hashed)
        match &self.kind {
            MenuItemKind::Action {
                label,
                action_id,
                shortcut,
                ..
            } => {
                0u8.hash(state);
                label.hash(state);
                action_id.hash(state);
                shortcut.hash(state);
            }
            MenuItemKind::Submenu { label, items } => {
                1u8.hash(state);
                label.hash(state);
                items.hash(state);
            }
            MenuItemKind::Separator => {
                2u8.hash(state);
            }
        }
        self.visual_state.hash(state);
        self.enabled.hash(state);
        self.height.to_bits().hash(state);
    }
}
impl MenuItem {
    /// Create new menu item data
    pub fn new(kind: MenuItemKind) -> Self {
        let height = match kind {
            MenuItemKind::Separator => 9.0,
            _ => 22.0, // LINE_HEIGHT
        };

        Self {
            kind,
            visual_state: VisualState::Normal,
            enabled: true,
            height,
        }
    }

    /// Create an action item
    pub fn action(label: impl Into<String>) -> Self {
        Self::new(MenuItemKind::Action {
            label: label.into(),
            action_id: None,
            shortcut: None,
        })
    }

    /// Create a separator
    pub fn separator() -> Self {
        Self::new(MenuItemKind::Separator)
    }

    /// Create a submenu item
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::new(MenuItemKind::Submenu {
            label: label.into(),
            items,
        })
    }

    // === Getters ===

    pub fn kind(&self) -> &MenuItemKind {
        &self.kind
    }

    pub fn visual_state(&self) -> VisualState {
        self.visual_state
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_separator(&self) -> bool {
        matches!(self.kind, MenuItemKind::Separator)
    }

    pub fn has_submenu(&self) -> bool {
        matches!(self.kind, MenuItemKind::Submenu { .. })
    }

    pub fn label(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { label, .. } => Some(label),
            MenuItemKind::Submenu { label, .. } => Some(label),
            MenuItemKind::Separator => None,
        }
    }

    pub fn shortcut(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { shortcut, .. } => shortcut.as_deref(),
            _ => None,
        }
    }

    pub fn action_id(&self) -> Option<&str> {
        match &self.kind {
            MenuItemKind::Action { action_id, .. } => action_id.as_deref(),
            _ => None,
        }
    }

    pub fn submenu_items(&self) -> Option<&[MenuItem]> {
        match &self.kind {
            MenuItemKind::Submenu { items, .. } => Some(items),
            _ => None,
        }
    }

    // === Builder API ===

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        if let MenuItemKind::Action {
            label, action_id, ..
        } = self.kind
        {
            self.kind = MenuItemKind::Action {
                label,
                action_id,
                shortcut: Some(shortcut.into()),
            };
        }
        self
    }

    pub fn with_action_id(mut self, id: impl Into<String>) -> Self {
        if let MenuItemKind::Action {
            label, shortcut, ..
        } = self.kind
        {
            self.kind = MenuItemKind::Action {
                label,
                action_id: Some(id.into()),
                shortcut,
            };
        }
        self
    }

    pub fn with_visual_state(mut self, state: VisualState) -> Self {
        self.visual_state = state;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self.visual_state = VisualState::Disabled;
        self
    }

    // === State Mutations ===

    pub fn set_visual_state(&mut self, state: VisualState) {
        self.visual_state = state;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.visual_state = VisualState::Disabled;
        }
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        if self.enabled {
            self.visual_state = if hovered {
                VisualState::Hovered
            } else {
                VisualState::Normal
            };
        }
    }
}

impl Default for MenuItem {
    fn default() -> Self {
        Self::separator()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_item() {
        let item = MenuItem::action("Copy").with_shortcut("Ctrl+C");

        assert_eq!(item.label(), Some("Copy"));
        assert_eq!(item.shortcut(), Some("Ctrl+C"));
        assert!(!item.has_submenu());
        assert!(item.is_enabled());
        assert_eq!(item.height, 22.0);
    }

    #[test]
    fn test_separator() {
        let item = MenuItem::separator();

        assert!(item.is_separator());
        assert_eq!(item.label(), None);
        assert_eq!(item.height, 9.0);
    }

    #[test]
    fn test_submenu() {
        let item = MenuItem::submenu(
            "File",
            vec![MenuItem::action("New"), MenuItem::action("Open")],
        );

        assert!(item.has_submenu());
        assert_eq!(item.label(), Some("File"));
        assert_eq!(item.submenu_items().unwrap().len(), 2);
    }

    #[test]
    fn test_hover_state() {
        let mut item = MenuItem::action("Test");

        assert_eq!(item.visual_state(), VisualState::Normal);

        item.set_hovered(true);
        assert_eq!(item.visual_state(), VisualState::Hovered);

        item.set_hovered(false);
        assert_eq!(item.visual_state(), VisualState::Normal);
    }

    #[test]
    fn test_disabled_state() {
        let mut item = MenuItem::action("Test").disabled();

        assert!(!item.is_enabled());
        assert_eq!(item.visual_state(), VisualState::Disabled);

        // Hovering disabled item should not change visual state
        item.set_hovered(true);
        assert_eq!(item.visual_state(), VisualState::Disabled);
    }
}
