mod application_window;

use smithay_client_toolkit::seat::pointer::PointerEvent;
use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use std::sync::{Arc, Mutex, RwLock};
use wayland_client::protocol::wl_seat;

use crate::app_runner::AppContext;
pub use crate::protocols::otto_surface_style_v1;
use crate::surfaces::{SurfaceError, ToplevelSurface};

pub use application_window::{ApplicationWindow, WindowLayout};

/// Default layer augmentation - applies rounded corners
fn default_layer_augmentation(layer: &otto_surface_style_v1::OttoSurfaceStyleV1) {
    layer.set_corner_radius(16.0);
    layer.set_masks_to_bounds(otto_surface_style_v1::ClipMode::Enabled);
}

/// Window component using ToplevelSurface
///
/// This is a high-level window component that uses ToplevelSurface for
/// surface management while providing a simple API for window content.
///
/// By default, windows have rounded corners (12px radius). Use `on_layer()`
/// to customize or override the default layer augmentation.
///
/// Window uses the shared layers rendering engine from AppContext.
/// Assign a Layer node to this window to render it.
///
/// Window is Clone-able, allowing it to be shared across the application.
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Window {
    surface: Arc<RwLock<Option<ToplevelSurface>>>,
    background_color: Arc<RwLock<skia_safe::Color>>,
    title: Arc<RwLock<String>>,
    on_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
}

impl Window {
    /// Create a new window with ToplevelSurface
    ///
    /// Uses AppContext to access all required Wayland states.
    /// Automatically registers with AppRunner to handle configuration.
    /// Creates sc_layer immediately if available, with default rounded corners.
    pub fn new(title: &str, width: i32, height: i32) -> Result<Self, SurfaceError> {
        // Get all required states from AppContext

        let surface = ToplevelSurface::new(title, width, height)?;

        // Apply default layer styling immediately
        if let Some(surface_style) = surface.surface_style() {
            eprintln!("Applying corner radius to window surface style");
            default_layer_augmentation(surface_style);
        } else {
            eprintln!("Warning: No surface style available - window will not have rounded corners");
        }
        #[allow(clippy::arc_with_non_send_sync)]
        let window = Self {
            surface: Arc::new(RwLock::new(Some(surface))),
            background_color: Arc::new(RwLock::new(skia_safe::Color::from_rgb(245, 245, 245))),
            title: Arc::new(RwLock::new(title.to_string())),
            on_draw_fn: Arc::new(Mutex::new(None)),
        };

        // Auto-register configure handler now that Window is Clone
        let window_clone = window.clone();
        AppContext::register_configure_handler(move || {
            if let Some((_surface_id, configure, serial)) = AppContext::current_surface_configure()
            {
                // Just call configure for all configure events since we can't reliably match surface IDs
                // The window will check if it's configured and render
                window_clone.on_configure(configure, serial);
            }
        });

        // Register window for automatic updates
        AppContext::register_window(window.clone());

        Ok(window)
    }

    /// Set the background color
    pub fn with_background(self, color: impl Into<skia_safe::Color>) -> Self {
        if let Ok(mut bg_guard) = self.background_color.write() {
            *bg_guard = color.into();
        }
        self
    }

    /// Set the background color (mutable version)
    pub fn set_background(&mut self, color: impl Into<skia_safe::Color>) {
        if let Ok(mut bg_guard) = self.background_color.write() {
            *bg_guard = color.into();
        }
        // Request a frame to redraw with new background
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.request_frame();
            }
        }
    }

    /// Set a custom content drawing function
    pub fn with_on_draw<F>(self, draw_fn: F) -> Self
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.on_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
        self
    }

    /// Set a custom content drawing function (mutable version)
    pub fn on_draw<F>(&mut self, draw_fn: F)
    where
        F: FnMut(&skia_safe::Canvas) + Send + 'static,
    {
        *self.on_draw_fn.lock().unwrap() = Some(Box::new(draw_fn));
    }

    /// Assign a layer node to render in this window
    ///
    /// The layer and all its children will be rendered when the window draws.
    ///
    /// # Example
    /// ```no_run
    /// let layer = LayerFrame::new();
    /// layer.set_size(200.0, 100.0);
    /// window.set_layer_node(layer.layer().clone());
    /// ```
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                surface.set_layer_node(layer);
            }
        }
    }

    /// Get the layer node assigned to this window
    pub fn layer_node(&self) -> Option<layers::prelude::Layer> {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                return surface.layer_node().cloned();
            }
        }
        None
    }

    /// Get direct access to the surface style for configuration
    ///
    /// Returns None if surface style was not available when the window was created.
    ///
    /// # Example
    /// ```no_run
    /// if let Some(surface_style) = window.surface_style() {
    ///     surface_style.set_corner_radius(24.0);
    ///     surface_style.set_opacity(0.9);
    /// }
    /// ```
    pub fn surface_style(&self) -> Option<otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface.read().ok()?.as_ref()?.surface_style().cloned()
    }

    /// Internal: Handle window configure event
    fn on_configure(&self, configure: WindowConfigure, serial: u32) {
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                let _ = surface.handle_configure(configure, serial);
            }
        }
        self.render();
    }

    /// Render the window content
    fn render_with<F>(&self, render_extra: F)
    where
        F: FnOnce(),
    {
        #[allow(clippy::single_match)]
        match self.surface.read() {
            Ok(surface_guard) => {
                if let Some(ref surface) = *surface_guard {
                    if !surface.is_configured() {
                        return;
                    }

                    let on_draw_fn = self.on_draw_fn.clone();
                    let background_color = self
                        .background_color
                        .read()
                        .ok()
                        .map(|c| *c)
                        .unwrap_or(skia_safe::Color::WHITE);

                    surface.draw(|canvas| {
                        canvas.clear(background_color);

                        // Draw custom content on top if provided
                        if let Ok(mut draw_fn_guard) = on_draw_fn.lock() {
                            if let Some(ref mut content_fn) = *draw_fn_guard {
                                content_fn(canvas);
                            }
                        }
                    });

                    // Render extra content (e.g., subsurfaces)
                    render_extra();
                }
            }
            Err(_) => {}
        }
    }

    /// Render the window content
    fn render(&self) {
        self.render_with(|| {});
    }

    /// Update the window - render if dirty
    pub(crate) fn update(&self) {
        if let Some(surface) = self.surface() {
            if surface.is_dirty() {
                self.render();
                surface.clear_dirty();
            }
        }
    }

    /// Get the underlying ToplevelSurface
    pub fn surface(&self) -> Option<ToplevelSurface> {
        self.surface.read().ok()?.clone()
    }

    /// Check if the window is configured
    pub fn is_configured(&self) -> bool {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.is_configured()))
            .unwrap_or(false)
    }

    /// Get window dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        self.surface
            .read()
            .ok()
            .and_then(|s| s.as_ref().map(|surf| surf.dimensions()))
            .unwrap_or((0, 0))
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> Option<wayland_client::protocol::wl_surface::WlSurface> {
        let guard = self.surface.read().ok()?;
        guard.as_ref().map(|s| s.wl_surface().clone())
    }

    /// Register a pointer event handler for this window
    /// The callback receives all pointer events when they occur
    ///
    /// # Example
    /// ```no_run
    /// window.on_pointer_event(|events| {
    ///     for event in events {
    ///         match &event.kind {
    ///             PointerEventKind::Press { button, serial, .. } => {
    ///                 // Handle button press
    ///             }
    ///             _ => {}
    ///         }
    ///     }
    /// });
    /// ```
    pub fn on_pointer_event<F>(&self, mut callback: F)
    where
        F: FnMut(&[PointerEvent]) + 'static,
    {
        // Clone the window Arc to check surface on each event
        let window_clone = self.clone();

        AppContext::register_pointer_callback(move |events| {
            // Get our surface ID dynamically each time
            if let Some(our_wl_surface) = window_clone.wl_surface() {
                use wayland_client::Proxy;
                // Filter to only events for our surface
                let our_events: Vec<&PointerEvent> = events
                    .iter()
                    .filter(|e| e.surface.id() == our_wl_surface.id())
                    .collect();

                if !our_events.is_empty() {
                    // eprintln!("Window got {} pointer events", our_events.len());
                    let borrowed_events: Vec<PointerEvent> =
                        our_events.iter().map(|&e| e.clone()).collect();
                    callback(&borrowed_events);
                }
            }
        });
    }

    /// Start an interactive window move
    /// Call this in response to a pointer button press to make the window draggable
    ///
    /// # Arguments
    /// * `seat` - The seat that initiated the move
    /// * `serial` - The serial from the pointer button press event
    ///
    /// # Example
    /// ```no_run
    /// window.on_pointer_event(|events| {
    ///     for event in events {
    ///         if let PointerEventKind::Press { serial, .. } = event.kind {
    ///             // Start window move when pressed
    ///             window.start_move(seat, serial);
    ///         }
    ///     }
    /// });
    /// ```
    pub fn start_move(&self, seat: &wl_seat::WlSeat, serial: u32) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.xdg_window().move_(seat, serial);
            }
        }
    }
    pub fn request_frame(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                surface.request_frame();
            }
        }
    }
    pub fn title(&self) -> String {
        self.title
            .read()
            .ok()
            .map(|t| t.clone())
            .unwrap_or_default()
    }
    pub fn set_title(&mut self, title: &str) {
        if let Ok(mut title_guard) = self.title.write() {
            *title_guard = title.to_string();
        }
    }
}
