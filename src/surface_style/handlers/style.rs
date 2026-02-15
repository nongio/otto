use layers::types::BorderRadius;
use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use crate::{
    Otto, config::Config, state::Backend, surface_style::handlers::{
        OttoLayerUserData, accumulate_change, find_active_transaction_for_client, trigger_window_update, wl_fixed_to_f32
    }
};

use super::super::protocol::{
    gen::otto_surface_style_v1::{self, OttoSurfaceStyleV1},
    SurfaceStyleHandler,
};

impl<BackendData: Backend> Dispatch<OttoSurfaceStyleV1, OttoLayerUserData> for Otto<BackendData> {
    fn request(
        state: &mut Self,
        _client: &Client,
        layer_obj: &OttoSurfaceStyleV1,
        request: otto_surface_style_v1::Request,
        _data: &OttoLayerUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let layer_id = layer_obj.id();

        // Find the surface style in any parent's list
        let surface_style = state
            .surfaces_style
            .values()
            .flat_map(|layers| layers.iter())
            .find(|layer| layer.wl_style.id() == layer_id);

        let Some(sstyle) = surface_style else {
            tracing::warn!("Layer {:?} not found in state", layer_id);
            return;
        };

        // Find active transaction for this client (if any)
        let active_transaction = find_active_transaction_for_client(state, _client);

        match request {
            otto_surface_style_v1::Request::SetPosition { x, y } => {
                let x = wl_fixed_to_f32(x);
                let y = wl_fixed_to_f32(y);

                if let Some(txn_id) = active_transaction {
                    // Accumulate change in transaction
                    let change = sstyle
                        .layer
                        .change_position(layers::types::Point { x, y });
                    accumulate_change(state, txn_id, change);
                } else {
                    // Apply immediately
                    sstyle.layer.set_position((x, y), None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetSize { width, height } => {
                let width = wl_fixed_to_f32(width);
                let height = wl_fixed_to_f32(height);

                if let Some(txn_id) = active_transaction {
                    let change = sstyle
                        .layer
                        .change_size(layers::types::Size::points(width, height));
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle
                        .layer
                        .set_size(layers::types::Size::points(width, height), None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetScale { x, y } => {
                let x = wl_fixed_to_f32(x);
                let y = wl_fixed_to_f32(y);

                if let Some(txn_id) = active_transaction {
                    let change = sstyle.layer.change_scale(layers::types::Point { x, y });
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle.layer.set_scale((x, y), None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetAnchorPoint { x, y } => {
                let x = wl_fixed_to_f32(x);
                let y = wl_fixed_to_f32(y);

                if let Some(txn_id) = active_transaction {
                    let change = sstyle
                        .layer
                        .change_anchor_point(layers::types::Point { x, y });
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle.layer.set_anchor_point((x, y), None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetOpacity { opacity } => {
                let opacity = wl_fixed_to_f32(opacity).clamp(0.0, 1.0);

                if let Some(txn_id) = active_transaction {
                    let change = sstyle.layer.change_opacity(opacity);
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle.layer.set_opacity(opacity, None);
                }
            }

            otto_surface_style_v1::Request::SetBackgroundColor {
                red,
                green,
                blue,
                alpha,
            } => {
                let red = wl_fixed_to_f32(red);
                let green = wl_fixed_to_f32(green);
                let blue = wl_fixed_to_f32(blue);
                let alpha = wl_fixed_to_f32(alpha);

                if let Some(txn_id) = active_transaction {
                    let color = layers::types::Color::new_rgba(red, green, blue, alpha);
                    let change = sstyle.layer.change_background_color(color);
                    accumulate_change(state, txn_id, change);
                } else {
                    let color = layers::types::Color::new_rgba(red, green, blue, alpha);
                    sstyle.layer.set_background_color(color, None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetCornerRadius { radius } => {
                let radius = wl_fixed_to_f32(radius);
                let screen_scale = Config::with(|c| c.screen_scale) as f32;
                let scaled_radius = radius * screen_scale;

                if let Some(txn_id) = active_transaction {
                    let change = sstyle.layer.change_border_corner_radius(scaled_radius);
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle
                        .layer
                        .set_border_corner_radius(BorderRadius::new_single(scaled_radius), None);
                    // trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetBorder {
                width,
                red,
                green,
                blue,
                alpha,
            } => {
                let width = wl_fixed_to_f32(width);
                let red = wl_fixed_to_f32(red);
                let green = wl_fixed_to_f32(green);
                let blue = wl_fixed_to_f32(blue);
                let alpha = wl_fixed_to_f32(alpha);

                let color = layers::types::Color::new_rgba(red, green, blue, alpha);

                if let Some(txn_id) = active_transaction {
                    // Create both changes before accumulating
                    let layer = sstyle.layer.clone();
                    let width_change = layer.change_border_width(width);
                    let color_change = layer.change_border_color(color);

                    // Accumulate both changes
                    accumulate_change(state, txn_id.clone(), width_change);
                    accumulate_change(state, txn_id, color_change);
                } else {
                    // Apply immediately
                    sstyle.layer.set_border_width(width, None);
                    sstyle.layer.set_border_color(color, None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetShadow {
                opacity,
                radius,
                offset_x,
                offset_y,
                red,
                green,
                blue,
            } => {
                let opacity = wl_fixed_to_f32(opacity);
                let radius = wl_fixed_to_f32(radius);
                let offset_x = wl_fixed_to_f32(offset_x);
                let offset_y = wl_fixed_to_f32(offset_y);
                let red = wl_fixed_to_f32(red);
                let green = wl_fixed_to_f32(green);
                let blue = wl_fixed_to_f32(blue);

                // Shadow properties in lay-rs
                sstyle.layer.set_shadow_color(
                    layers::prelude::Color::new_rgba255(
                        (red * 255.0) as u8,
                        (green * 255.0) as u8,
                        (blue * 255.0) as u8,
                        (opacity * 255.0) as u8,
                    ),
                    None,
                );
                sstyle.layer.set_shadow_radius(radius, None);
                sstyle.layer.set_shadow_offset((offset_x, offset_y), None);

                trigger_window_update(state, &sstyle.surface.id());
            }

            otto_surface_style_v1::Request::SetHidden { visibility } => {
                use super::super::protocol::gen::otto_surface_style_v1::Visibility;

                let hidden = match visibility.into_result().ok() {
                    Some(Visibility::Visible) => false,
                    Some(Visibility::Hidden) => true,
                    _ => {
                        tracing::warn!("Invalid visibility value: {:?}", visibility);
                        return;
                    }
                };

                // Hidden doesn't animate, always apply immediately
                sstyle.layer.set_hidden(hidden);
                trigger_window_update(state, &sstyle.surface.id());
            }

            otto_surface_style_v1::Request::SetMasksToBounds { clip_mode } => {
                use super::super::protocol::gen::otto_surface_style_v1::ClipMode;

                let masks_to_bounds = match clip_mode.into_result().ok() {
                    Some(ClipMode::Disabled) => false,
                    Some(ClipMode::Enabled) => true,
                    _ => {
                        tracing::warn!("Invalid clip_mode value: {:?}", clip_mode);
                        return;
                    }
                };

                sstyle.layer.set_clip_content(masks_to_bounds, None);
            }

            otto_surface_style_v1::Request::SetBlendMode { mode } => {
                use super::super::protocol::gen::otto_surface_style_v1::BlendMode;
                use layers::types::BlendMode as LayrsBlendMode;

                let blend_mode = match mode.into_result().ok() {
                    Some(BlendMode::Normal) => LayrsBlendMode::default(),
                    Some(BlendMode::BackgroundBlur) => LayrsBlendMode::BackgroundBlur,
                    _ => {
                        tracing::warn!("Invalid blend_mode value: {:?}", mode);
                        return;
                    }
                };

                // Blend mode doesn't animate, always apply immediately
                sstyle.layer.set_blend_mode(blend_mode);
                trigger_window_update(state, &sstyle.surface.id());
            }

            otto_surface_style_v1::Request::SetZOrder { z_order } => {
                use super::super::protocol::gen::otto_surface_style_v1::ZOrder;
                use crate::surface_style::OttoSurfaceStyleZOrder;

                // Update z-order configuration
                let new_z_order = match z_order.into_result().ok() {
                    Some(ZOrder::BelowSurface) => OttoSurfaceStyleZOrder::BelowSurface,
                    Some(ZOrder::AboveSurface) => OttoSurfaceStyleZOrder::AboveSurface,
                    _ => {
                        tracing::warn!("Invalid z_order value: {:?}", z_order);
                        return;
                    }
                };

                // Find window and reattach layer
                let surface_id = sstyle.surface.id();
                if let Some(window) = state
                    .workspaces
                    .get_window_for_surface(&surface_id)
                    .cloned()
                {
                    // TODO: lay-rs doesn't support remove_sublayer yet
                    // For now we just add it again (this may cause duplication)
                    // window.layer().remove_sublayer(&sstyle.layer);

                    // Reattach based on new z-order
                    // TODO: lay-rs doesn't support insert_sublayer_at yet
                    // For now we can only add to the top
                    match new_z_order {
                        OttoSurfaceStyleZOrder::BelowSurface => {
                            window.layer().add_sublayer(&sstyle.layer);
                        }
                        OttoSurfaceStyleZOrder::AboveSurface => {
                            window.layer().add_sublayer(&sstyle.layer);
                        }
                    }

                    // Update stored z-order
                    if let Some(layers) = state.surfaces_style.get_mut(&surface_id) {
                        if let Some(layer) = layers.iter_mut().find(|l| l.wl_style.id() == layer_id)
                        {
                            layer.z_order = new_z_order;
                        }
                    }

                    tracing::debug!("Updated surface style z-order to {:?}", new_z_order);
                }
            }

            otto_surface_style_v1::Request::Destroy => {
                // Handled by destructor
            }

            _ => {
                tracing::warn!("Unimplemented surface style request: {:?}", request);
            }
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &OttoSurfaceStyleV1,
        _data: &OttoLayerUserData,
    ) {
        let layer_id = resource.id();

        // Find and remove the surface style from the appropriate parent's list
        let surface_style = state
            .surfaces_style
            .values()
            .flat_map(|layers| layers.iter())
            .find(|layer| layer.wl_style.id() == layer_id)
            .cloned();

        if let Some(surface_style) = surface_style {
            SurfaceStyleHandler::destroy_surface_style(state, &surface_style);
        }
    }
}
