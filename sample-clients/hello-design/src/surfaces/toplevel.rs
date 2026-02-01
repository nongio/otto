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

use super::common::{sc_layer_shell_v1, sc_layer_v1, ScLayerAugment, SkiaBackedSurface, Surface, SurfaceCore, SurfaceError};
use crate::rendering::SkiaSurface;

/// Manages an XDG toplevel window surface with Skia rendering
///
/// This surface type represents a top-level application window.
/// It handles window configuration, provides a Skia canvas for drawing,
/// and supports optional sc_layer protocol augmentation for visual effects.

#[derive(Clone)]
pub struct ToplevelSurface {
    core: SurfaceCore,
    window: Window,
    configured: bool,
}

impl ToplevelSurface {
    /// Create a new toplevel surface
    ///
    /// # Arguments
    /// * `title` - Window title
    /// * `width` - Initial width in logical pixels
    /// * `height` - Initial height in logical pixels
    /// * `compositor` - Compositor state
    /// * `xdg_shell` - XDG shell state
    /// * `sc_layer_shell` - Optional SC layer shell for augmentation
    /// * `qh` - Queue handle for creating objects
    pub fn new<D>(
        title: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        sc_layer_shell: Option<&sc_layer_shell_v1::ScLayerShellV1>,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, WindowData>
            + Dispatch<xdg_toplevel::XdgToplevel, WindowData>
            + Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, WindowData>
            + Dispatch<sc_layer_v1::ScLayerV1, ()>
            + Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
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

        // Create sc_layer immediately if sc_layer_shell is available
        let sc_layer = sc_layer_shell.map(|shell| shell.get_layer(&wl_surface, qh, ()));

        // Commit to trigger initial configure
        wl_surface.commit();

        let mut core = SurfaceCore::new(wl_surface, width, height, buffer_scale);
        core.sc_layer = sc_layer;

        let toplevel = Self {
            core,
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
            _ => self.core.dimensions(),
        };

        // Initialize or resize Skia surface
        if !self.configured {
            self.core.create_skia_surface()?;
            self.configured = true;
        }
        
        if width != self.core.width || height != self.core.height {
            self.core.resize(width, height);
        }

        Ok(())
    }

    /// Check if surface is configured
    pub fn is_configured(&self) -> bool {
        self.configured
    }
}

impl SkiaBackedSurface for ToplevelSurface {
    fn skia_surface(&self) -> Option<&SkiaSurface> {
        self.core.skia_surface.as_ref()
    }
    
    fn can_draw(&self) -> bool {
        self.configured
    }
    
    fn layer_node(&self) -> Option<layers::prelude::Layer> {
        self.core.layer_node.clone()
    }
}

impl Surface for ToplevelSurface {
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        if !self.configured {
            eprintln!("Warning: Drawing on unconfigured ToplevelSurface");
            return;
        }
        
        self.draw_skia(draw_fn);
    }

    fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.core.wl_surface()
    }

    fn dimensions(&self) -> (i32, i32) {
        self.core.dimensions()
    }
}

impl ScLayerAugment for ToplevelSurface {
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
        self.configured
    }
}

impl ToplevelSurface {
    /// Get window dimensions
    pub fn dimensions(&self) -> (i32, i32) {
        self.core.dimensions()
    }

    /// Resize the surface manually
    pub fn resize(&mut self, width: i32, height: i32) {
        self.core.resize(width, height);
    }

    /// Get the window object (used by Menu component)
    pub fn window(&self) -> &Window {
        &self.window
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
