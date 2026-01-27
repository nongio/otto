# Container System

The container system provides a flexible foundation for building user interfaces in hello-design. It offers an abstraction that allows you to create complex UIs using simple drawing-based rendering initially, and then optimize specific parts with subsurface-backed rendering when needed.

## Core Concepts

### Container Trait

The `Container` trait is the foundation of the system. All container components implement this trait:

```rust
pub trait Container {
    fn bounds(&self) -> Rect;
    fn set_position(&mut self, x: f32, y: f32);
    fn set_size(&mut self, width: f32, height: f32);
    fn render(&mut self, canvas: &Canvas);
    fn handle_pointer(&mut self, x: f32, y: f32) -> bool;
    fn children_mut(&mut self) -> Vec<&mut dyn Container>;
    fn add_child(&mut self, child: Box<dyn Container>);
}
```

### Backend Abstraction

Containers can use two different rendering backends:

1. **DrawingBackend** - Renders directly on the parent canvas
   - Simpler, good for prototyping
   - Lower memory overhead
   - Re-renders on every frame

2. **SurfaceBackend** - Uses Wayland subsurfaces for optimized rendering
   - Better performance for complex/static content
   - Caches rendering between frames
   - Slightly more memory overhead

The backend system allows you to switch between implementations without changing your UI code.

## Components

### Frame

A rectangular container with styling support (background, borders, shadows, padding).

#### Basic Usage

```rust
use hello_design::components::container::{FrameBuilder, EdgeInsets};
use skia_safe::Color;

// Simple frame with background
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .with_corner_radius(8.0)
    .build();

// Frame with styling
let styled_frame = FrameBuilder::new(200.0, 100.0)
    .at(10.0, 20.0)
    .with_background(Color::from_rgb(240, 240, 240))
    .with_corner_radius(12.0)
    .with_border(Color::from_rgb(200, 200, 200), 1.0)
    .with_padding(16.0)
    .build();
```

#### Custom Drawing

Frames support custom drawing functions:

```rust
let custom_frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .with_draw_fn(|canvas, bounds| {
        // Custom drawing code
        let mut paint = Paint::default();
        paint.set_color(Color::BLUE);
        canvas.draw_circle(bounds.center(), 20.0, &paint);
    })
    .build();
```

#### Surface-Backed Rendering

For performance optimization, use surface-backed rendering:

```rust
// Automatically chooses backend
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .use_surface()  // Enable surface-backed rendering
    .build_auto();

// Or explicitly build with surface backend
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .build_with_surface();
```

### Stack

A container that arranges children in a linear layout (horizontal or vertical).

#### Vertical Stack

```rust
use hello_design::components::container::{Stack, StackDirection, StackAlignment};

let mut stack = Stack::new(StackDirection::Vertical)
    .with_gap(8.0)
    .with_padding(EdgeInsets::uniform(16.0))
    .with_alignment(StackAlignment::Center);

// Add children
stack.add(Box::new(frame1));
stack.add(Box::new(frame2));
stack.add(Box::new(frame3));
```

#### Horizontal Stack

```rust
let mut stack = Stack::new(StackDirection::Horizontal)
    .with_gap(12.0)
    .with_alignment(StackAlignment::Start);

stack.add(Box::new(button1));
stack.add(Box::new(button2));
```

#### Alignment Options

- `StackAlignment::Start` - Align to start (left/top)
- `StackAlignment::Center` - Center children
- `StackAlignment::End` - Align to end (right/bottom)
- `StackAlignment::Stretch` - Stretch children to fill

## Styling

### Corner Radius

```rust
use hello_design::components::container::CornerRadius;

// Uniform radius
let radius = CornerRadius::uniform(8.0);

// Top corners only
let radius = CornerRadius::top(12.0);

// Bottom corners only
let radius = CornerRadius::bottom(12.0);

// Custom per-corner
let radius = CornerRadius {
    top_left: 12.0,
    top_right: 8.0,
    bottom_right: 4.0,
    bottom_left: 0.0,
};
```

### Borders

```rust
use hello_design::components::container::Border;

let border = Border::new(
    Color::from_rgb(200, 200, 200),
    1.0  // width
);

frame.set_border(border);
```

### Shadows

```rust
use hello_design::components::container::BoxShadow;

let shadow = BoxShadow::new(
    Color::from_argb(50, 0, 0, 0),  // semi-transparent black
    0.0,   // offset_x
    4.0,   // offset_y
    8.0    // blur_radius
).with_spread(2.0);

frame.set_shadow(shadow);
```

### Padding

```rust
use hello_design::components::container::EdgeInsets;

// Uniform padding
let padding = EdgeInsets::uniform(16.0);

// Symmetric padding
let padding = EdgeInsets::symmetric(12.0, 16.0);  // vertical, horizontal

// Custom per-edge
let padding = EdgeInsets::only(8.0, 12.0, 16.0, 12.0);  // top, right, bottom, left
```

## Layout Constraints

Containers can have layout constraints:

```rust
use hello_design::components::container::LayoutConstraints;

let frame = FrameBuilder::new(100.0, 100.0)
    .with_min_width(150.0)
    .with_max_width(300.0)
    .with_min_height(100.0)
    .with_max_height(200.0)
    .build();

// When size is set, it's automatically constrained
frame.set_size(100.0, 250.0);
// Actual size will be (150.0, 200.0) due to constraints
```

## Building Complex UIs

### Nested Containers

```rust
// Outer container
let mut outer = FrameBuilder::new(400.0, 300.0)
    .with_background(Color::from_rgb(240, 240, 240))
    .with_padding(16.0)
    .build();

// Inner stack
let mut inner_stack = Stack::new(StackDirection::Vertical)
    .with_gap(8.0);

inner_stack.add(Box::new(header_frame));
inner_stack.add(Box::new(content_frame));
inner_stack.add(Box::new(footer_frame));

outer.add_child(Box::new(inner_stack));
```

### Card Layout Example

```rust
fn create_card(title: &str, content: &str) -> Frame {
    let mut card = FrameBuilder::new(300.0, 200.0)
        .with_background(Color::WHITE)
        .with_corner_radius(12.0)
        .with_shadow(BoxShadow::new(
            Color::from_argb(30, 0, 0, 0),
            0.0, 4.0, 12.0
        ))
        .with_padding(16.0)
        .build();

    // Add title and content as children...
    
    card
}
```

## Performance Optimization

### When to Use Surface-Backed Rendering

Use surface-backed rendering when:

1. Content is complex and expensive to render
2. Content is mostly static (doesn't change every frame)
3. You need to render many similar components
4. You're experiencing performance issues with drawing-based rendering

Example:

```rust
// Expensive custom drawing - use surface backend
let complex_widget = FrameBuilder::new(500.0, 400.0)
    .with_draw_fn(|canvas, bounds| {
        // Complex rendering: gradients, images, text, etc.
        draw_complex_chart(canvas, bounds);
    })
    .use_surface()  // Cache rendering in subsurface
    .build_auto();
```

### Marking Surfaces Dirty

When using surface-backed rendering, mark content dirty when it needs to be redrawn:

```rust
if let Some(surface_backend) = frame.backend_mut().downcast_mut::<SurfaceBackend>() {
    surface_backend.mark_dirty();
}
```

## Migration Path

The container system is designed to support gradual optimization:

1. **Start simple**: Build your UI with drawing-based containers
2. **Profile**: Identify performance bottlenecks
3. **Optimize**: Switch specific containers to surface-backed rendering
4. **Iterate**: Fine-tune which containers use which backend

Example migration:

```rust
// Before (all drawing-based)
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .build();

// After (optimized with surface)
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .use_surface()  // Single line change!
    .build_auto();
```

## Examples

See `examples/containers.rs` for a complete demonstration of the container system.

Run with:
```sh
cargo run --example containers
```
