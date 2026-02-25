use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
};

use wayland_client::{
    protocol::{wl_subcompositor, wl_subsurface},
    Dispatch,
};

use super::common::{
    otto_surface_style_manager_v1, otto_surface_style_v1, BaseWaylandSurface, SurfaceError,
};

/// Manages a Wayland subsurface with Skia rendering
///
/// This surface type represents a child surface positioned relative to a parent.
/// It's useful for elements like menubars, decorations, or overlays that need
/// to be part of a window but managed separately.
pub struct SubsurfaceSurface {
    base_surface: BaseWaylandSurface,
}

impl SubsurfaceSurface {
    /// Create a new subsurface using global AppContext
    ///
    /// This simplified constructor uses the global AppContext and AppRunnerDefault,
    /// avoiding the need to pass compositor, subcompositor, and queue handle.
    ///
    /// # Arguments
    /// * `parent_surface` - The parent Wayland surface
    /// * `x` - X position relative to parent in logical pixels
    /// * `y` - Y position relative to parent in logical pixels
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    ///
    /// # Example
    /// ```no_run
    /// use otto_kit::surfaces::SubsurfaceSurface;
    ///
    /// let subsurface = SubsurfaceSurface::new(&parent_surface, 0, 0, 200, 100)?;
    /// ```
    pub fn new(
        parent_surface: &wl_surface::WlSurface,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<Self, SurfaceError> {
        use crate::app_runner::AppContext;

        let compositor = AppContext::compositor_state();
        let subcompositor = AppContext::subcompositor()
            .ok_or_else(|| SurfaceError::WaylandError("Subcompositor not available".to_string()))?;
        let qh = AppContext::queue_handle();

        Self::new_typed(
            parent_surface,
            x,
            y,
            width,
            height,
            compositor,
            subcompositor,
            qh,
        )
    }

    /// Create a new subsurface (typed version)
    ///
    /// This version allows you to pass explicit Wayland protocol states.
    /// Most users should use `new()` instead.
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
    #[allow(clippy::too_many_arguments)]
    pub fn new_typed<D>(
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
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
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
        let sc_layer = AppContext::surface_style_manager()
            .map(|shell| shell.get_surface_style(&wl_surface, qh, ()));

        // Commit the surface
        wl_surface.commit();

        let mut core = BaseWaylandSurface::new(wl_surface, width, height, buffer_scale);
        core.surface_style = sc_layer;
        core.create_skia_surface()?;

        Ok(Self { base_surface: core })
    }

    /// Resize the subsurface
    pub fn resize(&mut self, width: i32, height: i32) {
        self.base_surface.resize(width, height);
    }

    /// Set position relative to parent surface
    pub fn set_position(&self, _x: i32, _y: i32) {
        // Note: subsurface handle is not stored, position must be set during creation
        // This method is kept for API compatibility but does nothing
    }

    /// Commit changes to the subsurface
    pub fn commit(&self) {
        self.base_surface.wl_surface().commit();
    }

    /// Get direct access to the sc_layer
    pub fn layer(&self) -> Option<&otto_surface_style_v1::OttoSurfaceStyleV1> {
        self.base_surface.surface_style()
    }

    /// Assign a layer node to render in this surface
    pub fn set_layer_node(&mut self, layer: layers::prelude::Layer) {
        self.base_surface.set_layer_node(layer);
    }

    /// Get the layer node assigned to this surface
    pub fn layer_node(&self) -> Option<&layers::prelude::Layer> {
        self.base_surface.layer_node()
    }
}

impl SubsurfaceSurface {
    /// Get reference to the base surface
    pub fn base_surface(&self) -> &BaseWaylandSurface {
        &self.base_surface
    }

    /// Get mutable reference to the base surface
    pub fn base_surface_mut(&mut self) -> &mut BaseWaylandSurface {
        &mut self.base_surface
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> &wayland_client::protocol::wl_surface::WlSurface {
        self.base_surface.wl_surface()
    }

    /// Get dimensions (width, height) in logical pixels
    pub fn dimensions(&self) -> (i32, i32) {
        self.base_surface.dimensions()
    }

    /// Draw on the surface using a callback
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        self.base_surface.draw(draw_fn);
    }

    /// Register a callback to be called on every compositor frame
    pub fn on_frame<F>(&self, callback: F)
    where
        F: FnMut() + 'static,
    {
        self.base_surface.on_frame(callback);
    }
}
