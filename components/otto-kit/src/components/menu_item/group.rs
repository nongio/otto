use crate::components::menu_item::{MenuItem, MenuItemRenderer, MenuItemStyle};

use skia_safe::Canvas;

/// A vertical group of menu items
///
/// Handles layout and rendering of multiple menu items.
#[derive(Clone)]
pub struct MenuItemGroup {
    items: Vec<MenuItem>,
    x: f32,
    y: f32,
    width: f32,
    style: MenuItemStyle,
}

impl MenuItemGroup {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            x: 0.0,
            y: 0.0,
            width: 200.0,
            style: MenuItemStyle::default(),
        }
    }

    // === Builder API ===

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn add_item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: Vec<MenuItem>) -> Self {
        self.items = items;
        self
    }

    // === Access ===

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn get_items(&self) -> &[MenuItem] {
        &self.items
    }

    /// Calculate total height of the group
    pub fn height(&self) -> f32 {
        self.items.iter().map(|item| item.height).sum()
    }

    // === Rendering ===

    /// Render all items in the group
    pub fn render(&self, canvas: &Canvas) {
        let mut current_y = self.y;

        for item in &self.items {
            MenuItemRenderer::render(canvas, item, &self.style, self.x, current_y, self.width);
            current_y += item.height;
        }
    }
}

impl Default for MenuItemGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MenuItemGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuItemGroup")
            .field("items", &self.items.len())
            .field("x", &self.x)
            .field("y", &self.y)
            .field("width", &self.width)
            .finish()
    }
}
