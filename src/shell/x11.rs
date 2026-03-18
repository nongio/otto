use std::{cell::RefCell, os::unix::io::OwnedFd};

use layers::prelude::Transition;
use smithay::{
    desktop::WindowSurface,
    input::pointer::Focus,
    reexports::wayland_server::Resource,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
            SelectionTarget,
        },
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
        X11Surface, X11Wm, XwmHandler,
    },
};
use tracing::{error, trace};

use crate::{focus::KeyboardFocusTarget, state::Backend, Otto};

use super::{
    FullscreenSurface, PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData, ResizeState,
    SurfaceData, TouchMoveSurfaceGrab,
};

#[derive(Debug, Default)]
struct OldGeometry(RefCell<Option<Rectangle<i32, Logical>>>);
impl OldGeometry {
    pub fn save(&self, geo: Rectangle<i32, Logical>) {
        *self.0.borrow_mut() = Some(geo);
    }

    pub fn restore(&self) -> Option<Rectangle<i32, Logical>> {
        self.0.borrow_mut().take()
    }
}

impl<BackendData: Backend> XwmHandler for Otto<BackendData> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).unwrap();
        // Actual mapping deferred to XWaylandShellHandler::surface_associated,
        // which fires once the wl_surface association is committed and wl_surface() is valid.
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(
            "x11::mapped_override_redirect_window: geometry={:?} title={:?} (deferring to surface_associated)",
            window.geometry(),
            window.title()
        );
        // Do not map here: wl_surface is not yet available, so window_element.id() would panic.
        // Actual mapping is done in XWaylandShellHandler::surface_associated when the wl_surface
        // is associated, which handles both regular and override-redirect windows.
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let maybe = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| {
                    if let WindowSurface::X11(x) = e.underlying_surface() {
                        x == &window
                    } else {
                        false
                    }
                })
                .cloned()
        });
        if let Some(elem) = maybe {
            if let Some(surface) = elem.wl_surface() {
                self.workspaces.unmap_window(&surface.as_ref().id());
            } else if let Some(space) = self.workspaces.space_mut() {
                space.unmap_elem(&elem);
            }
        }
        if !window.is_override_redirect() {
            window.set_mapped(false).unwrap();
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // we just set the new size, but don't let windows move themselves around freely
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        tracing::debug!(
            "x11::configure_notify: geometry={:?} title={:?}",
            geometry,
            window.title()
        );
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == &window))
                .cloned()
        }) else {
            return;
        };
        self.workspaces.map_window(&elem, geometry.loc, false, None);
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.maximize_request_x11(&window);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == &window))
                .cloned()
        }) else {
            return;
        };

        window.set_maximized(false).unwrap();
        if let Some(old_geo) = window
            .user_data()
            .get::<OldGeometry>()
            .and_then(|data| data.restore())
        {
            tracing::debug!(
                "x11::unmaximize_request: restoring to old_geo={:?} title={:?}",
                old_geo,
                window.title()
            );
            window.configure(old_geo).unwrap();
            self.workspaces
                .map_window(&elem, old_geo.loc, false, Some(Transition::ease_out(0.3)));
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == &window))
                .cloned()
        }) else {
            return;
        };

        if elem.is_fullscreen() {
            return;
        }

        let outputs_for_window = self.workspaces.outputs_for_element(&elem);
        let output = outputs_for_window
            .first()
            .or_else(|| self.workspaces.outputs().next())
            .expect("No outputs found")
            .clone();
        let geometry = self.workspaces.output_geometry(&output).unwrap();

        let id = elem.id();

        // Save the current geometry so unfullscreen can restore it
        if let Some(mut view) = self.workspaces.get_window_view(&id) {
            let current_element_geometry = self.workspaces.element_geometry(&elem).unwrap();
            view.unmaximised_rect = current_element_geometry;
            self.workspaces.set_window_view(&id, view);
        }

        // Register for direct scanout
        output
            .user_data()
            .insert_if_missing(FullscreenSurface::default);
        output
            .user_data()
            .get::<FullscreenSurface>()
            .unwrap()
            .set(elem.clone());

        self.backend_data.reset_buffers(&output);

        // Create a dedicated fullscreen workspace
        let current_workspace_index = self.workspaces.get_current_workspace_index();
        let (next_workspace_index, next_workspace) = self.workspaces.get_next_free_workspace();
        next_workspace.set_fullscreen_mode(true);

        self.workspaces.expose_set_visible(false);

        elem.set_fullscreen(true, next_workspace_index);
        elem.set_workspace(current_workspace_index);

        self.workspaces
            .move_window_to_workspace(&elem, next_workspace_index, (0, 0));

        let transition = Transition::ease_in_out_quad(1.4);
        self.workspaces
            .set_current_workspace_index(next_workspace_index, Some(transition));

        self.workspaces.set_fullscreen_overlay_visibility(true);
        self.workspaces.dock.hide(None);

        window.set_fullscreen(true).unwrap();
        window.configure(geometry).unwrap();
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == &window))
                .cloned()
        }) else {
            return;
        };

        if !elem.is_fullscreen() {
            return;
        }

        // Clear direct-scanout registration
        if let Some(output) = self
            .workspaces
            .outputs()
            .find(|o| {
                o.user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                    .map(|w| w == elem)
                    .unwrap_or(false)
            })
            .cloned()
        {
            output
                .user_data()
                .get::<FullscreenSurface>()
                .unwrap()
                .clear();
            self.backend_data.reset_buffers(&output);
        }

        let prev_workspace = elem.get_workspace();
        let fullscreen_workspace_index = self.workspaces.get_current_workspace_index();

        if let Some(workspace) = self.workspaces.get_current_workspace() {
            workspace.set_fullscreen_mode(false);
            workspace.set_fullscreen_animating(false);
        }

        elem.set_fullscreen(false, 0);
        window.set_fullscreen(false).unwrap();

        let restore_loc = self
            .workspaces
            .get_window_view(&elem.id())
            .map(|v| v.unmaximised_rect.loc)
            .unwrap_or_default();

        let transition = Transition::ease_in_out_quad(1.4);

        self.workspaces
            .move_window_to_workspace(&elem, prev_workspace, restore_loc);
        self.workspaces
            .set_current_workspace_index(prev_workspace, Some(transition));

        // Remove workspace BEFORE calling expose_set_visible so that its on_finish
        // callback doesn't capture freed layer nodes from the fullscreen workspace.
        self.workspaces
            .remove_workspace_at(fullscreen_workspace_index);

        self.workspaces.expose_set_visible(false);
        self.workspaces.set_fullscreen_overlay_visibility(false);
        self.workspaces.dock.show(None);

        // Park the window layer under dnd_view before removing the fullscreen workspace,
        // then animate it to the restored position and re-parent it to the workspace on finish.
        // This mirrors the XDG unfullscreen flow and prevents freed-node panics from pending
        // transaction callbacks still holding references to the removed workspace's layers.
        if let Some(view) = self.workspaces.get_window_view(&elem.id()) {
            let scale = self
                .workspaces
                .outputs_for_element(&elem)
                .first()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0);
            let position = restore_loc.to_f64().to_physical(scale);

            let target_workspace = self.workspaces.get_workspace_at(prev_workspace);
            let workspace_layer = target_workspace.map(|ws| ws.windows_layer.clone());

            if let Err(e) = self
                .workspaces
                .dnd_view
                .layer
                .add_sublayer(&view.window_layer)
            {
                tracing::warn!("x11 unfullscreen: failed to park window in dnd layer: {e}");
            }

            view.window_layer
                .set_position(
                    layers::types::Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    },
                    Some(transition),
                )
                .on_finish(
                    move |l: &layers::prelude::Layer, _| {
                        if let Some(wl) = workspace_layer.as_ref() {
                            if let Err(e) = wl.add_sublayer(l) {
                                tracing::warn!("x11 unfullscreen: failed to reparent window: {e}");
                            }
                        }
                    },
                    true,
                );
        }

        // Tell X11 app its restored size
        let bbox = self.workspaces.space().and_then(|s| s.element_bbox(&elem));
        window.configure(bbox).unwrap();

        trace!("Unfullscreening X11: {:?}", elem);
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _button: u32,
        edges: X11ResizeEdge,
    ) {
        let start_data = self.pointer.grab_start_data().unwrap();

        let Some(element) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == &window))
                .cloned()
        }) else {
            return;
        };

        let geometry = element.geometry();
        let loc = self.workspaces.element_location(&element).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(&element.wl_surface().unwrap(), move |states| {
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
            window: element.clone(),
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        self.move_request_x11(&window)
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        if let Some(keyboard) = self.seat.get_keyboard() {
            // check that an X11 window is focused
            if let Some(KeyboardFocusTarget::Window(w)) = keyboard.current_focus() {
                if let WindowSurface::X11(surface) = w.underlying_surface() {
                    if surface.xwm_id().unwrap() == xwm {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland clipboard for Xwayland",
                    );
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland primary selection for Xwayland",
                    );
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        trace!(?selection, ?mime_types, "Got Selection from X11",);
        // TODO check, that focused windows is X11 window before doing this
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }
}

impl<BackendData: Backend> Otto<BackendData> {
    pub fn maximize_request_x11(&mut self, window: &X11Surface) {
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == window))
                .cloned()
        }) else {
            return;
        };

        let old_geo = self
            .workspaces
            .space()
            .and_then(|s| s.element_bbox(&elem))
            .unwrap();
        let outputs_for_window = self.workspaces.outputs_for_element(&elem);
        let output = outputs_for_window
            .first()
            .or_else(|| self.workspaces.outputs().next())
            .expect("No outputs found")
            .clone();
        let geometry = self.workspaces.output_geometry(&output).unwrap();

        tracing::debug!(
            "x11::maximize_request: title={:?} old_geo={:?} new_geometry={:?}",
            window.title(),
            old_geo,
            geometry
        );

        window.set_maximized(true).unwrap();
        window.configure(geometry).unwrap();
        window.user_data().insert_if_missing(OldGeometry::default);
        window
            .user_data()
            .get::<OldGeometry>()
            .unwrap()
            .save(old_geo);
        self.workspaces
            .map_window(&elem, geometry.loc, false, Some(Transition::ease_out(0.3)));
    }

    pub fn unmaximize_request_x11(&mut self, window: &X11Surface) {
        let Some(elem) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == window))
                .cloned()
        }) else {
            return;
        };

        window.set_maximized(false).unwrap();
        if let Some(old_geo) = window
            .user_data()
            .get::<OldGeometry>()
            .and_then(|data| data.restore())
        {
            tracing::debug!(
                "x11::unmaximize_request_x11: restoring to old_geo={:?} title={:?}",
                old_geo,
                window.title()
            );
            window.configure(old_geo).unwrap();
            self.workspaces
                .map_window(&elem, old_geo.loc, false, Some(Transition::ease_out(0.3)));
        }
    }

    pub fn move_request_x11(&mut self, window: &X11Surface) {
        if let Some(touch) = self.seat.get_touch() {
            if let Some(start_data) = touch.grab_start_data() {
                let element = self
                    .workspaces
                    .space()
                    .and_then(|s| {
                        s.elements()
                            .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == window))
                            .cloned()
                    });

                if let Some(element) = element {
                    let mut initial_window_location =
                        self.workspaces.element_location(&element).unwrap();

                    if window.is_maximized() {
                        let maximized_geometry = self
                            .workspaces
                            .space()
                            .and_then(|s| s.element_bbox(&element))
                            .unwrap();
                        let touch_location = start_data.location;

                        let grab_offset_x = touch_location.x - maximized_geometry.loc.x as f64;
                        let grab_offset_y = touch_location.y - maximized_geometry.loc.y as f64;

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

                        window.set_maximized(false).unwrap();

                        if let Some(old_geo) = window
                            .user_data()
                            .get::<OldGeometry>()
                            .and_then(|data| data.restore())
                        {
                            let new_grab_offset_x = grab_ratio_x * old_geo.size.w as f64;
                            let new_grab_offset_y = grab_ratio_y * old_geo.size.h as f64;

                            let new_x = touch_location.x - new_grab_offset_x;
                            let new_y = touch_location.y - new_grab_offset_y;

                            initial_window_location = (new_x as i32, new_y as i32).into();

                            window
                                .configure(Rectangle::new(initial_window_location, old_geo.size))
                                .unwrap();
                        } else {
                            let pos = start_data.location;
                            initial_window_location = (pos.x as i32, pos.y as i32).into();
                        }
                    }

                    let grab = TouchMoveSurfaceGrab {
                        start_data,
                        window: element.clone(),
                        initial_window_location,
                    };

                    touch.set_grab(self, grab, SERIAL_COUNTER.next_serial());
                    return;
                }
            }
        }

        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };

        let Some(element) = self.workspaces.space().and_then(|s| {
            s.elements()
                .find(|e| matches!(e.underlying_surface(), WindowSurface::X11(x) if x == window))
                .cloned()
        }) else {
            return;
        };

        let mut initial_window_location = self.workspaces.element_location(&element).unwrap();

        if window.is_maximized() {
            let maximized_geometry = self
                .workspaces
                .space()
                .and_then(|s| s.element_bbox(&element))
                .unwrap();
            let pointer_location = self.pointer.current_location();

            let grab_offset_x = pointer_location.x - maximized_geometry.loc.x as f64;
            let grab_offset_y = pointer_location.y - maximized_geometry.loc.y as f64;

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

            window.set_maximized(false).unwrap();

            if let Some(old_geo) = window
                .user_data()
                .get::<OldGeometry>()
                .and_then(|data| data.restore())
            {
                let new_grab_offset_x = grab_ratio_x * old_geo.size.w as f64;
                let new_grab_offset_y = grab_ratio_y * old_geo.size.h as f64;

                let new_x = pointer_location.x - new_grab_offset_x;
                let new_y = pointer_location.y - new_grab_offset_y;

                initial_window_location = (new_x as i32, new_y as i32).into();

                window
                    .configure(Rectangle::new(initial_window_location, old_geo.size))
                    .unwrap();
            } else {
                let pos = self.pointer.current_location();
                initial_window_location = (pos.x as i32, pos.y as i32).into();
            }
        }

        let grab = PointerMoveSurfaceGrab {
            start_data,
            window: element.clone(),
            initial_window_location,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }
}
