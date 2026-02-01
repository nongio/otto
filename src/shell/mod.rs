use std::cell::RefCell;

#[cfg(feature = "xwayland")]
use smithay::xwayland::XWaylandClientData;
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{
        find_popup_root_surface, layer_map_for_output, LayerSurface, PopupKind, WindowSurface,
        WindowSurfaceType,
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
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{XdgPopupSurfaceData, XdgToplevelSurfaceData},
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
            let maybe_dmabuf = with_states(surface, |surface_data| {
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
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend_data.early_import(surface);

        let sync = is_sync_subsurface(surface);
        let surface_id = surface.id();

        if !sync {
            // Check if this is a layer shell surface first
            if self.layer_surfaces.contains_key(&surface_id) {
                self.update_layer_surface(&surface_id);

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

                let window = root_id
                    .as_ref()
                    .and_then(|id| self.workspaces.get_window_for_surface(id).cloned())
                    .or_else(|| self.workspaces.get_window_for_surface(&surface_id).cloned());

                if let Some(window) = window {
                    window.on_commit();
                    self.update_window_view(&window);

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
        self.popups.commit(surface);

        // ensure_initial_configure(surface, self.space(), &mut self.popups)
        ensure_initial_configure(surface, self);
        self.backend_data.request_redraw();
        self.schedule_event_loop_dispatch();
    }
    fn destroyed(&mut self, surface: &WlSurface) {
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
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .unwrap_or_else(|| self.workspaces.outputs().next().unwrap().clone());

        // Create the Smithay LayerSurface wrapper
        let layer_surface = LayerSurface::new(surface.clone(), namespace.clone());

        // Create a lay_rs layer for rendering
        let layer = self
            .workspaces
            .create_layer_shell_layer(wlr_layer, &namespace);

        // Create our compositor-owned wrapper
        let layer_shell_surface = LayerShellSurface::new(
            layer_surface.clone(),
            layer,
            output.clone(),
            wlr_layer,
            namespace,
        );

        // Store in our map
        let surface_id = surface.wl_surface().id();
        self.layer_surfaces.insert(surface_id, layer_shell_surface);

        // Also register with Smithay's layer map for protocol compliance
        let mut map = layer_map_for_output(&output);
        map.map_layer(&layer_surface).unwrap();

        tracing::info!(
            "New layer surface: layer={:?}, namespace={}",
            wlr_layer,
            layer_surface.namespace()
        );

        // Arrange the layer map which will handle the exclusive zone
        map.arrange();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let surface_id = surface.wl_surface().id();

        // Remove from our compositor map and clean up lay_rs layer
        if let Some(layer_shell_surface) = self.layer_surfaces.remove(&surface_id) {
            let output = layer_shell_surface.output().clone();
            self.workspaces
                .remove_layer_shell_layer(&layer_shell_surface.layer);
            tracing::info!(
                "Layer surface destroyed: namespace={}",
                layer_shell_surface.namespace()
            );
            // Recalculate exclusive zones after removal
            self.recalculate_exclusive_zones(&output);
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
