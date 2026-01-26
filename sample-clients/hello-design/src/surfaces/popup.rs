use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
    shell::xdg::{
        popup::{Popup, PopupConfigure, PopupData},
        XdgPositioner, XdgShell,
    },
};
use wayland_client::{Dispatch, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_surface};

use super::common::{sc_layer_shell_v1, sc_layer_v1, ScLayerAugment, Surface, SurfaceError};
use crate::rendering::SkiaSurface;

/// Manages an XDG popup surface with Skia rendering
///
/// This surface type represents a popup menu or tooltip that appears
/// relative to a parent surface. It handles popup positioning and
/// configuration, provides a Skia canvas for drawing, and supports
/// sc_layer augmentation for visual effects.
pub struct PopupSurface {
    wl_surface: wl_surface::WlSurface,
    popup: Option<Popup>,
    skia_surface: Option<SkiaSurface>,
    width: i32,
    height: i32,
    configured: bool,
    buffer_scale: i32,
    // sc_layer support
    sc_layer: Option<sc_layer_v1::ScLayerV1>,
}

impl PopupSurface {
    /// Create a new popup surface
    ///
    /// # Arguments
    /// * `parent_surface` - The parent XDG surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    /// * `compositor` - Compositor state
    /// * `xdg_shell` - XDG shell state
    /// * `qh` - Queue handle for creating objects
    pub fn new<D>(
        parent_surface: &xdg_surface::XdgSurface,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, PopupData>
            + Dispatch<xdg_popup::XdgPopup, PopupData>
            + Dispatch<sc_layer_v1::ScLayerV1, ()>
            + Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()>
            + 'static,
    {
        let wl_surface = compositor.create_surface(qh);

        // Create popup with parent XdgSurface
        let popup = Popup::from_surface(
            Some(parent_surface),
            positioner,
            qh,
            wl_surface.clone(),
            xdg_shell,
        )
        .map_err(|_| SurfaceError::CreationFailed)?;

        // Set window geometry
        // popup.xdg_surface().set_window_geometry(0, 0, width, height);

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

        let popup_surface = Self {
            wl_surface: wl_surface.clone(),
            popup: Some(popup),
            skia_surface: Some(skia_surface),
            width,
            height,
            configured: false,
            buffer_scale,
            sc_layer: None,
        };

        // Commit the surface to trigger configure event from compositor
        wl_surface.commit();

        Ok(popup_surface)
    }

    /// Handle popup configure event
    ///
    /// This should be called from the popup configure event handler.
    pub fn handle_configure(
        &mut self,
        _configure: PopupConfigure,
        serial: u32,
    ) -> Result<(), SurfaceError> {
        if let Some(ref popup) = self.popup {
            popup.xdg_surface().ack_configure(serial);
        }

        self.configured = true;
        Ok(())
    }

    /// Check if popup is configured
    pub fn is_configured(&self) -> bool {
        self.configured
    }

    /// Mark popup as configured without acking
    /// Use this when the configure has already been acked elsewhere
    pub fn mark_configured(&mut self) {
        self.configured = true;
    }

    /// Get the popup object
    pub fn popup(&self) -> Option<&Popup> {
        self.popup.as_ref()
    }

    /// Get the XDG surface
    pub fn xdg_surface(&self) -> Option<&xdg_surface::XdgSurface> {
        self.popup.as_ref().map(|p| p.xdg_surface())
    }

    /// Close the popup (hides it but keeps surface and Skia context)
    /// Use this to hide the popup without losing the EGL context
    pub fn close(&mut self) {
        // Destroy the wl_surface to fully reset for next show()
        self.wl_surface.destroy();

        // Drop the popup - this will destroy xdg_popup and xdg_surface
        self.popup.take();
        self.sc_layer.take();
        self.configured = false;
    }

    /// Show the popup by recreating it with the same positioner
    /// This recreates the popup on the existing wl_surface
    pub fn show<D>(
        &mut self,
        parent_surface: &xdg_surface::XdgSurface,
        positioner: &XdgPositioner,
        xdg_shell: &XdgShell,
        compositor: &CompositorState,
        qh: &QueueHandle<D>,
    ) -> Result<(), SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, PopupData>
            + Dispatch<xdg_popup::XdgPopup, PopupData>
            + 'static,
    {
        // If popup already exists, nothing to do
        if self.popup.is_some() {
            return Ok(());
        }

        // If surface was destroyed, recreate everything
        if !self.wl_surface.is_alive() {
            use crate::app_runner::AppContext;

            // Create new wl_surface
            self.wl_surface = compositor.create_surface(qh);
            self.wl_surface.set_buffer_scale(self.buffer_scale);

            // Recreate Skia surface using shared context
            let skia_surface = AppContext::skia_context(|ctx| {
                ctx.create_surface(
                    &self.wl_surface,
                    self.width * self.buffer_scale,
                    self.height * self.buffer_scale,
                )
            })
            .ok_or(SurfaceError::SkiaError(
                "SkiaContext not initialized".to_string(),
            ))?
            .map_err(|e| SurfaceError::SkiaError(e))?;

            self.skia_surface = Some(skia_surface);

            // Create new popup on the new wl_surface
            let popup = Popup::from_surface(
                Some(parent_surface),
                positioner,
                qh,
                self.wl_surface.clone(),
                xdg_shell,
            )
            .map_err(|_| SurfaceError::CreationFailed)?;

            self.popup = Some(popup);
        }

        // Set window geometry
        if let Some(popup) = &self.popup {
            popup
                .xdg_surface()
                .set_window_geometry(0, 0, self.width, self.height);
        }

        self.configured = false;

        // Commit to trigger configure
        self.wl_surface.commit();

        Ok(())
    }

    /// Destroy the popup completely
    pub fn destroy(&mut self) {
        if let Some(popup) = self.popup.take() {
            // Just destroy the xdg_popup, let the Popup's Drop handle xdg_surface
            popup.xdg_popup().destroy();
            self.configured = false;
        }
    }

    /// Check if popup is still active
    pub fn is_active(&self) -> bool {
        self.popup.is_some()
    }
}

impl Surface for PopupSurface {
    fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        use crate::app_runner::AppContext;

        if !self.configured {
            eprintln!("Warning: Drawing on unconfigured PopupSurface");
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
        &self.wl_surface
    }

    fn dimensions(&self) -> (i32, i32) {
        (self.width, self.height)
    }
}

impl ScLayerAugment for PopupSurface {
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
        self.configured
    }
}

impl Drop for PopupSurface {
    fn drop(&mut self) {
        // Just drop the popup - let Rust handle the cleanup chain
        // The Popup's Drop will clean up xdg_popup and xdg_surface
        // The wl_surface, SkiaContext, and SkiaSurface have their own Drop impls
        self.popup.take();
    }
}
