use smithay::reexports::wayland_server;
use wayland_server::Resource;

use crate::{state::Backend, Otto};

use smithay::reexports::wayland_server::protocol::*;

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;

    wayland_scanner::generate_interfaces!("./protocols/otto-surface-style-unstable-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/otto-surface-style-unstable-v1.xml");
}

pub use gen::otto_style_transaction_v1::OttoStyleTransactionV1 as ZTransactionV1;
pub use gen::otto_surface_style_v1::OttoSurfaceStyleV1 as ZSurfaceStyleV1;

/// Z-order configuration for surface style relative to parent surface content
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OttoSurfaceStyleZOrder {
    BelowSurface,
    AboveSurface,
}

impl Default for OttoSurfaceStyleZOrder {
    fn default() -> Self {
        Self::AboveSurface
    }
}

/// Compositor-side layer state (pure augmentation, no wl_surface)
#[derive(Debug, Clone)]
pub struct SurfaceStyle {
    /// The Wayland protocol object
    pub wl_style: ZSurfaceStyleV1,

    /// The lay-rs layer backing this augmentation
    pub layer: layers::prelude::Layer,

    /// Surface being augmented (any role)
    pub surface: wl_surface::WlSurface,

    /// Z-order relative to surface content
    pub z_order: OttoSurfaceStyleZOrder,
}

impl PartialEq for SurfaceStyle {
    fn eq(&self, other: &Self) -> bool {
        self.wl_style.id() == other.wl_style.id()
    }
}

/// Transaction state for batching animated changes
pub struct StyleTransaction {
    /// The protocol object
    pub wl_style_transaction: ZTransactionV1,

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

    pub animation: Option<layers::engine::AnimationRef>,
}

impl Clone for StyleTransaction {
    fn clone(&self) -> Self {
        Self {
            wl_style_transaction: self.wl_style_transaction.clone(),
            duration: self.duration,
            delay: self.delay,
            timing_function: self.timing_function,
            spring_uses_duration: self.spring_uses_duration,
            spring_bounce: self.spring_bounce,
            spring_initial_velocity: self.spring_initial_velocity,
            send_completion: self.send_completion,
            accumulated_changes: self.accumulated_changes.clone(),
            animation: self.animation,
        }
    }
}

impl std::fmt::Debug for StyleTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StyleTransaction")
            .field("duration", &self.duration)
            .field("delay", &self.delay)
            .field("send_completion", &self.send_completion)
            .field("num_changes", &self.accumulated_changes.len())
            .finish()
    }
}

/// Handler for Surface Style protocol events
pub trait SurfaceStyleHandler {
    /// Create a new layer surface
    fn new_surface_style(&mut self, layer: SurfaceStyle);

    /// A layer was destroyed
    fn destroy_surface_style(&mut self, _layer: &SurfaceStyle) {}
}

impl<BackendData: Backend> SurfaceStyleHandler for Otto<BackendData> {
    fn new_surface_style(&mut self, mut layer: SurfaceStyle) {
        let surface_id = layer.surface.id();

        if let Some(rendering_layer) = self.surface_layers.get(&surface_id).cloned() {
            layer.layer = rendering_layer.clone();
            self.surface_layers
                .insert(surface_id.clone(), rendering_layer);
        } else {
            self.surface_layers
                .insert(surface_id.clone(), layer.layer.clone());
        }

        self.surfaces_style
            .entry(surface_id.clone())
            .or_default()
            .push(layer);
    }

    fn destroy_surface_style(&mut self, layer: &SurfaceStyle) {
        // Remove from surface's list
        let surface_id = layer.surface.id();
        if let Some(layers) = self.surfaces_style.get_mut(&surface_id) {
            layers.retain(|l| l.wl_style.id() != layer.wl_style.id());
            if layers.is_empty() {
                self.surfaces_style.remove(&surface_id);
            }
        }

        // Remove from surface_layers map (rendering layer reference)
        self.surface_layers.remove(&surface_id);

        tracing::info!("Destroyed surface style {:?}", layer.wl_style.id());
    }
}
