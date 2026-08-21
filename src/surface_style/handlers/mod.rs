use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::{
    state::Backend,
    surface_style::gen::otto_surface_style_manager_v1::{self, OttoSurfaceStyleManagerV1},
    Otto,
};
use layers::prelude::{Spring, TimingFunction, Transition};

use super::protocol::{SurfaceStyle, SurfaceStyleHandler};

pub mod timing_function;

/// User data for surface style
pub struct OttoLayerUserData {
    pub layer_id: smithay::reexports::wayland_server::backend::ObjectId,
}

// Helper to convert wl_fixed to f32 (protocol now sends f64)
fn wl_fixed_to_f32(fixed: f64) -> f32 {
    fixed as f32
}

// Decode a clip_mode argument into a plain boolean. Both set_masks_to_bounds
// and set_clip_children take the same enum, so they share the decoding — and a
// value the client made up is reported as None rather than silently treated as
// "disabled", which would leave the surface looking clipped-or-not at random.
fn clip_mode_enabled(
    mode: smithay::reexports::wayland_server::WEnum<
        crate::surface_style::gen::otto_surface_style_v1::ClipMode,
    >,
) -> Option<bool> {
    use crate::surface_style::gen::otto_surface_style_v1::ClipMode;

    match mode.into_result().ok() {
        Some(ClipMode::Disabled) => Some(false),
        Some(ClipMode::Enabled) => Some(true),
        _ => None,
    }
}

// Helper to find active transaction for a client
fn find_active_transaction_for_client<BackendData: Backend>(
    state: &Otto<BackendData>,
    client: &Client,
) -> Option<smithay::reexports::wayland_server::backend::ObjectId> {
    state
        .style_transactions
        .iter()
        .find(|(_, txn)| {
            !txn.committed && txn.wl_style_transaction.client().map(|c| c.id()) == Some(client.id())
        })
        .map(|(id, _)| id.clone())
}

// Helper to accumulate a layer change in a transaction
fn accumulate_change<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    txn_id: smithay::reexports::wayland_server::backend::ObjectId,
    change: layers::engine::AnimatedNodeChange,
) {
    if let Some(txn) = state.style_transactions.get_mut(&txn_id) {
        tracing::debug!(
            "accumulate_change: total changes now {}",
            txn.accumulated_changes.len() + 1
        );
        txn.accumulated_changes.push(change);
    }
}

// Helper to trigger window redraw after layer property change
fn trigger_window_update<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    surface_id: &smithay::reexports::wayland_server::backend::ObjectId,
) {
    if let Some(window) = state.workspaces.get_window_for_surface(surface_id).cloned() {
        state.update_window_view(&window);
    }
}

/// Re-derive whether the compositor draws anything for `surface_id`'s window
/// that the client's buffer does not already contain — a background colour, a
/// backdrop blur, a rounded clip, a border — and take the window off its
/// scanout plane if it is on one.
///
/// A material lives on the window's own scene layer, and promotion picks how
/// to make room for the plane: a plain window has that whole layer hidden, a
/// material one only has its texture blanked so the material keeps rendering
/// (see `Workspaces::set_scanout_windows`). A window already promoted the
/// plain way when the material arrives has the wrong treatment and would show
/// no frost, so it is demoted here; the next candidate pass re-promotes it
/// with the material intact.
///
/// Only the window's own surface counts: a subsurface keeps rendering in the
/// windows plane even while the root is promoted, so a material on one is not
/// at risk.
fn refresh_window_material<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    surface_id: &smithay::reexports::wayland_server::backend::ObjectId,
) {
    let material = state
        .surfaces_style
        .get(surface_id)
        .map(|styles| {
            styles
                .iter()
                .any(|s| s.background_alpha > 0.0 || s.background_blur || s.rounded || s.bordered)
        })
        .unwrap_or(false);

    let Some(window) = state.workspaces.get_window_for_surface(surface_id).cloned() else {
        return;
    };
    if window.set_has_material(material) == material {
        return;
    }
    if material {
        // It may already be on a plane: take it back this frame rather than
        // waiting for the next candidate pass.
        state.workspaces.remove_scanout_window(surface_id);
    }
}

// Helper to commit a transaction and apply all accumulated changes
fn commit_transaction<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    txn_id: smithay::reexports::wayland_server::backend::ObjectId,
) {
    let Some(txn) = state.style_transactions.get_mut(&txn_id) else {
        return;
    };

    txn.committed = true;

    // Use client-configured timing function, or create default from duration
    let mut transition = if let Some(mut trans) = txn.timing_function.take() {
        // Update timing function duration (timing functions are created with 0.0 duration)
        if let Some(duration) = txn.duration {
            // Recreate the timing function with the correct duration
            trans.timing = match trans.timing {
                TimingFunction::Easing(easing, _) => {
                    tracing::debug!("Transaction commit: Easing timing, duration={}s", duration);
                    TimingFunction::Easing(easing, duration)
                }
                TimingFunction::Spring(s) => {
                    if txn.spring_uses_duration {
                        // Duration-based spring - use stored bounce and velocity
                        if let Some(bounce) = txn.spring_bounce {
                            tracing::debug!(
                                "Transaction commit: duration-based spring: duration={}s, bounce={}, initial_velocity={}",
                                duration,
                                bounce,
                                txn.spring_initial_velocity
                            );
                            TimingFunction::Spring(Spring::with_duration_bounce_and_velocity(
                                duration,
                                bounce,
                                txn.spring_initial_velocity,
                            ))
                        } else {
                            tracing::debug!(
                                "Transaction commit: spring fallback (no bounce), duration={}s",
                                duration
                            );
                            // Fallback if bounce not set
                            TimingFunction::Spring(Spring::with_duration_and_bounce(duration, 0.0))
                        }
                    } else {
                        tracing::debug!(
                            "Transaction commit: physics-based spring (ignoring duration)"
                        );
                        // Physics-based spring from timing function - keep as is
                        TimingFunction::Spring(s)
                    }
                }
                other => other,
            };
        } else {
            tracing::debug!("Transaction commit: timing function present but no duration");
        }
        Some(trans)
    } else {
        tracing::debug!(
            "Transaction commit: no timing function, using default ease_out_quad, duration={:?}",
            txn.duration
        );
        txn.duration.map(Transition::ease_out_quad)
    };

    // Apply delay if configured
    if let Some(delay) = txn.delay {
        if let Some(ref mut trans) = transition {
            trans.delay = delay;
        }
    }

    // Schedule all accumulated changes together
    if !txn.accumulated_changes.is_empty() {
        if let Some(ref trans) = transition {
            // Collect gravity info for diagnostics (before mutable borrow of state)
            let gravities: Vec<_> = state
                .surfaces_style
                .values()
                .flatten()
                .map(|s| (s.wl_style.id(), s.contents_gravity))
                .collect();
            tracing::debug!(
                "Committing animation: {} changes, duration={:?}s, delay={:?}s, timing={:?}, surface_gravities={:?}",
                txn.accumulated_changes.len(),
                txn.duration,
                txn.delay,
                trans.timing,
                gravities,
            );
            // Create animation and store it in the transaction
            let animation = state
                .layers_engine
                .add_animation_from_transition(trans, false);

            txn.animation = Some(animation);

            state
                .layers_engine
                .schedule_changes(&txn.accumulated_changes, animation);

            // Add on_finish callback if completion event requested
            if txn.send_completion {
                let wl_txn = txn.wl_style_transaction.clone();
                state.layers_engine.on_animation_finish(
                    animation,
                    move |_| {
                        wl_txn.completed();
                    },
                    false,
                );
            }

            state.layers_engine.start_animation(animation, trans.delay);
        } else {
            tracing::debug!(
                "Committing {} changes immediately (no animation)",
                txn.accumulated_changes.len()
            );
            // No animation - send completion immediately if requested
            if txn.send_completion {
                if let Some(txn) = state.style_transactions.remove(&txn_id) {
                    txn.wl_style_transaction.completed();
                }
            }
        }
        // If no transition, changes were already applied immediately via set_* methods
    } else {
        // No changes - send completion immediately if requested
        if txn.send_completion {
            tracing::info!("No changes, sending completed event immediately");
            if let Some(txn) = state.style_transactions.remove(&txn_id) {
                txn.wl_style_transaction.completed();
            }
        }
    }
}

pub mod style;
pub mod transactions;

/// Create the sc_layer_shell global
pub fn create_style_manager_global<BackendData: Backend + 'static>(
    display: &DisplayHandle,
) -> smithay::reexports::wayland_server::backend::GlobalId {
    // Version 2 added set_clip_children, version 3 output placement and
    // output-relative sizing; clients binding an older version keep working
    // because wayland only ever hands them the requests their version knows.
    display.create_global::<Otto<BackendData>, OttoSurfaceStyleManagerV1, _>(3, ())
}

impl<BackendData: Backend> GlobalDispatch<OttoSurfaceStyleManagerV1, ()> for Otto<BackendData> {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OttoSurfaceStyleManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl<BackendData: Backend> Dispatch<OttoSurfaceStyleManagerV1, ()> for Otto<BackendData> {
    fn request(
        state: &mut Self,
        _client: &Client,
        _shell: &OttoSurfaceStyleManagerV1,
        request: otto_surface_style_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            otto_surface_style_manager_v1::Request::GetSurfaceStyle { id, surface } => {
                // Per protocol spec: "It can augment any surface type"
                // We just verify the surface is alive and valid
                if !surface.is_alive() {
                    // shell.post_error(
                    //     otto_style_surface_v1::Error::InvalidSurface,
                    //     "Surface does not exist",
                    // );
                    return;
                }

                // Create lay-rs layer
                let layer = state.layers_engine.new_layer();

                // Set some defaults
                layer.set_layout_style(layers::taffy::Style {
                    position: layers::taffy::Position::Absolute,
                    ..Default::default()
                });

                // Initialize the wayland object - we'll use a placeholder ID for now
                let wl_layer = data_init.init(
                    id,
                    OttoLayerUserData {
                        layer_id: surface.id(), // Temporary placeholder, will be overwritten
                    },
                );

                // Now get the actual layer ID and set it properly
                let layer_id = wl_layer.id();
                let layer_id_str = format!("surface_style_{:?}", layer_id);
                layer.set_key(layer_id_str.clone());

                // Create compositor state
                let surface_style = SurfaceStyle {
                    wl_style: wl_layer.clone(),
                    layer: layer.clone(),
                    surface: surface.clone(),
                    z_order: crate::surface_style::OttoSurfaceStyleZOrder::default(),
                    contents_gravity: crate::surface_style::ContentsGravity::default(),
                    shared_gravity: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
                    client_owns_size: false,
                    background_alpha: 0.0,
                    background_blur: false,
                    rounded: false,
                    bordered: false,
                    output_centered: false,
                    last_size_px: None,
                };

                // Notify handler
                SurfaceStyleHandler::new_surface_style(state, surface_style);
            }

            otto_surface_style_manager_v1::Request::BeginTransaction { id } => {
                use super::protocol::StyleTransaction;

                let wl_transaction = data_init.init(id, ());
                let txn_id = wl_transaction.id();
                let transaction = StyleTransaction {
                    wl_style_transaction: wl_transaction.clone(),
                    duration: None,
                    delay: None,
                    timing_function: None,
                    spring_uses_duration: false,
                    spring_bounce: None,
                    spring_initial_velocity: 0.0,
                    send_completion: false,
                    accumulated_changes: Vec::new(),
                    animation: None,
                    committed: false,
                };

                state.style_transactions.insert(txn_id.clone(), transaction);
            }

            otto_surface_style_manager_v1::Request::CreateTimingFunction { id } => {
                use timing_function::ScTimingFunctionData;

                let timing_data = ScTimingFunctionData::new();

                data_init.init(id, timing_data);
            }

            otto_surface_style_manager_v1::Request::Destroy => {
                // Nothing to do
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clip_mode_enabled;
    use crate::surface_style::gen::otto_surface_style_v1::ClipMode;
    use smithay::reexports::wayland_server::WEnum;

    #[test]
    fn clip_mode_maps_to_bool() {
        assert_eq!(
            clip_mode_enabled(WEnum::Value(ClipMode::Enabled)),
            Some(true)
        );
        assert_eq!(
            clip_mode_enabled(WEnum::Value(ClipMode::Disabled)),
            Some(false)
        );
    }

    #[test]
    fn unknown_clip_mode_is_rejected() {
        // A value outside the enum must not quietly become "disabled": the
        // caller warns and leaves the layer as it was.
        assert_eq!(clip_mode_enabled(WEnum::Unknown(42)), None);
    }
}
