# Surface Component Refactoring

## Overview

Refactor hello-design to use foundational surface components that encapsulate Wayland surface management, Skia rendering, and sc_layer protocol augmentation.

## Architecture

### Base Surface Types

#### 1. ToplevelSurface
Manages XDG toplevel window surfaces.

**Responsibilities:**
- Create and manage `wl_surface` + `xdg_toplevel`
- Handle window configure events
- Manage window geometry, decorations, title
- Provide Skia rendering context
- Support sc_layer augmentation on configure

**API:**
```rust
pub struct ToplevelSurface {
    wl_surface: wl_surface::WlSurface,
    window: Window,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    width: i32,
    height: i32,
    configured: bool,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl ToplevelSurface {
    /// Create a new toplevel surface
    pub fn new<D>(
        title: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>
    where D: /* trait bounds */;

    /// Draw on the surface using a callback
    pub fn draw<F>(&mut self, draw_fn: F) 
    where F: FnOnce(&skia_safe::Canvas);

    /// Apply sc_layer augmentation
    pub fn augment<F>(&mut self, augment_fn: F) -> Result<(), SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);

    /// Check if surface is configured
    pub fn is_configured(&self) -> bool;
    
    /// Get surface dimensions
    pub fn dimensions(&self) -> (i32, i32);
    
    /// Handle window configure event
    pub fn handle_configure(&mut self, configure: WindowConfigure, serial: u32);
}
```

#### 2. SubsurfaceSurface
Manages Wayland subsurfaces.

**Responsibilities:**
- Create and manage `wl_surface` + `wl_subsurface`
- Handle positioning relative to parent
- Provide Skia rendering context
- Support sc_layer augmentation

**API:**
```rust
pub struct SubsurfaceSurface {
    wl_surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl SubsurfaceSurface {
    /// Create a new subsurface
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>
    where D: /* trait bounds */;

    /// Draw on the surface using a callback
    pub fn draw<F>(&mut self, draw_fn: F) 
    where F: FnOnce(&skia_safe::Canvas);

    /// Apply sc_layer augmentation
    pub fn augment<F>(&mut self, augment_fn: F) -> Result<(), SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);

    /// Update position
    pub fn set_position(&mut self, x: i32, y: i32);
    
    /// Resize the surface
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), SurfaceError>;
    
    /// Get surface dimensions
    pub fn dimensions(&self) -> (i32, i32);
}
```

#### 3. PopupSurface
Manages XDG popup surfaces (for menus).

**Responsibilities:**
- Create and manage `wl_surface` + `xdg_popup`
- Handle popup positioning with positioner
- Handle popup configure events
- Provide Skia rendering context
- Support sc_layer augmentation

**API:**
```rust
pub struct PopupSurface {
    wl_surface: wl_surface::WlSurface,
    popup: Popup,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    width: i32,
    height: i32,
    configured: bool,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl PopupSurface {
    /// Create a new popup surface
    pub fn new<D>(
        parent_surface: &XdgSurface,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>
    where D: /* trait bounds */;

    /// Draw on the surface using a callback
    pub fn draw<F>(&mut self, draw_fn: F) 
    where F: FnOnce(&skia_safe::Canvas);

    /// Apply sc_layer augmentation
    pub fn augment<F>(&mut self, augment_fn: F) -> Result<(), SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);

    /// Check if popup is configured
    pub fn is_configured(&self) -> bool;
    
    /// Handle popup configure event
    pub fn handle_configure(&mut self, configure: PopupConfigure, serial: u32);
    
    /// Get the popup wl_surface
    pub fn wl_surface(&self) -> &wl_surface::WlSurface;
    
    /// Destroy the popup
    pub fn destroy(self);
}
```

#### 4. LayerShellSurface
Manages layer shell surfaces (for compositor-level surfaces like panels, overlays).

**Responsibilities:**
- Create and manage `wl_surface` + layer shell surface
- Handle layer shell configuration (layer, anchor, size, margin)
- Provide Skia rendering context
- Support sc_layer augmentation

**API:**
```rust
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

pub struct LayerShellSurface {
    wl_surface: wl_surface::WlSurface,
    layer_surface: /* layer shell surface type */,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    width: i32,
    height: i32,
    configured: bool,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl LayerShellSurface {
    /// Create a new layer shell surface
    pub fn new<D>(
        output: Option<&wl_output::WlOutput>,
        layer: Layer,
        namespace: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        layer_shell: &LayerShell, // from zwlr_layer_shell protocol
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>
    where D: /* trait bounds */;

    /// Draw on the surface using a callback
    pub fn draw<F>(&mut self, draw_fn: F) 
    where F: FnOnce(&skia_safe::Canvas);

    /// Apply sc_layer augmentation
    pub fn augment<F>(&mut self, augment_fn: F) -> Result<(), SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);

    /// Set anchor edges
    pub fn set_anchor(&mut self, anchor: Anchor);
    
    /// Set exclusive zone
    pub fn set_exclusive_zone(&mut self, zone: i32);
    
    /// Set margin
    pub fn set_margin(&mut self, top: i32, right: i32, bottom: i32, left: i32);
}
```

### Common Traits

```rust
/// Common trait for all surface types
pub trait Surface {
    /// Draw on the surface
    fn draw<F>(&mut self, draw_fn: F) where F: FnOnce(&skia_safe::Canvas);
    
    /// Get the underlying wl_surface
    fn wl_surface(&self) -> &wl_surface::WlSurface;
    
    /// Get dimensions
    fn dimensions(&self) -> (i32, i32);
}

/// Trait for surfaces that support sc_layer augmentation
pub trait ScLayerAugmentable {
    /// Apply sc_layer augmentation
    fn augment<F>(&mut self, augment_fn: F) -> Result<(), SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);
    
    /// Check if sc_layer is available
    fn has_sc_layer(&self) -> bool;
}
```

### Error Handling

```rust
#[derive(Debug)]
pub enum SurfaceError {
    CreationFailed,
    NotConfigured,
    SkiaError(String),
    WaylandError(String),
    ScLayerNotAvailable,
}
```

## Component Refactoring

### Window Component

**Before:**
```rust
pub struct SimpleWindow {
    width: i32,
    height: i32,
    title: String,
    background_color: Color,
    augment_fn: Option<Box<dyn Fn(&sc_layer_v1::ScLayerV1)>>,
}
```

**After:**
```rust
pub struct Window {
    surface: ToplevelSurface,
    background_color: Color,
    content_fn: Option<Box<dyn FnMut(&Canvas)>>,
}

impl Window {
    pub fn new<D>(
        title: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>;
    
    pub fn with_background(mut self, color: Color) -> Self;
    
    pub fn with_content<F>(mut self, content_fn: F) -> Self
    where F: FnMut(&Canvas) + 'static;
    
    pub fn with_sc_layer_augmentation<F>(mut self, augment_fn: F) -> Result<Self, SurfaceError>
    where F: FnOnce(&sc_layer_v1::ScLayerV1);
    
    pub fn render(&mut self);
}
```

### Menu Component

**Before:**
```rust
pub struct Menu {
    items: Vec<MenuItem>,
    style: MenuStyle,
    root: Option<MenuSurface>,  // MenuSurface manages surface internally
    // ... event handling state
}
```

**After:**
```rust
pub struct Menu {
    items: Vec<MenuItem>,
    style: MenuStyle,
    surface: Option<PopupSurface>,  // Use base PopupSurface
    // ... event handling state
}

impl Menu {
    pub fn open<D>(&mut self, 
        parent: &XdgSurface,
        position: Position,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
    ) -> Result<(), MenuError>;
    
    pub fn render(&mut self);
}
```

### MenuBar Component

**Before:**
```rust
pub struct MenuBarSurface {
    wl_surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    menu_bar: MenuBar,
    // ...
}
```

**After:**
```rust
pub struct MenuBar {
    items: Vec<MenuBarItem>,
    menus: HashMap<String, Menu>,
    // UI state only, no surface management
}

pub struct MenuBarView {
    surface: SubsurfaceSurface,  // Use base SubsurfaceSurface
    menu_bar: MenuBar,
}

impl MenuBarView {
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        menu_bar: MenuBar,
        width: i32,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>;
    
    pub fn render(&mut self);
}
```

### Layer Component

**Before:**
```rust
pub struct LayerSurface {
    wl_surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    skia_context: SkiaContext,
    skia_surface: SkiaSurface,
    layer: Layer,
    // ...
}
```

**After:**
```rust
pub struct Layer {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    draw_fn: Option<Box<dyn FnMut(&Canvas)>>,
}

pub struct LayerView {
    surface: SubsurfaceSurface,  // Use base SubsurfaceSurface
    layer: Layer,
}

impl LayerView {
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        layer: Layer,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
        display_ptr: *mut std::ffi::c_void,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
    ) -> Result<Self, SurfaceError>;
    
    pub fn render(&mut self);
}
```

## Benefits

1. **Code Reuse**: Surface creation logic centralized in base components
2. **Consistency**: All surfaces handle sc_layer augmentation uniformly
3. **Separation of Concerns**: Component logic separated from surface management
4. **Type Safety**: Each surface type has appropriate methods and guarantees
5. **Easier Testing**: Can mock surface types for testing component logic
6. **Future Extensibility**: Easy to add new surface types (e.g., drag surfaces)

## Migration Path

1. Create base surface types in `src/surfaces/` module
2. Add common traits and error types
3. Refactor Window component to use ToplevelSurface
4. Refactor Layer/MenuBar to use SubsurfaceSurface
5. Refactor Menu to use PopupSurface
6. Update examples to use new API
7. Add LayerShellSurface for future use

## File Structure

```
src/
  surfaces/
    mod.rs              # Re-exports
    common.rs           # Traits, errors
    toplevel.rs         # ToplevelSurface
    subsurface.rs       # SubsurfaceSurface  
    popup.rs            # PopupSurface
    layer_shell.rs      # LayerShellSurface (future)
  components/
    window/
      mod.rs            # Window using ToplevelSurface
    menu/
      mod.rs            # Menu component
      surface.rs        # Menu using PopupSurface
      data.rs
      drawing.rs
    menu_bar/
      mod.rs            # MenuBar component
      view.rs           # MenuBarView using SubsurfaceSurface
    layer/
      mod.rs            # Layer component
      view.rs           # LayerView using SubsurfaceSurface
```
