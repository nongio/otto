use smithay::reexports::wayland_server;
use wayland_server::Resource;

use crate::{state::Backend, Otto};

use smithay::reexports::wayland_server::protocol::*;

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;

    wayland_scanner::generate_interfaces!("./protocols/otto-scene-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/otto-scene-v1.xml");
}

pub use gen::otto_scene_surface_v1::OttoSceneSurfaceV1 as ZscLayerV1;
pub use gen::otto_transaction_v1::OttoTransactionV1;

/// Z-order configuration for sc-layer relative to parent surface content
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScLayerZOrder {
    BelowSurface,
    AboveSurface,
}

impl Default for ScLayerZOrder {
    fn default() -> Self {
        Self::AboveSurface
    }
}

/// Compositor-side layer state (pure augmentation, no wl_surface)
#[derive(Debug, Clone)]
pub struct ScLayer {
    /// The Wayland protocol object
    pub wl_layer: ZscLayerV1,

    /// The lay-rs layer backing this augmentation
    pub layer: layers::prelude::Layer,

    /// Surface being augmented (any role)
    pub surface: wl_surface::WlSurface,

    /// Z-order relative to surface content
    pub z_order: ScLayerZOrder,
}

impl PartialEq for ScLayer {
    fn eq(&self, other: &Self) -> bool {
        self.wl_layer.id() == other.wl_layer.id()
    }
}

/// Transaction state for batching animated changes
pub struct ScTransaction {
    /// The protocol object
    pub wl_transaction: OttoTransactionV1,

    /// Animation duration in seconds (None = immediate)
    pub duration: Option<f32>,

    /// Animation delay in seconds
    pub delay: Option<f32>,

    /// Timing function configured by client
    pub timing_function: Option<layers::prelude::Transition>,

    /// If true, spring animations should use the transaction's duration
    pub spring_uses_duration: bool,

    /// Bounce parameter for duration-based springs
    pub spring_bounce: Option<f32>,

    /// Initial velocity for springs
    pub spring_initial_velocity: f32,

    /// Whether to send completion event
    pub send_completion: bool,

    /// Accumulated layer changes ready for scheduling
    pub accumulated_changes: Vec<layers::engine::AnimatedNodeChange>,
}

impl Clone for ScTransaction {
    fn clone(&self) -> Self {
        Self {
            wl_transaction: self.wl_transaction.clone(),
            duration: self.duration,
            delay: self.delay,
            timing_function: self.timing_function,
            spring_uses_duration: self.spring_uses_duration,
            spring_bounce: self.spring_bounce,
            spring_initial_velocity: self.spring_initial_velocity,
            send_completion: self.send_completion,
            accumulated_changes: self.accumulated_changes.clone(),
        }
    }
}

impl std::fmt::Debug for ScTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScTransaction")
            .field("duration", &self.duration)
            .field("delay", &self.delay)
            .field("send_completion", &self.send_completion)
            .field("num_changes", &self.accumulated_changes.len())
            .finish()
    }
}

/// Handler for sc_layer_shell
pub trait ScLayerShellHandler {
    /// Create a new layer surface
    fn new_layer(&mut self, layer: ScLayer);

    /// A layer was destroyed
    fn destroy_layer(&mut self, _layer: &ScLayer) {}
}

impl<BackendData: Backend> ScLayerShellHandler for Otto<BackendData> {
    fn new_layer(&mut self, mut layer: ScLayer) {
        let surface_id = layer.surface.id();

        if let Some(rendering_layer) = self.surface_layers.get(&surface_id).cloned() {
            layer.layer = rendering_layer.clone();
            self.surface_layers
                .insert(surface_id.clone(), rendering_layer);
        } else {
            self.surface_layers
                .insert(surface_id.clone(), layer.layer.clone());
        }

        self.sc_layers
            .entry(surface_id.clone())
            .or_default()
            .push(layer);
    }

    fn destroy_layer(&mut self, layer: &ScLayer) {
        // Remove from surface's list
        let surface_id = layer.surface.id();
        if let Some(layers) = self.sc_layers.get_mut(&surface_id) {
            layers.retain(|l| l.wl_layer.id() != layer.wl_layer.id());
            if layers.is_empty() {
                self.sc_layers.remove(&surface_id);
            }
        }

        // Remove from surface_layers map (rendering layer reference)
        self.surface_layers.remove(&surface_id);

        tracing::info!("Destroyed sc-layer {:?}", layer.wl_layer.id());
    }
}
