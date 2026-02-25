use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
};
use std::cell::RefCell;
use std::rc::Rc;
use wayland_client::Dispatch;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use super::common::{
    otto_surface_style_manager_v1, otto_surface_style_v1, BaseWaylandSurface, SurfaceError,
};

/// Internal state for LayerShellSurface that can be shared
struct LayerShellSurfaceInner {
    base_surface: BaseWaylandSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    configured: bool,
    on_configure: Option<Rc<dyn Fn()>>,
}

/// Manages a wlr-layer-shell surface with Skia rendering
///
/// This surface type represents a layer-shell surface (topbar, panel, overlay, etc.)
/// that can be anchored to screen edges and request exclusive zones.
/// It handles layer surface configuration, provides a Skia canvas for drawing,
/// and supports optional sc_layer protocol augmentation for visual effects.
#[derive(Clone)]
pub struct LayerShellSurface {
    inner: Rc<RefCell<LayerShellSurfaceInner>>,
}

impl LayerShellSurface {
    /// Create a new layer shell surface using global AppContext
    ///
    /// This simplified constructor uses the global AppContext and AppRunnerDefault,
    /// avoiding the need to pass compositor, layer_shell, and queue handle.
    ///
    /// # Arguments
    /// * `layer` - Layer position (Background, Bottom, Top, or Overlay)
    /// * `namespace` - Unique namespace for this layer surface (e.g., "panel", "dock")
    /// * `width` - Initial width in logical pixels (0 = fill available width)
    /// * `height` - Initial height in logical pixels (0 = fill available height)
    ///
    /// # Example
    /// ```no_run
    /// use otto_kit::surfaces::LayerShellSurface;
    /// use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
    ///
    /// let surface = LayerShellSurface::new(Layer::Top, "my-panel", 1920, 32)?;
    /// ```
    pub fn new(
        layer: Layer,
        namespace: &str,
        width: u32,
        height: u32,
    ) -> Result<Self, SurfaceError> {
        use crate::app_runner::AppContext;

        let compositor = AppContext::compositor_state();
        let layer_shell = AppContext::wlr_layer_shell()
            .ok_or_else(|| SurfaceError::WaylandError("Layer shell not available".to_string()))?;
        let sc_layer_shell = AppContext::surface_style_manager();
        let qh = AppContext::queue_handle();

        Self::new_typed(
            layer,
            namespace,
            width,
            height,
            compositor,
            layer_shell,
            sc_layer_shell,
            qh,
        )
    }

    /// Create a new layer shell surface (typed version)
    ///
    /// This version allows you to pass explicit Wayland protocol states.
    /// Most users should use `new()` instead.
    ///
    /// # Arguments
    /// * `layer` - Layer position (Background, Bottom, Top, or Overlay)
    /// * `namespace` - Unique namespace for this layer surface (e.g., "panel", "dock")
    /// * `width` - Initial width in logical pixels (0 = fill available width)
    /// * `height` - Initial height in logical pixels (0 = fill available height)
    /// * `compositor` - Compositor state
    /// * `layer_shell` - wlr-layer-shell protocol object
    /// * `sc_layer_shell` - Optional SC layer shell for augmentation
    /// * `qh` - Queue handle for creating objects
    #[allow(clippy::too_many_arguments)]
    pub fn new_typed<D>(
        layer: Layer,
        namespace: &str,
        width: u32,
        height: u32,
        compositor: &CompositorState,
        layer_shell: &ZwlrLayerShellV1,
        surface_style: Option<&otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1>,
        qh: &QueueHandle<D>,
    ) -> Result<Self, SurfaceError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<ZwlrLayerSurfaceV1, ()>
            + Dispatch<otto_surface_style_v1::OttoSurfaceStyleV1, ()>
            + Dispatch<otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1, ()>
            + 'static,
    {
        // Create the wl_surface
        let wl_surface = compositor.create_surface(qh);

        // Create the layer surface
        let layer_surface = layer_shell.get_layer_surface(
            &wl_surface,
            None, // Use first available output
            layer,
            namespace.to_string(),
            qh,
            (),
        );

        // Use 2x buffer scale for HiDPI rendering
        let buffer_scale = 2;
        wl_surface.set_buffer_scale(buffer_scale);

        // Create sc_layer immediately if sc_layer_shell is available
        let surface_style = surface_style.map(|shell| shell.get_surface_style(&wl_surface, qh, ()));

        // Set initial size on the layer surface
        layer_surface.set_size(width, height);

        // Commit to trigger initial configure
        wl_surface.commit();

        let mut core =
            BaseWaylandSurface::new(wl_surface, width as i32, height as i32, buffer_scale);
        core.surface_style = surface_style;

        let inner = LayerShellSurfaceInner {
            base_surface: core,
            layer_surface: layer_surface.clone(),
            configured: false,
            on_configure: None,
        };

        let layer_shell_surface = Self {
            inner: Rc::new(RefCell::new(inner)),
        };

        // Register configure callback to handle layer surface configure events
        use crate::app_runner::AppContext;
        use wayland_client::Proxy;

        let layer_surface_id = layer_surface.id();
        let inner_clone = layer_shell_surface.inner.clone();

        AppContext::register_layer_configure_callback(
            layer_surface_id,
            move |width, height, serial| {
                tracing::debug!(
                    "Layer configure callback: {}x{}, serial: {}",
                    width,
                    height,
                    serial
                );

                // Extract callback before borrowing to avoid double borrow
                let callback_to_call = {
                    let mut inner = inner_clone.borrow_mut();

                    // Acknowledge the configure
                    inner.layer_surface.ack_configure(serial);

                    // Update dimensions
                    let width = if width > 0 {
                        width
                    } else {
                        inner.base_surface.width
                    };
                    let height = if height > 0 {
                        height
                    } else {
                        inner.base_surface.height
                    };

                    // Initialize or resize Skia surface
                    let mut callback_to_call = None;
                    if !inner.configured {
                        // First time configuration - create the Skia surface
                        inner.base_surface.width = width;
                        inner.base_surface.height = height;
                        if let Err(e) = inner.base_surface.create_skia_surface() {
                            eprintln!("Error creating Skia surface: {:?}", e);
                            return;
                        }
                        // Update layer node size if using layers engine
                        if let Some(layer) = &inner.base_surface.layer_node {
                            layer.set_size(
                                layers::types::Size::points(width as f32, height as f32),
                                None,
                            );
                            layer.engine.update(0.0);
                        }
                        inner.configured = true;

                        // Extract the callback to call it after dropping the borrow
                        callback_to_call = inner.on_configure.clone();
                    } else if width != inner.base_surface.width
                        || height != inner.base_surface.height
                    {
                        // Subsequent resize - use resize method
                        inner.base_surface.resize(width, height);
                    }

                    callback_to_call
                }; // Drop the mutable borrow here

                // Now call the callback without holding the borrow
                if let Some(callback) = callback_to_call {
                    callback();
                }
            },
        );

        Ok(layer_shell_surface)
    }

    /// Configure anchoring for the layer surface
    ///
    /// # Example
    /// ```
    /// // Anchor to top-left-right (creates a topbar)
    /// surface.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right);
    /// ```
    pub fn set_anchor(&self, anchor: Anchor) {
        self.inner.borrow().layer_surface.set_anchor(anchor);
    }

    /// Set the exclusive zone
    ///
    /// Positive values reserve space at the anchor edge, pushing other surfaces away.
    /// Zero means no exclusive zone.
    /// -1 means the entire surface is exclusive.
    pub fn set_exclusive_zone(&self, zone: i32) {
        self.inner.borrow().layer_surface.set_exclusive_zone(zone);
    }

    /// Set keyboard interactivity
    pub fn set_keyboard_interactivity(&self, interactivity: KeyboardInteractivity) {
        self.inner
            .borrow()
            .layer_surface
            .set_keyboard_interactivity(interactivity);
    }

    /// Set the size of the layer surface
    ///
    /// Width or height of 0 means the surface will be sized to fill available space
    /// in that dimension (constrained by anchors).
    pub fn set_size(&self, width: u32, height: u32) {
        self.inner.borrow().layer_surface.set_size(width, height);
    }

    /// Set margins from the anchor edges
    pub fn set_margin(&self, top: i32, right: i32, bottom: i32, left: i32) {
        self.inner
            .borrow()
            .layer_surface
            .set_margin(top, right, bottom, left);
    }

    /// Set a callback to be called when the surface is first configured
    ///
    /// This is useful for triggering an initial draw after the compositor
    /// sends the configure event and the Skia surface is ready.
    pub fn on_configure<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        self.inner.borrow_mut().on_configure = Some(Rc::new(callback));
    }

    /// Handle layer surface closed event
    pub fn handle_closed(&mut self) {
        self.inner.borrow_mut().configured = false;
    }

    /// Check if surface is configured
    pub fn is_configured(&self) -> bool {
        self.inner.borrow().configured
    }

    /// Get the underlying wlr-layer-surface
    pub fn layer_surface(&self) -> ZwlrLayerSurfaceV1 {
        self.inner.borrow().layer_surface.clone()
    }

    /// Destroy the layer surface
    pub fn destroy(&self) {
        self.inner.borrow().layer_surface.destroy();
    }

    /// Get reference to the base surface
    pub fn base_surface(&self) -> &BaseWaylandSurface {
        unsafe {
            let ptr = self.inner.as_ptr();
            &(*ptr).base_surface
        }
    }

    /// Get mutable reference to the base surface
    pub fn base_surface_mut(&mut self) -> &mut BaseWaylandSurface {
        unsafe {
            let ptr = self.inner.as_ptr();
            &mut (*ptr).base_surface
        }
    }

    /// Get the underlying Wayland surface
    pub fn wl_surface(&self) -> wayland_client::protocol::wl_surface::WlSurface {
        self.base_surface().wl_surface().clone()
    }

    /// Get dimensions (width, height) in logical pixels
    pub fn dimensions(&self) -> (i32, i32) {
        self.base_surface().dimensions()
    }

    /// Draw on the surface using a callback
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        self.base_surface().draw(draw_fn);
    }

    /// Register a callback to be called on every compositor frame
    pub fn on_frame<F>(&self, callback: F)
    where
        F: FnMut() + 'static,
    {
        self.base_surface().on_frame(callback);
    }
}
