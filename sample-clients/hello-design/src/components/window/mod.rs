use smithay_client_toolkit::shell::xdg::window::WindowConfigure;
use std::sync::{Arc, Mutex, RwLock};

use crate::app_runner::{App, AppContext};
pub use crate::components::menu::sc_layer_v1;
use crate::surfaces::{Surface, SurfaceError, ToplevelSurface};
use crate::ScLayerAugment;

/// Default layer augmentation - applies rounded corners
fn default_layer_augmentation(layer: &sc_layer_v1::ScLayerV1) {
    layer.set_corner_radius(24.0);
    layer.set_masks_to_bounds(1);
}

/// Window component using ToplevelSurface
///
/// This is a high-level window component that uses ToplevelSurface for
/// surface management while providing a simple API for window content.
///
/// By default, windows have rounded corners (12px radius). Use `on_layer()`
/// to customize or override the default layer augmentation.
///
/// Window is Clone-able, allowing it to be shared across the application.
#[derive(Clone)]
pub struct Window {
    surface: Arc<RwLock<Option<ToplevelSurface>>>,
    background_color: skia_safe::Color,
    on_draw_fn: Arc<Mutex<Option<Box<dyn FnMut(&skia_safe::Canvas) + Send>>>>,
}

impl Window {
    /// Create a new window with ToplevelSurface
    ///
    /// Uses AppContext to access all required Wayland states.
    /// Automatically registers with AppRunner to handle configuration.
    /// Creates sc_layer immediately if available, with default rounded corners.
    pub fn new<A: App + 'static>(
        title: &str,
        width: i32,
        height: i32,
    ) -> Result<Self, SurfaceError> {
        // Get all required states from AppContext
        let compositor = AppContext::compositor_state();
        let xdg_shell = AppContext::xdg_shell_state();
        let sc_layer_shell = AppContext::sc_layer_shell();
        let qh = AppContext::queue_handle::<A>();

        let surface = ToplevelSurface::new(
            title,
            width,
            height,
            compositor,
            xdg_shell,
            sc_layer_shell,
            qh,
        )?;

        // Apply default layer styling immediately
        if let Some(layer) = surface.layer() {
            default_layer_augmentation(layer);
        }

        let window = Self {
            surface: Arc::new(RwLock::new(Some(surface))),
            background_color: skia_safe::Color::from_rgb(245, 245, 245),
            on_draw_fn: Arc::new(Mutex::new(None)),
        };

        // Auto-register configure handler now that Window is Clone
        let window_clone = window.clone();
        AppContext::register_configure_handler(move || {
            if let Some((surface_id, configure, serial)) = AppContext::current_surface_configure() {
                // Check if this configure is for our window's surface
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

    /// Set the background color
    pub fn with_background(mut self, color: impl Into<skia_safe::Color>) -> Self {
        self.background_color = color.into();
        self
    }

    /// Set the background color (mutable version)
    pub fn set_background(&mut self, color: impl Into<skia_safe::Color>) {
        self.background_color = color.into();
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

    /// Get direct access to the sc_layer for configuration
    ///
    /// Returns None if sc_layer_shell was not available when the window was created.
    ///
    /// # Example
    /// ```no_run
    /// if let Some(layer) = window.layer() {
    ///     layer.set_corner_radius(24.0);
    ///     layer.set_opacity(0.9);
    /// }
    /// ```
    pub fn layer(&self) -> Option<sc_layer_v1::ScLayerV1> {
        self.surface.read().ok()?.as_ref()?.layer().cloned()
    }

    /// Internal: Handle window configure event
    fn on_configure<A: App + 'static>(&self, configure: WindowConfigure, serial: u32) {
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                let _ = surface.handle_configure(configure, serial);
            }
        }
        self.render();
    }

    /// Render the window content
    pub fn render(&self) {
        if let Ok(surface_guard) = self.surface.read() {
            if let Some(ref surface) = *surface_guard {
                if !surface.is_configured() {
                    return;
                }

                let on_draw_fn = self.on_draw_fn.clone();

                surface.draw(|canvas| {
                    canvas.clear(self.background_color);

                    // Draw custom content if provided
                    if let Ok(mut draw_fn_guard) = on_draw_fn.lock() {
                        if let Some(ref mut content_fn) = *draw_fn_guard {
                            content_fn(canvas);
                        }
                    }
                });
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
        guard.as_ref().map(|s| {
            use crate::surfaces::Surface;
            s.wl_surface().clone()
        })
    }
}
