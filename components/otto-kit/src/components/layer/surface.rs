use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, Connection, QueueHandle},
};
use wayland_client::{
    protocol::{wl_subcompositor, wl_subsurface},
    Dispatch,
};

use crate::rendering::SkiaSurface;

use super::Layer;
// Re-export sc-layer protocol from menu component
use crate::protocols::{otto_surface_style_manager_v1, otto_surface_style_v1};

/// LayerSurface - manages the Wayland subsurface and rendering for a Layer component
///
/// This handles the creation and management of the subsurface, including positioning
/// and rendering through the Skia rendering system.
pub struct LayerSurface {
    pub wl_surface: wl_surface::WlSurface,
    pub subsurface: wl_subsurface::WlSubsurface,
    pub skia_surface: Option<SkiaSurface>,
    pub layer: Layer,
    pub width: i32,
    pub height: i32,
    _buffer_scale: i32,
    x: i32,
    y: i32,
    configured: bool,
}

impl LayerSurface {
    /// Create a new LayerSurface as a subsurface of the given parent
    ///
    /// # Arguments
    /// * `parent_surface` - The parent Wayland surface
    /// * `layer` - The Layer component with drawing logic
    /// * `compositor` - The compositor state
    /// * `subcompositor` - The Wayland subcompositor global
    /// * `qh` - Queue handle for creating surfaces
    /// * `conn` - Connection for roundtrip
    pub fn new<D>(
        parent_surface: &wl_surface::WlSurface,
        layer: Layer,
        compositor: &CompositorState,
        subcompositor: &wl_subcompositor::WlSubcompositor,
        qh: &QueueHandle<D>,
        conn: &Connection,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<wl_subsurface::WlSubsurface, ()>
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
            + 'static,
    {
        let x = layer.x();
        let y = layer.y();
        let width = layer.width();
        let height = layer.height();

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
        .ok_or("SkiaContext not initialized")?
        .map_err(|e| e)?;

        let mut layer_surface = Self {
            wl_surface: wl_surface.clone(),
            subsurface,
            skia_surface: Some(skia_surface),
            layer,
            width,
            height,
            _buffer_scale: buffer_scale,
            x,
            y,
            configured: false,
        };

        // Commit and wait for first configure
        wl_surface.commit();
        conn.roundtrip()?;

        // Apply augmentation if sc_layer_shell is available and augment_fn is set
        if let Some(sc_layer_shell) = AppContext::surface_style_manager() {
            if let Some(augment_fn) = layer_surface.layer.augment_fn() {
                let layer = sc_layer_shell.get_surface_style(&wl_surface, qh, ());
                augment_fn(&layer);
                layer_surface.configured = true;
            }
        }

        Ok(layer_surface)
    }

    /// Render the layer content
    pub fn render(&mut self) {
        use crate::app_runner::AppContext;

        if let Some(surface) = &mut self.skia_surface {
            AppContext::skia_context(|ctx| {
                surface.draw(ctx, |canvas| {
                    self.layer.render(canvas);
                });
                surface.swap_buffers(ctx);
                surface.commit();
            });
        }
    }

    /// Update the position of the subsurface
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.subsurface.set_position(x, y);
    }

    /// Resize the subsurface
    pub fn resize(&mut self, width: i32, height: i32) {
        use crate::app_runner::AppContext;

        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;

            // Recreate Skia surface with new dimensions using shared context
            let buffer_scale = 2;

            if let Some(new_surface) = AppContext::skia_context(|ctx| {
                ctx.create_surface(
                    &self.wl_surface,
                    width * buffer_scale,
                    height * buffer_scale,
                )
                .ok()
            })
            .flatten()
            {
                self.skia_surface = Some(new_surface);
                self.render();
            }
        }
    }

    /// Get the underlying Wayland surface
    pub fn surface(&self) -> &wl_surface::WlSurface {
        &self.wl_surface
    }

    /// Get the layer component
    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    /// Get mutable layer component (allows augmenting the layer)
    pub fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }

    /// Get current X position
    pub fn x(&self) -> i32 {
        self.x
    }

    /// Get current Y position
    pub fn y(&self) -> i32 {
        self.y
    }

    /// Get current width
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Get current height
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Apply sc_layer augmentation (can be called to re-augment or update)
    pub fn apply_augmentation<D>(&mut self, qh: &QueueHandle<D>)
    where
        D: Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()> + 'static,
    {
        use crate::app_runner::AppContext;
        if let Some(sc_layer_shell) = AppContext::surface_style_manager() {
            if let Some(augment_fn) = self.layer.augment_fn() {
                let layer = sc_layer_shell.get_surface_style(&self.wl_surface, qh, ());
                augment_fn(&layer);
                self.configured = true;
            }
        }
    }
}
