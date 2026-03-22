use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use std::sync::{Arc, Mutex, RwLock};

use crate::protocols::otto_surface_style_v1;
use crate::surfaces::{SubsurfaceSurface, SurfaceError, ToplevelSurface};
use crate::{
    app_runner::{App, AppContext},
    protocols::otto_surface_style_v1::BlendMode,
};

/// Layout configuration for ApplicationWindow
#[derive(Debug, Clone)]
pub struct WindowLayout {
    /// Height of the titlebar in logical pixels
    pub titlebar_height: i32,
    /// Width of the sidebar in logical pixels (0 = no sidebar)
    pub sidebar_width: i32,
    /// Sidebar position (true = left, false = right)
    pub sidebar_left: bool,
}

impl Default for WindowLayout {
    fn default() -> Self {
        Self {
            titlebar_height: 60,
            sidebar_width: 0,
            sidebar_left: true,
        }
    }
}

/// Application window component (simplified version)
///
/// Currently has a toplevel window with a titlebar subsurface.
/// Content and sidebar subsurfaces will be added incrementally.
#[derive(Clone)]
pub struct ApplicationWindow {
    surface: Arc<RwLock<Option<ToplevelSurface>>>,
    titlebar: Arc<RwLock<Option<SubsurfaceSurface>>>,
    sidebar: Arc<RwLock<Option<SubsurfaceSurface>>>,
    content: Arc<RwLock<Option<SubsurfaceSurface>>>,
    layout: Arc<RwLock<WindowLayout>>,
    initial_width: i32,
    initial_height: i32,
    background_color: skia_safe::Color,
    on_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
    titlebar_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
    sidebar_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
    content_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
}

impl ApplicationWindow {
    /// Create a new application window
    ///
    /// # Arguments
    /// * `title` - Window title
    /// * `width` - Total window width in logical pixels
    /// * `height` - Total window height in logical pixels
    /// * `layout` - Layout configuration (for future subsurfaces)
    pub fn new<A: App + 'static>(
        title: &str,
        width: i32,
        height: i32,
        layout: WindowLayout,
    ) -> Result<Self, SurfaceError> {
        // Create the toplevel surface
        let surface = ToplevelSurface::new(title, width, height)?;

        // Apply default layer styling to toplevel
        if let Some(layer) = surface.surface_style() {
            // layer.set_background_color(0.6, 0.6, 0.6, 0.0);
            // layer.set_blend_mode(BlendMode::BackgroundBlur);
            layer.set_border(2.0, 1.0, 1.0, 1.0, 0.6);
            layer.set_corner_radius(36.0);
            layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
        }

        let window = Self {
            surface: Arc::new(RwLock::new(Some(surface))),
            titlebar: Arc::new(RwLock::new(None)),
            sidebar: Arc::new(RwLock::new(None)),
            content: Arc::new(RwLock::new(None)),
            layout: Arc::new(RwLock::new(layout)),
            initial_width: width,
            initial_height: height,
            background_color: skia_safe::Color::from_rgb(200, 220, 255),
            on_draw_fn: Arc::new(Mutex::new(None)),
            titlebar_draw_fn: Arc::new(Mutex::new(None)),
            sidebar_draw_fn: Arc::new(Mutex::new(None)),
            content_draw_fn: Arc::new(Mutex::new(None)),
        };

        // Register configure handler
        let window_clone = window.clone();
        AppContext::register_configure_handler(move || {
            if let Some((surface_id, configure, serial)) = AppContext::current_surface_configure() {
                if let Some(our_surface) = window_clone.wl_surface() {
                    use wayland_client::Proxy;
                    if our_surface.id() == surface_id {
                        window_clone.on_configure::<A>(configure, serial);
                    }
                }
            }
        });

        Ok(window)
    }

    /// Set the background color for the window
    pub fn set_background(&mut self, color: impl Into<skia_safe::Color>) {
        self.background_color = color.into();
    }

    /// Set a custom drawing function for the window
    pub fn on_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.on_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Set a custom drawing function for the titlebar
    pub fn on_titlebar_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.titlebar_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Set a custom drawing function for the sidebar
    pub fn on_sidebar_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.sidebar_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Set a custom drawing function for the content area
    pub fn on_content_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.content_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Get direct access to the toplevel sc_layer
    pub fn layer(&self) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface.read().ok()?.as_ref()?.surface_style().cloned()
    }

    /// Update the window layout
    pub fn set_layout(&mut self, layout: WindowLayout) {
        *self.layout.write().unwrap() = layout;
        self.layout_subsurfaces();
        self.render();
    }

    /// Get the current layout
    pub fn layout(&self) -> WindowLayout {
        self.layout.read().unwrap().clone()
    }

    /// Create titlebar subsurface (called on first configure)
    fn create_titlebar<A: App + 'static>(&self) -> Result<(), SurfaceError> {
        let layout = self.layout.read().unwrap().clone();
        let width = self.initial_width;

        let wl_surface = self.wl_surface().ok_or_else(|| {
            SurfaceError::WaylandError("Toplevel surface not available".to_string())
        })?;

        // Calculate titlebar position and width to avoid sidebar overlap
        let (titlebar_x, titlebar_width) = if layout.sidebar_width > 0 && layout.sidebar_left {
            // Sidebar on left: titlebar starts after sidebar
            (0, width)
        } else if layout.sidebar_width > 0 && !layout.sidebar_left {
            // Sidebar on right: titlebar ends before sidebar
            (0, width - layout.sidebar_width)
        } else {
            // No sidebar: full width
            (0, width)
        };

        // Create titlebar subsurface at top
        let titlebar = SubsurfaceSurface::new(
            &wl_surface,
            titlebar_x,
            0,
            titlebar_width,
            layout.titlebar_height,
        )?;

        // Apply styling
        if let Some(layer) = titlebar.layer() {
            println!("Setting titlebar corner radius to 16.0");
            // layer.set_background_color(0.8, 0.8, 0.8, 1.0);
            // layer.set_border(2.0, 0.0, 0.0, 0.0, 0.6);
            // layer.set_corner_radius(16.0);
            layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
        } else {
            println!("WARNING: No layer available for titlebar!");
        }

        // Store titlebar
        *self.titlebar.write().unwrap() = Some(titlebar);

        Ok(())
    }

    /// Create sidebar subsurface (called on first configure)
    /// Sidebar is full height and positioned at left or right edge
    fn create_sidebar<A: App + 'static>(&self) -> Result<(), SurfaceError> {
        let layout = self.layout.read().unwrap().clone();

        // Only create sidebar if width > 0
        if layout.sidebar_width <= 0 {
            return Ok(());
        }

        let height = self.initial_height;

        let wl_surface = self.wl_surface().ok_or_else(|| {
            SurfaceError::WaylandError("Toplevel surface not available".to_string())
        })?;

        // Calculate sidebar position based on layout
        let x = if layout.sidebar_left {
            0
        } else {
            self.initial_width - layout.sidebar_width
        };

        // Create sidebar subsurface - FULL HEIGHT, starting at y=0
        let sidebar = SubsurfaceSurface::new(
            &wl_surface,
            x,
            0, // Full height, starts at top
            layout.sidebar_width,
            height, // Full window height
        )?;

        // Apply styling
        if let Some(layer) = sidebar.layer() {
            println!("Setting sidebar styling (full height)");
            layer.set_background_color(0.9, 0.9, 0.9, 0.9);
            // layer.set_corner_radius(36.0);
            // layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
            layer.set_blend_mode(BlendMode::BackgroundBlur);
        } else {
            println!("WARNING: No layer available for sidebar!");
        }

        // Store sidebar
        *self.sidebar.write().unwrap() = Some(sidebar);

        Ok(())
    }

    /// Create content subsurface (called on first configure)
    /// Content area has top margin to avoid titlebar overlap
    fn create_content<A: App + 'static>(&self) -> Result<(), SurfaceError> {
        let layout = self.layout.read().unwrap().clone();

        let height = self.initial_height;
        let width = self.initial_width;

        let wl_surface = self.wl_surface().ok_or_else(|| {
            SurfaceError::WaylandError("Toplevel surface not available".to_string())
        })?;

        // Calculate content position - overlaps sidebar by 40 points
        let overlap = 0;
        let x = if layout.sidebar_left {
            layout.sidebar_width - overlap
        } else {
            overlap
        };

        // Content width includes overlap with sidebar
        let content_width = width - layout.sidebar_width + overlap;

        // Create content subsurface - full height, no top margin
        let content = SubsurfaceSurface::new(
            &wl_surface,
            x,
            0, // Start at top
            content_width,
            height, // Full height
        )?;

        // Apply styling
        if let Some(layer) = content.layer() {
            println!("Setting content area styling");
            // layer.set_border(10.0, 1.0, 0.0, 0.0, 1.0);
            layer.set_background_color(0.8, 0.8, 0.8, 1.0);
            layer.set_shadow(0.3, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            // layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
        } else {
            println!("WARNING: No layer available for content!");
        }

        // Store content
        *self.content.write().unwrap() = Some(content);

        Ok(())
    }

    /// Layout subsurfaces based on current window dimensions
    fn layout_subsurfaces(&self) {
        let layout = self.layout.read().unwrap().clone();
        let (width, height) = self.dimensions();

        if width <= 0 || height <= 0 {
            return;
        }

        // Update titlebar
        if let Ok(mut titlebar_guard) = self.titlebar.write() {
            if let Some(ref mut titlebar) = *titlebar_guard {
                let (titlebar_x, titlebar_width) =
                    if layout.sidebar_width > 0 && layout.sidebar_left {
                        (0, width)
                    } else if layout.sidebar_width > 0 && !layout.sidebar_left {
                        (0, width - layout.sidebar_width)
                    } else {
                        (0, width)
                    };

                titlebar.resize(titlebar_width, layout.titlebar_height);
                titlebar.set_position(titlebar_x, 0);

                // Commit once after all changes
                titlebar.commit();
            }
        }

        // Update sidebar
        if let Ok(mut sidebar_guard) = self.sidebar.write() {
            if let Some(ref mut sidebar) = *sidebar_guard {
                if layout.sidebar_width > 0 {
                    let x = if layout.sidebar_left {
                        0
                    } else {
                        width - layout.sidebar_width
                    };

                    sidebar.resize(layout.sidebar_width, height);
                    sidebar.set_position(x, 0);

                    // Commit once after all changes
                    sidebar.commit();
                }
            }
        }

        // Update content
        if let Ok(mut content_guard) = self.content.write() {
            if let Some(ref mut content) = *content_guard {
                let overlap = 0;
                let x = if layout.sidebar_left {
                    layout.sidebar_width - overlap
                } else {
                    overlap
                };

                let content_width = width - layout.sidebar_width + overlap;

                content.resize(content_width, height);
                content.set_position(x, 0);

                // Commit once after all changes
                content.commit();
            }
        }
    }

    /// Internal: Handle window configure event
    fn on_configure<A: App + 'static>(&self, configure: WindowConfigure, serial: u32) {
        // Handle configure first - this updates the toplevel dimensions
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                let _ = surface.handle_configure(configure, serial);
            }
        }

        // Commit the toplevel surface after layer resize
        if let Some(wl_surface) = self.wl_surface() {
            wl_surface.commit();
        }

        // Create subsurfaces on first configure by checking if they exist
        // Order: sidebar, content, then titlebar (so titlebar overlaps)
        if self.sidebar.read().unwrap().is_none() {
            if let Err(e) = self.create_sidebar::<A>() {
                eprintln!("Failed to create sidebar: {:?}", e);
            }
        }

        if self.content.read().unwrap().is_none() {
            if let Err(e) = self.create_content::<A>() {
                eprintln!("Failed to create content: {:?}", e);
            }
        }

        if self.titlebar.read().unwrap().is_none() {
            if let Err(e) = self.create_titlebar::<A>() {
                eprintln!("Failed to create titlebar: {:?}", e);
            }
        }

        // Handle resize - update subsurface geometries and layer sizes
        self.layout_subsurfaces();

        // Commit the toplevel surface after layout to sync all subsurface changes
        if let Some(wl_surface) = self.wl_surface() {
            wl_surface.commit();
        }

        self.render();
    }

    /// Render the window
    pub fn render(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                if !surface.is_configured() {
                    return;
                }

                // Render main surface FIRST
                let on_draw_fn = self.on_draw_fn.clone();
                // let bg_color = self.background_color;

                surface.draw(|canvas| {
                    // canvas.clear(bg_color);

                    // Draw custom content if provided
                    if let Ok(mut draw_fn_guard) = on_draw_fn.lock() {
                        if let Some(ref mut content_fn) = *draw_fn_guard {
                            content_fn(canvas);
                        }
                    }
                });

                // Render titlebar if it exists
                if let Ok(titlebar_guard) = self.titlebar.read() {
                    if let Some(ref titlebar) = *titlebar_guard {
                        let titlebar_draw_fn = self.titlebar_draw_fn.clone();
                        titlebar.draw(|canvas| {
                            // Clear to orange
                            // canvas.clear(skia_safe::Color::from_rgb(255, 150, 80));

                            // Draw custom titlebar content if provided
                            if let Ok(mut draw_fn_guard) = titlebar_draw_fn.lock() {
                                if let Some(ref mut content_fn) = *draw_fn_guard {
                                    content_fn(canvas);
                                }
                            }
                        });
                    }
                }

                // Render sidebar if it exists
                if let Ok(sidebar_guard) = self.sidebar.read() {
                    if let Some(ref sidebar) = *sidebar_guard {
                        let sidebar_draw_fn = self.sidebar_draw_fn.clone();
                        sidebar.draw(|canvas| {
                            // Draw custom sidebar content if provided
                            if let Ok(mut draw_fn_guard) = sidebar_draw_fn.lock() {
                                if let Some(ref mut content_fn) = *draw_fn_guard {
                                    content_fn(canvas);
                                }
                            }
                        });
                    }
                }

                // Render content area if it exists
                if let Ok(content_guard) = self.content.read() {
                    if let Some(ref content) = *content_guard {
                        let content_draw_fn = self.content_draw_fn.clone();
                        content.draw(|canvas| {
                            // Draw custom content if provided
                            if let Ok(mut draw_fn_guard) = content_draw_fn.lock() {
                                if let Some(ref mut content_fn) = *draw_fn_guard {
                                    content_fn(canvas);
                                }
                            }
                        });
                    }
                }
            }
        }
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> Option<wayland_client::protocol::wl_surface::WlSurface> {
        let guard = self.surface.read().ok()?;
        guard.as_ref().map(|s| s.wl_surface().clone())
    }

    /// Get window dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.dimensions()))
            .unwrap_or((0, 0))
    }

    /// Check if the window is configured
    pub fn is_configured(&self) -> bool {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.is_configured()))
            .unwrap_or(false)
    }
}
