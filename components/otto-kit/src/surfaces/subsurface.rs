use smithay_client_toolkit::{
    compositor::{CompositorState, SurfaceData},
    reexports::client::{protocol::wl_surface, QueueHandle},
};

use wayland_client::{
    protocol::{wl_subcompositor, wl_subsurface},
    Dispatch,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

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
    subsurface: Option<wl_subsurface::WlSubsurface>,
    /// The surface this one hangs from, whose commit carries its position.
    parent: wl_surface::WlSurface,
    /// Its `wp_viewport`, made the first time it is cropped or stretched.
    viewport: Option<WpViewport>,
    /// The source and destination last sent to the viewport; `None` while
    /// the viewport is unset and the buffer shows at its own size.
    view: Option<(skia_safe::Rect, (i32, i32))>,
    /// Whether a buffer is attached. A surface is unmapped by attaching none,
    /// and only its next draw maps it again.
    mapped: std::cell::Cell<bool>,
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
    /// `no_run`: needs a running app connected to a Wayland compositor.
    ///
    /// ```no_run
    /// use otto_kit::surfaces::SubsurfaceSurface;
    /// # use wayland_client::protocol::wl_surface::WlSurface;
    ///
    /// # fn demo(parent_surface: &WlSurface) -> Result<(), otto_kit::surfaces::SurfaceError> {
    /// let subsurface = SubsurfaceSurface::new(parent_surface, 0, 0, 200, 100)?;
    /// # Ok(())
    /// # }
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
        D: Dispatch<wl_surface::WlSurface, SurfaceData<()>>
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

        Ok(Self {
            base_surface: core,
            subsurface: Some(subsurface),
            parent: parent_surface.clone(),
            viewport: None,
            view: None,
            mapped: std::cell::Cell::new(true),
        })
    }

    /// Resize the subsurface
    pub fn resize(&mut self, width: i32, height: i32) {
        self.base_surface.resize(width, height);
    }

    /// Set position relative to the parent surface, in logical points.
    ///
    /// Where a surface is *drawn* can be driven from the surface style
    /// instead, and a component that animates a subsurface around should do
    /// that — the style is animatable and this is not. But the two are separate
    /// things: the compositor hit-tests the pointer against the wl position,
    /// and reports coordinates relative to it, so a surface moved only by its
    /// style receives events for wherever it *used* to be. Anything that reads
    /// pointer positions has to keep this in step with the style.
    ///
    /// Subsurface position is part of the parent's state: it takes effect when
    /// the parent commits, even for a desynchronised subsurface. Smithay
    /// compositors, Otto among them, take it with this surface's own next
    /// commit instead, so a position meant to land now is sent before this
    /// surface commits, and the parent commits after.
    pub fn set_position(&self, x: i32, y: i32) {
        if let Some(ref sub) = self.subsurface {
            sub.set_position(x, y);
        }
    }

    /// Place this subsurface above a sibling surface in the stacking order.
    pub fn place_above(&self, sibling: &wayland_client::protocol::wl_surface::WlSurface) {
        if let Some(ref sub) = self.subsurface {
            sub.place_above(sibling);
        }
    }

    /// Place this subsurface below a sibling surface in the stacking order.
    pub fn place_below(&self, sibling: &wayland_client::protocol::wl_surface::WlSurface) {
        if let Some(ref sub) = self.subsurface {
            sub.place_below(sibling);
        }
    }

    /// Commit changes to the subsurface
    pub fn commit(&self) {
        self.base_surface.wl_surface().commit();
    }

    /// Commit the parent, which is what applies this surface's position and
    /// stacking: both are the parent's state, even for a desynchronised
    /// subsurface.
    pub fn commit_parent(&self) {
        self.parent.commit();
    }

    /// The surface this one hangs from.
    pub fn parent_surface(&self) -> &wl_surface::WlSurface {
        &self.parent
    }

    /// Show `source`, a rect in the surface's own points, at `destination`
    /// points; `None` shows the whole buffer at its own size. Surface state:
    /// it lands with this surface's next commit, and the source has to lie
    /// inside the buffer that commit carries.
    ///
    /// Returns `false` when the compositor has no `wp_viewporter`, in which
    /// case nothing is sent and the buffer keeps showing whole.
    pub fn set_view(&mut self, view: Option<(skia_safe::Rect, (i32, i32))>) -> bool {
        use wayland_client::Proxy;

        if self.view == view {
            return true;
        }
        if self.viewport.is_none() {
            let Some(viewporter) = crate::app_runner::AppContext::viewporter() else {
                return false;
            };
            if view.is_none() {
                return true;
            }
            let surface = self.base_surface.wl_surface();
            self.viewport = Some(viewporter.get_viewport(
                surface,
                crate::app_runner::AppContext::queue_handle(),
                (),
            ));
        }
        let Some(viewport) = self.viewport.as_ref().filter(|v| v.is_alive()) else {
            return false;
        };
        match view {
            Some((source, (width, height))) => {
                viewport.set_source(
                    source.left as f64,
                    source.top as f64,
                    source.width() as f64,
                    source.height() as f64,
                );
                viewport.set_destination(width.max(1), height.max(1));
            }
            // -1 everywhere unsets both: the buffer shows whole again.
            None => {
                viewport.set_source(-1.0, -1.0, -1.0, -1.0);
                viewport.set_destination(-1, -1);
            }
        }
        self.view = view;
        true
    }

    /// Take the surface off the screen by attaching no buffer, and commit.
    /// Its children go with it. Only a draw maps it again: see
    /// [`Self::is_mapped`].
    pub fn unmap(&self) {
        if !self.mapped.replace(false) {
            return;
        }
        let surface = self.base_surface.wl_surface();
        surface.attach(None, 0, 0);
        surface.commit();
    }

    /// Whether a buffer is attached: `false` after [`Self::unmap`] until the
    /// next [`Self::draw`].
    pub fn is_mapped(&self) -> bool {
        self.mapped.get()
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
    /// Destroy the subsurface and its underlying wl_surface. Calling it again
    /// does nothing.
    ///
    /// Skia and EGL go first, then the style object, then the Wayland objects
    /// they were built on; whatever was pacing itself on this surface's frame
    /// callbacks stops waiting for one.
    pub fn destroy(&mut self) {
        use wayland_client::Proxy;

        let Some(subsurface) = self.subsurface.take() else {
            return;
        };
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
        drop(self.base_surface.take_skia_surface());
        self.base_surface.take_surface_style();
        subsurface.destroy();
        let surface = self.base_surface.wl_surface();
        crate::app_runner::AppContext::forget_surface(&surface.id());
        surface.destroy();
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

    /// Draw on the surface using a callback. The buffer it produces maps the
    /// surface if it was unmapped.
    pub fn draw<F>(&self, draw_fn: F)
    where
        F: FnOnce(&skia_safe::Canvas),
    {
        self.base_surface.draw(draw_fn);
        self.mapped.set(true);
    }

    /// See [`BaseWaylandSurface::claim_with_next_buffer`].
    pub fn claim_with_next_buffer(&self, claim: super::common::BufferClaim) {
        self.base_surface.claim_with_next_buffer(claim);
    }

    /// Register a callback to be called on every compositor frame
    pub fn on_frame<F>(&self, callback: F)
    where
        F: FnMut() + 'static,
    {
        self.base_surface.on_frame(callback);
    }
}
