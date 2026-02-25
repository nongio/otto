# Hello Design - Wayland Skia Client

A simple Wayland client demonstrating how to create surfaces with Skia-based rendering, plus a design system with reusable UI components.

## Components

Hello Design includes the following UI components:

### MenuBar

A horizontal menu bar with toggleable menu labels. Each label controls showing/hiding a separate menu component.

**Features:**
- Horizontal layout with auto-calculated bounds
- Toggle behavior (click to open/close)
- Mutual exclusion (only one menu open at a time)
- Customizable colors and styling
- Skia-based rendering

**Documentation:** See [docs/MenuBar.md](docs/MenuBar.md)

**Example:**
```rust
let mut menu_bar = MenuBar::new()
    .with_height(32.0)
    .with_background(Color::from_rgb(240, 240, 240));

menu_bar.add_item("File", vec![
    MenuItem::action("file.new", "New").build(),
    MenuItem::action("file.open", "Open...").build(),
]);

menu_bar.render(canvas, width);
```

**Run example:** `cargo run --example menu_bar`

### Menu

A popup menu component with support for items, separators, and submenus.

**Run example:** `cargo run --example simple_menu`

### ContextMenu

A popup context menu component with keyboard and mouse navigation support.

**Features:**
- Popup positioning relative to parent window
- Support for menu items, separators, and nested submenus
- Mouse hover and click handling
- Keyboard navigation:
  - **Down arrow**: Navigate to next menu item (starts at first item)
  - **Up arrow**: Navigate to previous menu item
  - **Enter/Space**: Activate selected item or open submenu
  - **Right arrow**: Open submenu (if available)
  - **Left arrow**: Close submenu or menu
  - **Escape**: Close menu
- Automatic selection highlighting for keyboard navigation
- Customizable width and styling
- Item click callbacks

**Example:**
```rust
let context_menu = ContextMenu::new()
    .with_items(vec![
        MenuItem::action("New Window").with_shortcut("⌘N"),
        MenuItem::separator(),
        MenuItem::submenu("Recent Files", vec![
            MenuItem::action("document1.txt"),
            MenuItem::action("document2.txt"),
        ]),
        MenuItem::separator(),
        MenuItem::action("Quit").with_shortcut("⌘Q"),
    ])
    .with_width(220.0)
    .on_item_click(|label| {
        println!("Menu item clicked: {}", label);
    });

// Show menu at position
context_menu.show_at::<MyApp>(&window, x, y)?;

// Handle keyboard events in your app
impl App for MyApp {
    fn on_keyboard_event(&mut self, key: u32, state: wl_keyboard::KeyState) {
        self.context_menu.lock().unwrap().on_keyboard_event::<MyApp>(key, state);
    }
}
```

**Run example:** `cargo run --example dock_application_simple`

### Window

A high-level window component using AppRunner framework.

**Features:**
- Automatic Wayland protocol handling via AppRunner
- Default rounded corners (12px radius)
- Simple API for rendering custom content
- Support for layer customization (opacity, blur, borders, etc.)

**Example:**
```rust
let mut window = Window::new::<MyApp>("My Window", 800, 600)?;
window.set_background(Color::from_rgb(255, 255, 255));

window.on_draw(|canvas| {
    // Draw your content here
    let font = styles::H1.font();
    let paint = Paint::new(Color::from_rgb(0, 0, 0), None);
    canvas.draw_str("Hello World", (x, y), &font, &paint);
});
```

**Run examples:** 
- `cargo run --example simple_window` - Basic window
- `cargo run --example rounded_corners` - Demonstrates default rounded corners
- `cargo run --example simple_app` - Window with custom layer effects

### Typography System

A design system with font caching and predefined text styles using Inter font.

**Features:**
- Thread-local font cache for performance
- Predefined text styles: Display, H1-H3, Title, Body, Label, Caption
- Automatic fallback to system fonts if Inter is not available
- Subpixel antialiasing enabled by default

**Example:**
```rust
use otto_kit::prelude::*;

window.on_draw(|canvas| {
    let font = styles::H1.font();
    let paint = Paint::new(Color::from_rgb(0, 0, 0), None);
    canvas.draw_str("Hello World", (x, y), &font, &paint);
});
```

**Run example:** `cargo run --example typography_demo`

## Architecture

This example uses:
- **smithay-client-toolkit**: High-level Wayland client framework
- **wayland-egl**: Bridge between Wayland surfaces and EGL
- **khronos-egl**: EGL display/context management
- **skia-safe**: 2D graphics rendering with GPU acceleration
- **gl**: OpenGL function loading

## How It Works

1. **Wayland Connection**: Connects to the Wayland compositor using smithay-client-toolkit
2. **Window Creation**: Creates an XDG toplevel window
3. **EGL Setup**: 
   - Gets EGL display from Wayland display pointer
   - Creates OpenGL ES 2.0 context
   - Creates `WlEglSurface` bridging Wayland surface to EGL
   - Creates EGL window surface
4. **Skia Integration**:
   - Loads GL function pointers via EGL
   - Creates Skia GL interface
   - Creates Skia DirectContext for GPU-accelerated rendering
5. **Rendering Loop**:
   - Queries framebuffer info from GL
   - Wraps EGL surface as Skia backend render target
   - Uses Skia canvas API to draw (clear, shapes, text, etc.)
   - Flushes to GPU and swaps buffers

## Running

```bash
cargo build --release -p hello-design
./target/release/hello-design
```

The client will display a window with a rotating blue rectangle rendered using Skia.

## Next Steps

This foundation can be extended to:
1. Create multiple subsurfaces with independent Skia canvases
2. Position and layer subsurfaces
3. Handle input events (keyboard, mouse)
4. Implement more complex UI with the full Skia API
5. Add animations and transitions

## Key Code Pattern

The main pattern for Skia-on-Wayland surfaces:

```rust
// 1. Create Wayland surface
let wl_surface = compositor.create_surface(&qh);

// 2. Get Wayland display pointer for EGL
let display_ptr = conn.backend().display_ptr();

// 3. Initialize EGL with Wayland display
let egl = khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required()?;
let egl_display = egl.get_display(display_ptr as NativeDisplayType)?;

// 4. Create EGL context
let egl_context = egl.create_context(display, config, None, &context_attribs)?;

// 5. Create WlEglSurface bridging Wayland to EGL
let wl_egl_surface = wayland_egl::WlEglSurface::new(wl_surface.id(), width, height)?;

// 6. Create EGL surface
let egl_surface = egl.create_window_surface(display, config, wl_egl_surface.ptr(), None)?;

// 7. Make context current
egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(egl_context))?;

// 8. Load GL and create Skia context
gl::load_with(|name| egl.get_proc_address(name).unwrap() as *const _);
let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
    egl.get_proc_address(name).unwrap() as *const _
})?;
let skia_context = skia_safe::gpu::direct_contexts::make_gl(interface, None)?;

// 9. In render loop: wrap as Skia surface and draw
let backend_rt = skia_safe::gpu::backend_render_targets::make_gl(...);
let surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
    &mut skia_context, &backend_rt, ...
)?;
let canvas = surface.canvas();
// ... draw with canvas ...
skia_context.flush_and_submit();
egl.swap_buffers(display, surface);
```

This pattern can be reused for each subsurface you want to create with its own Skia canvas.
