# HelloDesign Component Creation Guide

**Type:** Skill/Pattern Guide  
**Scope:** `components/hello-design/src/components/`  
**When to use:** Creating new UI components for the hello-design library

---

## Overview

This guide defines the standard architecture pattern for creating HelloDesign UI components with clear separation of concerns.

# Creating a HelloDesign Component

## Component Architecture Layers

Every HelloDesign component consists of up to 4 layers:

1. **State Layer** - Component state/model
2. **Style Layer** - Visual configuration
3. **Renderer Layer** - Pure drawing functions
4. **Component Layer** - Public API

Surface-owning components use existing surface types directly:
- `ToplevelSurface`, `PopupSurface`, `SubsurfaceSurface`, `LayerShellSurface`

## Step-by-Step Guide

### 1. Create Component Directory Structure

```
src/components/my_component/
├── mod.rs           # Re-exports
├── state.rs         # State/model
├── style.rs         # Styling
├── renderer.rs      # Drawing logic
└── my_component.rs  # Main component API
```

### 2. Define the State Layer (`state.rs`)

Pure state - no rendering, no surface logic.

```rust
/// State for MyComponent
#[derive(Clone, Debug)]
pub struct MyComponentState {
    // Core data
    items: Vec<String>,
    selected_index: Option<usize>,
    
    // Interaction state
    hover_index: Option<usize>,
    is_focused: bool,
}

impl MyComponentState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected_index: None,
            hover_index: None,
            is_focused: false,
        }
    }
    
    // Getters
    pub fn items(&self) -> &[String] {
        &self.items
    }
    
    pub fn selected(&self) -> Option<usize> {
        self.selected_index
    }
    
    // State mutations
    pub fn add_item(&mut self, item: String) {
        self.items.push(item);
    }
    
    pub fn select(&mut self, index: Option<usize>) {
        self.selected_index = index;
    }
    
    pub fn set_hover(&mut self, index: Option<usize>) {
        self.hover_index = index;
    }
    
    pub fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }
    
    // Pure logic (no I/O)
    pub fn next_item(&mut self) {
        if let Some(current) = self.selected_index {
            if current + 1 < self.items.len() {
                self.selected_index = Some(current + 1);
            }
        } else if !self.items.is_empty() {
            self.selected_index = Some(0);
        }
    }
}

impl Default for MyComponentState {
    fn default() -> Self {
        Self::new()
    }
}
```

### 3. Define the Style Layer (`style.rs`)

Visual configuration only.

```rust
use skia_safe::Color;

/// Visual styling for MyComponent
#[derive(Clone, Debug)]
pub struct MyComponentStyle {
    // Dimensions
    pub item_height: f32,
    pub padding: f32,
    pub min_width: f32,
    
    // Colors
    pub background_color: Color,
    pub text_color: Color,
    pub hover_color: Color,
    pub selected_color: Color,
    pub border_color: Color,
    
    // Typography
    pub font_size: f32,
    pub line_height: f32,
    
    // Borders/Shapes
    pub border_width: f32,
    pub corner_radius: f32,
}

impl Default for MyComponentStyle {
    fn default() -> Self {
        Self {
            item_height: 32.0,
            padding: 8.0,
            min_width: 120.0,
            
            background_color: Color::WHITE,
            text_color: Color::from_rgb(40, 40, 40),
            hover_color: Color::from_argb(20, 0, 0, 0),
            selected_color: Color::from_argb(40, 0, 120, 215),
            border_color: Color::from_rgb(200, 200, 200),
            
            font_size: 14.0,
            line_height: 1.5,
            
            border_width: 1.0,
            corner_radius: 4.0,
        }
    }
}

impl MyComponentStyle {
    /// Calculate total height needed for given number of items
    pub fn height_for_items(&self, item_count: usize) -> f32 {
        item_count as f32 * self.item_height + self.padding * 2.0
    }
    
    /// Calculate width for given text (if needed)
    pub fn width_for_text(&self, text: &str) -> f32 {
        // Use typography helpers if needed
        let text_width = crate::typography::measure_text(text, self.font_size);
        text_width + self.padding * 2.0
    }
}
```

### 4. Define the Renderer Layer (`renderer.rs`)

Pure functions - stateless drawing.

```rust
use skia_safe::{Canvas, Paint, Rect, Color};
use super::{MyComponentState, MyComponentStyle};


    /// Main render function
    pub fn draw_mycomponent(
        canvas: &Canvas,
        state: &MyComponentState,
        style: &MyComponentStyle,
        width: f32,
        height: f32,
    ) {
        // Draw background
        Self::draw_background(canvas, style, width, height);
        //...
    }
    
    /// Calculate required dimensions for the component
    pub fn measure(state: &MyComponentState, style: &MyComponentStyle) -> (f32, f32) {
        let height = style.height_for_items(state.items().len());
        let width = state.items()
            .iter()
            .map(|item| style.width_for_text(item))
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(style.min_width)
            .max(style.min_width);
        
        (width, height)
    }
}
```

### 5. Define the Component Layer (`my_component.rs`)

Public API gluing everything together.

```rust
use std::rc::Rc;
use std::cell::RefCell;

use super::{MyComponentState, MyComponentStyle, MyComponentRenderer};
use crate::surfaces::{PopupSurface, Surface, SurfaceError};
use skia_safe::Canvas;

/// High-level MyComponent
/// 
/// Can be used in two modes:
/// 1. As a rendered component (no surface) - call `render_to(canvas)`
/// 2. As a surface-owning component - call `with_popup()` then `render()`
pub struct MyComponent {
    state: Rc<RefCell<MyComponentState>>,
    style: MyComponentStyle,
    
    // Optional surface (for popup/window mode)
    surface: Option<PopupSurface>,
    
    // Cached dimensions
    width: f32,
    height: f32,
    
    // Callbacks
    on_select: Option<Rc<dyn Fn(&str)>>,
    on_hover: Option<Rc<dyn Fn(Option<usize>)>>,
}

impl MyComponent {
    // === Construction ===
    
    /// Create a new component without a surface
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(MyComponentState::new())),
            style: MyComponentStyle::default(),
            surface: None,
            width: 200.0,
            height: 100.0,
            on_select: None,
            on_hover: None,
        }
    }
    
    /// Attach a popup surface to this component
    pub fn with_popup(
        mut self,
        parent: &smithay_client_toolkit::shell::xdg::XdgSurface,
        positioner: &smithay_client_toolkit::shell::xdg::XdgPositioner,
    ) -> Result<Self, SurfaceError> {
        let (width, height) = self.measure();
        self.width = width;
        self.height = height;
        
        let surface = PopupSurface::new(
            parent,
            positioner,
            width as i32,
            height as i32,
        )?;
        
        self.surface = Some(surface);
        Ok(self)
    }
    
    // === Builder API ===
    
    pub fn add_item(self, item: impl Into<String>) -> Self {
        self.state.borrow_mut().add_item(item.into());
        self
    }
    
    pub fn style(mut self, style: MyComponentStyle) -> Self {
        self.style = style;
        self
    }
    
    pub fn on_select<F>(mut self, callback: F) -> Self
    where
        F: Fn(&str) + 'static,
    {
        self.on_select = Some(Rc::new(callback));
        self
    }
    
    pub fn on_hover<F>(mut self, callback: F) -> Self
    where
        F: Fn(Option<usize>) + 'static,
    {
        self.on_hover = Some(Rc::new(callback));
        self
    }
    
    // === State Access ===
    
    pub fn state(&self) -> &Rc<RefCell<MyComponentState>> {
        &self.state
    }
    
    pub fn style(&self) -> &MyComponentStyle {
        &self.style
    }
    
    // === Rendering ===
    
    /// Render to the component's own surface (if it has one)
    pub fn render(&self) {
        if let Some(surface) = &self.surface {
            surface.draw(|canvas| {
                self.render_to(canvas);
            });
        }
    }
    
    /// Render to a provided canvas (for embedded rendering)
    pub fn render_to(&self, canvas: &Canvas) {
        let state = self.state.borrow();
        MyComponentRenderer::render(
            canvas,
            &state,
            &self.style,
            self.width,
            self.height,
        );
    }
    
    /// Measure the component's required dimensions
    pub fn measure(&self) -> (f32, f32) {
        let state = self.state.borrow();
        MyComponentRenderer::measure(&state, &self.style)
    }
    
    // === Event Handling ===
    
    pub fn handle_click(&mut self, x: f32, y: f32) {
        let state = self.state.borrow();
        let item_index = ((y - self.style.padding) / self.style.item_height) as usize;
        
        if item_index < state.items().len() {
            if let Some(callback) = &self.on_select {
                let item = state.items()[item_index].clone();
                callback(&item);
            }
            drop(state);
            self.state.borrow_mut().select(Some(item_index));
            self.request_redraw();
        }
    }
    
    pub fn handle_hover(&mut self, x: f32, y: f32) {
        let state = self.state.borrow();
        let item_index = if y >= self.style.padding && y <= self.height - self.style.padding {
            Some(((y - self.style.padding) / self.style.item_height) as usize)
        } else {
            None
        };
        
        let item_index = item_index.filter(|&idx| idx < state.items().len());
        
        if state.hover_index != item_index {
            if let Some(callback) = &self.on_hover {
                callback(item_index);
            }
            drop(state);
            self.state.borrow_mut().set_hover(item_index);
            self.request_redraw();
        }
    }
    
    pub fn handle_key(&mut self, key: u32) {
        const KEY_UP: u32 = 103;
        const KEY_DOWN: u32 = 108;
        const KEY_ENTER: u32 = 28;
        
        match key {
            KEY_DOWN => {
                self.state.borrow_mut().next_item();
                self.request_redraw();
            }
            KEY_UP => {
                // Implement previous_item in state
                self.request_redraw();
            }
            KEY_ENTER => {
                let state = self.state.borrow();
                if let Some(idx) = state.selected() {
                    if let Some(callback) = &self.on_select {
                        let item = state.items()[idx].clone();
                        callback(&item);
                    }
                }
            }
            _ => {}
        }
    }
    
    // === Utilities ===
    
    fn request_redraw(&self) {
        if let Some(surface) = &self.surface {
            surface.request_frame();
        }
    }
}

impl Default for MyComponent {
    fn default() -> Self {
        Self::new()
    }
}
```

### 6. Create Module Export (`mod.rs`)

```rust
mod state;
mod style;
mod renderer;
mod my_component;

pub use state::MyComponentState;
pub use style::MyComponentStyle;
pub use renderer::MyComponentRenderer;
pub use my_component::MyComponent;
```

### 7. Add to Parent Module

In `src/components/mod.rs`:

```rust
pub mod my_component;
pub use my_component::MyComponent;
```

## Component Patterns

### Pattern A: Rendered Component Only

```rust
let component = MyComponent::new()
    .add_item("Option 1")
    .add_item("Option 2")
    .style(MyComponentStyle::default());

// Render to parent canvas
component.render_to(canvas);
```

### Pattern B: Surface-Owning Component

```rust
let component = MyComponent::new()
    .add_item("Option 1")
    .add_item("Option 2")
    .with_popup(&parent_xdg, &positioner)?;

// Renders to its own surface
component.render();
```

### Pattern C: Shared State

```rust
let state = Rc::new(RefCell::new(MyComponentState::new()));

let component1 = MyComponent::new_with_state(state.clone());
let component2 = MyComponent::new_with_state(state.clone());

// Both components share the same state
```

## Testing Strategy

### Test State Layer

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_state_operations() {
        let mut state = MyComponentState::new();
        state.add_item("Item 1".into());
        
        assert_eq!(state.items().len(), 1);
        assert_eq!(state.selected(), None);
        
        state.select(Some(0));
        assert_eq!(state.selected(), Some(0));
    }
    
    #[test]
    fn test_navigation() {
        let mut state = MyComponentState::new();
        state.add_item("A".into());
        state.add_item("B".into());
        
        state.next_item();
        assert_eq!(state.selected(), Some(0));
        
        state.next_item();
        assert_eq!(state.selected(), Some(1));
    }
}
```

### Test Renderer

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_measure() {
        let state = MyComponentState::new();
        state.add_item("Short".into());
        
        let style = MyComponentStyle::default();
        let (width, height) = MyComponentRenderer::measure(&state, &style);
        
        assert!(width >= style.min_width);
        assert!(height > 0.0);
    }
}
```

## Checklist for New Component

- [ ] Create directory structure
- [ ] Define state in `state.rs`
- [ ] Define style in `style.rs`
- [ ] Implement renderer in `renderer.rs`
- [ ] Create component API in `{component}.rs`
- [ ] Export from `mod.rs`
- [ ] Add to parent `mod.rs`
- [ ] Write state tests
- [ ] Write renderer tests
- [ ] Document public API
- [ ] Add usage examples

## Common Pitfalls

❌ **Don't**: Mix rendering logic in state
✅ **Do**: Keep state pure, rendering in renderer

❌ **Don't**: Store Wayland objects in state
✅ **Do**: Use surfaces at component level only

❌ **Don't**: Put callbacks in style
✅ **Do**: Callbacks belong in component layer

❌ **Don't**: Make renderer stateful
✅ **Do**: Pass all needed data as parameters
