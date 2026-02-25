// use crate::prelude::MenuItemKind;

// use super::{MenuItem, MenuItemRenderer, MenuItemStyle};
// use skia_safe::Canvas;

// #[derive(Clone)]
// pub struct MenuItem {
//     data: MenuItem,
//     style: MenuItemStyle,
// }

// impl MenuItem {
//     /// Create new MenuItem from data
//     pub fn new(data: MenuItem) -> Self {
//         Self {
//             data,
//             style: MenuItemStyle::default(),
//         }
//     }

//     /// Create an action item
//     pub fn action(label: impl Into<String>) -> Self {
//         Self::new(MenuItem::action(label))
//     }

//     /// Create a separator
//     pub fn separator() -> Self {
//         Self::new(MenuItem::separator())
//     }

//     /// Create a submenu item
//     pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
//         Self::new(MenuItem::submenu(label, items))
//     }

//     // === Builder API ===

//     pub fn with_style(mut self, style: MenuItemStyle) -> Self {
//         self.style = style;
//         self
//     }

//     pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
//         self.data = self.data.with_shortcut(shortcut);
//         self
//     }

//     pub fn with_callback<F>(mut self, callback: F) -> Self
//     where
//         F: Fn() + 'static,
//     {
//         self.data = self.data.with_callback(callback);
//         self
//     }

//     pub fn disabled(mut self) -> Self {
//         self.data = self.data.disabled();
//         self
//     }

//     // === Access ===

//     pub fn data(&self) -> &MenuItem {
//         &self.data
//     }

//     pub fn data_mut(&mut self) -> &mut MenuItem {
//         &mut self.data
//     }

//     pub fn style(&self) -> &MenuItemStyle {
//         &self.style
//     }

//     pub fn height(&self) -> f32 {
//         self.data.height
//     }

//     /// Access the kind (for backward compatibility with old code)
//     pub fn kind(&self) -> &MenuItemKind {
//         self.data.kind()
//     }

//     /// For backward compatibility - old code used with_state
//     pub fn with_state(mut self, state: super::VisualState) -> Self {
//         self.data.set_visual_state(state);
//         self
//     }

//     // === Rendering ===

//     /// Render this menu item at the given position
//     pub fn render_at(&self, canvas: &Canvas, x: f32, y: f32, width: f32) {
//         MenuItemRenderer::render(canvas, &self.data, &self.style, x, y, width);
//     }
// }

// impl std::fmt::Debug for MenuItem {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("MenuItem")
//             .field("data", &self.data)
//             .finish()
//     }
// }

// impl From<MenuItem> for MenuItem {
//     fn from(data: MenuItem) -> Self {
//         Self::new(data)
//     }
// }
