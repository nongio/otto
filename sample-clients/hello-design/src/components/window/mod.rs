use smithay_client_toolkit::{reexports::client::QueueHandle, shell::xdg::window::WindowConfigure};
use std::sync::{Arc, Mutex, RwLock};
use wayland_client::Dispatch;

// Import from the surfaces module (sibling to components in src/)
use super::super::app_runner::{App, AppContext};
use super::super::surfaces::{Surface, SurfaceError, ToplevelSurface};
use crate::ScLayerAugment;
// Re-export sc-layer protocol for convenience
pub use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};

/// Default layer augmentation - applies rounded corners
fn default_layer_augmentation(layer: &sc_layer_v1::ScLayerV1) {
    layer.set_corner_radius(12.0);
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
    on_layer_fn: Arc<Mutex<Option<Box<dyn Fn(&sc_layer_v1::ScLayerV1) + Send>>>>,
}

impl Window {
    /// Create a new window with ToplevelSurface
    ///
    /// Uses AppContext to access all required Wayland states.
    /// Automatically registers with AppRunner to handle configuration.
    /// By default, applies rounded corners to the window.
    pub fn new<A: App + 'static>(
        title: &str,
        width: i32,
        height: i32,
    ) -> Result<Self, SurfaceError> {
        // Get all required states from AppContext
        let compositor = AppContext::compositor_state();
        let xdg_shell = AppContext::xdg_shell_state();
        let qh = AppContext::queue_handle::<A>();

        let surface = ToplevelSurface::new(title, width, height, compositor, xdg_shell, qh)?;

        let window = Self {
            surface: Arc::new(RwLock::new(Some(surface))),
            background_color: skia_safe::Color::from_rgb(245, 245, 245),
            on_draw_fn: Arc::new(Mutex::new(None)),
            on_layer_fn: Arc::new(Mutex::new(Some(Box::new(default_layer_augmentation)))),
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
    pub fn with_background(mut self, color: skia_safe::Color) -> Self {
        self.background_color = color;
        self
    }

    /// Set the background color (mutable version)
    pub fn set_background(&mut self, color: skia_safe::Color) {
        self.background_color = color;
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

    /// Set sc_layer augmentation function to apply on configure
    pub fn with_layer_fn<F>(self, layer_fn: F) -> Self
    where
        F: Fn(&sc_layer_v1::ScLayerV1) + Send + 'static,
    {
        *self.on_layer_fn.lock().unwrap() = Some(Box::new(layer_fn));
        self
    }

    /// Set sc_layer augmentation function (mutable version)
    pub fn on_layer<F>(&mut self, layer_fn: F)
    where
        F: Fn(&sc_layer_v1::ScLayerV1) + Send + 'static,
    {
        *self.on_layer_fn.lock().unwrap() = Some(Box::new(layer_fn));
    }

    /// Internal: Handle window configure event
    fn on_configure<A: App + 'static>(&self, configure: WindowConfigure, serial: u32) {
        println!("Window received configure event");
        if let Ok(mut surface_guard) = self.surface.write() {
            if let Some(ref mut surface) = *surface_guard {
                let _ = surface.handle_configure(configure, serial);
                // Apply sc_layer augmentation if available and configured
                if surface.is_configured() && surface.has_sc_layer() {
                    println!("Applying sc_layer augmentation on configure");
                    if let Ok(layer_fn_guard) = self.on_layer_fn.lock() {
                        if let Some(ref augment_fn) = *layer_fn_guard {
                            let qh = AppContext::queue_handle::<A>();
                            surface.augment_with_qh(augment_fn, qh);
                        }
                    }
                }
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
                    // Clear with background color
                    if self.background_color != skia_safe::Color::from_argb(0, 0, 0, 0) {
                        canvas.clear(self.background_color);
                    }

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
            use super::super::surfaces::Surface;
            s.wl_surface().clone()
        })
    }
}

/// Legacy SimpleWindow type alias for backwards compatibility
///
/// This allows existing code using SimpleWindow to continue working
/// while we migrate to the new Window component.
pub struct SimpleWindow {
    width: i32,
    height: i32,
    title: String,
    background_color: skia_safe::Color,
    augment_fn: Option<Box<dyn Fn(&sc_layer_v1::ScLayerV1)>>,
}

impl SimpleWindow {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            title: "Simple Window".to_string(),
            background_color: skia_safe::Color::from_rgb(245, 245, 245),
            augment_fn: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_background(mut self, color: skia_safe::Color) -> Self {
        self.background_color = color;
        self
    }

    pub fn with_augment_fn<F>(mut self, augment_fn: F) -> Self
    where
        F: Fn(&sc_layer_v1::ScLayerV1) + 'static,
    {
        self.augment_fn = Some(Box::new(augment_fn));
        self
    }

    pub fn set_augment_fn<F>(&mut self, augment_fn: F)
    where
        F: Fn(&sc_layer_v1::ScLayerV1) + 'static,
    {
        self.augment_fn = Some(Box::new(augment_fn));
    }

    pub fn augment_fn(&self) -> Option<&Box<dyn Fn(&sc_layer_v1::ScLayerV1)>> {
        self.augment_fn.as_ref()
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn render(&self, canvas: &skia_safe::Canvas) {
        canvas.clear(self.background_color);

        // Draw centered text using typography system
        let font = crate::typography::styles::BODY.font();
        let paint = skia_safe::Paint::new(skia_safe::Color4f::new(0.2, 0.2, 0.2, 1.0), None);

        let text = "Simple Window Component";
        let (_, bounds) = font.measure_str(text, Some(&paint));

        let x = (self.width as f32 - bounds.width()) / 2.0;
        let y = (self.height as f32) / 2.0;

        canvas.draw_str(text, (x, y), &font, &paint);
    }
}

impl Default for SimpleWindow {
    fn default() -> Self {
        Self::new(400, 300)
    }
}
