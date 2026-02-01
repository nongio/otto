use std::sync::atomic::{AtomicU32, Ordering};

use layers::prelude::Layer;
use smithay::{
    desktop::LayerSurface,
    output::Output,
    reexports::wayland_server::{backend::ObjectId, Resource},
    utils::{Logical, Rectangle, Size},
    wayland::shell::wlr_layer::{
        Anchor, ExclusiveZone, KeyboardInteractivity, Layer as WlrLayer,
        LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
    },
};

/// Compositor-owned state for a wlr-layer-shell surface.
///
/// This struct bridges Smithay's `LayerSurface` with the compositor's
/// lay_rs scene graph. It tracks configure serials, geometry, and the
/// associated rendering layer.
#[derive(Debug)]
pub struct LayerShellSurface {
    /// The underlying Smithay layer surface handle
    layer_surface: LayerSurface,
    /// The lay_rs layer used for rendering this surface
    pub layer: Layer,
    /// The output this surface is bound to
    output: Output,
    /// The wlr-layer-shell layer (background, bottom, top, overlay)
    wlr_layer: WlrLayer,
    /// Namespace provided by the client (e.g., "panel", "wallpaper")
    namespace: String,
    /// Last configure serial we sent
    last_configure_serial: AtomicU32,
    /// Computed geometry after layout (position + size in output coordinates)
    geometry: Rectangle<i32, Logical>,
}

impl LayerShellSurface {
    /// Create a new LayerShellSurface wrapper
    pub fn new(
        layer_surface: LayerSurface,
        layer: Layer,
        output: Output,
        wlr_layer: WlrLayer,
        namespace: String,
    ) -> Self {
        Self {
            layer_surface,
            layer,
            output,
            wlr_layer,
            namespace,
            last_configure_serial: AtomicU32::new(0),
            geometry: Rectangle::default(),
        }
    }

    /// Get the underlying Smithay LayerSurface
    pub fn layer_surface(&self) -> &LayerSurface {
        &self.layer_surface
    }

    /// Get the wlr_layer_surface handle
    pub fn wlr_layer_surface(&self) -> &WlrLayerSurface {
        self.layer_surface.layer_surface()
    }

    /// Get the surface ObjectId for use as a map key
    pub fn id(&self) -> ObjectId {
        self.layer_surface.wl_surface().id()
    }

    /// Get the output this surface is bound to
    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Get the wlr-layer-shell layer
    pub fn wlr_layer(&self) -> WlrLayer {
        self.wlr_layer
    }

    /// Get the namespace
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Get the computed geometry
    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        self.geometry
    }

    /// Update the computed geometry
    pub fn set_geometry(&mut self, geometry: Rectangle<i32, Logical>) {
        self.geometry = geometry;
    }

    /// Get the last configure serial
    pub fn last_configure_serial(&self) -> u32 {
        self.last_configure_serial.load(Ordering::SeqCst)
    }

    /// Set the last configure serial
    pub fn set_last_configure_serial(&self, serial: u32) {
        self.last_configure_serial.store(serial, Ordering::SeqCst);
    }

    /// Check if this surface can receive keyboard focus
    pub fn can_receive_keyboard_focus(&self) -> bool {
        self.layer_surface.can_receive_keyboard_focus()
    }

    /// Get the keyboard interactivity mode from cached state
    pub fn keyboard_interactivity(&self) -> KeyboardInteractivity {
        smithay::wayland::compositor::with_states(self.layer_surface.wl_surface(), |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current()
        })
        .keyboard_interactivity
    }

    /// Get the anchor flags from cached state
    pub fn anchor(&self) -> Anchor {
        smithay::wayland::compositor::with_states(self.layer_surface.wl_surface(), |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current()
        })
        .anchor
    }

    /// Get the exclusive zone from cached state
    pub fn exclusive_zone(&self) -> ExclusiveZone {
        smithay::wayland::compositor::with_states(self.layer_surface.wl_surface(), |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current()
        })
        .exclusive_zone
    }

    /// Get the margin from cached state
    pub fn margin(&self) -> (i32, i32, i32, i32) {
        let state =
            smithay::wayland::compositor::with_states(self.layer_surface.wl_surface(), |states| {
                *states
                    .cached_state
                    .get::<LayerSurfaceCachedState>()
                    .current()
            });
        (
            state.margin.top,
            state.margin.right,
            state.margin.bottom,
            state.margin.left,
        )
    }

    /// Get the requested size from cached state (0 means auto-size on that axis)
    pub fn requested_size(&self) -> Size<i32, Logical> {
        smithay::wayland::compositor::with_states(self.layer_surface.wl_surface(), |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current()
        })
        .size
    }

    /// Check if the surface is anchored to all four edges (fullscreen-like)
    pub fn is_anchored_to_all_edges(&self) -> bool {
        let anchor = self.anchor();
        anchor.contains(Anchor::TOP)
            && anchor.contains(Anchor::BOTTOM)
            && anchor.contains(Anchor::LEFT)
            && anchor.contains(Anchor::RIGHT)
    }

    /// Compute the geometry for this surface based on anchors, margins, and output size
    pub fn compute_geometry(
        &self,
        output_geometry: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let anchor = self.anchor();
        let (margin_top, margin_right, margin_bottom, margin_left) = self.margin();
        let requested_size = self.requested_size();

        let mut width = requested_size.w;
        let mut height = requested_size.h;

        // Handle horizontal anchoring
        let anchor_left = anchor.contains(Anchor::LEFT);
        let anchor_right = anchor.contains(Anchor::RIGHT);

        let x = if anchor_left && anchor_right {
            // Stretch horizontally
            width = output_geometry.size.w - margin_left - margin_right;
            output_geometry.loc.x + margin_left
        } else if anchor_left {
            output_geometry.loc.x + margin_left
        } else if anchor_right {
            output_geometry.loc.x + output_geometry.size.w - width - margin_right
        } else {
            // Center horizontally
            output_geometry.loc.x + (output_geometry.size.w - width) / 2
        };

        // Handle vertical anchoring
        let anchor_top = anchor.contains(Anchor::TOP);
        let anchor_bottom = anchor.contains(Anchor::BOTTOM);

        let y = if anchor_top && anchor_bottom {
            // Stretch vertically
            height = output_geometry.size.h - margin_top - margin_bottom;
            output_geometry.loc.y + margin_top
        } else if anchor_top {
            output_geometry.loc.y + margin_top
        } else if anchor_bottom {
            output_geometry.loc.y + output_geometry.size.h - height - margin_bottom
        } else {
            // Center vertically
            output_geometry.loc.y + (output_geometry.size.h - height) / 2
        };

        // Ensure non-negative dimensions
        width = width.max(0);
        height = height.max(0);

        Rectangle::new((x, y).into(), (width, height).into())
    }
}

impl PartialEq for LayerShellSurface {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
