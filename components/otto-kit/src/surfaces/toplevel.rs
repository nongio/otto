use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowData, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
};

use wayland_client::Dispatch;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel};

use super::common::{
    otto_surface_style_manager_v1, otto_surface_style_v1, BaseWaylandSurface, SurfaceError,
};
use crate::AppContext;

/// Manages an XDG toplevel window surface with Skia rendering
///
/// This surface type represents a top-level application window.
/// It handles window configuration, provides a Skia canvas for drawing,
/// and supports optional sc_layer protocol augmentation for visual effects.

#[derive(Clone)]
pub struct ToplevelSurface {
    base_surface: BaseWaylandSurface,
    window: Window,
    configured: bool,
}

impl ToplevelSurface {
    /// Create a new toplevel surface using global AppContext
    ///
    /// This simplified constructor uses the global AppContext and AppRunnerDefault,
    /// avoiding the need to pass compositor, xdg_shell, and queue handle.
    ///
    /// # Arguments
    /// * `title` - Window title
    /// * `width` - Initial width in logical pixels
    /// * `height` - Initial height in logical pixels
    ///
    /// # Example
    /// ```no_run
    /// use otto_kit::surfaces::ToplevelSurface;
    ///
    /// let surface = ToplevelSurface::new("My Window", 800, 600)?;
    /// ```
    pub fn new(title: &str, width: i32, height: i32) -> Result<Self, SurfaceError> {
        let compositor = AppContext::compositor_state();
        let xdg_shell = AppContext::xdg_shell_state();
        let surface_style_manager = AppContext::surface_style_manager();
        let qh = AppContext::queue_handle();

        Self::new_typed(
            title,
            width,
            height,
            compositor,
            xdg_shell,
            surface_style_manager,
            qh,
        )
    }

    /// Create a new toplevel surface (typed version)
    ///
    /// This version allows you to pass explicit Wayland protocol states.
    /// Most users should use `new()` instead.
    ///
    /// # Arguments
    /// * `title` - Window title
    /// * `width` - Initial width in logical pixels
    /// * `height` - Initial height in logical pixels
    /// * `compositor` - Compositor state
    /// * `xdg_shell` - XDG shell state
    /// * `surface_style_manager` - Optional surface style manager for augmentation
    /// * `qh` - Queue handle for creating objects
    pub fn new_typed<D>(
        title: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        surface_style_manager: Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1>,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, WindowData>
            + Dispatch<xdg_toplevel::XdgToplevel, WindowData>
            + Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, WindowData>
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
            + WindowHandler
            + 'static,
    {
        // Create the window
        let window = xdg_shell.create_window(
            compositor.create_surface(qh),
            WindowDecorations::ServerDefault,
            qh,
        );

        window.set_title(title.to_string());
        window.set_min_size(Some((width as u32, height as u32)));

        let wl_surface = window.wl_surface().clone();

        // Use 2x buffer scale for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // Create surface style immediately if surface_style_manager is available
        let surface_style =
            surface_style_manager.map(|manager| manager.get_surface_style(&wl_surface, qh, ()));

        // Commit to trigger initial configure
        wl_surface.commit();

        let mut base_surface = BaseWaylandSurface::new(wl_surface, width, height, buffer_scale);
        base_surface.surface_style = surface_style;

        let toplevel = Self {
            base_surface,
            window,
            configured: false,
        };

        Ok(toplevel)
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

    /// Get the underlying XDG window
    /// This allows access to window operations like move, resize, etc.
    pub fn xdg_window(&self) -> &smithay_client_toolkit::shell::xdg::window::Window {
        &self.window
    }
}

impl ToplevelSurface {
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
        self.window.commit();
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

    /// Get the window object (used by Menu component)
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Get direct access to the surface style
    pub fn surface_style(&self) -> Option<&otto_surface_style_v1::OttoSurfaceStyleV1> {
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
