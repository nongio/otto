use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
    shell::xdg::{
        popup::{Popup, PopupData},
        XdgPositioner, XdgShell,
    },
};

use wayland_client::{Dispatch, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_surface};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

use super::common::{
    otto_surface_style_manager_v1, otto_surface_style_v1, BaseWaylandSurface, SurfaceError,
};

/// Manages an XDG popup surface with Skia rendering
///
/// This surface type represents a popup menu or tooltip that appears
/// relative to a parent surface. It handles popup positioning and
/// configuration, provides a Skia canvas for drawing, and supports
/// sc_layer augmentation for visual effects.
pub struct PopupSurface {
    base_surface: BaseWaylandSurface,
    popup: Option<Popup>,
    configured: bool,
    dirty: std::cell::Cell<bool>,
    // last_acked_serial: std::cell::Cell<Option<u32>>,
}

impl PopupSurface {
    /// Create a new popup surface using global AppContext
    ///
    /// This simplified constructor uses the global AppContext and AppRunnerDefault,
    /// avoiding the need to pass compositor, xdg_shell, and queue handle.
    ///
    /// # Arguments
    /// * `parent_surface` - The parent XDG surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    ///
    /// # Example
    /// ```no_run
    /// use otto_kit::surfaces::PopupSurface;
    ///
    /// let popup = PopupSurface::new(&parent_surface, &positioner, 200, 100)?;
    /// ```
    pub fn new(
        parent_surface: &xdg_surface::XdgSurface,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
    ) -> Result<Self, SurfaceError> {
        Self::new_with_grab(parent_surface, positioner, width, height, Some(0))
    }

    /// Create a new popup surface with an explicit grab serial
    ///
    /// # Arguments
    /// * `parent_surface` - The parent XDG surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    /// * `grab_serial` - Optional serial from input event for grab (None = no grab)
    pub fn new_with_grab(
        parent_surface: &xdg_surface::XdgSurface,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
        grab_serial: Option<u32>,
    ) -> Result<Self, SurfaceError> {
        use crate::app_runner::AppContext;

        let compositor = AppContext::compositor_state();
        let xdg_shell = AppContext::xdg_shell_state();
        let qh = AppContext::queue_handle();

        Self::new_typed(
            parent_surface,
            positioner,
            width,
            height,
            grab_serial,
            compositor,
            xdg_shell,
            qh,
        )
    }

    /// Create a new popup surface (typed version)
    ///
    /// This version allows you to pass explicit Wayland protocol states.
    /// Most users should use `new()` or `new_with_grab()` instead.
    ///
    /// # Arguments
    /// * `parent_surface` - The parent XDG surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    /// * `grab_serial` - Optional serial from input event for grab (None = no grab)
    /// * `compositor` - Compositor state
    /// * `xdg_shell` - XDG shell state
    /// * `qh` - Queue handle for creating objects
    pub fn new_typed<D>(
        parent_surface: &xdg_surface::XdgSurface,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
        grab_serial: Option<u32>,
        compositor: &CompositorState,
        xdg_shell: &XdgShell,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, PopupData>
            + Dispatch<xdg_popup::XdgPopup, PopupData>
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
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

        // Request an explicit grab if we have a valid serial
        // This allows menus to receive keyboard events (arrows, Enter, Escape)
        if let Some(serial) = grab_serial {
            let seat_state = super::super::app_runner::AppContext::seat_state();
            if let Some(seat) = seat_state.seats().next() {
                popup.xdg_popup().grab(&seat, serial);
            }
        }

        // Use 2x buffer for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // CRITICAL: Commit immediately after grab to trigger configure event
        wl_surface.commit();

        // Create Skia surface using shared context
        let mut core = BaseWaylandSurface::new(wl_surface, width, height, buffer_scale);

        use crate::app_runner::AppContext;
        core.surface_style = AppContext::surface_style_manager()
            .map(|shell| shell.get_surface_style(core.wl_surface(), qh, ()));
        core.create_skia_surface()?;

        let popup_surface = Self {
            base_surface: core,
            popup: Some(popup),
            configured: false,
            dirty: std::cell::Cell::new(true), // Start as dirty to trigger initial draw
                                               // last_acked_serial: std::cell::Cell::new(None),
        };

        Ok(popup_surface)
    }

    /// Create a new popup surface for a layer shell parent using global AppContext
    ///
    /// This simplified constructor uses the global AppContext and AppRunnerDefault.
    ///
    /// # Arguments
    /// * `layer_surface` - The parent layer shell surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    pub fn new_for_layer(
        layer_surface: &ZwlrLayerSurfaceV1,
        positioner: &XdgPositioner,
        width: i32,
        height: i32,
    ) -> Result<Self, SurfaceError> {
        use crate::app_runner::AppContext;
        println!("Creating popup surface for layer shell parent...");
        let compositor = AppContext::compositor_state();
        let xdg_shell = AppContext::xdg_shell_state();
        let qh = AppContext::queue_handle();

        Self::new_for_layer_typed(
            layer_surface,
            positioner,
            width,
            height,
            compositor,
            xdg_shell,
            qh,
        )
    }

    /// Create a new popup surface for a layer shell parent (typed version)
    ///
    /// This creates a popup with NO XDG parent, then assigns it to the layer surface
    /// using the wlr-layer-shell `get_popup` request.
    ///
    /// # Arguments
    /// * `layer_surface` - The parent layer shell surface
    /// * `positioner` - XDG positioner defining popup position and size
    /// * `width` - Width in logical pixels
    /// * `height` - Height in logical pixels
    /// * `compositor` - Compositor state
    /// * `xdg_shell` - XDG shell state
    /// * `qh` - Queue handle for creating objects
    pub fn new_for_layer_typed<D>(
        layer_surface: &ZwlrLayerSurfaceV1,
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
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
            + 'static,
    {
        let wl_surface = compositor.create_surface(qh);

        // Create popup with NULL parent (required for layer shells)
        let popup = Popup::from_surface(
            None, // NULL parent
            positioner,
            qh,
            wl_surface.clone(),
            xdg_shell,
        )
        .map_err(|_| SurfaceError::CreationFailed)?;

        // Assign popup to layer surface via get_popup
        layer_surface.get_popup(popup.xdg_popup());

        // Request an implicit grab for keyboard and pointer focus
        // This allows menus to receive keyboard events (arrows, Enter, Escape)
        let seat_state = super::super::app_runner::AppContext::seat_state();
        if let Some(seat) = seat_state.seats().next() {
            popup.xdg_popup().grab(&seat, 0);
        }

        // Use 2x buffer for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // Create Skia surface using shared context
        let mut core = BaseWaylandSurface::new(wl_surface, width, height, buffer_scale);

        use crate::app_runner::AppContext;
        core.surface_style = AppContext::surface_style_manager()
            .map(|shell| shell.get_surface_style(core.wl_surface(), qh, ()));
        core.create_skia_surface()?;

        let popup_surface = Self {
            base_surface: core,
            popup: Some(popup),
            configured: false,
            dirty: std::cell::Cell::new(true), // Start as dirty to trigger initial draw
                                               // last_acked_serial: std::cell::Cell::new(None),
        };

        // Commit the surface WITHOUT a buffer to trigger configure event
        // The compositor needs this to send us the configure event with positioning
        // But since we have no buffer attached, nothing will be visible yet
        popup_surface.base_surface.wl_surface().commit();

        Ok(popup_surface)
    }

    pub fn is_configured(&self) -> bool {
        self.configured
    }

    pub fn mark_configured(&mut self) {
        self.configured = true;
        self.dirty.set(true); // Mark dirty when configured to trigger initial draw
    }

    /// Mark this surface as needing a redraw
    /// This should be called from event handlers (mouse, keyboard, etc.)
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// Check if this surface needs redrawing
    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Render this surface if it's dirty (called by render loop)
    /// This is the preferred method for the render loop to use
    pub fn render_if_dirty<F>(&self, draw_fn: F) -> bool
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        if !self.configured || !self.dirty.get() {
            return false;
        }

        self.draw(draw_fn);
        self.dirty.set(false);
        true // Indicate that we rendered
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
        self.base_surface.wl_surface().destroy();

        // Drop the popup - this will destroy xdg_popup and xdg_surface
        self.popup.take();
        self.base_surface.surface_style.take();
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
        if !self.base_surface.wl_surface().is_alive() {
            // Create new wl_surface
            let new_surface = compositor.create_surface(qh);
            new_surface.set_buffer_scale(self.base_surface.buffer_scale);
            self.base_surface.wl_surface = new_surface.clone();

            // Recreate Skia surface using shared context
            self.base_surface.create_skia_surface()?;

            // Create new popup on the new wl_surface
            let popup =
                Popup::from_surface(Some(parent_surface), positioner, qh, new_surface, xdg_shell)
                    .map_err(|_| SurfaceError::CreationFailed)?;

            self.popup = Some(popup);
        }

        // Set window geometry
        if let Some(popup) = &self.popup {
            let (width, height) = self.base_surface.dimensions();
            popup.xdg_surface().set_window_geometry(0, 0, width, height);
        }

        self.configured = false;

        // Commit to trigger configure
        self.base_surface.wl_surface().commit();

        Ok(())
    }

    /// Destroy the popup completely
    pub fn destroy(&mut self) {
        // Clean up Skia/EGL resources first
        if let Some(skia_surface) = self.base_surface.skia_surface.take() {
            // Drop the Rc, which will trigger SkiaSurface::Drop if it's the last reference
            drop(skia_surface);
        }

        // Clean up sc_layer
        self.base_surface.surface_style.take();

        // Destroy Wayland objects
        if let Some(popup) = self.popup.take() {
            popup.xdg_popup().destroy();
            self.configured = false;
        }

        // Destroy wl_surface to fully reset
        self.base_surface.wl_surface.destroy();

        // Flush the connection to ensure compositor processes the destruction immediately
        // This is critical for maintaining proper popup hierarchy destruction order
        if let Some(conn) = self.base_surface.wl_surface.backend().upgrade() {
            let _ = conn.flush();
        }
    }

    /// Check if popup is still active
    pub fn is_active(&self) -> bool {
        self.popup.is_some()
    }

    pub fn mark_as_dirty(&self) {
        self.dirty.set(true);
    }

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

impl Drop for PopupSurface {
    fn drop(&mut self) {
        // Just drop the popup - let Rust handle the cleanup chain
        // The Popup's Drop will clean up xdg_popup and xdg_surface
        // The wl_surface, SkiaContext, and SkiaSurface have their own Drop impls
        self.popup.take();
    }
}
