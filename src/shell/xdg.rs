use std::cell::RefCell;

use layers::prelude::{taffy, Interpolate, Layer, Transition};
use smithay::{
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Window, WindowSurface,
        WindowSurfaceType,
    },
    input::{pointer::Focus, Seat},
    output::Output,
    reexports::{
        wayland_protocols::xdg::{decoration as xdg_decoration, shell::server::xdg_toplevel},
        wayland_server::{
            protocol::{wl_output, wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::{with_states, with_surface_tree_downward, TraversalAction},
        seat::WaylandFocus,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState,
        },
    },
};
use tracing::warn;

use crate::{
    focus::KeyboardFocusTarget,
    shell::TouchResizeSurfaceGrab,
    state::{Backend, Otto},
    workspaces::ApplicationsInfo,
};

use super::{
    fullscreen_output_geometry, FullscreenSurface, PointerMoveSurfaceGrab,
    PointerResizeSurfaceGrab, ResizeData, ResizeState, SurfaceData, TouchMoveSurfaceGrab,
    WindowElement,
};

impl<BackendData: Backend> XdgShellHandler for Otto<BackendData> {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured

        let window_layer = self.layers_engine.new_layer();
        let expose_mirror_layer = self.layers_engine.new_layer();

        // Set key to match View rendering pipeline format so sc-layer can find it immediately
        let surface_id = surface.wl_surface().id();
        window_layer.set_key(format!("surface_{:?}", surface_id));

        expose_mirror_layer.set_draw_content(window_layer.as_content());
        expose_mirror_layer.set_picture_cached(false);
        expose_mirror_layer.set_key(format!("mirror_window_{}", window_layer.id.0));
        expose_mirror_layer.set_layout_style(taffy::Style {
            position: taffy::Position::Absolute,
            ..Default::default()
        });
        window_layer.add_follower_node(&expose_mirror_layer);

        let window_element = WindowElement::new(
            Window::new_wayland_window(surface.clone()),
            window_layer.clone(),
            expose_mirror_layer,
        );
        let pointer_location = self.pointer.current_location();

        let (bounds, location) = self.workspaces.new_window_placement_at(pointer_location);

        // set the initial toplevel bounds
        #[allow(irrefutable_let_patterns)]
        if let WindowSurface::Wayland(surface) = window_element.underlying_surface() {
            surface.with_pending_state(|state| {
                state.bounds = Some(bounds.size);
            });
        }

        tracing::info!("SC::new_toplevel at({}, {})", location.x, location.y);

        // If the current workspace is in fullscreen mode, decide where to map the new window
        let current_workspace = self.workspaces.get_current_workspace();
        let current_index = self.workspaces.get_current_workspace_index();

        if current_workspace.get_fullscreen_mode() {
            // Check if the new window belongs to the same app as the fullscreen window
            if let Some(fullscreen_window) = self.workspaces.get_fullscreen_window() {
                let new_app_id = window_element.display_app_id(&self.display_handle);
                let fullscreen_app_id = fullscreen_window.display_app_id(&self.display_handle);

                if !new_app_id.is_empty() && new_app_id == fullscreen_app_id {
                    // Same app: keep in the fullscreen workspace (e.g., dialogs)
                    tracing::info!(
                        "New window from same fullscreen app '{}': keeping in workspace {}",
                        new_app_id,
                        current_index
                    );
                } else {
                    // Different app: redirect to previous workspace
                    if current_index > 0 {
                        let prev_workspace = current_index - 1;
                        tracing::info!(
                            "New window from different app in fullscreen workspace: redirecting from workspace {} to workspace {}",
                            current_index,
                            prev_workspace
                        );
                        self.workspaces
                            .set_current_workspace_index(prev_workspace, None);
                    } else {
                        tracing::warn!(
                            "Fullscreen workspace 0 detected, cannot redirect to previous workspace"
                        );
                    }
                }
            }
        } else {
            tracing::debug!(
                "Normal workspace: mapping new window to current workspace {}",
                current_index
            );
        }

        self.workspaces
            .map_window(&window_element, location, true, None);

        // Register with foreign toplevel protocols (both ext and wlr)
        let surface_id = surface.wl_surface().id();
        let app_id = window_element.xdg_app_id();
        let title = window_element.xdg_title();

        let ext_handle = self
            .foreign_toplevel_list_state
            .new_toplevel::<Self>(&app_id, &title);
        let wlr_handle = self.wlr_foreign_toplevel_state.new_toplevel::<Self>(
            &self.display_handle,
            &app_id,
            &title,
        );

        let handles = crate::state::foreign_toplevel_shared::ForeignToplevelHandles::new(
            ext_handle, wlr_handle,
        );
        self.foreign_toplevels.insert(surface_id.clone(), handles);

        // Create the rendering layer for sc_layers to find
        self.get_or_create_layer_for_surface(surface.wl_surface());

        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, Some(window_element.into()), Serial::from(0));
    }

    fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
        let id = toplevel.wl_surface().id();

        // Cascade destroy all sc-layers attached to this window
        if let Some(layers) = self.sc_layers.remove(&id) {
            for layer in layers {
                self.layers_engine.mark_for_delete(layer.layer.id());
                tracing::info!(
                    "Cascade destroyed sc-layer {:?} with parent window {:?}",
                    layer.wl_layer.id(),
                    id
                );
            }
        }

        if let Some(window) = self.workspaces.get_window_for_surface(&id) {
            if window.is_fullscreen() {
                let fullscreen_workspace = window.get_fullscreen_workspace();
                if let Some(workspace) = self.workspaces.get_workspace_at(fullscreen_workspace) {
                    workspace.set_fullscreen_mode(false);
                    workspace.set_fullscreen_animating(false);
                    workspace.set_name(None);
                }
                if self.workspaces.get_current_workspace_index() == fullscreen_workspace {
                    let prev_workspace = (fullscreen_workspace as i32 - 1).min(0) as usize;
                    self.workspaces
                        .set_current_workspace_index(prev_workspace, None);
                }
            }
        }
        let removed_surface_ids = self.workspaces.unmap_window(&id);

        // Notify foreign toplevel list that this toplevel is closed
        if let Some(handle) = self.foreign_toplevels.remove(&id) {
            handle.send_closed();
        }

        // Clean up surface_layers and sc_layers for the main window and removed popup surfaces
        self.surface_layers.remove(&id);
        self.sc_layers.remove(&id);
        self.view_warm_cache.remove(&id);
        
        for surface_id in removed_surface_ids {
            self.surface_layers.remove(&surface_id);
            self.sc_layers.remove(&surface_id);
        }

        if let Some(keyboard) = self.seat.get_keyboard() {
            if let Some(focus) = keyboard.current_focus() {
                if focus.same_client_as(&id) {
                    let current_space_elements = self.workspaces.space().elements();
                    let top_element = current_space_elements.last().cloned();
                    if let Some(window_element) = top_element {
                        keyboard.set_focus(self, Some(window_element.into()), Serial::from(0));
                    }
                }
            }
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured

        self.unconstrain_popup(&surface);

        let popup_kind = PopupKind::from(surface.clone());
        let popup_surface = popup_kind.wl_surface();
        let popup_id = popup_surface.id();

        // Cache the root surface mapping for fast lookup during commit/destroy
        if let Ok(root) = find_popup_root_surface(&popup_kind) {
            self.popup_root_cache.insert(popup_id.clone(), root.id());
        }

        if let Err(err) = self.popups.track_popup(popup_kind) {
            warn!("Failed to track popup: {}", err);
        }
    }

    fn popup_destroyed(&mut self, popup_surface: PopupSurface) {
        // Use cached root lookup - O(1) instead of traversing popup tree
        let popup_id = popup_surface.wl_surface().id();

        // Remove from popup overlay layer
        self.workspaces.popup_overlay.remove_popup(&popup_id);

        // Clean up layers for this popup surface
        self.destroy_layer_for_surface(&popup_id);
        // Also clean up any sc-layers attached to these surfaces
        self.sc_layers.remove(&popup_id);

        if let Some(root_id) = self.popup_root_cache.remove(&popup_id) {
            if let Some(window) = self.workspaces.get_window_for_surface(&root_id).cloned() {
                window.on_commit();
                self.update_window_view(&window);
            }
        }
    }
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<Otto<BackendData>> = Seat::from_resource(&seat).unwrap();
        self.move_request_xdg(&surface, &seat, serial)
    }

    fn resize_request(
        &mut self,
        top_level: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat: Seat<Otto<BackendData>> = Seat::from_resource(&seat).unwrap();
        let sid = top_level.wl_surface().id();
        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();
                tracing::info!(?start_data);

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let Some(window) = self.workspaces.get_window_for_surface(&sid) else {
                    tracing::info!("no window");
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&top_level.wl_surface().id())
                {
                    tracing::warn!("SC::resize on different surface");
                    return;
                }
                let geometry = window.geometry();
                let loc = self.workspaces.element_location(window).unwrap();
                let (initial_window_location, initial_window_size) = (loc, geometry.size);

                with_states(top_level.wl_surface(), move |states| {
                    states
                        .data_map
                        .get::<RefCell<SurfaceData>>()
                        .unwrap()
                        .borrow_mut()
                        .resize_state = ResizeState::Resizing(ResizeData {
                        edges: edges.into(),
                        initial_window_location,
                        initial_window_size,
                    });
                });

                let grab = TouchResizeSurfaceGrab {
                    start_data,
                    window: window.clone(),
                    edges: edges.into(),
                    initial_window_location,
                    initial_window_size,
                    last_window_size: initial_window_size,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        let window = self.workspaces.get_window_for_surface(&sid).unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&top_level.wl_surface().id())
        {
            return;
        }

        let geometry = window.geometry();
        let loc = self.workspaces.element_location(window).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(top_level.wl_surface(), move |states| {
            states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut()
                .resize_state = ResizeState::Resizing(ResizeData {
                edges: edges.into(),
                initial_window_location,
                initial_window_size,
            });
        });

        let grab = PointerResizeSurfaceGrab {
            start_data,
            window: window.clone(),
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        if let Configure::Toplevel(configure) = configure {
            if let Some(serial) = with_states(&surface, |states| {
                if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                    if let ResizeState::WaitingForFinalAck(_, serial) = data.borrow().resize_state {
                        return Some(serial);
                    }
                }

                None
            }) {
                // When the resize grab is released the surface
                // resize state will be set to WaitingForFinalAck
                // and the client will receive a configure request
                // without the resize state to inform the client
                // resizing has finished. Here we will wait for
                // the client to acknowledge the end of the
                // resizing. To check if the surface was resizing
                // before sending the configure we need to use
                // the current state as the received acknowledge
                // will no longer have the resize state set
                let id = surface.id();
                let is_resizing = self
                    .workspaces
                    .get_window_for_surface(&id)
                    .and_then(|w| w.toplevel())
                    .map(|toplevel| {
                        toplevel.with_committed_state(|current| {
                            current
                                .is_some_and(|s| s.states.contains(xdg_toplevel::State::Resizing))
                        })
                    })
                    .unwrap_or(false);

                if configure.serial >= serial && is_resizing {
                    with_states(&surface, |states| {
                        let mut data = states
                            .data_map
                            .get::<RefCell<SurfaceData>>()
                            .unwrap()
                            .borrow_mut();
                        if let ResizeState::WaitingForFinalAck(resize_data, _) = data.resize_state {
                            data.resize_state = ResizeState::WaitingForCommit(resize_data);
                        } else {
                            unreachable!()
                        }
                    });
                }
            }

            let _surface_clone = surface.clone();
            use std::borrow::Cow;

            let window = self
                .workspaces
                .spaces_elements()
                .find(|element| element.wl_surface() == Some(Cow::Borrowed(&surface)));
            if let Some(_window) = window {
                use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
                let _is_ssd = configure
                    .state
                    .decoration_mode
                    .map(|mode| mode == Mode::ServerSide)
                    .unwrap_or(false);
                // window.set_ssd(false);
                if !self.is_resizing {
                    self.workspaces.update_workspace_model();
                }
            }
        }
    }

    // Request to set the window as fullscreen
    // a preferred output may be specified
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        mut wl_output: Option<wl_output::WlOutput>,
    ) {
        if surface.with_pending_state(|state| {
            state
                .capabilities
                .contains(xdg_toplevel::WmCapabilities::Fullscreen)
        }) {
            // the surface size is either output size
            // or the current workspace size
            let wl_surface = surface.wl_surface();

            let geometry = fullscreen_output_geometry(wl_output.as_ref(), &self.workspaces);

            // if let Some(geometry) = output_geometry {
            let output = wl_output
                .as_ref()
                .and_then(Output::from_resource)
                .unwrap_or_else(|| self.workspaces.outputs().next().unwrap().clone());
            let client = self.display_handle.get_client(wl_surface.id()).unwrap();
            for output in output.client_outputs(&client) {
                wl_output = Some(output);
            }

            let wid = surface.wl_surface().id();
            let window = self
                .workspaces
                .get_window_for_surface(&wid)
                .unwrap()
                .clone();

            // Ignore the request if the window is already fullscreen or animating towards fullscreen
            if window.is_fullscreen() {
                return;
            }

            // Also ignore if any workspace is currently animating towards fullscreen
            // This prevents multiple workspaces from being created when F11 is held down
            let mut i = 0;
            while let Some(ws) = self.workspaces.get_workspace_at(i) {
                if ws.get_fullscreen_animating() {
                    return;
                }
                i += 1;
            }

            let id = window.id();

            if let Some(mut view) = self.workspaces.get_window_view(&id) {
                let current_element_geometry = self.workspaces.element_geometry(&window).unwrap();
                view.unmaximised_rect = current_element_geometry;
                self.workspaces.set_window_view(&id, view);
            }
            output
                .user_data()
                .insert_if_missing(FullscreenSurface::default);
            output
                .user_data()
                .get::<FullscreenSurface>()
                .unwrap()
                .set(window.clone());

            // Reset buffers to force a full redraw when entering fullscreen
            // This prevents damage tracking artifacts from the scene-based rendering
            self.backend_data.reset_buffers(&output);

            let (next_workspace_index, next_workspace) = self.workspaces.get_next_free_workspace();
            next_workspace.set_fullscreen_mode(true);
            next_workspace.set_fullscreen_animating(true);

            // Exit expose mode when entering fullscreen
            self.workspaces.expose_set_visible(false);

            // Fetch app info asynchronously to get the proper display name
            let app_id = window.display_app_id(&self.display_handle);
            if !app_id.is_empty() {
                let workspace_clone = next_workspace.clone();
                tokio::spawn(async move {
                    if let Some(app_info) = ApplicationsInfo::get_app_info_by_id(&app_id).await {
                        if let Some(name) = app_info.desktop_name() {
                            workspace_clone.set_name(Some(name));
                        } else {
                            // Fallback to app_id if no desktop name
                            workspace_clone.set_name(Some(app_id));
                        }
                    } else {
                        // Fallback to app_id if app info not found
                        workspace_clone.set_name(Some(app_id));
                    }
                });
            }

            window.set_fullscreen(true, next_workspace_index);

            let current_workspace_index = self.workspaces.get_current_workspace_index();

            let id = window.id();
            if let Some(view) = self.workspaces.get_window_view(&id) {
                let transition = Transition::ease_in_out_quad(1.4);

                // Fade out layer_shell_overlay when entering fullscreen
                self.workspaces.set_fullscreen_overlay_visibility(true);

                self.workspaces
                    .move_window_to_workspace(&window, next_workspace_index, (0, 0));
                window.set_workspace(current_workspace_index);
                self.workspaces
                    .set_current_workspace_index(next_workspace_index, Some(transition));

                let surface_clone = surface.clone();
                let wl_output_ref = wl_output.clone();
                let next_workspace_layer = next_workspace.windows_layer.clone();
                let next_workspace_clone = next_workspace.clone();

                // Animate size during fullscreen transition
                let current_element_geometry = self.workspaces.element_geometry(&window).unwrap();
                let animation = self
                    .layers_engine
                    .add_animation_from_transition(&transition, false);

                let current_width = current_element_geometry.size.w as f32;
                let current_height = current_element_geometry.size.h as f32;
                let target_width = geometry.size.w as f32;
                let target_height = geometry.size.h as f32;

                let s = surface.clone();
                self.layers_engine.on_animation_update(
                    animation,
                    move |p: f32| {
                        let width = current_width.interpolate(&target_width, p) as i32;
                        let height = current_height.interpolate(&target_height, p) as i32;
                        let size = Rectangle::new((0, 0).into(), (width, height).into());
                        s.with_pending_state(|state| {
                            state.size = Some(size.size);
                        });
                        s.send_configure();
                    },
                    false,
                );
                self.layers_engine.start_animation(animation, 0.0);

                self.workspaces
                    .dnd_view
                    .layer
                    .add_sublayer(&view.window_layer);

                view.window_layer
                    .set_position(layers::types::Point { x: 0.0, y: 0.0 }, Some(transition))
                    .on_finish(
                        move |l: &Layer, _| {
                            surface_clone.with_pending_state(|state| {
                                state.states.set(xdg_toplevel::State::Fullscreen);
                                state.size = Some(geometry.size);
                                state.fullscreen_output = wl_output_ref.clone();
                            });
                            // println!("append window layer to workspace");
                            next_workspace_layer.add_sublayer(l);
                            // The protocol demands us to always reply with a configure,
                            // regardless of we fulfilled the request or not
                            surface_clone.send_configure();

                            // Clear the fullscreen animating flag now that the animation is complete
                            next_workspace_clone.set_fullscreen_animating(false);
                        },
                        true,
                    );
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if !surface
            .with_pending_state(|state| state.states.contains(xdg_toplevel::State::Fullscreen))
        {
            return;
        }

        let id = surface.wl_surface().id();

        if let Some(view) = self.workspaces.get_window_view(&id) {
            let output = surface.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.fullscreen_output.take()
            });
            if let Some(output) = output {
                let output = Output::from_resource(&output).unwrap();

                if let Some(fullscreen) = output.user_data().get::<FullscreenSurface>() {
                    fullscreen.clear();
                    self.backend_data.reset_buffers(&output);
                }
            }
            if let Some(we) = self.workspaces.get_window_for_surface(&id).cloned() {
                we.set_fullscreen(false, 0);
                let scale = self
                    .workspaces
                    .outputs_for_element(&we)
                    .first()
                    .unwrap()
                    .current_scale()
                    .fractional_scale();

                let position = view.unmaximised_rect.loc.to_f64().to_physical(scale);

                if let Some(next_workspace) = self.workspaces.get_workspace_at(we.get_workspace()) {
                    let transition = Transition::ease_in_out_quad(1.4);

                    // Get the fullscreen workspace index before switching away from it
                    let fullscreen_workspace_index = self.workspaces.get_current_workspace_index();
                    let workspace = self.workspaces.get_current_workspace();
                    workspace.set_fullscreen_mode(false);
                    workspace.set_fullscreen_animating(false);

                    // Exit expose mode when exiting fullscreen
                    self.workspaces.expose_set_visible(false);

                    // Fade in layer_shell_overlay when exiting fullscreen
                    self.workspaces.set_fullscreen_overlay_visibility(false);

                    // Show dock with animation at the start of unfullscreen transition
                    self.workspaces.dock.show(Some(transition));

                    self.workspaces.move_window_to_workspace(
                        &we,
                        we.get_workspace(),
                        view.unmaximised_rect.loc,
                    );
                    self.workspaces
                        .set_current_workspace_index(we.get_workspace(), Some(transition));

                    // Delete the temporary fullscreen workspace
                    self.workspaces
                        .remove_workspace_at(fullscreen_workspace_index);

                    // Animate size during unfullscreen transition
                    let current_element_geometry = self.workspaces.element_geometry(&we).unwrap();
                    let animation = self
                        .layers_engine
                        .add_animation_from_transition(&transition, false);

                    let current_width = current_element_geometry.size.w as f32;
                    let current_height = current_element_geometry.size.h as f32;
                    let target_width = view.unmaximised_rect.size.w as f32;
                    let target_height = view.unmaximised_rect.size.h as f32;

                    let s = surface.clone();
                    self.layers_engine.on_animation_update(
                        animation,
                        move |p: f32| {
                            let width = current_width.interpolate(&target_width, p) as i32;
                            let height = current_height.interpolate(&target_height, p) as i32;
                            let size = Rectangle::new((0, 0).into(), (width, height).into());
                            s.with_pending_state(|state| {
                                state.size = Some(size.size);
                            });
                            s.send_configure();
                        },
                        false,
                    );
                    self.layers_engine.start_animation(animation, 0.0);

                    let surface_clone = surface.clone();
                    let restored_size = view.unmaximised_rect.size;
                    let workspace_layer = next_workspace.windows_layer.clone();

                    self.workspaces
                        .dnd_view
                        .layer
                        .add_sublayer(&view.window_layer);

                    view.window_layer
                        .set_position(
                            layers::types::Point {
                                x: position.x as f32,
                                y: position.y as f32,
                            },
                            Some(transition),
                        )
                        .on_finish(
                            move |l: &Layer, _| {
                                surface_clone.with_pending_state(|state| {
                                    state.size = Some(restored_size);
                                });
                                workspace_layer.add_sublayer(l);
                                surface_clone.send_configure();
                            },
                            true,
                        );
                }
            }
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // NOTE: This should use layer-shell when it is implemented to
        // get the correct maximum size
        if surface.with_pending_state(|state| {
            state
                .capabilities
                .contains(xdg_toplevel::WmCapabilities::Maximize)
        }) {
            let id = surface.wl_surface().id();
            let window = self.workspaces.get_window_for_surface(&id).unwrap().clone();

            let current_element_geometry = self.workspaces.element_geometry(&window).unwrap();
            let id = surface.wl_surface().id();
            if let Some(mut view) = self.workspaces.get_window_view(&id) {
                view.unmaximised_rect = current_element_geometry;
                self.workspaces.set_window_view(&id, view);
            }
            let outputs_for_window = self.workspaces.outputs_for_element(&window);
            let output = outputs_for_window
                .first()
                // The window hasn't been mapped yet, use the primary output instead
                .or_else(|| self.workspaces.outputs().next())
                // Assumes that at least one output exists
                .expect("No outputs found")
                .clone(); // Clone to avoid borrow conflicts

            let output_geom = self.workspaces.output_geometry(&output).unwrap();

            // Recalculate exclusive zones for this output before using them
            // This ensures we have fresh data even if layer surfaces changed
            self.recalculate_exclusive_zones(&output);

            // Get tracked exclusive zones for this output (from layer shell surfaces)
            let output_name = output.name();
            let zones = self
                .exclusive_zones
                .get(&output_name)
                .cloned()
                .unwrap_or_default();

            // Calculate usable area from tracked exclusive zones
            let mut usable_zone = zones.apply_to_output(output_geom);

            // Get the actual dock geometry (position and size)
            let dock_geom = self.workspaces.get_dock_geometry();

            // Dock reduces available height from the bottom
            if dock_geom.size.h > 0 {
                let dock_top = dock_geom.loc.y;
                let available_bottom = usable_zone.loc.y + usable_zone.size.h;

                // If dock is in the usable area, reduce height to stop above dock
                if dock_top < available_bottom {
                    usable_zone.size.h = dock_top - usable_zone.loc.y;
                }
            }
            let new_geometry = usable_zone;

            let transition = Transition::ease_out(0.3);
            let animation = self
                .layers_engine
                .add_animation_from_transition(&transition, false);

            // Use minimum size for windows that open already maximized (size 0,0)
            let current_width = current_element_geometry.size.w.max(600) as f32;
            let current_height = current_element_geometry.size.h.max(400) as f32;

            let new_width = new_geometry.size.w as f32;
            let new_height = new_geometry.size.h as f32;

            let s = surface.clone();
            self.layers_engine.on_animation_update(
                animation,
                move |p: f32| {
                    let width = current_width.interpolate(&new_width, p) as i32;
                    let height = current_height.interpolate(&new_height, p) as i32;
                    let size = Rectangle::new((0, 0).into(), (width, height).into());
                    s.with_pending_state(|state| {
                        if (p - 1.0).abs() < f32::EPSILON {
                            state.states.set(xdg_toplevel::State::Maximized);
                        }
                        state.size = Some(size.size);
                    });
                    s.send_configure();
                },
                false,
            );
            self.layers_engine.start_animation(animation, 0.0);

            self.workspaces
                .map_window(&window, new_geometry.loc, true, Some(transition));
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if !surface
            .with_pending_state(|state| state.states.contains(xdg_toplevel::State::Maximized))
        {
            return;
        }

        let id = surface.wl_surface().id();
        let window = self.workspaces.get_window_for_surface(&id).unwrap().clone();
        if let Some(view) = self.workspaces.get_window_view(&id) {
            let current_element_geometry = self
                .workspaces
                .element_geometry(&window)
                .unwrap_or(view.unmaximised_rect);

            let transition = Transition::ease_out(0.3);
            let animation = self
                .layers_engine
                .add_animation_from_transition(&transition, false);

            let current_width = current_element_geometry.size.w as f32;
            let current_height = current_element_geometry.size.h as f32;

            let new_width = view.unmaximised_rect.size.w as f32;
            let new_height = view.unmaximised_rect.size.h as f32;

            let s = surface.clone();
            self.layers_engine.on_animation_update(
                animation,
                move |p: f32| {
                    let width = current_width.interpolate(&new_width, p) as i32;
                    let height = current_height.interpolate(&new_height, p) as i32;
                    let size = Rectangle::new((0, 0).into(), (width, height).into());
                    s.with_pending_state(|state| {
                        if (p - 1.0).abs() < f32::EPSILON {
                            state.states.unset(xdg_toplevel::State::Maximized);
                        }
                        state.size = Some(size.size);
                    });
                    s.send_configure();
                },
                false,
            );
            self.layers_engine.start_animation(animation, 0.0);

            self.workspaces
                .map_window(&window, view.unmaximised_rect.loc, true, Some(transition));
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if surface.with_pending_state(|state| {
            state
                .capabilities
                .contains(xdg_toplevel::WmCapabilities::Minimize)
        }) {
            let id = surface.wl_surface().id();
            let window = self.workspaces.get_window_for_surface(&id).unwrap().clone();

            let current_element_geometry = self.workspaces.element_geometry(&window).unwrap();

            if let Some(mut view) = self.workspaces.get_window_view(&id) {
                view.unmaximised_rect = current_element_geometry;
                self.workspaces.set_window_view(&id, view);
            }

            let next_focus = self.workspaces.minimize_window(&window);
            match next_focus {
                Some(wid) => self.set_keyboard_focus_on_surface(&wid),
                None => self.clear_keyboard_focus(),
            }
        }

        // The protocol demands us to always reply with a configure,
        // regardless of we fulfilled the request or not
        surface.send_configure();
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<Otto<BackendData>> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);
        if let Some(root) = find_popup_root_surface(&kind).ok().and_then(|root| {
            self.workspaces
                .spaces_elements()
                .find(|w| w.wl_surface().map(|s| *s == root).unwrap_or(false))
                .cloned()
                .map(KeyboardFocusTarget::from)
                .or_else(|| {
                    self.workspaces
                        .outputs()
                        .find_map(|o| {
                            let map = layer_map_for_output(o);
                            map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                                .cloned()
                        })
                        .map(KeyboardFocusTarget::LayerSurface)
                })
        }) {
            let ret = self.popups.grab_popup(root, kind, &seat, serial);

            if let Ok(mut grab) = ret {
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    keyboard.set_focus(self, grab.current_grab(), serial);
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    if pointer.is_grabbed()
                        && !(pointer.has_grab(serial)
                            || pointer
                                .has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
            }
        }
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    pub fn move_request_xdg(
        &mut self,
        surface: &ToplevelSurface,
        seat: &Seat<Self>,
        serial: Serial,
    ) {
        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let id = surface.wl_surface().id();
                let Some(window) = self.workspaces.get_window_for_surface(&id) else {
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&surface.wl_surface().id())
                {
                    return;
                }

                let mut initial_window_location = self.workspaces.element_location(window).unwrap();

                // If surface is maximized then unmaximize it
                let is_maximized = surface.with_pending_state(|state| {
                    state.states.contains(xdg_toplevel::State::Maximized)
                });
                if is_maximized {
                    // Get current maximized geometry before unmaximizing
                    let maximized_geometry = self.workspaces.element_geometry(window).unwrap();
                    let touch_location = start_data.location;

                    // Calculate grab point relative to maximized window
                    let grab_offset_x = touch_location.x - maximized_geometry.loc.x as f64;
                    let grab_offset_y = touch_location.y - maximized_geometry.loc.y as f64;

                    // Calculate grab ratio (0.0 to 1.0)
                    let grab_ratio_x = if maximized_geometry.size.w > 0 {
                        (grab_offset_x / maximized_geometry.size.w as f64).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let grab_ratio_y = if maximized_geometry.size.h > 0 {
                        (grab_offset_y / maximized_geometry.size.h as f64).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };

                    surface.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Maximized);
                        state.size = None;
                    });

                    surface.send_configure();

                    // Get restored window size from unmaximised_rect
                    let id = surface.wl_surface().id();
                    if let Some(view) = self.workspaces.get_window_view(&id) {
                        let restored_size = view.unmaximised_rect.size;

                        // Calculate new grab offset based on restored size
                        let new_grab_offset_x = grab_ratio_x * restored_size.w as f64;
                        let new_grab_offset_y = grab_ratio_y * restored_size.h as f64;

                        // Position window so grab point stays under touch point
                        let new_x = touch_location.x - new_grab_offset_x;
                        let new_y = touch_location.y - new_grab_offset_y;

                        initial_window_location = (new_x as i32, new_y as i32).into();
                    } else {
                        // Fallback: use touch location
                        initial_window_location = start_data.location.to_i32_round();
                    }
                }

                let grab = TouchMoveSurfaceGrab {
                    start_data,
                    window: window.clone(),
                    initial_window_location,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the client disconnects after requesting a move
        // we can just ignore the request
        let id = surface.wl_surface().id();
        let Some(window) = self.workspaces.get_window_for_surface(&id) else {
            return;
        };

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&surface.wl_surface().id())
        {
            return;
        }

        let mut initial_window_location = self.workspaces.element_location(window).unwrap();

        // If surface is maximized then unmaximize it
        let is_maximized = surface
            .with_pending_state(|state| state.states.contains(xdg_toplevel::State::Maximized));
        if is_maximized {
            // Get current maximized geometry before unmaximizing
            let maximized_geometry = self.workspaces.element_geometry(window).unwrap();
            let pointer_location = pointer.current_location();

            // Calculate grab point relative to maximized window
            let grab_offset_x = pointer_location.x - maximized_geometry.loc.x as f64;
            let grab_offset_y = pointer_location.y - maximized_geometry.loc.y as f64;

            // Calculate grab ratio (0.0 to 1.0)
            let grab_ratio_x = if maximized_geometry.size.w > 0 {
                (grab_offset_x / maximized_geometry.size.w as f64).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let grab_ratio_y = if maximized_geometry.size.h > 0 {
                (grab_offset_y / maximized_geometry.size.h as f64).clamp(0.0, 1.0)
            } else {
                0.5
            };

            surface.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = None;
            });

            surface.send_configure();

            // Get restored window size from unmaximised_rect
            let id = surface.wl_surface().id();
            if let Some(view) = self.workspaces.get_window_view(&id) {
                let restored_size = view.unmaximised_rect.size;

                // Calculate new grab offset based on restored size
                let new_grab_offset_x = grab_ratio_x * restored_size.w as f64;
                let new_grab_offset_y = grab_ratio_y * restored_size.h as f64;

                // Position window so grab point stays under cursor
                let new_x = pointer_location.x - new_grab_offset_x;
                let new_y = pointer_location.y - new_grab_offset_y;

                initial_window_location = (new_x as i32, new_y as i32).into();
            } else {
                // Fallback: position window centered under cursor
                let pos = pointer.current_location();
                initial_window_location = (pos.x as i32, pos.y as i32).into();
            }
        }

        let grab = PointerMoveSurfaceGrab {
            start_data,
            window: window.clone(),
            initial_window_location,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let id = root.id();
        let Some(window) = self.workspaces.get_window_for_surface(&id) else {
            return;
        };

        let mut outputs_for_window = self.workspaces.outputs_for_element(window);
        if outputs_for_window.is_empty() {
            return;
        }

        // Get a union of all outputs' geometries.
        let mut outputs_geo = self
            .workspaces
            .output_geometry(&outputs_for_window.pop().unwrap())
            .unwrap();
        for output in outputs_for_window {
            outputs_geo = outputs_geo.merge(self.workspaces.output_geometry(&output).unwrap());
        }

        let window_geo = self.workspaces.element_geometry(window).unwrap();

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = outputs_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }

    /// Get or create a layer for a surface
    /// Returns existing layer from cache if available, otherwise creates a new one
    pub(crate) fn get_or_create_layer_for_surface(&mut self, surface: &WlSurface) -> Layer {
        let surface_id = surface.id();
        
        if let Some(layer) = self.surface_layers.get(&surface_id) {
            return layer.clone();
        }
        
        let key = format!("surface_{:?}", surface_id);
        let layer = self.layers_engine.new_layer();
        layer.set_key(&key);

        self.surface_layers
            .insert(surface_id.clone(), layer.clone());
        layer
    }

    /// Ensure all surfaces in the tree have rendering layers
    /// This is called during rendering to lazily create layers as needed
    /// Note: get_or_create_layer_for_surface already handles cache insertion
    pub(crate) fn ensure_surface_tree_layers(&mut self, surface: &WlSurface) {
        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, _states, _| {
                // This will create and cache the layer if it doesn't exist
                // If it exists, it just returns it without modifying the cache
                let _ = self.get_or_create_layer_for_surface(surface);
            },
            |_, _, _| true,
        );
    }

    /// Destroy the layer associated with a surface
    /// Removes from surface_layers hashmap and marks for deletion in layers_engine
    pub(crate) fn destroy_layer_for_surface(
        &mut self,
        surface_id: &smithay::reexports::wayland_server::backend::ObjectId,
    ) {
        if let Some(layer) = self.surface_layers.remove(surface_id) {
            self.layers_engine.mark_for_delete(layer.id);
        }
    }
}
