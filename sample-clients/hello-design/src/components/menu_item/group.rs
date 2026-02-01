use skia_safe::Canvas;

use crate::common::Renderable;
use crate::components::menu_item::MenuItem;

/// A vertical group of menu items
#[derive(Debug, Clone)]
pub struct MenuItemGroup {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub items: Vec<MenuItem>,
}

impl MenuItemGroup {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            items: Vec::new(),
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn add(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: Vec<MenuItem>) -> Self {
        self.items = items;
        self
    }

    pub fn build(self) -> Self {
        self
    }

    /// Calculate total height of the group
    pub fn height(&self) -> f32 {
        self.items.iter().map(|item| item.height).sum()
    }
}

impl Default for MenuItemGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for MenuItemGroup {
    fn render(&self, canvas: &Canvas) {
        let mut current_y = self.y;

        for item in &self.items {
            let positioned_item = item.clone()
                .at(self.x, current_y)
                .with_size(self.width, item.height);
            
            positioned_item.render(canvas);
            current_y += item.height;
        }
    }
}
