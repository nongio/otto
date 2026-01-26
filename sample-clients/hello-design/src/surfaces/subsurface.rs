use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
};
use wayland_client::{
    protocol::{wl_subcompositor, wl_subsurface},
    Dispatch,
};

use super::common::{sc_layer_shell_v1, sc_layer_v1, ScLayerAugment, Surface, SurfaceError};
use crate::rendering::SkiaSurface;

/// Manages a Wayland subsurface with Skia rendering
///
/// This surface type represents a child surface positioned relative to a parent.
/// It's useful for elements like menubars, decorations, or overlays that need
/// to be part of a window but managed separately.
pub struct SubsurfaceSurface {
    wl_surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    skia_surface: Option<SkiaSurface>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    buffer_scale: i32,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl SubsurfaceSurface {
    /// Create a new subsurface
    ///
    /// # Arguments
    /// * `parent_surface` - The parent Wayland surface
    /// * `x` - X position relative to parent in logical pixels
    /// * `y` - Y position relative to parent in logical pixels
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    /// * `compositor` - Compositor state
    /// * `subcompositor` - Subcompositor global
    /// * `qh` - Queue handle for creating objects
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<wl_subsurface::WlSubsurface, ()>
            + Dispatch<sc_layer_v1::ScLayerV1, ()>
            + Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + 'static,
    {
        // Create the Wayland surface
        let wl_surface = compositor.create_surface(qh);

        // Create subsurface
        let subsurface = subcompositor.get_subsurface(&wl_surface, parent_surface, qh, ());

        // Position the subsurface
        subsurface.set_position(x, y);
        subsurface.set_desync();

        // Use 2x buffer for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // Create Skia surface using shared context
        use crate::app_runner::AppContext;
        let skia_surface = AppContext::skia_context(|ctx| {
            ctx.create_surface(&wl_surface, width * buffer_scale, height * buffer_scale)
        })
        .ok_or(SurfaceError::SkiaError(
            "SkiaContext not initialized".to_string(),
        ))?
        .map_err(|e| SurfaceError::SkiaError(e))?;

        let mut subsurface_surface = Self {
            wl_surface: wl_surface.clone(),
            subsurface,
            skia_surface: Some(skia_surface),
            width,
            height,
            x,
            y,
            buffer_scale,
            sc_layer: None,
        };

        // Commit the surface
        wl_surface.commit();

        // Apply sc_layer augmentation if available
        if let Some(sc_layer_shell) = AppContext::sc_layer_shell() {
            let layer = sc_layer_shell.get_layer(&wl_surface, qh, ());
            subsurface_surface.sc_layer = Some(layer);
        }

        Ok(subsurface_surface)
    }

    /// Update the position of the subsurface
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.subsurface.set_position(x, y);
    }

    /// Get current position
    pub fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// Resize the subsurface
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), SurfaceError> {
        use crate::app_runner::AppContext;

        if self.width == width && self.height == height {
            return Ok(());
        }

        self.width = width;
        self.height = height;

        // Recreate Skia surface with new dimensions using shared context
        let new_surface = AppContext::skia_context(|ctx| {
            ctx.create_surface(
                &self.wl_surface,
                width * self.buffer_scale,
                height * self.buffer_scale,
            )
        })
        .ok_or(SurfaceError::SkiaError(
            "SkiaContext not initialized".to_string(),
        ))?
        .map_err(|e| SurfaceError::SkiaError(e))?;

        self.skia_surface = Some(new_surface);

        Ok(())
    }

    /// Get the subsurface object
    pub fn subsurface(&self) -> &wl_subsurface::WlSubsurface {
        &self.subsurface
    }
}

impl Surface for SubsurfaceSurface {
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if let Some(surface) = &self.skia_surface {
            AppContext::skia_context(|ctx| {
                surface.draw(ctx, |canvas| {
                    draw_fn(canvas);
                });
                surface.swap_buffers(ctx);
                surface.commit();
            });
        }
    }

    fn wl_surface(&self) -> &wl_surface::WlSurface {
        &self.wl_surface
    }

    fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

impl ScLayerAugment for SubsurfaceSurface {
    fn has_sc_layer(&self) -> bool {
        self.sc_layer.is_some()
    }

    fn sc_layer_mut(&mut self) -> Option<&mut Option<sc_layer_v1::ScLayerV1>> {
        Some(&mut self.sc_layer)
    }

    fn sc_layer_shell(&self) -> Option<&sc_layer_shell_v1::ScLayerShellV1> {
        use crate::app_runner::AppContext;
        AppContext::sc_layer_shell()
    }

    fn is_configured(&self) -> bool {
        true // Subsurfaces don't have explicit configuration
    }
}
