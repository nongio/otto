use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
    shell::xdg::window::{WindowConfigure, WindowData, WindowHandler},
};

use wayland_client::Dispatch;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel};

use super::common::{
    otto_surface_style_manager_v1, otto_surface_style_v1, BaseWaylandSurface, SurfaceError,
};
use crate::{protocols::otto_dock_item_v1, AppContext};

#[derive(Clone)]
pub struct DockItem {
    base_surface: BaseWaylandSurface,
    configured: bool,
    dock_item: otto_dock_item_v1::OttoDockItemV1,
}

impl DockItem {
    pub fn new(app_id: String, width: i32, height: i32) -> Result<Self, SurfaceError> {
        let compositor = AppContext::compositor_state();
        let qh = AppContext::queue_handle();

        Self::new_typed(app_id, width, height, compositor, qh)
    }

    pub fn new_typed<D>(
        app_id: String,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, WindowData>
            + Dispatch<xdg_toplevel::XdgToplevel, WindowData>
            + Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, WindowData>
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
            + Dispatch<otto_dock_item_v1::OttoDockItemV1, ()>
            + WindowHandler
            + 'static,
    {
        let wl_surface = compositor.create_surface(qh);
        // Create a new Wayland surface        // Use 2x buffer scale for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // Commit to trigger initial configure
        wl_surface.commit();

        let mut core = BaseWaylandSurface::new(wl_surface.clone(), width, height, buffer_scale);

        core.create_skia_surface()?;

        let dock_item = AppContext::otto_dock_manager()
            .ok_or(SurfaceError::CreationFailed)?
            .get_dock_item(app_id, qh, ());
        let docksurface = Self {
            base_surface: core,
            configured: true,
            dock_item,
        };

        Ok(docksurface)
    }

    /// Handle window configure event
    ///
    /// This should be called from the WindowHandler::configure callback.
    /// It initializes or resizes the Skia rendering context.
    pub fn handle_configure(
        &mut self,
        configure: WindowConfigure,
        _serial: u32,
    ) -> Result<(), SurfaceError> {
        println!(
            "DockItemSurface configure event: new_size={:?}, serial={}",
            configure.new_size, _serial
        );
        // Get configured size or use initial size
        let (width, height) = match configure.new_size {
            (Some(w), Some(h)) => (w.get() as i32, h.get() as i32),
            _ => self.base_surface.dimensions(),
        };

        // Initialize or resize Skia surface
        if !self.configured {
            self.base_surface.create_skia_surface()?;
            self.configured = true;
        }

        if width != self.base_surface.width || height != self.base_surface.height {
            self.base_surface.resize(width, height);
        }

        Ok(())
    }

    /// Check if surface is configured
    pub fn is_configured(&self) -> bool {
        self.configured
    }
    pub fn dock_item(&self) -> &otto_dock_item_v1::OttoDockItemV1 {
        &self.dock_item
    }
}

impl DockItem {
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

    /// Get window dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        self.base_surface.dimensions()
    }

    /// Resize the surface manually
    pub fn resize(&mut self, width: i32, height: i32) {
        self.base_surface.resize(width, height);
    }

    /// Get direct access to the surface style
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

    /// Request a frame to redraw the surface
    ///
    /// Marks the surface as dirty and commits.
    /// On the next frame event from compositor, render if dirty.
    pub fn request_frame(&self) {
        // Mark surface as dirty
        self.base_surface
            .dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if surface needs rendering
    pub fn is_dirty(&self) -> bool {
        self.base_surface
            .dirty
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Clear dirty flag after rendering
    pub fn clear_dirty(&self) {
        self.base_surface
            .dirty
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
