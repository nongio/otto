use layers::types::Size;
use std::fmt;
use std::rc::Rc;
use wayland_client::{protocol::wl_surface, Proxy};

// Re-export sc-layer protocol for convenience
pub use crate::protocols::{otto_surface_style_manager_v1, otto_surface_style_v1};

use crate::{rendering::SkiaSurface, AppContext};

/// Core surface with all rendering functionality built-in
///
/// This struct contains the common fields and methods used by
/// ToplevelSurface, SubsurfaceSurface, and PopupSurface.
/// All surface operations are implemented directly on this type.
#[derive(Clone)]
pub struct BaseWaylandSurface {
    // The underlying Wayland surface
    pub(super) wl_surface: wl_surface::WlSurface,
    // Skia surface for rendering
    pub(super) skia_surface: Option<Rc<SkiaSurface>>,
    // Surface style for animation and blending
    pub(super) surface_style: Option<otto_surface_style_v1::OttoSurfaceStyleV1>,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) buffer_scale: i32,
    // not sure this is needed
    pub(super) dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // Optional layer node for layers engine rendering
    pub(super) layer_node: Option<layers::prelude::Layer>,
}

impl BaseWaylandSurface {
    /// Create a new surface core
    pub fn new(
        wl_surface: wl_surface::WlSurface,
        width: i32,
        height: i32,
        buffer_scale: i32,
    ) -> Self {
        let layer_node = AppContext::layers_engine().and_then(|engine| {
            let l = engine.new_layer();
            l.set_size(Size::points(width as f32, height as f32), None);
            engine.add_layer(&l);
            Some(l)
        });
        let surface_style = AppContext::surface_style_manager().and_then(|manager| {
            Some(manager.get_surface_style(&wl_surface, AppContext::queue_handle(), ()))
        });
        Self {
            wl_surface,
            skia_surface: None,
            width,
            height,
            buffer_scale,
            surface_style,
            layer_node,
            dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Create the Skia surface using shared context
    pub fn create_skia_surface(&mut self) -> Result<(), SurfaceError> {
        use crate::app_runner::AppContext;
        use crate::rendering::SkiaContext;

        let surface = AppContext::skia_context(|ctx| {
            ctx.create_surface(
                &self.wl_surface,
                self.width * self.buffer_scale,
                self.height * self.buffer_scale,
            )
        });

        if let Some(result) = surface {
            // Shared context exists, use it
            self.skia_surface = Some(Rc::new(
                result.map_err(|e| SurfaceError::SkiaError(e.to_string()))?,
            ));
        } else {
            // No shared context yet - create it with this first surface
            println!(
                "No shared context yet , Creating new Skia context for surface {}",
                self.wl_surface.id()
            );
            let (new_ctx, new_surface) = SkiaContext::new(
                AppContext::display_ptr(),
                &self.wl_surface,
                self.width * self.buffer_scale,
                self.height * self.buffer_scale,
            )
            .map_err(|e| SurfaceError::SkiaError(e.to_string()))?;

            AppContext::set_skia_context(new_ctx);
            self.skia_surface = Some(Rc::new(new_surface));
        }

        Ok(())
    }

    /// Resize the surface and recreate Skia surface
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;

        // Only recreate Skia surface if one already exists
        if self.skia_surface.is_some() {
            let res = self.create_skia_surface();
            if let Err(e) = res {
                eprintln!("Error resizing surface {}: {}", self.wl_surface.id(), e);
            }
        }

        // Update layer node size if using layers engine
        if let Some(layer) = &self.layer_node {
            layer.set_size(
                layers::types::Size::points(width as f32, height as f32),
                None,
            );
            layer.engine.update(0.0);
        }
    }

    /// Get the Wayland surface
    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        &self.wl_surface
    }

    /// Get the surface style if available
    pub fn surface_style(&self) -> Option<&otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.surface_style.as_ref()
    }

    /// Set the layer node for layers engine rendering
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        self.layer_node = Some(layer);
    }

    /// Get the layer node
    pub fn layer_node(&self) -> Option<&layers::prelude::Layer> {
        self.layer_node.as_ref()
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Get reference to the SkiaSurface
    pub fn skia_surface(&self) -> Option<&Rc<SkiaSurface>> {
        self.skia_surface.as_ref()
    }

    /// Check if surface is configured and ready to draw
    ///
    /// Can be overridden by specific surface types if needed
    pub fn is_surface_configured(&self) -> bool {
        true
    }

    /// Check if surface should allow drawing
    pub fn can_draw(&self) -> bool {
        self.is_surface_configured()
    }

    /// Draw on the surface using the shared Skia context
    ///
    /// The draw_fn callback is called for custom rendering.
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if !self.can_draw() {
            return;
        }

        let Some(surface) = self.skia_surface.as_ref() else {
            return;
        };

        AppContext::skia_context(|ctx| {
            // Draw custom content
            surface.draw(ctx, |canvas| {
                draw_fn(canvas);
            });

            // Present the frame
            surface.swap_buffers(ctx);
            surface.commit();
        });
    }

    /// Render the layer node if one is assigned
    ///
    /// This can be called to render layers content onto the surface canvas.
    pub fn render_layer_node(&self, canvas: &skia_safe::Canvas) {
        use crate::app_runner::AppContext;

        let Some(engine) = AppContext::layers_engine() else {
            return;
        };

        let Some(layer) = &self.layer_node else {
            return;
        };

        // Render the assigned layer node from the shared engine
        layers::prelude::draw_scene(canvas, engine.scene(), layer.id());
    }

    /// Register a callback to be called on every compositor frame
    ///
    /// This hooks into the Wayland frame callback mechanism, ensuring
    /// your callback is synchronized with the compositor's refresh rate.
    ///
    /// # Example
    /// ```ignore
    /// surface.on_frame(|| {
    ///     println!("Frame rendered!");
    ///     // Render your content here
    /// });
    /// ```
    pub fn on_frame<F>(&self, callback: F)
    where
        F: FnMut() + 'static,
    {
        use crate::app_runner::AppContext;
        use wayland_client::Proxy;

        let surface_id = self.wl_surface.id();
        AppContext::register_frame_callback(surface_id, callback);

        // Request the initial frame callback to start the loop
        AppContext::request_initial_frame(&self.wl_surface);
    }

    /// Check if surface style protocol is available for this surface
    pub fn has_surface_style(&self) -> bool {
        self.surface_style.is_some()
    }

    /// Get reference to the surface style manager
    pub fn surface_style_manager(
        &self,
    ) -> Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1> {
        use crate::app_runner::AppContext;
        AppContext::surface_style_manager()
    }
}

/// Error type for surface operations
#[derive(Debug)]
pub enum SurfaceError {
    /// Failed to create the surface
    CreationFailed,
    /// Surface not yet configured by compositor
    NotConfigured,
    /// Skia rendering error
    SkiaError(String),
    /// Wayland protocol error
    WaylandError(String),
    /// sc_layer protocol not available
    SceneSurface,
    /// Failed to resize surface
    ResizeFailed,
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurfaceError::CreationFailed => write!(f, "Failed to create surface"),
            SurfaceError::NotConfigured => write!(f, "Surface not yet configured"),
            SurfaceError::SkiaError(e) => write!(f, "Skia error: {}", e),
            SurfaceError::WaylandError(e) => write!(f, "Wayland error: {}", e),
            SurfaceError::SceneSurface => write!(f, "scene_surface protocol not available"),
            SurfaceError::ResizeFailed => write!(f, "Failed to resize surface"),
        }
    }
}

impl std::error::Error for SurfaceError {}
