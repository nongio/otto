use std::fmt;
use wayland_client::{protocol::wl_surface, Dispatch, QueueHandle};

// Re-export sc-layer protocol from menu component for convenience
pub use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};

use crate::rendering::SkiaSurface;

/// Core surface fields shared by all surface types
/// 
/// This struct contains the common fields and methods used by
/// ToplevelSurface, SubsurfaceSurface, and PopupSurface.
#[derive(Clone)]
pub struct SurfaceCore {
    pub(super) wl_surface: wl_surface::WlSurface,
    pub(super) skia_surface: Option<SkiaSurface>,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) buffer_scale: i32,
    pub(super) sc_layer: Option<sc_layer_v1::ScLayerV1>,
    pub(super) layer_node: Option<layers::prelude::Layer>,
}

impl SurfaceCore {
    /// Create a new surface core
    pub fn new(
        wl_surface: wl_surface::WlSurface,
        width: i32,
        height: i32,
        buffer_scale: i32,
    ) -> Self {
        Self {
            wl_surface,
            skia_surface: None,
            width,
            height,
            buffer_scale,
            sc_layer: None,
            layer_node: None,
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
            self.skia_surface = Some(result.map_err(|e| SurfaceError::SkiaError(e.to_string()))?);
        } else {
            // No shared context yet - create it with this first surface
            let (new_ctx, new_surface) = SkiaContext::new(
                AppContext::display_ptr(),
                &self.wl_surface,
                self.width * self.buffer_scale,
                self.height * self.buffer_scale,
            )
            .map_err(|e| SurfaceError::SkiaError(e.to_string()))?;

            AppContext::set_skia_context(new_ctx);
            self.skia_surface = Some(new_surface);
        }
        
        Ok(())
    }

    /// Resize the surface and recreate Skia surface
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        let _ = self.create_skia_surface(); // Ignore errors on resize
    }

    /// Get the Wayland surface
    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        &self.wl_surface
    }

    /// Get the sc_layer if available
    pub fn sc_layer(&self) -> Option<&sc_layer_v1::ScLayerV1> {
        self.sc_layer.as_ref()
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
    ScLayerNotAvailable,
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
            SurfaceError::ScLayerNotAvailable => write!(f, "sc_layer protocol not available"),
            SurfaceError::ResizeFailed => write!(f, "Failed to resize surface"),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// Trait for surfaces with Skia rendering support
/// 
/// Provides default implementation for drawing using SkiaSurface.
/// Surfaces only need to implement `skia_surface()` to get full drawing support.
/// 
/// Also supports rendering layers from the layers engine.
pub trait SkiaBackedSurface {
    /// Get reference to the SkiaSurface
    fn skia_surface(&self) -> Option<&SkiaSurface>;
    
    /// Get reference to the layer node to render (if any)
    fn layer_node(&self) -> Option<layers::prelude::Layer> {
        None // Default: no layer
    }
    
    /// Check if surface should allow drawing (e.g., is configured)
    fn can_draw(&self) -> bool {
        true // Default: always allow drawing
    }
    
    /// Draw on the surface using the shared Skia context
    /// 
    /// The draw_fn callback is called BEFORE layers are rendered,
    /// allowing you to clear the background or draw content underneath.
    fn draw_skia<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if !self.can_draw() {
            return;
        }
        if let Some(surface) = self.skia_surface() {
            AppContext::skia_context(|ctx| {
                surface.draw(ctx, |canvas| {
                    // Call custom draw function FIRST (e.g., to clear background)
                    draw_fn(canvas);

                    if let Some(engine) = AppContext::layers_engine() {
                    // Update engine animations/layout once per frame
                        if let Some(layer) = self.layer_node() {
                            let needs_redraw = AppContext::layers_renderer(|renderer| {
                                renderer.update()
                            });
                            let needs_redraw = needs_redraw.unwrap_or(false);

                            // Render the assigned layer node from the shared engine

                            if !needs_redraw {
                                // return;
                            }
                            layers::prelude::draw_scene(canvas, engine.scene(), layer.id());
                        }
                    }
                });
                surface.swap_buffers(ctx);
                surface.commit();
            });
        }
    }
}

/// Common trait for all surface types
pub trait Surface {
    /// Draw on the surface using a callback
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas);

    /// Get the underlying wl_surface
    fn wl_surface(&self) -> &wl_surface::WlSurface;

    /// Get dimensions (width, height) in logical pixels
    fn dimensions(&self) -> (i32, i32);
}

/// Trait for surfaces that support sc_layer augmentation
pub trait ScLayerAugment: Surface {
    /// Check if sc_layer is available for this surface
    fn has_sc_layer(&self) -> bool;

    /// Get mutable access to the sc_layer storage
    fn sc_layer_mut(&mut self) -> Option<&mut Option<sc_layer_v1::ScLayerV1>>;

    /// Get reference to the sc_layer_shell
    fn sc_layer_shell(&self) -> Option<&sc_layer_shell_v1::ScLayerShellV1>;

    /// Check if surface is configured
    fn is_configured(&self) -> bool;

    /// Apply sc_layer augmentation with queue handle
    ///
    /// This version can be called from the configure handler where
    /// we have access to the queue handle. It automatically applies
    /// default menu styling and then calls the optional augment_fn
    /// for additional customization.
    fn augment<F, D>(
        &mut self,
        augment_fn: Option<F>,
        qh: &QueueHandle<D>,
    ) -> Result<(), SurfaceError>
    where
        F: FnOnce(&sc_layer_v1::ScLayerV1),
        D: Dispatch<sc_layer_v1::ScLayerV1, ()> + 'static,
    {
        if !self.is_configured() {
            return Err(SurfaceError::NotConfigured);
        }

        // Clone the sc_layer_shell to avoid borrow conflicts
        let sc_layer_shell = self
            .sc_layer_shell()
            .ok_or(SurfaceError::ScLayerNotAvailable)?
            .clone();

        // Get wl_surface before mutable borrow
        let wl_surface = self.wl_surface().clone();

        let sc_layer = self
            .sc_layer_mut()
            .ok_or(SurfaceError::ScLayerNotAvailable)?;

        augment_surface_with_sc_layer(&wl_surface, &sc_layer_shell, sc_layer, augment_fn, qh);

        Ok(())
    }
}

/// Create or get an sc_layer for a surface and apply styling
///
/// This is a generic helper that can be used by any surface type to apply
/// sc_layer augmentation. It will create the sc_layer if it doesn't exist,
/// apply default menu styling, and optionally call a custom augment function.
///
/// # Arguments
/// * `wl_surface` - The Wayland surface to augment
/// * `sc_layer_shell` - The sc_layer_shell protocol object
/// * `sc_layer` - Mutable reference to store the created sc_layer
/// * `augment_fn` - Optional custom augmentation function
/// * `qh` - Queue handle for creating objects
pub fn augment_surface_with_sc_layer<F, D>(
    wl_surface: &wl_surface::WlSurface,
    sc_layer_shell: &sc_layer_shell_v1::ScLayerShellV1,
    sc_layer: &mut Option<sc_layer_v1::ScLayerV1>,
    augment_fn: Option<F>,
    qh: &QueueHandle<D>,
) where
    F: FnOnce(&sc_layer_v1::ScLayerV1),
    D: Dispatch<sc_layer_v1::ScLayerV1, ()> + 'static,
{
    // Create sc_layer if not exists
    if sc_layer.is_none() {
        let layer = sc_layer_shell.get_layer(wl_surface, qh, ());
        *sc_layer = Some(layer);
    }

    if let Some(ref layer) = sc_layer {
        // Allow caller to override or add more properties
        if let Some(f) = augment_fn {
            f(layer);
        }
    }
}
