use std::cell::RefCell;

#[cfg(feature = "xwayland")]
use smithay::xwayland::XWaylandClientData;
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{
        find_popup_root_surface, layer_map_for_output, utils::with_surfaces_surface_tree,
        LayerSurface, PopupKind, WindowSurface, WindowSurfaceType,
    },
    output::Output,
    reexports::{
        calloop::Interest,
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output, wl_surface::WlSurface},
            Client, Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface, with_states,
            with_surface_tree_upward, BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState, SurfaceAttributes, TraversalAction,
        },
        dmabuf::get_dmabuf,
        shell::{
            wlr_layer::{
                KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
                WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::{PopupSurface, XdgPopupSurfaceData, XdgToplevelSurfaceData},
        },
    },
};

use crate::{
    state::{Backend, Otto},
    workspaces::Workspaces,
    ClientState,
};

mod element;
mod grabs;
mod layer;
pub(crate) mod ssd;
mod tiling;
#[cfg(feature = "xwayland")]
mod x11;
mod xdg;

pub use self::element::*;
pub use self::grabs::*;
pub use self::layer::*;

// the surface size is either output size
// or the current workspace size
fn fullscreen_output_geometry(
    // wl_surface: &WlSurface,
    wl_output: Option<&wl_output::WlOutput>,
    workspaces: &Workspaces,
) -> Rectangle<i32, Logical> {
    // First test if a specific output has been requested
    // if the requested output is not found ignore the request
    wl_output
        .and_then(Output::from_resource)
        .and_then(|o| workspaces.output_geometry(&o))
        .unwrap_or_else(|| workspaces.get_logical_rect())
}

#[derive(Default)]
pub struct FullscreenSurface(RefCell<Option<WindowElement>>);

impl FullscreenSurface {
    pub fn set(&self, window: WindowElement) {
        *self.0.borrow_mut() = Some(window);
    }

    pub fn get(&self) -> Option<WindowElement> {
        self.0.borrow().clone()
    }

    pub fn clear(&self) -> Option<WindowElement> {
        self.0.borrow_mut().take()
    }
}

impl<BackendData: Backend> BufferHandler for Otto<BackendData> {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl<BackendData: Backend> CompositorHandler for Otto<BackendData> {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        #[cfg(feature = "xwayland")]
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        panic!("Unknown client data type")
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<Self, _>(surface, move |state, _dh, surface| {
            // Explicit-sync (wp_linux_drm_syncobj) acquire point, if the client
            // committed one. smithay only validates these points in its own
            // commit_hook — the compositor must add the blocker so the surface
            // transaction waits until the client's GPU render completes. Without
            // it, KMS plane scanout flips a half-rendered buffer (tearing), and
            // Otto samples the buffer before it is rendered and shows a black
            // texture (fullscreen Proton games, e.g. Cuphead via DXVK/vkd3d).
            // udev-only: DrmSyncobjCachedState lives behind smithay's
            // backend_drm feature.
            #[cfg(feature = "udev")]
            let mut acquire_point = None;
            let maybe_dmabuf = with_states(surface, |surface_data| {
                #[cfg(feature = "udev")]
                acquire_point.clone_from(
                    &surface_data
                        .cached_state
                        .get::<smithay::wayland::drm_syncobj::DrmSyncobjCachedState>()
                        .pending()
                        .acquire_point,
                );
                surface_data
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|assignment| match assignment {
                        BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                        _ => None,
                    })
            });
            if let Some(dmabuf) = maybe_dmabuf {
                // Prefer the explicit acquire fence when the client provided one.
                #[cfg(feature = "udev")]
                if let Some(acquire_point) = acquire_point {
                    match acquire_point.generate_blocker() {
                        Ok((blocker, source)) => {
                            if let Some(client) = surface.client() {
                                let res = state.handle.insert_source(source, move |_, _, data| {
                                    let dh = data.display_handle.clone();
                                    data.client_compositor_state(&client)
                                        .blocker_cleared(data, &dh);
                                    Ok(())
                                });
                                if res.is_ok() {
                                    add_blocker(surface, blocker);
                                    // Don't also add the implicit blocker for this commit.
                                    return;
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                ?err,
                                "explicit-sync acquire_point.generate_blocker failed; \
                                 falling back to the implicit dmabuf fence"
                            );
                        }
                    }
                }
                // Fall back to the implicit dmabuf fence.
                if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                    if let Some(client) = surface.client() {
                        let res = state.handle.insert_source(source, move |_, _, data| {
                            let dh = data.display_handle.clone();
                            data.client_compositor_state(&client)
                                .blocker_cleared(data, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                        }
                    }
                }
            }
        });

        // Note: Layers are created lazily via get_or_create_layer_for_surface when needed
        // Layer shells will have already registered their workspace layer before this point
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend_data.early_import(surface);

        // A drag icon's commit can carry an anchor: the offset that puts the
        // point the user grabbed under the cursor, rather than the icon's
        // corner. It is relative to the last one, so it accumulates, and it is
        // taken here because the compositor is the only place that sees it —
        // nothing else in the pipeline reads `buffer_delta` for a role-less
        // surface.
        if self.dnd_icon.as_ref() == Some(surface) {
            let delta = with_states(surface, |states| {
                states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take()
            });
            if let Some(delta) = delta {
                self.dnd_icon_offset += delta;
            }
        }

        let sync = is_sync_subsurface(surface);
        let surface_id = surface.id();

        if !sync {
            // A lock surface is not in any workspace, space or layer map: it
            // belongs to the output's lock plane and is mirrored straight into
            // the scene. Its subsurfaces reach here too, and are handled by the
            // root's own sync below.
            if let Some(output_name) = self.lock_surface_output(&surface_id).or_else(|| {
                let mut root = surface.clone();
                while let Some(parent) = get_parent(&root) {
                    root = parent;
                }
                self.lock_surface_output(&root.id())
            }) {
                self.update_lock_surface(&output_name);
                // A lock surface's commit has to ask for a frame like any
                // other surface's. Returning without this updates the scene and
                // then tells nobody to draw it: the panel freezes on whichever
                // frame some other redraw happened to carry, and — because the
                // frame callback is sent from the presentation path — the
                // client never learns its frame arrived either. It then paints
                // on its own timeout, at a tenth of the rate, into a screen
                // that is not being redrawn. Nothing on a lock screen animates
                // if this is missed, and nothing else redraws while locked.
                self.backend_data.invalidate_scene_prefetch();
                self.backend_data.request_redraw();
                self.schedule_event_loop_dispatch();
                return;
            }

            if let Some(_layer_shell_surf) = self.layer_surfaces.get(&surface_id) {
                // Layer shells don't need build_cache_for_view - they use the workspace layer directly
                self.update_layer_shell_surface(&surface_id);

                // Don't recalculate here - it causes deadlock since layer_map is borrowed
                // Recalculation will happen during arrange in ensure_initial_configure
            } else {
                // Find the root surface for this commit
                // 1. Check popup cache first (O(1))
                // 2. Try PopupManager for popups
                // 3. Traverse subsurface hierarchy to find root
                let root_id = self
                    .popup_root_cache
                    .get(&surface_id)
                    .cloned()
                    .or_else(|| {
                        self.popups
                            .find_popup(surface)
                            .and_then(|popup| find_popup_root_surface(&popup).ok().map(|r| r.id()))
                    })
                    .or_else(|| {
                        // Traverse subsurface hierarchy to find root
                        let mut root = surface.clone();
                        while let Some(parent) = get_parent(&root) {
                            root = parent;
                        }
                        // Only return if we found a different root
                        if root.id() != surface_id {
                            Some(root.id())
                        } else {
                            None
                        }
                    });

                // Check if the root is a layer shell surface
                let is_layer_shell = root_id
                    .as_ref()
                    .map(|id| self.layer_surfaces.contains_key(id))
                    .or_else(|| Some(self.layer_surfaces.contains_key(&surface_id)))
                    .unwrap_or(false);

                if is_layer_shell {
                    // Popup belongs to a layer shell - update the layer shell to render the popup
                    let layer_id = root_id.as_ref().unwrap_or(&surface_id);
                    self.update_layer_shell_surface(layer_id);
                } else {
                    // Handle regular window popups
                    let window = root_id
                        .as_ref()
                        .and_then(|id| self.workspaces.get_window_for_surface(id).cloned())
                        .or_else(|| self.workspaces.get_window_for_surface(&surface_id).cloned());

                    if let Some(window) = window {
                        window.on_commit();
                        self.settle_initial_placement(&window);

                        if self.popups.find_popup(surface).is_some() {
                            tracing::debug!(
                                target: "otto::popups",
                                "popup surface {:?} commit; root window {:?} scanned_out={}",
                                surface_id,
                                window.id(),
                                window.is_scanned_out()
                            );
                        }

                        // Skip scene damage propagation when this window is on a
                        // scanout plane — its buffer goes straight to the display,
                        // and importing it would only re-render the (hidden)
                        // content layer into the windows plane. The pending flag
                        // makes the backend draw a frame anyway so the new buffer
                        // reaches its plane (a skipped import produces no scene
                        // damage, which is otherwise the draw trigger).
                        //
                        // Only the promoted ROOT surface's own commits qualify:
                        // popup commits (overlay plane) and SSD subsurface
                        // commits (windows plane) still composite in the scene,
                        // so skipping them loses their updates — e.g. a popup
                        // that maps or redraws while its parent is promoted
                        // would never reach its layer.
                        let is_root_commit = window
                            .wl_surface()
                            .map(|root| root.id() == surface_id)
                            .unwrap_or(false);
                        if window.is_scanned_out() && is_root_commit {
                            self.workspaces
                                .scanout_commit_pending
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            // The content buffer scans out directly, but the
                            // shadow still renders in the windows plane — keep
                            // its geometry in sync (tile/resize) or it ghosts at
                            // the pre-change size while the content tiles.
                            self.refresh_window_chrome_geometry(&window);
                        } else {
                            self.update_window_view_for_commit(&window, Some(surface));
                        }

                        // Update foreign toplevel list only if title or app_id actually changed
                        if let Some(handle) = root_id
                            .or(Some(surface_id))
                            .and_then(|id| self.foreign_toplevels.get(&id))
                        {
                            let title = window.xdg_title();
                            let app_id = window.xdg_app_id();

                            // Only send updates if the values have changed
                            // Note: send_title/send_app_id internally check if values changed
                            // but we still need to avoid sending unnecessary done events
                            let title_changed = handle.title() != title;
                            let app_id_changed = handle.app_id() != app_id;

                            if title_changed || app_id_changed {
                                if title_changed {
                                    handle.send_title(&title);
                                }
                                if app_id_changed {
                                    handle.send_app_id(&app_id);
                                }
                                handle.send_done();
                            }
                        }
                    }
                }
            }
        }
        // The commit above created or refreshed the surface's layer; now
        // the blur region it carried can be applied to that layer.
        self.apply_background_effect(surface);
        self.popups.commit(surface);

        // ensure_initial_configure(surface, self.space(), &mut self.popups)
        ensure_initial_configure(surface, self);
        self.backend_data.invalidate_scene_prefetch();
        self.backend_data.request_redraw();
        self.schedule_event_loop_dispatch();
    }
    fn destroyed(&mut self, surface: &WlSurface) {
        // Clean up the layer for this surface
        self.destroy_layer_for_surface(&surface.id());
        self.forget_background_effect(&surface.id());

        // A decoration mode stashed for a surface that never became a toplevel
        // has nothing left to apply to.
        self.pending_kde_decorations.remove(&surface.id());

        // Find root surface for this destroyed surface
        // 1. Check popup cache first (O(1)) - entry removal happens in popup_destroyed
        // 2. Try PopupManager for popups
        // 3. Traverse subsurface hierarchy to find root
        let root_id = self
            .popup_root_cache
            .get(&surface.id())
            .cloned()
            .or_else(|| {
                self.popups
                    .find_popup(surface)
                    .and_then(|popup| find_popup_root_surface(&popup).ok().map(|r| r.id()))
            })
            .or_else(|| {
                // Traverse subsurface hierarchy to find root
                let mut root = surface.clone();
                while let Some(parent) = get_parent(&root) {
                    root = parent;
                }
                // Only return if we found a different root
                if root.id() != surface.id() {
                    Some(root.id())
                } else {
                    None
                }
            });

        let window = root_id
            .and_then(|id| self.workspaces.get_window_for_surface(&id).cloned())
            .or_else(|| {
                self.workspaces
                    .get_window_for_surface(&surface.id())
                    .cloned()
            });

        if let Some(window) = window {
            window.on_commit();
            self.update_window_view(&window);
        }
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    /// Mirror a surface tree into lay-rs layers: one layer per surface,
    /// configured from the committed buffer, with the subsurface parenting and
    /// ordering reproduced. Shared by wlr-layer-shell and session-lock
    /// surfaces, which differ only in the container they hang from.
    pub(crate) fn sync_surface_tree_layers(
        &mut self,
        wl_surface: &WlSurface,
        scale_factor: f64,
        key_prefix: &str,
    ) {
        // Ensure all surfaces in the tree have rendering layers
        self.ensure_surface_tree_layers(wl_surface);

        // Collect render elements from the surface tree (same as update_window_view)
        let mut render_elements = std::collections::VecDeque::new();
        let initial_location: smithay::utils::Point<f64, smithay::utils::Physical> =
            (0.0, 0.0).into();
        let initial_context = (initial_location, initial_location, None);

        // Collect all surfaces and build parent-child map
        #[allow(clippy::mutable_key_type, clippy::type_complexity)]
        let mut surface_info: std::collections::HashMap<
            smithay::reexports::wayland_server::backend::ObjectId,
            (
                WlSurface,
                smithay::utils::Point<f64, smithay::utils::Physical>,
                Option<smithay::reexports::wayland_server::backend::ObjectId>,
            ),
        > = std::collections::HashMap::new();

        // Track per-parent child ordering as Smithay delivers it
        // (respects wl_subsurface.place_above / place_below reordering)
        #[allow(clippy::mutable_key_type)]
        let mut children_order: std::collections::HashMap<
            smithay::reexports::wayland_server::backend::ObjectId,
            Vec<smithay::reexports::wayland_server::backend::ObjectId>,
        > = std::collections::HashMap::new();

        smithay::wayland::compositor::with_surface_tree_downward(
            wl_surface,
            initial_context,
            |surface, states, (location, _parent_location, _parent_id)| {
                let mut location = *location;
                let data = states
                    .data_map
                    .get::<smithay::backend::renderer::utils::RendererSurfaceStateUserData>(
                );
                let mut cached_state = states
                    .cached_state
                    .get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                let cached_state = cached_state.current();
                let surface_geometry = cached_state.geometry.unwrap_or_default();

                if let Some(data) = data {
                    let data = data.lock().unwrap();
                    if let Some(view) = data.view() {
                        location += view.offset.to_f64().to_physical(scale_factor);
                        location -= surface_geometry.loc.to_f64().to_physical(scale_factor);
                        smithay::wayland::compositor::TraversalAction::DoChildren((
                            location,
                            location,
                            Some(surface.id()),
                        ))
                    } else {
                        smithay::wayland::compositor::TraversalAction::SkipChildren
                    }
                } else {
                    smithay::wayland::compositor::TraversalAction::SkipChildren
                }
            },
            |surface, states, (location, parent_location, parent_id)| {
                let relative_offset = if parent_id.is_some() {
                    *location - *parent_location
                } else {
                    *location
                };

                if let Some(wvs) = self.window_view_for_surface(
                    surface,
                    states,
                    &relative_offset,
                    scale_factor,
                    parent_id.clone(),
                ) {
                    render_elements.push_front(wvs.clone());
                    let sid = surface.id();
                    surface_info
                        .insert(sid.clone(), (surface.clone(), *location, parent_id.clone()));
                    // Record child ordering per parent for subsurface reordering
                    if let Some(pid) = parent_id {
                        children_order.entry(pid.clone()).or_default().push(sid);
                    }
                } else {
                    // Null buffer — hide the layer so unmapped subsurfaces don't linger
                    if let Some(layer) = self.surface_layers.get(&surface.id()) {
                        layer.set_hidden(true);
                    }
                }
            },
            |_, _, _| true,
        );

        // Now sync the layer hierarchy to match the surface tree (same as windows)
        for (surface_id, (_surface, _pos, parent_id)) in surface_info.iter() {
            let surface_layer =
                self.get_or_create_layer_for_surface(&surface_info.get(surface_id).unwrap().0);

            // Set key for proper opacity inheritance (like window content layers)
            surface_layer.set_key(format!("{key_prefix}_{:?}", surface_id));

            if let Some(wvs) = render_elements.iter().find(|e| &e.id == surface_id) {
                // Configure layer with all properties and draw callback
                surface_layer.set_hidden(false);
                let style = self
                    .surfaces_style
                    .get(surface_id)
                    .and_then(|v: &Vec<_>| v.first());
                let gravity = style.map(|s| s.contents_gravity).unwrap_or_default();
                let client_owns_size = style.map(|s| s.client_owns_size).unwrap_or(false);
                let shared_gravity = style.map(|s| s.shared_gravity.clone());
                crate::workspaces::utils::configure_surface_layer(
                    &surface_layer,
                    wvs,
                    gravity,
                    client_owns_size,
                    shared_gravity,
                );
                // …then correct the opacity claim it just made. Layer-shell
                // (and lock) surfaces are routinely fullscreen and mostly
                // transparent — the launcher is a fullscreen overlay with a
                // search field on it — and a blanket `content_opaque` turns
                // one into an occluder that culls the wallpaper and every
                // window beneath it. That is invisible on the KMS path, where
                // each plane subtree is rendered in isolation, and blacks out
                // the whole screen on winit, which composites the output as a
                // single tree. Ask the client instead: opaque only where it
                // declared an opaque region (or committed a buffer with no
                // alpha at all). Cheap enough to redo every commit — this is a
                // plain field write in lay-rs, it schedules nothing.
                let fully_opaque = smithay::wayland::compositor::with_states(
                    &surface_info.get(surface_id).unwrap().0,
                    crate::workspaces::utils::surface_is_fully_opaque,
                );
                surface_layer.set_content_opaque(fully_opaque);

                // Set up parent-child relationship
                // Only append if there's a parent - root surface is handled separately below
                if let Some(parent_id) = parent_id {
                    if surface_id != parent_id {
                        if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                            let _ = self
                                .layers_engine
                                .append_layer(&surface_layer, parent_layer.id());
                        }
                    }
                }
            }
        }

        // Re-append children in Smithay's subsurface order so that
        // place_above / place_below reordering is reflected in lay-rs.
        for (parent_id, child_ids) in children_order.iter() {
            if let Some(parent_layer) = self.surface_layers.get(parent_id) {
                let parent_node = parent_layer.id();
                for child_id in child_ids {
                    if let Some(child_layer) = self.surface_layers.get(child_id) {
                        let _ = self.layers_engine.append_layer(child_layer, parent_node);
                    }
                }
            }
        }
    }

    /// Is a modal overlay layer-shell surface on screen?
    ///
    /// A client on the overlay layer that asks for *exclusive* keyboard
    /// interactivity is presenting something the user has to answer — the
    /// portal Access dialog otto-islands draws, for one. Fullscreen otherwise
    /// hides the layer-shell chrome and scans the window out on the primary
    /// plane alone, so such a dialog would never be seen.
    pub fn has_modal_overlay_layer(&self) -> bool {
        self.layer_surfaces.values().any(|s| {
            s.wlr_layer() == Layer::Overlay
                && matches!(s.keyboard_interactivity(), KeyboardInteractivity::Exclusive)
        })
    }

    /// Bring the fullscreen chrome back while a modal overlay dialog is up,
    /// and hide it again once the dialog is answered. A no-op when nothing is
    /// fullscreen — the chrome is visible anyway.
    pub fn refresh_modal_overlay(&mut self) {
        let modal = self.has_modal_overlay_layer();
        if modal == self.modal_overlay_shown {
            return;
        }
        self.modal_overlay_shown = modal;
        if self.workspaces.get_fullscreen_window().is_some() {
            // `set_fullscreen_overlay_visibility(true)` HIDES the chrome.
            self.workspaces.set_fullscreen_overlay_visibility(!modal);
        }
    }

    fn update_layer_shell_surface(
        &mut self,
        surface_id: &smithay::reexports::wayland_server::backend::ObjectId,
    ) {
        // Extract needed data first to avoid borrow conflicts
        let (geometry, wl_surface) = {
            let Some(layer_shell_surf) = self.layer_surfaces.get(surface_id) else {
                return;
            };

            let output = layer_shell_surf.output().clone();
            let Some(output_geo) = self.workspaces.output_geometry(&output) else {
                return;
            };
            let geometry = layer_shell_surf.compute_geometry(output_geo);
            let wl_surface = layer_shell_surf.layer_surface().wl_surface().clone();

            (geometry, wl_surface)
        };

        let scale_factor = crate::config::Config::with(|c| c.screen_scale);

        // Handle popups for this layer shell surface (e.g., waybar calendar)
        let layer_position = layers::types::Point {
            x: (geometry.loc.x as f64 * scale_factor) as f32,
            y: (geometry.loc.y as f64 * scale_factor) as f32,
        };

        use smithay::desktop::PopupManager;

        PopupManager::popups_for_surface(&wl_surface).for_each(|(popup, popup_offset)| {
            let offset: smithay::utils::Point<f64, smithay::utils::Physical> =
                popup_offset.to_physical_precise_round(scale_factor);
            let popup_surface = popup.wl_surface();
            let popup_id = popup_surface.id();

            // Calculate absolute popup position (layer shell position + popup offset)
            let popup_position = layers::types::Point {
                x: layer_position.x + offset.x as f32,
                y: layer_position.y + offset.y as f32,
            };

            // Collect surfaces for this popup
            let mut popup_surfaces = Vec::new();
            let popup_origin: smithay::utils::Point<f64, smithay::utils::Physical> =
                (0.0, 0.0).into();
            with_surfaces_surface_tree(popup_surface, |surface, states| {
                if let Some(window_view) =
                    self.window_view_for_surface(surface, states, &popup_origin, scale_factor, None)
                {
                    popup_surfaces.push(window_view);
                }
            });

            // Send popup to the overlay layer and register its surface layers
            #[allow(clippy::mutable_key_type)]
            let popup_layers = self.workspaces.popup_overlay.update_popup(
                &popup_id,
                surface_id,
                popup_position,
                popup_surfaces,
                None,
                &self.layers_engine,
                &self.surface_layers,
            );

            self.surface_layers.extend(popup_layers);
            // Visibility is owned by update_popup: the layer starts hidden and
            // is shown on the first update that carries renderable content.
        });

        self.sync_surface_tree_layers(&wl_surface, scale_factor, "layer_shell_surface");

        // Update the container layer's Taffy layout style from anchors/margins/size.
        // This lets the layout engine position the layer automatically — including
        // during surface-style size animations (the animated size feeds back into
        // Taffy and the position adjusts every frame).
        let layer = {
            let Some(layer_shell_surf) = self.layer_surfaces.get(surface_id) else {
                return;
            };
            layer_shell_surf.layer.clone()
        };

        let container_style = self
            .surfaces_style
            .get(surface_id)
            .and_then(|v: &Vec<_>| v.first());
        let container_owns_size = container_style.map(|s| s.client_owns_size).unwrap_or(false);

        // Always refresh the Taffy style so position tracks anchor+margin changes.
        // When the client owns the size (surface style animation), Taffy still
        // controls the inset positioning — the animated size on the layer feeds
        // back into layout automatically.
        {
            let Some(layer_shell_surf) = self.layer_surfaces.get(surface_id) else {
                return;
            };
            let style = layer_shell_surf.taffy_style(scale_factor);
            layer.set_layout_style(style);
        }

        if !container_owns_size {
            // Rounded, not truncated: the size is a logical integer times the
            // output scale, so on a fractional scale it is fractional (a 34pt
            // bar at 1.75 is 59.5), and truncating loses most of a pixel off
            // the far edge. The container's ORIGIN comes from the Taffy
            // layout, which rounds, so rounding the extent here keeps both
            // edges on the grid — see `snap_extent_px`.
            let container_w = (geometry.size.w as f64 * scale_factor).round() as f32;
            let container_h = (geometry.size.h as f64 * scale_factor).round() as f32;
            layer.set_size(layers::types::Size::points(container_w, container_h), None);
        }
        layer.set_hidden(false);

        // A dialog may have just opened (or closed) on this surface.
        self.refresh_modal_overlay();

        // For layer shells, the workspace layer IS the surface layer
        // Don't try to append it to itself - it's already added in create_layer_shell_layer
        // (Regular windows would need to append surface layers to window container layer here)
    }
}

impl<BackendData: Backend> WlrLayerShellHandler for Otto<BackendData> {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        wlr_layer: Layer,
        namespace: String,
    ) {
        // A NULL wl_output leaves the choice to us. It must be a real output:
        // the chrome containers (`layer_shell_top`/`layer_shell_overlay`) live
        // in the PRIMARY output's overlay plane, and that plane is only pushed
        // while `is_overlay_ui_active` sees a Top/Overlay layer in the primary
        // output's layer map. Assigning the surface to a virtual output (RDP,
        // mirror) therefore renders it into a plane nobody scans out — mako
        // notifications and rofi stayed invisible until an unrelated popup
        // happened to activate the plane.
        let Some(output) = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.workspaces.default_client_output().cloned())
        else {
            tracing::warn!("new_layer_surface: no output available for {namespace}");
            return;
        };

        // Create the Smithay LayerSurface wrapper
        let layer_surface = LayerSurface::new(surface.clone(), namespace.clone());

        // Create a lay_rs layer for rendering (container layer for the layer shell surface)
        let layer = self
            .workspaces
            .create_layer_shell_layer(wlr_layer, &namespace, &output);

        // For layer shells, the workspace layer IS the rendering layer
        // Register it in surface_layers so get_or_create_layer_for_surface returns it
        let surface_id = surface.wl_surface().id();
        self.surface_layers
            .insert(surface_id.clone(), layer.clone());

        // Create our compositor-owned wrapper
        let layer_shell_surface = LayerShellSurface::new(
            layer_surface.clone(),
            layer.clone(),
            output.clone(),
            wlr_layer,
            namespace,
        );

        // Store in our map
        self.layer_surfaces.insert(surface_id, layer_shell_surface);

        // Also register with Smithay's layer map for protocol compliance
        let mut map = layer_map_for_output(&output);
        map.map_layer(&layer_surface).unwrap();
        // Get the current size from the layer surface state
        // let _size = layer_surface.cached_state().size;

        // Arrange the layer map which will handle the exclusive zone
        map.arrange();
        // A new panel may have taken space away from the dock (see
        // `refresh_dock_metrics`); it takes the map itself, so drop ours.
        drop(map);
        self.workspaces.refresh_dock_metrics();
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        // At this point Smithay has already set the popup's XdgPopupSurfaceData.parent
        // to the layer surface's wl_surface (in zwlr_layer_surface_v1.get_popup handler).
        // The earlier XdgShellHandler::new_popup fired before the parent was set, so
        // track_popup failed there. Re-track now that the parent chain is valid.
        let popup_kind = PopupKind::from(popup.clone());
        let popup_id = popup.wl_surface().id();

        if let Ok(root) = find_popup_root_surface(&popup_kind) {
            let root_id = root.id();
            self.popup_root_cache.insert(popup_id.clone(), root_id);
            tracing::debug!("layer new_popup: {:?} root={:?}", popup_id, root.id());
        }

        if let Err(err) = self.popups.track_popup(popup_kind) {
            tracing::warn!("layer new_popup: failed to track popup: {}", err);
        }

        // Unconstrain now that the parent chain is established
        self.unconstrain_popup(&popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let surface_id = surface.wl_surface().id();

        // A layer surface that takes the keyboard — the launcher, a locker, a
        // panel with a search field — holds it until it goes away, and then
        // nothing holds it: the focus target is destroyed and every key after
        // it falls on the floor. The window underneath looks focused, is
        // marked activated, and is deaf.
        //
        // It bites hardest where the layer surface *hands over* to a window:
        // the launcher activates the window it was asked for, then the key
        // release that dismissed it arrives and is routed back to the layer
        // (see `keyboard_key_to_action`), which then dies holding the focus it
        // just took back.
        let held_focus = matches!(
            self.seat.get_keyboard().and_then(|k| k.current_focus()),
            Some(crate::focus::KeyboardFocusTarget::LayerSurface(ref l))
                if l.wl_surface().id() == surface_id
        );

        // Remove from our compositor map and clean up lay_rs layer
        if let Some(layer_shell_surface) = self.layer_surfaces.remove(&surface_id) {
            let output = layer_shell_surface.output().clone();

            // Clear the warm cache for this surface to prevent dangling layer references
            self.view_warm_cache.remove(&surface_id);

            // Clear the surface_layers cache entry for the layer shell
            self.surface_layers.remove(&surface_id);

            self.workspaces
                .remove_layer_shell_layer(&layer_shell_surface.layer);
            tracing::info!(
                "Layer surface destroyed: namespace={}",
                layer_shell_surface.namespace()
            );
            // Recalculate exclusive zones after removal
            self.recalculate_exclusive_zones(&output);
            // The dialog may have gone away with the surface.
            self.refresh_modal_overlay();
        }

        if held_focus {
            // Back to the window that had the keyboard most recently, which is
            // the one the user chose if the layer surface was choosing one —
            // the launcher activates a window and *then* takes the focus back
            // for the key release. The topmost window would be the wrong
            // answer there: it is whatever the stack says, not what was asked
            // for. Falling back to the workspace's top window covers a layer
            // surface that was never handing over to anything.
            match self
                .workspaces
                .last_focused_window()
                .and_then(|id| self.workspaces.get_window_for_surface(&id).cloned())
            {
                Some(window) => self.set_keyboard_focus_on_window(&window),
                None => {
                    let index = self
                        .workspaces
                        .focused_output_workspaces()
                        .map(|ows| ows.current_workspace)
                        .unwrap_or_else(|| self.workspaces.get_current_workspace_index());
                    self.focus_top_window_or_clear(index);
                }
            }
        }

        // Also unmap from Smithay's layer map
        if let Some((mut map, layer)) = self.workspaces.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            layer.map(|layer| (map, layer))
        }) {
            map.unmap_layer(&layer);
        }
    }
}

#[derive(Default)]
pub struct SurfaceData {
    pub geometry: Option<Rectangle<i32, Logical>>,
    pub resize_state: ResizeState,
}

/// A window at most this fraction of the usable area, in **both** axes, is
/// treated as a dialog and centered.
///
/// There is no protocol signal to key on: GTK dialogs send `set_parent(nil)`
/// and don't implement `xdg-dialog-v1`, so "small" is the only thing that
/// actually distinguishes a dialog from a document window across toolkits.
const DIALOG_MAX_FRACTION: f64 = 0.6;
/// Logical pixels a centred window steps down and right when the spot it
/// would take is already occupied, and how many times it may step.
const CASCADE_STEP: i32 = 32;
const CASCADE_MAX_STEPS: usize = 8;

impl<BackendData: crate::state::Backend> crate::state::Otto<BackendData> {
    /// Re-place a freshly mapped window now that its real size is known.
    ///
    /// Initial placement runs at `new_toplevel`, before the client has
    /// configured, so it assumes a default size and spreads windows by least
    /// overlap — which puts a small dialog in the top-left corner. Once the
    /// first sized commit arrives we can tell a dialog from a normal window and
    /// center it. Runs at most once per window.
    fn settle_initial_placement(&mut self, window: &WindowElement) {
        let id = window.id();
        let size = window.geometry().size;
        // Still unsized — wait for a later commit.
        if size.w <= 0 || size.h <= 0 {
            return;
        }

        // A client may map at one size and grow to its real one a commit later:
        // Chrome maps at its minimum size, gets treated as a dialog and centered
        // by the branch below, and then expands from that centered origin — which
        // is how a browser window ends up with its top-left corner in the middle
        // of the screen, hanging off the right and bottom edges. So keep
        // re-placing the window until two consecutive commits agree on a size.
        let Some(last_seen) = self.pending_initial_placement.get_mut(&id) else {
            return;
        };
        if *last_seen == Some(size) {
            self.pending_initial_placement.remove(&id);
        } else {
            *last_seen = Some(size);
        }

        // Fullscreen/maximized windows own their geometry already.
        if window.is_fullscreen() || window.is_maximized() {
            return;
        }

        // On a tiling workspace the tree owns the window's rectangle: it
        // joins the tree next to whatever is focused and the relayout places
        // it, so none of the floating placement below applies.
        if self.tiling_adopt_window(window) {
            return;
        }

        let Some(output) = self.workspaces.output_for_window(window) else {
            return;
        };
        let Some(usable) = self.workspaces.usable_geometry(&output) else {
            return;
        };

        let is_dialog = (size.w as f64) <= usable.size.w as f64 * DIALOG_MAX_FRACTION
            && (size.h as f64) <= usable.size.h as f64 * DIALOG_MAX_FRACTION;
        if !is_dialog {
            // Not a dialog, so it keeps the spot the placement picked — but that
            // spot was chosen for an assumed 800x600 window. A window wider or
            // taller than that (a browser, an IDE) hangs off the right or bottom
            // edge from a corner candidate, which reads as "it opened with its
            // top-left corner in the middle of the screen". Pull it back inside.
            let Some(location) = self.workspaces.element_location(window) else {
                return;
            };
            let clamped = smithay::utils::Point::<i32, Logical>::from((
                location
                    .x
                    .min(usable.loc.x + (usable.size.w - size.w).max(0))
                    .max(usable.loc.x),
                location
                    .y
                    .min(usable.loc.y + (usable.size.h - size.h).max(0))
                    .max(usable.loc.y),
            ));
            if clamped != location {
                tracing::debug!(
                    "settle_initial_placement: pulling {}x{} window back inside from {:?} to {:?}",
                    size.w,
                    size.h,
                    location,
                    clamped
                );
                self.workspaces
                    .map_window_on_output(&output, window, clamped, false, None);
            }
            return;
        }

        let centre = smithay::utils::Point::<i32, Logical>::from((
            usable.loc.x + (usable.size.w - size.w) / 2,
            usable.loc.y + (usable.size.h - size.h) / 2,
        ));

        // Two windows of the same size centre on the same point and stack
        // exactly, leaving the one underneath without an edge to click. Step
        // each new arrival down and to the right until it clears what is
        // already there, the way a cascade does.
        let taken: Vec<smithay::utils::Point<i32, Logical>> = self
            .workspaces
            .spaces_elements()
            .filter(|other| other.id() != id)
            .filter_map(|other| self.workspaces.element_location(other))
            .collect();
        let mut location = centre;
        for _ in 0..CASCADE_MAX_STEPS {
            let collides = taken.iter().any(|p| {
                (p.x - location.x).abs() < CASCADE_STEP && (p.y - location.y).abs() < CASCADE_STEP
            });
            if !collides {
                break;
            }
            location.x += CASCADE_STEP;
            location.y += CASCADE_STEP;
        }
        // A cascade that would hang off the usable area is worse than the
        // stack it was avoiding — go back to the centre.
        if location.x + size.w > usable.loc.x + usable.size.w
            || location.y + size.h > usable.loc.y + usable.size.h
        {
            location = centre;
        }

        tracing::debug!(
            "settle_initial_placement: centering {}x{} dialog at {:?}",
            size.w,
            size.h,
            location
        );
        self.workspaces
            .map_window_on_output(&output, window, location, false, None);
    }
}

fn ensure_initial_configure<Backend: crate::state::Backend>(
    surface: &WlSurface,
    state: &mut Otto<Backend>, // space: &Space<WindowElement>,
                               // popups: &mut PopupManager,
) {
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            states
                .data_map
                .insert_if_missing(|| RefCell::new(SurfaceData::default()));
        },
        |_, _, _| true,
    );

    if let Some(window) = state
        .workspaces
        .get_window_for_surface(&surface.id())
        .cloned()
    {
        // send the initial configure if relevant
        #[cfg_attr(not(feature = "xwayland"), allow(irrefutable_let_patterns))]
        if let WindowSurface::Wayland(ref toplevel) = window.underlying_surface() {
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });
            if !initial_configure_sent {
                toplevel.send_configure();
            }
        }

        with_states(surface, |states| {
            let mut data = states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut();

            // Finish resizing.
            if let ResizeState::WaitingForCommit(_) = data.resize_state {
                data.resize_state = ResizeState::NotResizing;
            }
        });

        return;
    }

    if let Some(popup) = state.popups.find_popup(surface) {
        let popup = match popup {
            PopupKind::Xdg(ref popup) => popup,
            // Doesn't require configure
            PopupKind::InputMethod(ref _input_popup) => {
                return;
            }
        };

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgPopupSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });
        if !initial_configure_sent {
            // NOTE: This should never fail as the initial configure is always
            // allowed.
            popup.send_configure().expect("initial configure failed");
        }

        return;
    };

    // Find the output for this layer surface (clone to avoid borrow issues)
    let output = state
        .workspaces
        .outputs()
        .find(|o| {
            let map = layer_map_for_output(o);
            map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
        .cloned();

    if let Some(output) = output {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        let mut map = layer_map_for_output(&output);

        // arrange the layers before sending the initial configure
        // to respect any size the client may have sent
        map.arrange();

        // send the initial configure if relevant
        if !initial_configure_sent {
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .unwrap();

            layer.layer_surface().send_configure();
        }
        // The arrange above may have changed the exclusive zones (a panel
        // appearing or resizing), which is the dock's own space budget. Drop
        // the map first — refreshing takes it again.
        drop(map);
        state.workspaces.refresh_dock_metrics();
    };
}

pub fn fixup_positions(workspaces: &mut Workspaces, pointer_location: Point<f64, Logical>) {
    // fixup outputs
    let mut offset = Point::<i32, Logical>::from((0, 0));
    for output in workspaces
        .outputs()
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
    {
        let size = workspaces
            .output_geometry(&output)
            .map(|geo| geo.size)
            .unwrap_or_else(|| Size::from((0, 0)));
        workspaces.map_output(&output, offset);
        layer_map_for_output(&output).arrange();
        offset.x += size.w;
    }

    // fixup windows
    let mut orphaned_windows = Vec::new();
    let outputs = workspaces
        .outputs()
        .flat_map(|o| {
            let geo = workspaces.output_geometry(o)?;
            let map = layer_map_for_output(o);
            let zone = map.non_exclusive_zone();
            Some(Rectangle::new(geo.loc + zone.loc, zone.size))
        })
        .collect::<Vec<_>>();
    for window in workspaces.spaces_elements() {
        let window_location = match workspaces.element_location(window) {
            Some(loc) => loc,
            None => continue,
        };
        let geo_loc = window.bbox().loc + window_location;

        if !outputs.iter().any(|o_geo| o_geo.contains(geo_loc)) {
            orphaned_windows.push(window.clone());
        }
    }
    // FIXME: when is this supposed to happen?
    // test pluggin / unplugging monitors
    for window in orphaned_windows.into_iter().as_ref() {
        let (_bounds, location) = workspaces.new_window_placement_at(pointer_location);
        workspaces.map_window(window, location, false, None);
    }
}
