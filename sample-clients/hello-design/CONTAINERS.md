# Container System - Quick Start

## Overview

The hello-design container system provides a flexible foundation for building UIs with support for both drawing-based and subsurface-backed rendering.

## Key Features

### ✨ Flexible Rendering Backends
- **DrawingBackend**: Simple direct rendering on canvas (default)
- **SurfaceBackend**: Optimized subsurface rendering (for performance)

### 📦 Core Components
- **Frame**: Styled rectangular containers
- **Stack**: Linear layout containers (horizontal/vertical)

### 🎨 Rich Styling
- Background colors
- Corner radius (uniform or per-corner)
- Borders
- Box shadows
- Padding

## Quick Example

```rust
use hello_design::prelude::*;
use hello_design::components::container::stack::StackAlignment;

// Create a styled frame
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .with_corner_radius(8.0)
    .with_border(Color::from_rgb(200, 200, 200), 1.0)
    .with_padding(16.0)
    .build();

// Create a vertical stack
let mut stack = Stack::new(StackDirection::Vertical)
    .with_gap(8.0)
    .with_alignment(StackAlignment::Center);

stack.add(Box::new(frame1));
stack.add(Box::new(frame2));

// Render
stack.render(canvas);
```

## Usage Patterns

### 1. Simple Colored Frames

```rust
let frame = FrameBuilder::new(100.0, 80.0)
    .with_background(Color::from_rgb(255, 100, 100))
    .with_corner_radius(8.0)
    .build();
```

### 2. Frames with Styling

```rust
let frame = FrameBuilder::new(120.0, 80.0)
    .with_background(Color::WHITE)
    .with_corner_radius(8.0)
    .with_border(Color::from_rgb(200, 200, 200), 2.0)
    .with_shadow(BoxShadow::new(
        Color::from_argb(50, 0, 0, 0),
        0.0, 4.0, 8.0
    ))
    .build();
```

### 3. Custom Drawing

```rust
let frame = FrameBuilder::new(300.0, 100.0)
    .with_background(Color::WHITE)
    .with_draw_fn(|canvas, bounds| {
        // Your custom drawing code
        let paint = Paint::default();
        canvas.draw_circle(bounds.center(), 20.0, &paint);
    })
    .build();
```

### 4. Nested Containers

```rust
let mut outer = FrameBuilder::new(400.0, 120.0)
    .with_background(Color::from_rgb(240, 240, 240))
    .with_padding(16.0)
    .build();

let mut inner_stack = Stack::new(StackDirection::Horizontal)
    .with_gap(8.0);

// Add children to inner stack...
outer.add_child(Box::new(inner_stack));
```

## Performance Optimization

### When to Use Surface-Backed Rendering

Switch to surface-backed rendering when:
- Content is expensive to render
- Content is mostly static
- You're experiencing performance issues

```rust
// Enable subsurface-backed rendering
let frame = FrameBuilder::new(200.0, 100.0)
    .with_background(Color::WHITE)
    .use_surface()  // Enable subsurface backend
    .build_auto();  // Automatically chooses backend
```

## Layout System

### Stack Directions
- `StackDirection::Vertical` - Top to bottom
- `StackDirection::Horizontal` - Left to right

### Stack Alignment
- `StackAlignment::Start` - Align to start (left/top)
- `StackAlignment::Center` - Center children
- `StackAlignment::End` - Align to end (right/bottom)
- `StackAlignment::Stretch` - Stretch to fill

## API Reference

### Frame Builder Methods
- `with_background(color)` - Set background color
- `with_corner_radius(radius)` - Set uniform corner radius
- `with_border(color, width)` - Add border
- `with_shadow(shadow)` - Add box shadow
- `with_padding(padding)` - Set padding
- `with_draw_fn(fn)` - Custom drawing function
- `use_surface()` - Enable subsurface backend

### Stack Methods
- `with_gap(gap)` - Set spacing between children
- `with_alignment(alignment)` - Set child alignment
- `with_padding(padding)` - Set padding
- `add(child)` - Add a child container

## Running the Example

```sh
cargo run --example containers
```

## Documentation

For detailed documentation, see [docs/containers.md](./docs/containers.md)

## Migration Path

1. **Start Simple**: Build UIs with drawing-based containers
2. **Profile**: Identify performance bottlenecks
3. **Optimize**: Switch specific containers to surface-backed rendering
4. **Iterate**: Fine-tune backend choices

The beauty of this system is you can change one line (`.use_surface()`) to switch rendering backends without refactoring your entire UI!
