use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
};
use wayland_client::{
    protocol::{wl_subcompositor, wl_subsurface},
    Dispatch,
};

use super::common::{sc_layer_shell_v1, sc_layer_v1, ScLayerAugment, SkiaBackedSurface, Surface, SurfaceCore, SurfaceError};
use crate::rendering::SkiaSurface;

/// Manages a Wayland subsurface with Skia rendering
///
/// This surface type represents a child surface positioned relative to a parent.
/// It's useful for elements like menubars, decorations, or overlays that need
/// to be part of a window but managed separately.
pub struct SubsurfaceSurface {
    core: SurfaceCore,
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

        // Apply sc_layer augmentation if available
        use crate::app_runner::AppContext;
        let sc_layer = AppContext::sc_layer_shell()
            .map(|shell| shell.get_layer(&wl_surface, qh, ()));

        // Commit the surface
        wl_surface.commit();

        let mut core = SurfaceCore::new(wl_surface, width, height, buffer_scale);
        core.sc_layer = sc_layer;
        core.create_skia_surface()?;

        Ok(Self { core })
    }

    /// Resize the subsurface
    pub fn resize(&mut self, width: i32, height: i32) {
        self.core.resize(width, height);
    }

    /// Set position relative to parent surface
    pub fn set_position(&self, x: i32, y: i32) {
        // Note: subsurface handle is not stored, position must be set during creation
        // This method is kept for API compatibility but does nothing
    }

    /// Commit changes to the subsurface
    pub fn commit(&self) {
        self.core.wl_surface().commit();
    }

    /// Get direct access to the sc_layer
    pub fn layer(&self) -> Option<&sc_layer_v1::ScLayerV1> {
        self.core.sc_layer()
    }

    /// Assign a layer node to render in this surface
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        self.core.set_layer_node(layer);
    }

    /// Get the layer node assigned to this surface
    pub fn layer_node(&self) -> Option<&layers::prelude::Layer> {
        self.core.layer_node()
    }
}

impl SkiaBackedSurface for SubsurfaceSurface {
    fn skia_surface(&self) -> Option<&SkiaSurface> {
        self.core.skia_surface.as_ref()
    }

    fn can_draw(&self) -> bool {
        self.core.skia_surface.is_some()
    }

    fn layer_node(&self) -> Option<layers::prelude::Layer> {
        self.core.layer_node.clone()
    }
}

impl Surface for SubsurfaceSurface {
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        self.draw_skia(draw_fn);
    }

    fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.core.wl_surface()
    }

    fn dimensions(&self) -> (i32, i32) {
        self.core.dimensions()
    }
}

impl ScLayerAugment for SubsurfaceSurface {
    fn has_sc_layer(&self) -> bool {
        self.core.sc_layer().is_some()
    }

    fn sc_layer_mut(&mut self) -> Option<&mut Option<sc_layer_v1::ScLayerV1>> {
        Some(&mut self.core.sc_layer)
    }

    fn sc_layer_shell(&self) -> Option<&sc_layer_shell_v1::ScLayerShellV1> {
        use crate::app_runner::AppContext;
        AppContext::sc_layer_shell()
    }

    fn is_configured(&self) -> bool {
        true // Subsurfaces don't have explicit configuration
    }
}
