use layers::types::BorderRadius;
use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use crate::{
    config::Config,
    state::Backend,
    surface_style::handlers::{
        accumulate_change, clip_mode_enabled, find_active_transaction_for_client,
        trigger_window_update, wl_fixed_to_f32, OttoLayerUserData,
    },
    Otto,
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

        // Find the surface style in any parent's list — clone it to release the borrow on state
        let surface_style = state
            .surfaces_style
            .values()
            .flat_map(|layers| layers.iter())
            .find(|layer| layer.wl_style.id() == layer_id)
            .cloned();

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
                    let change = sstyle.layer.change_position(layers::types::Point { x, y });
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

                // Mark that the client now owns the layer bounds
                store_style(state, &sstyle, |s| {
                    s.client_owns_size = true;
                    s.last_size_px = Some((width, height));
                });

                let transaction = active_transaction.clone();
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
                if sstyle.output_centered {
                    center_on_output(state, &sstyle, (width, height), transaction);
                }
            }

            otto_surface_style_v1::Request::RequestOutputFrame => {
                let Some(output) = output_rect_px(state, &sstyle) else {
                    tracing::warn!("request_output_frame: no output for this surface");
                    return;
                };
                // The answer has to arrive in the coordinates the client sets
                // positions in — relative to the window this surface hangs
                // from — or it cannot use it.
                //
                // Not `render_position() - position()`: a style layer created
                // for a subsurface is not parented under its window's layer, so
                // that difference is zero and the answer comes back as if the
                // client's coordinates were the screen's.
                let (parent_x, parent_y) = surface_root_origin_px(state, &sstyle.surface);

                layer_obj.output_frame(
                    (output.0 - parent_x) as f64,
                    (output.1 - parent_y) as f64,
                    output.2 as f64,
                    output.3 as f64,
                );
            }

            otto_surface_style_v1::Request::SetOutputPlacement { placement } => {
                use super::super::protocol::gen::otto_surface_style_v1::OutputPlacement;

                let centered = matches!(
                    placement.into_result().ok(),
                    Some(OutputPlacement::OutputCentered)
                );
                store_style(state, &sstyle, |s| s.output_centered = centered);
                if centered {
                    let size = applied_size(&sstyle);
                    center_on_output(state, &sstyle, size, active_transaction);
                }
            }

            otto_surface_style_v1::Request::SetOutputRelativeSize {
                width,
                height,
                min_width,
                min_height,
            } => {
                let width_fraction = wl_fixed_to_f32(width).clamp(0.0, 1.0);
                let height_fraction = wl_fixed_to_f32(height).clamp(0.0, 1.0);
                let min_width = wl_fixed_to_f32(min_width).max(0.0);
                let min_height = wl_fixed_to_f32(min_height).max(0.0);

                let Some(output) = output_rect_px(state, &sstyle) else {
                    tracing::warn!("set_output_relative_size: no output for this surface");
                    return;
                };
                // A fraction of zero leaves that axis alone, so one axis can be
                // output relative while the other stays whatever set_size made
                // it.
                let (current_width, current_height) = applied_size(&sstyle);
                let width = if width_fraction > 0.0 {
                    (output.2 * width_fraction).max(min_width)
                } else {
                    current_width
                };
                let height = if height_fraction > 0.0 {
                    (output.3 * height_fraction).max(min_height)
                } else {
                    current_height
                };

                store_style(state, &sstyle, |s| {
                    s.client_owns_size = true;
                    s.last_size_px = Some((width, height));
                });

                let transaction = active_transaction.clone();
                if let Some(txn_id) = active_transaction {
                    let change = sstyle
                        .layer
                        .change_size(layers::types::Size::points(width, height));
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle
                        .layer
                        .set_size(layers::types::Size::points(width, height), None);
                }

                // Centring is measured against the size, so a resize moves the
                // surface too — otherwise it would sit centred for its old size
                // until something else nudged it.
                if sstyle.output_centered {
                    center_on_output(state, &sstyle, (width, height), transaction);
                }
                trigger_window_update(state, &sstyle.surface.id());
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

                // A background the compositor paints is a material whether it
                // arrives now or at the end of a transaction — the window is
                // ineligible for scanout either way.
                let surface_id = sstyle.surface.id();
                if let Some(style_list) = state.surfaces_style.get_mut(&surface_id) {
                    if let Some(s) = style_list.iter_mut().find(|l| l.wl_style.id() == layer_id) {
                        s.background_alpha = alpha;
                    }
                }
                state.refresh_window_material(&surface_id);
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

                // A rounded window is clipped to a shape its own buffer does
                // not have, so it cannot be scanned out raw — it needs the
                // compositor to draw it. Recorded whether the radius arrives
                // now or at the end of a transaction.
                let surface_id = sstyle.surface.id();
                if let Some(style_list) = state.surfaces_style.get_mut(&surface_id) {
                    if let Some(s) = style_list.iter_mut().find(|l| l.wl_style.id() == layer_id) {
                        s.rounded = radius > 0.0;
                    }
                }
                state.refresh_window_material(&surface_id);
            }

            otto_surface_style_v1::Request::SetBorder {
                width,
                red,
                green,
                blue,
                alpha,
            } => {
                // Logical points on the wire, like the corner radius: a
                // client asking for a one-point hairline was getting a single
                // device pixel, which on a 2x output is half the line the
                // compositor's own chrome draws beside it.
                let screen_scale = Config::with(|c| c.screen_scale) as f32;
                let width = wl_fixed_to_f32(width) * screen_scale;
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

                // Like the corner radius: a border the compositor strokes is
                // not in the client's buffer, so the window cannot be scanned
                // out raw. A fully transparent border draws nothing.
                let surface_id = sstyle.surface.id();
                if let Some(style_list) = state.surfaces_style.get_mut(&surface_id) {
                    if let Some(s) = style_list.iter_mut().find(|l| l.wl_style.id() == layer_id) {
                        s.bordered = width > 0.0 && alpha > 0.0;
                    }
                }
                state.refresh_window_material(&surface_id);
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
                let Some(masks_to_bounds) = clip_mode_enabled(clip_mode) else {
                    tracing::warn!("Invalid clip_mode value: {:?}", clip_mode);
                    return;
                };

                sstyle.layer.set_clip_content(masks_to_bounds, None);
            }

            otto_surface_style_v1::Request::SetClipChildren { clip_mode } => {
                let Some(clip_children) = clip_mode_enabled(clip_mode) else {
                    tracing::warn!("Invalid clip_mode value: {:?}", clip_mode);
                    return;
                };

                // Descendant surfaces are mirrored as child layers of this
                // surface's layer, so lay-rs `clip_children` is exactly the
                // scissor a client asks for here. Routing it through the
                // transaction lets a client turn cropping on in the same atomic
                // step that moves the scrolled child, instead of a frame apart.
                if let Some(txn_id) = active_transaction {
                    let change = sstyle.layer.change_clip_children(clip_children);
                    accumulate_change(state, txn_id, change);
                } else {
                    sstyle.layer.set_clip_children(clip_children, None);
                    trigger_window_update(state, &sstyle.surface.id());
                }
            }

            otto_surface_style_v1::Request::SetContentsGravity { gravity } => {
                use super::super::protocol::gen::otto_surface_style_v1::ContentsGravity as WlGravity;
                use crate::surface_style::ContentsGravity;

                let new_gravity = match gravity.into_result().ok() {
                    Some(WlGravity::Resize) => ContentsGravity::Resize,
                    Some(WlGravity::ResizeAspect) => ContentsGravity::ResizeAspect,
                    Some(WlGravity::ResizeAspectFill) => ContentsGravity::ResizeAspectFill,
                    Some(WlGravity::Center) => ContentsGravity::Center,
                    Some(WlGravity::TopLeft) => ContentsGravity::TopLeft,
                    Some(WlGravity::TopRight) => ContentsGravity::TopRight,
                    _ => {
                        tracing::warn!("Invalid contents_gravity value: {:?}", gravity);
                        return;
                    }
                };

                let surface_id = sstyle.surface.id();
                if let Some(style_list) = state.surfaces_style.get_mut(&surface_id) {
                    if let Some(s) = style_list.iter_mut().find(|l| l.wl_style.id() == layer_id) {
                        s.contents_gravity = new_gravity;
                        s.shared_gravity
                            .store(new_gravity as u8, std::sync::atomic::Ordering::Relaxed);
                    }
                }
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
                // What a window's material has to frost is usually the window
                // *below* it in the same plane, not just the wallpaper. The
                // plane's seeded backdrop holds the background alone — it is
                // built before the windows plane is painted — so seeding it
                // would land behind that same-pass content and leave the window
                // underneath sharp. Opt into the raw background plus a real
                // blur, exactly as the server-side titlebar does.
                sstyle
                    .layer
                    .set_blur_include_content(blend_mode == LayrsBlendMode::BackgroundBlur);
                trigger_window_update(state, &sstyle.surface.id());

                let surface_id = sstyle.surface.id();
                if let Some(style_list) = state.surfaces_style.get_mut(&surface_id) {
                    if let Some(s) = style_list.iter_mut().find(|l| l.wl_style.id() == layer_id) {
                        s.background_blur = blend_mode == LayrsBlendMode::BackgroundBlur;
                    }
                }
                state.refresh_window_material(&surface_id);
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
                            let _ = window.layer().add_sublayer(&sstyle.layer);
                        }
                        OttoSurfaceStyleZOrder::AboveSurface => {
                            let _ = window.layer().add_sublayer(&sstyle.layer);
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

/// Update the stored copy of a style. The request handler works from a clone,
/// so anything that has to outlive one request has to be written back.
fn store_style<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    sstyle: &super::super::protocol::SurfaceStyle,
    edit: impl FnOnce(&mut super::super::protocol::SurfaceStyle),
) {
    let surface_id = sstyle.surface.id();
    let style_id = sstyle.wl_style.id();
    if let Some(list) = state.surfaces_style.get_mut(&surface_id) {
        if let Some(stored) = list.iter_mut().find(|l| l.wl_style.id() == style_id) {
            edit(stored);
        }
    }
}

/// The size the layer is becoming, in surface pixels.
///
/// The rendered bounds lag by a frame, and centring against a stale size puts
/// the surface half a resize away from the middle, so the size the client last
/// asked for wins when there is one.
fn applied_size(sstyle: &super::super::protocol::SurfaceStyle) -> (f32, f32) {
    sstyle.last_size_px.unwrap_or_else(|| {
        let size = sstyle.layer.render_size();
        (size.x, size.y)
    })
}

/// The usable area of the output this surface is shown on, as a rect in scene
/// pixels — the output minus the dock and any layer-shell exclusive zones.
///
/// Chosen by which output contains the surface's centre rather than by asking
/// the surface, so it follows a window dragged from one display to another
/// without any bookkeeping. Falls back to the first output, since a surface
/// that is off every display still has to be given somewhere to be centred.
fn output_rect_px<BackendData: Backend>(
    state: &Otto<BackendData>,
    sstyle: &super::super::protocol::SurfaceStyle,
) -> Option<(f32, f32, f32, f32)> {
    let position = sstyle.layer.render_position();
    let size = sstyle.layer.render_size();
    let (cx, cy) = (position.x + size.x / 2.0, position.y + size.y / 2.0);

    let mut fallback = None;
    for output in state.workspaces.outputs() {
        // The *usable* area, not the raw output: a panel centred on the whole
        // display sits low, because the dock and any exclusive layer-shell
        // surfaces have taken space off the bottom and the top. Centring in
        // what is left is what puts it where the eye expects it.
        let Some(geometry) = state
            .workspaces
            .usable_geometry(output)
            .or_else(|| state.workspaces.output_geometry(output))
        else {
            continue;
        };
        // Layer coordinates are physical pixels; output geometry is logical.
        let scale = output.current_scale().fractional_scale() as f32;
        let rect = (
            geometry.loc.x as f32 * scale,
            geometry.loc.y as f32 * scale,
            geometry.size.w as f32 * scale,
            geometry.size.h as f32 * scale,
        );
        if fallback.is_none() {
            fallback = Some(rect);
        }
        if cx >= rect.0 && cx < rect.0 + rect.2 && cy >= rect.1 && cy < rect.1 + rect.3 {
            return Some(rect);
        }
    }
    fallback
}

/// Move a surface so that it sits in the middle of its output.
///
/// The layer's position is relative to its parent, and the parent's own
/// position is not something the client can know — which is the whole reason
/// this request exists. Here it is simply the difference between where the
/// layer is on screen and where it should be, applied to the local position;
/// no parent lookup, and correct however deep the surface is nested.
fn center_on_output<BackendData: Backend>(
    state: &mut Otto<BackendData>,
    sstyle: &super::super::protocol::SurfaceStyle,
    size: (f32, f32),
    transaction: Option<wayland_backend::server::ObjectId>,
) {
    let Some(output) = output_rect_px(state, sstyle) else {
        return;
    };
    let target_x = output.0 + (output.2 - size.0) / 2.0;
    let target_y = output.1 + (output.3 - size.1) / 2.0;

    let global = sstyle.layer.render_position();
    let local = sstyle.layer.position();
    let position = layers::types::Point {
        x: local.x + (target_x - global.x),
        y: local.y + (target_y - global.y),
    };

    tracing::debug!(
        target: "otto::surface_style::centering",
        "center_on_output: output={:?} size={:?} global={:?} local={:?} -> {:?}",
        output,
        size,
        (global.x, global.y),
        (local.x, local.y),
        (position.x, position.y),
    );

    if let Some(txn_id) = transaction {
        let change = sstyle.layer.change_position(position);
        accumulate_change(state, txn_id, change);
    } else {
        sstyle.layer.set_position((position.x, position.y), None);
        trigger_window_update(state, &sstyle.surface.id());
    }
}

/// Where the window a surface belongs to sits, in scene pixels.
///
/// A client sets a subsurface's position relative to its parent, so anything
/// reported back to it has to be in those coordinates, and that means knowing
/// where the parent is. Walking to the root of the surface tree and asking the
/// workspace where that window is put is the only thing that answers it: the
/// style layer itself cannot, because it is not parented under the window.
///
/// `(0, 0)` when the surface belongs to no window — a layer-shell surface, for
/// instance, whose positions are already the output's.
fn surface_root_origin_px<BackendData: Backend>(
    state: &Otto<BackendData>,
    surface: &wayland_server::protocol::wl_surface::WlSurface,
) -> (f32, f32) {
    use smithay::wayland::compositor::get_parent;

    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }

    let Some(window) = state
        .workspaces
        .spaces_elements()
        .find(|element| element.wl_surface().map(|s| *s == root).unwrap_or(false))
    else {
        tracing::debug!(target: "otto::surface_style::centering", "root_origin: no window for surface");
        return (0.0, 0.0);
    };
    let Some(geometry) = state.workspaces.element_geometry(window) else {
        tracing::debug!(target: "otto::surface_style::centering", "root_origin: window has no geometry");
        return (0.0, 0.0);
    };
    tracing::debug!(target: "otto::surface_style::centering", "root_origin: geometry={geometry:?}");
    // Window geometry is logical; layer coordinates are physical.
    let scale = state
        .workspaces
        .outputs()
        .next()
        .map(|output| output.current_scale().fractional_scale() as f32)
        .unwrap_or(1.0);
    (geometry.loc.x as f32 * scale, geometry.loc.y as f32 * scale)
}
