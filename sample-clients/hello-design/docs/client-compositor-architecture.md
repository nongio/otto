# Client-Compositor Architecture Design

## Overview

ScreenComposer clients communicate with the compositor's `lay-rs` scene graph. This document defines the recommended strategy for efficient layer usage and API design.

## Core Principle

**Client provides content, compositor does composition** - proven pattern across macOS Core Animation, Android SurfaceFlinger, and Web browsers.

## The Winning Strategy: Smart Container Layering

**Containers** → Compositor layers (subsurfaces)  
**Widgets** → Client-rendered into parent container

### Framework Automatically Decides

```rust
// Containers become subsurfaces (compositor layers)
let sidebar = Panel::new();  // Creates wl_subsurface
sidebar.add_child(Button::new("Save"));    // Client renders button
sidebar.add_child(Input::new());           // Client renders input
sidebar.add_child(Label::new("Status"));   // Client renders label

// Only the container is a compositor layer
sidebar.render();  // Re-renders this subsurface's texture
```

### Example: IDE Window Structure

```
Window (4 compositor layers)
├─ Subsurface: Sidebar (300×1080)
│   └─ Client renders: file tree + 5 buttons + search input
├─ Subsurface: Editor (1200×1080)
│   └─ Client renders: text + line numbers + cursor
├─ Subsurface: Bottom panel (1500×200)
│   └─ Client renders: terminal output + tabs
└─ Subsurface: Autocomplete (400×300)
    └─ Client renders: 10 results with icons
```

**Result:** 4 compositor layers instead of 50+, each container updates independently.

## When to Create Compositor Layers

✅ **Create layer when:**
- Different **update rates** (video @60fps + UI on change)
- Independent **animations** (sidebar slides while content stays)
- Needs **compositor effects** (fade, blur, transitions)
- Large **static content** (background rendered once)
- Different **z-order** requirements (overlays, modals)

❌ **Don't create layer for:**
- Static groupings (buttons in toolbar → one texture)
- Components updating together (form inputs → one texture)
- Pure organization (use client-side layout)
- Every UI widget (that's 100s of layers!)

## Layer Budget Guidelines

| App Type | Target Layers | Examples |
|----------|---------------|----------|
| Simple app | 1-3 | Calculator, clock |
| Text editor | 3-5 | Content, gutter, autocomplete |
| Video player | 4-6 | Video, controls, overlays, captions |
| IDE | 5-10 | Multiple panels, floating tools |
| Browser | 5-15 | Chrome UI, content, videos, fixed elements |
| **Danger zone** | 50+ | Likely over-layering |

## Antipatterns

### ❌ Layer Proliferation

```rust
// WRONG - 147 layers for form with 50 inputs
for input in &inputs {
    form.add_layer(input.background);
    form.add_layer(input.border);
    form.add_layer(input.text);
}

// RIGHT - 1 layer for entire form
form.render_all_to_texture(&inputs);
```

### ❌ Raw Pointer Antipattern

```rust
// WRONG - Dangling pointer
let window = Window::new();
let ptr = &window as *const _;
register(ptr);  // Stores pointer
Ok(window)  // Moves window, pointer now invalid!

// RIGHT - Use trait methods
impl App {
    fn on_configure(&mut self) {
        self.window.handle();  // Safe owned reference
    }
}
```

## Implementation in hello-design

### Container Components (Create Subsurfaces)

```rust
pub struct Panel {
    subsurface: SubsurfaceSurface,  // wl_subsurface
    canvas: Canvas,                  // Skia for rendering
    children: Vec<Box<dyn Widget>>,
}

impl Panel {
    pub fn render(&mut self) {
        // Render all child widgets to this surface's texture
        for child in &self.children {
            child.draw(&mut self.canvas);
        }
        self.subsurface.commit();
    }
}
```

### Widget Components (Render to Parent)

```rust
pub struct Button {
    label: String,
    bounds: Rect,
    // No subsurface - draws to parent canvas
}

impl Widget for Button {
    fn draw(&self, canvas: &mut Canvas) {
        canvas.draw_rect(&self.bounds);
        canvas.draw_text(&self.label);
    }
}
```

### Decision Tree

```
Component needs to be created?
├─ Container (Panel, ScrollArea, Tabs)?
│   └─ Needs independence? (animation, different update rate)
│       ├─ YES → Create wl_subsurface (compositor layer)
│       └─ NO → Just group widgets, render to parent
└─ Widget (Button, Input, Label)?
    └─ Always render to parent surface
```

## Comparison to Core Animation

### Similarities
- Layer hierarchies committed to compositor
- Client sets content, compositor composites
- Property animations (opacity, transform)
- Transaction model for atomic updates

### Key Insight: Implicit Animations

Core Animation animates property changes automatically:
```objective-c
[CATransaction begin];
layer.opacity = 0.5;  // Automatically animates over 0.3s
[CATransaction commit];
```

**We should adopt:**
```rust
Transaction::begin();
layer.set_opacity(0.5, Duration::from_millis(300));
Transaction::commit();  // Compositor animates smoothly
```

## Next Steps

1. **Stabilize Pattern 1** - Simple single-texture API
2. **Implement smart containers** - Panel/ScrollArea create subsurfaces
3. **Add transaction model** - Atomic multi-layer updates
4. **Implicit animations** - Compositor animates property changes
5. **Document sc-layer protocol** - Formalize multi-surface semantics

## References

- Wayland protocol handles texture ownership/lifetime
- dmabuf = zero-copy, SHM = compositor copies
- Compositor cleans up on client crash
- Frame callbacks sync client with display refresh
