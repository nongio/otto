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

use super::common::{sc_layer_shell_v1, sc_layer_v1, ScLayerAugment, Surface, SurfaceError};
use crate::rendering::{SkiaContext, SkiaSurface};

/// Manages an XDG toplevel window surface with Skia rendering
///
/// This surface type represents a top-level application window.
/// It handles window configuration, provides a Skia canvas for drawing,
/// and supports optional sc_layer protocol augmentation for visual effects.

#[derive(Clone)]
pub struct ToplevelSurface {
    wl_surface: wl_surface::WlSurface,
    window: Window,
    skia_surface: Option<SkiaSurface>,
    width: i32,
    height: i32,
    configured: bool,
    buffer_scale: i32,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
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
    /// * `qh` - Queue handle for creating objects
    pub fn new<D>(
        title: &str,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
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

        // Commit to trigger initial configure
        wl_surface.commit();

        let window = Self {
            wl_surface,
            window,
            skia_surface: None,
            width,
            height,
            configured: false,
            buffer_scale,
            sc_layer: None,
        };

        Ok(window)
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
        use crate::app_runner::AppContext;

        // Get configured size or use initial size
        println!("ToplevelSurface handling configure: {:?}", self.configured);
        let (width, height) = match configure.new_size {
            (Some(w), Some(h)) => (w.get() as i32, h.get() as i32),
            _ => (self.width, self.height),
        };

        // Initialize or resize Skia surface
        if !self.configured {
            // First configure - check if we need to initialize shared context
            let surface = AppContext::skia_context(|ctx| {
                // Context exists, create surface from it
                ctx.create_surface(
                    &self.wl_surface,
                    width * self.buffer_scale,
                    height * self.buffer_scale,
                )
            });

            if let Some(result) = surface {
                // Shared context exists, use it
                self.skia_surface =
                    Some(result.map_err(|e| SurfaceError::SkiaError(e.to_string()))?);
            } else {
                // No shared context yet - create it with this first surface
                let (new_ctx, new_surface) = SkiaContext::new(
                    AppContext::display_ptr(),
                    &self.wl_surface,
                    width * self.buffer_scale,
                    height * self.buffer_scale,
                )
                .map_err(|e| SurfaceError::SkiaError(e.to_string()))?;

                // Store the shared context
                AppContext::set_skia_context(new_ctx);
                self.skia_surface = Some(new_surface);
            }
            self.configured = true;
        } else if width != self.width || height != self.height {
            // Resize - recreate surface using shared context
            let surface = AppContext::skia_context(|ctx| {
                ctx.create_surface(
                    &self.wl_surface,
                    width * self.buffer_scale,
                    height * self.buffer_scale,
                )
            })
            .ok_or(SurfaceError::SkiaError(
                "SkiaContext not initialized".to_string(),
            ))?
            .map_err(|e| SurfaceError::SkiaError(e.to_string()))?;

            self.skia_surface = Some(surface);
        }

        self.width = width;
        self.height = height;

        Ok(())
    }

    /// Check if surface is configured
    pub fn is_configured(&self) -> bool {
        self.configured
    }

    /// Get the window object
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Request window close
    pub fn request_close(&self) {
        // The compositor will send a close request event
        // which the app should handle via WindowHandler
    }
}

impl Surface for ToplevelSurface {
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if !self.configured {
            eprintln!("Warning: Drawing on unconfigured ToplevelSurface");
            return;
        }

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
        // Note: This returns a reference to the stored wl_surface in self
        // The actual wl_surface is stored in EGL resources, but we keep a clone here
        &self.wl_surface
    }

    fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

impl ScLayerAugment for ToplevelSurface {
    fn has_sc_layer(&self) -> bool {
        use crate::app_runner::AppContext;
        AppContext::sc_layer_shell().is_some()
    }

    fn sc_layer_mut(&mut self) -> Option<&mut Option<sc_layer_v1::ScLayerV1>> {
        Some(&mut self.sc_layer)
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
    /// Apply sc_layer augmentation with queue handle
    ///
    /// This version can be called from the configure handler where
    /// we have access to the queue handle.
    pub fn augment_with_qh<F, D>(
        &mut self,
        augment_fn: F,
        qh: &QueueHandle<D>,
    ) -> Result<(), SurfaceError>
    where
        F: FnOnce(&sc_layer_v1::ScLayerV1),
        D: Dispatch<sc_layer_v1::ScLayerV1, ()> + 'static,
    {
        use crate::app_runner::AppContext;

        if !self.configured {
            return Err(SurfaceError::NotConfigured);
        }

        let sc_layer_shell =
            AppContext::sc_layer_shell().ok_or(SurfaceError::ScLayerNotAvailable)?;

        // Create sc_layer if not exists
        if self.sc_layer.is_none() {
            let layer = sc_layer_shell.get_layer(&self.wl_surface, qh, ());
            self.sc_layer = Some(layer);
        }

        if let Some(ref layer) = self.sc_layer {
            augment_fn(layer);
        }

        Ok(())
    }
}
