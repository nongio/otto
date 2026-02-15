use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::{Otto, state::Backend, surface_style::gen::otto_surface_style_manager_v1::{self, OttoSurfaceStyleManagerV1}};
use layers::prelude::{Spring, TimingFunction, Transition};

use super::protocol::{
    SurfaceStyle, SurfaceStyleHandler,
};

pub mod timing_function;

/// User data for surface style
pub struct OttoLayerUserData {
    pub layer_id: smithay::reexports::wayland_server::backend::ObjectId,
}

// Helper to convert wl_fixed to f32 (protocol now sends f64)
fn wl_fixed_to_f32(fixed: f64) -> f32 {
    fixed as f32
}

// Helper to find active transaction for a client
fn find_active_transaction_for_client<BackendData: Backend>(
    state: &Otto<BackendData>,
    client: &Client,
) -> Option<smithay::reexports::wayland_server::backend::ObjectId> {
    state
        .style_transactions
        .iter()
        .find(|(_, txn)| txn.wl_style_transaction.client().map(|c| c.id()) == Some(client.id()))
        .map(|(id, _)| id.clone())
}

// Helper to accumulate a layer change in a transaction
fn accumulate_change<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    txn_id: smithay::reexports::wayland_server::backend::ObjectId,
    change: layers::engine::AnimatedNodeChange,
) {
    if let Some(txn) = state.style_transactions.get_mut(&txn_id) {
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

// Helper to commit a transaction and apply all accumulated changes
fn commit_transaction<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    txn_id: smithay::reexports::wayland_server::backend::ObjectId,
) {
    let Some(txn) = state.style_transactions.get_mut(&txn_id) else {
        return;
    };

    // Use client-configured timing function, or create default from duration
    let mut transition = if let Some(mut trans) = txn.timing_function {
        // Update timing function duration (timing functions are created with 0.0 duration)
        if let Some(duration) = txn.duration {
            // Recreate the timing function with the correct duration
            trans.timing = match trans.timing {
                TimingFunction::Easing(easing, _) => TimingFunction::Easing(easing, duration),
                TimingFunction::Spring(_) => {
                    if txn.spring_uses_duration {
                        // Duration-based spring - use stored bounce and velocity
                        if let Some(bounce) = txn.spring_bounce {
                            tracing::debug!(
                                "Creating duration-based spring: duration={}s, bounce={}, initial_velocity={}",
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
                            // Fallback if bounce not set
                            TimingFunction::Spring(Spring::with_duration_and_bounce(duration, 0.0))
                        }
                    } else {
                        // Physics-based spring from timing function - keep as is
                        trans.timing
                    }
                }
            };
        }
        Some(trans)
    } else {
        txn.duration
            .map(|duration| Transition::ease_out_quad(duration))
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
            // No animation - send completion immediately if requested
            if txn.send_completion {
                if let Some(txn) = state.style_transactions.remove(&txn_id) {
                    txn.wl_style_transaction.completed();
                }
                return;
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
            return;
        }
    }
}

pub mod style;
pub mod transactions;

/// Create the sc_layer_shell global
pub fn create_style_manager_global<BackendData: Backend + 'static>(
    display: &DisplayHandle,
) -> smithay::reexports::wayland_server::backend::GlobalId {
    display.create_global::<Otto<BackendData>, OttoSurfaceStyleManagerV1, _>(1, ())
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
        shell: &OttoSurfaceStyleManagerV1,
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
                };

                state.style_transactions.insert(txn_id.clone(), transaction);
            }

            otto_surface_style_manager_v1::Request::CreateTimingFunction { id } => {
                use timing_function::ScTimingFunctionData;

                // Create default timing function (linear)
                let timing_data = ScTimingFunctionData {
                    timing: layers::prelude::TimingFunction::linear(0.0),
                    spring_uses_duration: false,
                    spring_bounce: None,
                    spring_initial_velocity: 0.0,
                };

                data_init.init(id, timing_data);
            }

            otto_surface_style_manager_v1::Request::Destroy => {
                // Nothing to do
            }
        }
    }
}
