use crate::{focus::PointerFocusTarget, shell::FullscreenSurface, state::Backend, Otto};
use layers::skia::Contains;
use smithay::{
    backend::input::{
        self, Axis, AxisSource, ButtonState, Event, InputBackend, PointerAxisEvent,
        PointerButtonEvent,
    },
    desktop::{utils::under_from_surface_tree, WindowSurfaceType},
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    reexports::wayland_server::{protocol::wl_pointer, Resource},
    utils::{IsAlive, Logical, Point, Serial, SERIAL_COUNTER as SCOUNTER},
    wayland::{input_method::InputMethodSeat, shell::wlr_layer::Layer as WlrLayer},
};

/// Check if a point (in surface-local logical coordinates) falls within the
/// parent surface's input region.  Returns `true` when the region allows input
/// at that point (or when no explicit region is set, which means "accept all").
///
/// This is used as an additional gate for layer-shell hit testing: Wayland
/// subsurfaces have their own input regions and are not clipped by the parent's
/// region, but for compositor UI (layer shells with subsurfaces) we want the
/// parent's input region to act as the authoritative clickable area.
fn point_in_surface_input_region(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    point: Point<f64, Logical>,
) -> bool {
    smithay::wayland::compositor::with_states(surface, |states| {
        let mut attrs = states
            .cached_state
            .get::<smithay::wayland::compositor::SurfaceAttributes>();
        let attrs = attrs.current();
        match &attrs.input_region {
            Some(region) => region.contains(point.to_i32_round()),
            None => true, // no region set = accept all
        }
    })
}

#[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
use smithay::backend::input::AbsolutePositionEvent;

#[cfg(any(feature = "winit", feature = "x11"))]
use smithay::output::Output;

#[cfg(feature = "udev")]
use smithay::{
    backend::input::PointerMotionEvent,
    input::pointer::RelativeMotionEvent,
    wayland::{
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        seat::WaylandFocus,
    },
};

use crate::config::Config;

impl<BackendData: Backend> Otto<BackendData> {
    pub(crate) fn on_pointer_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
        let serial = SCOUNTER.next_serial();
        let button = evt.button_code();

        let state = wl_pointer::ButtonState::from(evt.state());

        if !self.workspaces.get_show_all() && wl_pointer::ButtonState::Pressed == state {
            self.focus_window_under_cursor(serial);
        }
        let pointer = self.pointer.clone();
        let button_state = state.try_into().unwrap();

        // On press, re-resolve the lay-rs hover against live layer positions
        // before dispatching the button. `current_hover()` is otherwise only
        // refreshed on motion, so a layer that animated under a stationary
        // cursor (dock launch bounce, autohide slide-in, magnification settle)
        // would leave a stale hover and the click would be silently dropped.
        if button_state == ButtonState::Pressed {
            let (cx, cy) = self.cursor_physical_position;
            self.layers_engine
                .pointer_move(&(cx as f32, cy as f32).into(), None);

            // Re-resolve the Wayland pointer focus against live surface
            // positions before dispatching the button. Smithay's pointer focus
            // is otherwise only refreshed on motion, so a window that animated,
            // relayouted, or appeared under a stationary cursor (no motion event
            // in between) would leave a stale focus and the press would land on
            // the wrong surface — or none — and be silently dropped. This is the
            // Wayland-side counterpart to the lay-rs hover refresh above.
            let location = self.pointer.current_location();
            let under = self.surface_under(location);
            pointer.motion(
                self,
                under,
                &MotionEvent {
                    location,
                    serial,
                    time: evt.time_msec(),
                },
            );
            pointer.frame(self);
        }

        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: button_state,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
        match button_state {
            ButtonState::Pressed => {
                self.layers_engine.pointer_button_down();
            }
            ButtonState::Released => {
                self.layers_engine.pointer_button_up();
            }
        }
    }

    /// Update the focus on the topmost surface under the cursor in the current workspace
    /// The window is raised and the keyboard focus is set to the window.
    pub(crate) fn focus_window_under_cursor(&mut self, serial: Serial) {
        // A locked session has one keyboard focus, and the locker owns it. The
        // desktop is still there under the lock surface, so without this a click
        // anywhere on the lock screen would hand focus to whatever happens to be
        // beneath the cursor — and the password typed next would go there.
        if self.is_session_locked() {
            return;
        }

        let keyboard = self.seat.get_keyboard().unwrap();
        let input_method = self.seat.input_method();

        // Get current focus to deactivate it
        let _old_focus = keyboard.current_focus();

        // change the keyboard focus unless the pointer or keyboard is grabbed
        // We test for any matching surface type here but always use the root
        // (in case of a window the toplevel) surface for the focus.
        // So for example if a user clicks on a subsurface or popup the toplevel
        // will receive the keyboard focus. Directly assigning the focus to the
        // matching surface leads to issues with clients dismissing popups and
        // subsurface menus (for example firefox-wayland).
        // see here for a discussion about that issue:
        // https://gitlab.freedesktop.org/wayland/wayland/-/issues/294
        if !self.pointer.is_grabbed() && (!keyboard.is_grabbed() || input_method.keyboard_grabbed())
        {
            let output = self
                .workspaces
                .output_under(self.pointer.current_location())
                .next()
                .cloned();
            if let Some(output) = output.as_ref() {
                let output_geo = self.workspaces.output_geometry(output).unwrap();
                if let Some(window) = output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                {
                    if let Some((_, _)) = window.surface_under::<BackendData>(
                        self.pointer.current_location() - output_geo.loc.to_f64(),
                        WindowSurfaceType::ALL,
                    ) {
                        #[cfg(feature = "xwayland")]
                        if let smithay::desktop::WindowSurface::X11(surf) =
                            window.underlying_surface()
                        {
                            self.xwm.as_mut().unwrap().raise_window(surf).unwrap();
                        }

                        self.set_keyboard_focus_on_window(&window);
                        return;
                    }
                }

                // Check if a Top/Overlay layer shell surface should receive keyboard focus
                // using lay-rs hit testing (matches visual position from Taffy layout)
                // Sort by stacking order: Overlay above Top
                let scale = output.current_scale().fractional_scale();
                let phys = self.pointer.current_location().to_physical(scale);
                let mut found_layer_focus = false;
                let mut layer_surfs: Vec<_> = self
                    .layer_surfaces
                    .values()
                    .filter(|s| matches!(s.wlr_layer(), WlrLayer::Overlay | WlrLayer::Top))
                    .collect();
                layer_surfs.sort_by_key(|s| match s.wlr_layer() {
                    WlrLayer::Overlay => 0,
                    _ => 1,
                });
                for layer_shell_surf in layer_surfs {
                    let lay_layer = &layer_shell_surf.layer;
                    if lay_layer.cointains_point((phys.x as f32, phys.y as f32)) {
                        let ls = layer_shell_surf.layer_surface();
                        let render_pos = lay_layer.render_position();
                        let layer_abs_pos: Point<f64, Logical> =
                            Point::from((render_pos.x as f64 / scale, render_pos.y as f64 / scale));
                        let relative_pos = self.pointer.current_location() - layer_abs_pos;
                        // Gate on the parent surface's input region so that
                        // subsurfaces outside it don't intercept events.
                        if !point_in_surface_input_region(ls.wl_surface(), relative_pos) {
                            continue;
                        }
                        if ls
                            .surface_under(relative_pos, WindowSurfaceType::ALL)
                            .is_none()
                        {
                            continue;
                        }
                        if ls.can_receive_keyboard_focus() {
                            keyboard.set_focus(self, Some(ls.clone().into()), serial);
                            found_layer_focus = true;
                            break;
                        }
                    }
                }
                if found_layer_focus {
                    return;
                }
            }
            let scale = output
                .as_ref()
                .map(|o| o.current_scale().fractional_scale())
                .unwrap_or(1.0);
            let position = self.pointer.current_location();
            let scaled_position = position.to_physical(scale);
            // The dock only exists on the primary output — its scene bounds
            // are primary-local, so testing them from another output's
            // coordinates can produce false hits.
            let on_primary = output
                .as_ref()
                .zip(self.workspaces.primary_output())
                .map(|(o, p)| o.name() == p.name())
                .unwrap_or(false);
            if !(on_primary
                && self
                    .workspaces
                    .is_cursor_over_dock(scaled_position.x as f32, scaled_position.y as f32))
            {
                let window_under = self
                    .workspaces
                    .element_under(position)
                    .map(|(w, p)| (w.clone(), p));

                if let Some((window, _)) = window_under {
                    if let Some(id) = window.wl_surface().as_ref().map(|s| s.id()) {
                        if let Some(w) = self.workspaces.get_window_for_surface(&id) {
                            if w.is_minimised() {
                                return;
                            }
                            if w.is_fullscreen() {
                                return;
                            }
                            self.workspaces.focus_app_with_window(&id);

                            self.set_keyboard_focus_on_window(&window);
                            self.workspaces.update_workspace_model();
                        }
                    }

                    #[cfg(feature = "xwayland")]
                    if let smithay::desktop::WindowSurface::X11(surf) = &window.underlying_surface()
                    {
                        self.xwm.as_mut().unwrap().raise_window(surf).unwrap();
                    }
                }
            }

            // Check if a Bottom/Background layer shell surface should receive keyboard focus
            if let Some(output) = output.as_ref() {
                let scale = output.current_scale().fractional_scale();
                let phys = self.pointer.current_location().to_physical(scale);
                for layer_shell_surf in self.layer_surfaces.values() {
                    let wlr = layer_shell_surf.wlr_layer();
                    if !matches!(wlr, WlrLayer::Bottom | WlrLayer::Background) {
                        continue;
                    }
                    let lay_layer = &layer_shell_surf.layer;
                    if lay_layer.cointains_point((phys.x as f32, phys.y as f32)) {
                        let ls = layer_shell_surf.layer_surface();
                        let render_pos = lay_layer.render_position();
                        let layer_abs_pos: Point<f64, Logical> =
                            Point::from((render_pos.x as f64 / scale, render_pos.y as f64 / scale));
                        let relative_pos = self.pointer.current_location() - layer_abs_pos;
                        if !point_in_surface_input_region(ls.wl_surface(), relative_pos) {
                            continue;
                        }
                        if ls
                            .surface_under(relative_pos, WindowSurfaceType::ALL)
                            .is_none()
                        {
                            continue;
                        }
                        if ls.can_receive_keyboard_focus() {
                            keyboard.set_focus(self, Some(ls.clone().into()), serial);
                        }
                        break;
                    }
                }
            }
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget<BackendData>, Point<f64, Logical>)> {
        let output = self.workspaces.outputs().find(|o| {
            let geometry = self.workspaces.output_geometry(o).unwrap();
            geometry.contains(pos.to_i32_round())
        })?;
        let scale = output.current_scale().fractional_scale();
        let physical_pos = pos.to_physical(scale);
        let mut under = None;

        // A locked session sees nothing below the lock surface. Returning None
        // when this output has no lock surface is deliberate: the click must
        // land nowhere rather than fall through to the desktop.
        if self.is_session_locked() {
            // The offset is where the surface *is*, not where the pointer is
            // inside it: Smithay subtracts it from the event location to get
            // surface-local coordinates. A lock surface covers its output, so
            // that is the output's origin — passing the local point instead
            // made every click arrive at (0, 0) and miss every control.
            let origin = self
                .workspaces
                .output_geometry(output)
                .map(|g| g.loc)
                .unwrap_or_default();
            return self.lock_surface_for_output(output).map(|surface| {
                (
                    PointerFocusTarget::from(surface),
                    Point::<f64, Logical>::from((origin.x as f64, origin.y as f64)),
                )
            });
        }

        // App switcher check
        if self.workspaces.app_switcher.alive() {
            let focus = self.workspaces.app_switcher.as_ref().clone().into();
            return Some((focus, (0.0, 0.0).into()));
        }

        // Workspace selector — per output. Skip when a window drag is active so the window
        // selector keeps receiving motion events and the dragged window keeps following the
        // pointer.
        if self.workspaces.get_show_all() && !self.workspaces.is_window_selector_dragging() {
            if let Some(selector) = self.workspaces.output_selector(&output.name()) {
                // Output subtrees render at the scene origin regardless of the output's global
                // position, so hit-test in output-local physical coordinates.
                let origin = self
                    .workspaces
                    .output_geometry(output)
                    .map(|g| g.loc)
                    .unwrap_or_default();
                let local =
                    Point::<f64, Logical>::from((pos.x - origin.x as f64, pos.y - origin.y as f64));
                let local_phys = local.to_physical(scale);
                let layer = selector.layer.clone();

                if layer.cointains_point((local_phys.x as f32, local_phys.y as f32)) {
                    let focus = selector.as_ref().clone().into();
                    let position = layer.render_position();
                    return Some((focus, (position.x as f64, position.y as f64).into()));
                }
            }
        }
        // Window selector check — use the selector of the output under the
        // cursor (each output has its own expose grid showing its own
        // current workspace).
        if self.workspaces.get_show_all() {
            let ows = self.workspaces.output_workspaces.get(&output.name())?;
            let workspace = ows.workspace_views.get(ows.current_workspace)?;
            let focus = workspace.window_selector_view.as_ref().clone().into();
            // Return the output's global origin as the focus offset so motion
            // events arrive output-local: the selector's hit rects live in the
            // output's own scene space (every output subtree renders at the
            // scene origin).
            let origin = self
                .workspaces
                .output_geometry(output)
                .map(|g| g.loc)
                .unwrap_or_default();

            return Some((focus, (origin.x as f64, origin.y as f64).into()));
        }

        // Check popup surfaces (layer shell popups) — they sit above everything
        // Sort by stacking order: Overlay above Top
        if under.is_none() {
            use smithay::desktop::PopupManager;

            let mut layer_surfs: Vec<_> = self
                .layer_surfaces
                .values()
                .filter(|s| matches!(s.wlr_layer(), WlrLayer::Overlay | WlrLayer::Top))
                .collect();
            layer_surfs.sort_by_key(|s| match s.wlr_layer() {
                WlrLayer::Overlay => 0,
                _ => 1,
            });
            for layer_shell_surf in layer_surfs {
                let layer_wl = layer_shell_surf.layer_surface().wl_surface();
                let popups: Vec<_> = PopupManager::popups_for_surface(layer_wl).collect();
                if !popups.is_empty() {
                    let lay_layer = &layer_shell_surf.layer;
                    let render_pos = lay_layer.render_position();
                    let layer_abs_pos: Point<f64, Logical> =
                        Point::from((render_pos.x as f64 / scale, render_pos.y as f64 / scale));
                    let cursor_rel_layer = pos - layer_abs_pos;
                    // Match Smithay's LayerSurface::surface_under approach:
                    // offset = popup_location - popup.geometry().loc
                    for (popup, popup_location) in popups.into_iter() {
                        let offset = popup_location - popup.geometry().loc;
                        let popup_surface = popup.wl_surface();
                        if let Some((surface, surface_loc)) = under_from_surface_tree(
                            popup_surface,
                            cursor_rel_layer,
                            offset,
                            WindowSurfaceType::ALL,
                        ) {
                            under = Some((
                                PointerFocusTarget::WlSurface(surface),
                                layer_abs_pos + surface_loc.to_f64(),
                            ));
                            break;
                        }
                    }
                }
                if under.is_some() {
                    break;
                }
            }
        }

        // Check Top/Overlay layer shell surfaces using lay-rs hit testing
        // (Smithay's layer_map geometry doesn't reflect Taffy/animated positions)
        // Sort by stacking order: Overlay above Top
        if under.is_none() {
            let mut layer_surfs: Vec<_> = self
                .layer_surfaces
                .values()
                .filter(|s| matches!(s.wlr_layer(), WlrLayer::Overlay | WlrLayer::Top))
                .collect();
            layer_surfs.sort_by_key(|s| match s.wlr_layer() {
                WlrLayer::Overlay => 0,
                _ => 1,
            });
            for layer_shell_surf in layer_surfs {
                let lay_layer = &layer_shell_surf.layer;
                if lay_layer.cointains_point((physical_pos.x as f32, physical_pos.y as f32)) {
                    let render_pos = lay_layer.render_position();
                    let layer_abs_pos: Point<f64, Logical> =
                        Point::from((render_pos.x as f64 / scale, render_pos.y as f64 / scale));
                    let ls = layer_shell_surf.layer_surface().clone();
                    let relative_pos = pos - layer_abs_pos;
                    // Gate on the parent surface's input region so that
                    // subsurfaces outside it don't intercept events.
                    if !point_in_surface_input_region(ls.wl_surface(), relative_pos) {
                        continue;
                    }
                    if let Some((surface, surface_loc)) =
                        ls.surface_under(relative_pos, WindowSurfaceType::ALL)
                    {
                        under = Some((
                            PointerFocusTarget::WlSurface(surface),
                            layer_abs_pos + surface_loc.to_f64(),
                        ));
                        break;
                    }
                    // surface_under returned None — the point is outside the
                    // input region. Continue checking other layers / windows.
                }
            }
        }

        // Check dock — primary output only (its scene bounds are primary-local)
        if under.is_none()
            && self
                .workspaces
                .primary_output()
                .is_some_and(|p| p.name() == output.name())
            && self
                .workspaces
                .is_cursor_over_dock(physical_pos.x as f32, physical_pos.y as f32)
        {
            under = Some((
                self.workspaces.dock.as_ref().clone().into(),
                (0.0, 0.0).into(),
            ));
        }

        // Resize borders. A server-decorated client draws no frame of its
        // own, so it never offers a resize edge: the strip along the window's
        // own border is Otto's, and it is hit-tested ahead of both the
        // titlebar and the client's surfaces (a press in the top corners
        // resizes rather than moves).
        if under.is_none() {
            if let Some((window, loc)) = self.workspaces.element_under(pos) {
                if window.is_decorated()
                    && !window.is_minimised()
                    && !window.is_maximized()
                    && !window.is_fullscreen()
                {
                    let local = pos - loc.to_f64();
                    let size = smithay::desktop::space::SpaceElement::geometry(window).size;
                    if let Some(edges) = crate::workspaces::resize_edges_at(local, size) {
                        let view = crate::workspaces::WindowResizeView::new(window.clone(), edges);
                        return Some((view.into(), loc.to_f64()));
                    }
                }
            }
        }

        // Server-side titlebars. The strip belongs to the compositor, not to
        // the client, so it is checked before the window's surface tree — the
        // client's surfaces start below it anyway.
        if under.is_none() {
            if let Some((window, loc)) = self.workspaces.element_under(pos) {
                let deco_height = window.decoration_height();
                if deco_height > 0 && !window.is_minimised() {
                    let local_y = pos.y - loc.y as f64;
                    if local_y >= 0.0 && local_y < deco_height as f64 {
                        let view = crate::workspaces::WindowDecorationView::new(window.clone());
                        // The offset is the window's origin: Smithay subtracts
                        // it from the event, so the view sees titlebar-local
                        // coordinates — the space `WindowDecoration` hit-tests
                        // in.
                        return Some((view.into(), loc.to_f64()));
                    }
                }
            }
        }

        // Check windows
        if under.is_none() {
            if let Some((focus, location)) =
                self.workspaces
                    .element_under(pos)
                    .and_then(|(window, loc)| {
                        if let Some(id) = window.wl_surface().as_ref().map(|s| s.id()) {
                            if let Some(w) = self.workspaces.get_window_for_surface(&id) {
                                if w.is_minimised() {
                                    return None;
                                }
                            }
                        }
                        window
                            .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                            .map(|(surface, surf_loc)| (surface, (surf_loc + loc).to_f64()))
                    })
            {
                under = Some((focus, location));
            }
        }

        // Check Bottom/Background layer shell surfaces using lay-rs hit testing
        if under.is_none() {
            for layer_shell_surf in self.layer_surfaces.values() {
                let wlr = layer_shell_surf.wlr_layer();
                if !matches!(wlr, WlrLayer::Bottom | WlrLayer::Background) {
                    continue;
                }
                let lay_layer = &layer_shell_surf.layer;
                if lay_layer.cointains_point((physical_pos.x as f32, physical_pos.y as f32)) {
                    let render_pos = lay_layer.render_position();
                    let layer_abs_pos: Point<f64, Logical> =
                        Point::from((render_pos.x as f64 / scale, render_pos.y as f64 / scale));
                    let ls = layer_shell_surf.layer_surface().clone();
                    let relative_pos = pos - layer_abs_pos;
                    if !point_in_surface_input_region(ls.wl_surface(), relative_pos) {
                        continue;
                    }
                    if let Some((surface, surface_loc)) =
                        ls.surface_under(relative_pos, WindowSurfaceType::ALL)
                    {
                        under = Some((
                            PointerFocusTarget::WlSurface(surface),
                            layer_abs_pos + surface_loc.to_f64(),
                        ));
                    } else {
                        under = Some((ls.into(), layer_abs_pos));
                    }
                    break;
                }
            }
        }

        under
    }

    /// Fractional scale of the output the pointer is currently on, 1.0 if it is
    /// not over any output.
    pub(crate) fn pointer_output_scale(&self) -> f64 {
        let pointer_location = self.pointer.current_location();
        self.workspaces
            .outputs()
            .find(|o| {
                self.workspaces
                    .output_geometry(o)
                    .map(|geo| geo.contains(pointer_location.to_i32_round()))
                    .unwrap_or(false)
            })
            .map(|o| o.current_scale().fractional_scale())
            .unwrap_or(1.0)
    }

    pub(crate) fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        // Remember which window is being scrolled. A window the user is
        // scrolling is a window the user is watching, so it keeps full-rate
        // frame callbacks even without keyboard focus — see
        // `Otto::pointer_interacting_ids`.
        let scrolled = self
            .workspaces
            .element_under(self.pointer.current_location())
            .and_then(|(w, _)| w.wl_surface().as_ref().map(|s| s.id()))
            .and_then(|id| self.workspaces.get_window_for_surface(&id).map(|w| w.id()));
        if let Some(id) = scrolled {
            self.pointer_interaction = Some((id, std::time::Instant::now()));
        }

        // Touchpad (and other continuous) scrolling reports the finger travel in
        // device pixels, exactly like pointer motion does — so it needs the same
        // conversion to logical pixels before it goes to the client, or scrolling
        // runs `scale` times too fast on a HiDPI output. The v120 fallback below
        // is already a logical-pixel figure and must not be scaled.
        let scroll_speed = Config::with(|c| c.input.scroll_speed);
        let continuous_speed = match evt.source() {
            AxisSource::Finger | AxisSource::Continuous => {
                scroll_speed / self.pointer_output_scale()
            }
            _ => scroll_speed,
        };
        let horizontal_amount = evt
            .amount(input::Axis::Horizontal)
            .map(|amount| amount * continuous_speed)
            .unwrap_or_else(|| {
                evt.amount_v120(input::Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120. * scroll_speed
            });
        let vertical_amount = evt
            .amount(input::Axis::Vertical)
            .map(|amount| amount * continuous_speed)
            .unwrap_or_else(|| {
                evt.amount_v120(input::Axis::Vertical).unwrap_or(0.0) * 15.0 / 120. * scroll_speed
            });
        let horizontal_amount_discrete = evt.amount_v120(input::Axis::Horizontal);
        let vertical_amount_discrete = evt.amount_v120(input::Axis::Vertical);

        {
            let mut frame = AxisFrame::new(evt.time_msec()).source(evt.source());
            if horizontal_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Horizontal, evt.relative_direction(Axis::Horizontal));
                frame = frame.value(Axis::Horizontal, horizontal_amount);
                if let Some(discrete) = horizontal_amount_discrete {
                    frame = frame.v120(Axis::Horizontal, discrete as i32);
                }
            }
            if vertical_amount != 0.0 {
                frame = frame
                    .relative_direction(Axis::Vertical, evt.relative_direction(Axis::Vertical));
                frame = frame.value(Axis::Vertical, vertical_amount);
                if let Some(discrete) = vertical_amount_discrete {
                    frame = frame.v120(Axis::Vertical, discrete as i32);
                }
            }
            if evt.source() == AxisSource::Finger {
                if evt.amount(Axis::Horizontal) == Some(0.0) {
                    frame = frame.stop(Axis::Horizontal);
                }
                if evt.amount(Axis::Vertical) == Some(0.0) {
                    frame = frame.stop(Axis::Vertical);
                }
            }
            let pointer = self.pointer.clone();
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }

    /// Check if the pointer is in the dock hot zone (bottom edge of the primary output)
    /// and show/hide the dock accordingly when autohide is enabled.
    pub(crate) fn check_dock_hot_zone(&mut self, pos: (f64, f64)) {
        if !self.workspaces.dock.is_autohide_enabled() {
            return;
        }
        // Don't trigger show dock if we're in the middle of a workspace switch or showing all workspaces
        if self.workspaces.get_show_all() || self.workspaces.is_expose_transitioning() {
            return;
        }
        let pos = layers::skia::Point::new(pos.0 as f32, pos.1 as f32);
        let hot_zone = *self.workspaces.dock.cached_hot_zone.read().unwrap();
        let dock_bounds = *self.workspaces.dock.cached_dock_bounds.read().unwrap();
        if let Some(hot_zone) = hot_zone {
            if hot_zone.contains(pos) && self.workspaces.dock.is_hidden() {
                self.workspaces.dock.show_autohide();
            }
        }
        if let Some(dock_bounds) = dock_bounds {
            if !dock_bounds.contains(pos) && !self.workspaces.dock.is_hidden() {
                self.workspaces.dock.schedule_autohide();
            }
        }
    }
}

/// Clamp a pointer position to the area covered by `geometries`, the logical
/// geometries of the pointer-reachable outputs.
///
/// Works off the real bounding box of the layout instead of assuming outputs
/// are laid out left-to-right from x=0, so vertically stacked, negatively
/// positioned or non-adjacent arrangements clamp correctly. The vertical range
/// comes from the outputs that actually span the clamped x, so the pointer
/// can't walk into the empty space beside a shorter screen.
#[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
pub(crate) fn clamp_to_output_bounds(
    pos: Point<f64, Logical>,
    geometries: &[smithay::utils::Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    if geometries.is_empty() {
        return pos;
    }

    let min_x = geometries.iter().map(|g| g.loc.x).min().unwrap() as f64;
    let max_x = geometries.iter().map(|g| g.loc.x + g.size.w).max().unwrap() as f64;
    let clamped_x = pos.x.clamp(min_x, max_x);

    // Outputs whose horizontal span covers the clamped x. Bounds are inclusive
    // so a position exactly on a shared edge belongs to both neighbours.
    let spanning = || {
        geometries
            .iter()
            .filter(|g| clamped_x >= g.loc.x as f64 && clamped_x <= (g.loc.x + g.size.w) as f64)
    };

    let (min_y, max_y) = match (
        spanning().map(|g| g.loc.y).min(),
        spanning().map(|g| g.loc.y + g.size.h).max(),
    ) {
        // x fell in a horizontal gap between outputs — fall back to the
        // vertical extent of the whole layout.
        (Some(min), Some(max)) => (min as f64, max as f64),
        _ => (
            geometries.iter().map(|g| g.loc.y).min().unwrap() as f64,
            geometries.iter().map(|g| g.loc.y + g.size.h).max().unwrap() as f64,
        ),
    };

    (clamped_x, pos.y.clamp(min_y, max_y)).into()
}

#[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
impl<Backend: crate::state::Backend> Otto<Backend> {
    /// Geometries of the outputs the pointer is allowed to reach.
    ///
    /// Non-interactive virtual outputs are excluded, and so are outputs with no
    /// geometry yet — during a hotplug the output set can be observed
    /// half-updated, and an `unwrap()` there would either panic or let a stale
    /// entry dictate the bounds.
    pub(crate) fn reachable_output_geometries(
        &self,
    ) -> Vec<smithay::utils::Rectangle<i32, Logical>> {
        self.workspaces
            .outputs()
            .filter(|o| !crate::virtual_output::is_unreachable_virtual_output(o))
            .filter_map(|o| self.workspaces.output_geometry(o))
            .collect()
    }

    pub(crate) fn clamp_coords(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        clamp_to_output_bounds(pos, &self.reachable_output_geometries())
    }
    pub(crate) fn on_pointer_move_absolute_windowed<B: InputBackend>(
        &mut self,
        evt: B::PointerMotionAbsoluteEvent,
        output: &Output,
    ) {
        let output_geo = self.workspaces.output_geometry(output).unwrap();

        let pos = evt.position_transformed(output_geo.size) + output_geo.loc.to_f64();
        let serial = SCOUNTER.next_serial();

        let under = self.surface_under(pos);
        let pointer = self.pointer.clone();
        // Cache pointer location for use in button events
        self.last_pointer_location = (pos.x, pos.y);
        // One relaxed store, so an assistive technology can follow the pointer
        // without anything being asked of the input path. See
        // [`crate::a11y::pointer_locator`].
        self.a11y.pointer.set(pos.x, pos.y);

        // Update focused output for workspace selector display
        {
            let focused = self.workspaces.output_under(pos).next().cloned();
            self.workspaces.set_focused_output(focused.as_ref());
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        let scale = output.current_scale().fractional_scale();
        let pos = pos.to_physical(scale);
        self.cursor_physical_position = (pos.x, pos.y);
        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);

        self.check_dock_hot_zone(self.last_pointer_location);
    }
}

#[cfg(feature = "udev")]
impl crate::Otto<crate::udev::UdevData> {
    pub(crate) fn on_pointer_move<B: InputBackend>(
        &mut self,
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        evt: B::PointerMotionEvent,
    ) {
        let mut pointer_location = self.pointer.current_location();
        let current_scale = self.pointer_output_scale();
        let logical_delta = {
            let p = evt.delta();
            Point::from((p.x / current_scale, p.y / current_scale))
        };
        let logical_delta_unaccel = {
            let p = evt.delta_unaccel();
            Point::from((p.x / current_scale, p.y / current_scale))
        };
        let serial = SCOUNTER.next_serial();

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);

        let mut pointer_locked = false;
        let mut pointer_confined = false;
        let mut confine_region = None;
        if let Some((surface, surface_loc)) = under
            .as_ref()
            .and_then(|(target, l)| Some((target.wl_surface()?, l)))
        {
            with_pointer_constraint(&surface, &pointer, |constraint| match constraint {
                Some(constraint) if constraint.is_active() => {
                    // Constraint does not apply if not within region
                    if !constraint.region().is_none_or(|x| {
                        x.contains((pointer_location - *surface_loc).to_i32_round())
                    }) {
                        return;
                    }
                    match &*constraint {
                        PointerConstraint::Locked(_locked) => {
                            pointer_locked = true;
                        }
                        PointerConstraint::Confined(confine) => {
                            pointer_confined = true;
                            confine_region = confine.region().cloned();
                        }
                    }
                }
                _ => {}
            });
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta: logical_delta,
                delta_unaccel: logical_delta_unaccel,
                utime: evt.time(),
            },
        );

        // If pointer is locked, only emit relative motion
        if pointer_locked {
            pointer.frame(self);
            return;
        }

        pointer_location += logical_delta;

        // clamp to screen limits
        // this event is never generated by winit
        pointer_location = self.clamp_coords(pointer_location);

        let new_under = self.surface_under(pointer_location);

        // If confined, don't move pointer if it would go outside surface or region
        if pointer_confined {
            if let Some((surface, surface_loc)) = &under {
                if new_under.as_ref().and_then(|(under, _)| under.wl_surface())
                    != surface.wl_surface()
                {
                    pointer.frame(self);
                    return;
                }
                if let Some(region) = confine_region {
                    if !region.contains((pointer_location - *surface_loc).to_i32_round()) {
                        pointer.frame(self);
                        return;
                    }
                }
            }
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        // Cache pointer location for use in button events
        self.last_pointer_location = (pointer_location.x, pointer_location.y);
        self.a11y
            .pointer
            .set(pointer_location.x, pointer_location.y);

        // Track which output the pointer is on — drives the flattened model
        // (expose, workspace selector, new-window routing). Cheap: no-op
        // when unchanged.
        {
            let focused = self
                .workspaces
                .output_under(pointer_location)
                .next()
                .cloned();
            self.workspaces.set_focused_output(focused.as_ref());
        }

        let scale = Config::with(|c| c.screen_scale);
        // lay-rs scene coordinates are OUTPUT-LOCAL (output subtrees overlap
        // at (0,0)) — rebase the global pointer to the focused output before
        // hit-testing, or hover/interaction never lands on secondary outputs.
        let origin = self
            .workspaces
            .focused_output()
            .map(|o| o.current_location())
            .unwrap_or_default();
        let pos = (pointer_location - origin.to_f64()).to_physical(scale);
        self.cursor_physical_position = (pos.x, pos.y);

        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);

        self.check_dock_hot_zone(self.last_pointer_location);

        // Schedule a redraw to update the cursor position
        self.schedule_event_loop_dispatch();

        // If pointer is now in a constraint region, activate it
        if let Some((under, surface_location)) =
            new_under.and_then(|(target, loc)| Some((target.wl_surface()?.into_owned(), loc)))
        {
            with_pointer_constraint(&under, &pointer, |constraint| match constraint {
                Some(constraint) if !constraint.is_active() => {
                    let point = (pointer_location - surface_location).to_i32_round();
                    if constraint
                        .region()
                        .is_none_or(|region| region.contains(point))
                    {
                        constraint.activate();
                    }
                }
                _ => {}
            });
        }
    }

    pub(crate) fn on_pointer_move_absolute<B: InputBackend>(
        &mut self,
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        evt: B::PointerMotionAbsoluteEvent,
    ) {
        let serial = SCOUNTER.next_serial();

        // Absolute events are normalised against the bounding box of the
        // pointer-reachable outputs. Same filtering as `clamp_coords`, or an
        // unreachable virtual output would stretch the mapping and the pointer
        // would only ever cover part of the real screens.
        let geometries = self.reachable_output_geometries();
        let (Some(min_x), Some(max_x), Some(min_y), Some(max_y)) = (
            geometries.iter().map(|g| g.loc.x).min(),
            geometries.iter().map(|g| g.loc.x + g.size.w).max(),
            geometries.iter().map(|g| g.loc.y).min(),
            geometries.iter().map(|g| g.loc.y + g.size.h).max(),
        ) else {
            return;
        };

        let mut pointer_location = (
            min_x as f64 + evt.x_transformed(max_x - min_x),
            min_y as f64 + evt.y_transformed(max_y - min_y),
        )
            .into();

        // clamp to screen limits
        pointer_location = self.clamp_coords(pointer_location);

        let pointer = self.pointer.clone();
        let under = self.surface_under(pointer_location);
        // Cache pointer location for use in button events
        self.last_pointer_location = (pointer_location.x, pointer_location.y);
        self.a11y
            .pointer
            .set(pointer_location.x, pointer_location.y);

        // Track which output the pointer is on (see on_pointer_move).
        {
            let focused = self
                .workspaces
                .output_under(pointer_location)
                .next()
                .cloned();
            self.workspaces.set_focused_output(focused.as_ref());
        }

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pointer_location,
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);

        let scale = Config::with(|c| c.screen_scale);
        // lay-rs scene coordinates are OUTPUT-LOCAL (output subtrees overlap
        // at (0,0)) — rebase the global pointer to the focused output before
        // hit-testing, or hover/interaction never lands on secondary outputs.
        let origin = self
            .workspaces
            .focused_output()
            .map(|o| o.current_location())
            .unwrap_or_default();
        let pos = (pointer_location - origin.to_f64()).to_physical(scale);
        self.cursor_physical_position = (pos.x, pos.y);

        self.layers_engine
            .pointer_move(&(pos.x as f32, pos.y as f32).into(), None);

        self.check_dock_hot_zone(self.last_pointer_location);

        // Schedule a redraw to update the cursor position
        self.schedule_event_loop_dispatch();
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Logical, Point, Rectangle};
    use smithay::wayland::compositor::RegionAttributes;

    /// Simulates the input-region gate used in layer-shell hit testing.
    /// When a region is set, only points inside it should pass.
    /// When no region is set (None), all points pass.
    fn region_contains(region: &Option<RegionAttributes>, point: Point<i32, Logical>) -> bool {
        match region {
            Some(r) => r.contains(point),
            None => true,
        }
    }

    #[cfg(any(feature = "winit", feature = "x11", feature = "udev"))]
    mod clamp {
        use super::super::clamp_to_output_bounds;
        use smithay::utils::{Logical, Point, Rectangle};

        fn geo(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
            Rectangle::new((x, y).into(), (w, h).into())
        }

        fn clamp(pos: (f64, f64), geometries: &[Rectangle<i32, Logical>]) -> (f64, f64) {
            let p: Point<f64, Logical> = clamp_to_output_bounds(pos.into(), geometries);
            (p.x, p.y)
        }

        #[test]
        fn no_outputs_passes_position_through() {
            assert_eq!(clamp((-40.0, 9000.0), &[]), (-40.0, 9000.0));
        }

        #[test]
        fn single_output_clamps_to_its_extent() {
            let outputs = [geo(0, 0, 1920, 1080)];
            assert_eq!(clamp((-10.0, -10.0), &outputs), (0.0, 0.0));
            assert_eq!(clamp((5000.0, 5000.0), &outputs), (1920.0, 1080.0));
            assert_eq!(clamp((100.0, 200.0), &outputs), (100.0, 200.0));
        }

        #[test]
        fn side_by_side_uses_the_output_under_x_for_height() {
            // Tall laptop panel on the left, short external on the right.
            let outputs = [geo(0, 0, 1920, 1200), geo(1920, 0, 1280, 720)];
            assert_eq!(clamp((500.0, 1100.0), &outputs), (500.0, 1100.0));
            // Over the short output the pointer must stop at its bottom edge,
            // not at the taller neighbour's.
            assert_eq!(clamp((2500.0, 1100.0), &outputs), (2500.0, 720.0));
            assert_eq!(clamp((4000.0, 100.0), &outputs), (3200.0, 100.0));
        }

        #[test]
        fn vertically_stacked_outputs_are_both_reachable() {
            let outputs = [geo(0, 0, 1920, 1080), geo(0, 1080, 1920, 1080)];
            // Summing widths would have capped x at 3840 and y at 1080,
            // making the bottom screen unreachable.
            assert_eq!(clamp((1000.0, 1500.0), &outputs), (1000.0, 1500.0));
            assert_eq!(clamp((1000.0, 9000.0), &outputs), (1000.0, 2160.0));
            assert_eq!(clamp((3000.0, 500.0), &outputs), (1920.0, 500.0));
        }

        #[test]
        fn negative_origin_is_reachable() {
            // External monitor placed to the left of the laptop.
            let outputs = [geo(-1280, 0, 1280, 720), geo(0, 0, 1920, 1080)];
            assert_eq!(clamp((-600.0, 300.0), &outputs), (-600.0, 300.0));
            assert_eq!(clamp((-5000.0, 300.0), &outputs), (-1280.0, 300.0));
            assert_eq!(clamp((-600.0, 900.0), &outputs), (-600.0, 720.0));
        }

        #[test]
        fn horizontal_gap_falls_back_to_the_full_vertical_extent() {
            let outputs = [geo(0, 0, 800, 600), geo(2000, 0, 800, 1000)];
            let (x, y) = clamp((1500.0, 5000.0), &outputs);
            assert_eq!(x, 1500.0);
            assert_eq!(y, 1000.0);
        }
    }

    #[test]
    fn no_region_accepts_all_points() {
        let region: Option<RegionAttributes> = None;
        assert!(region_contains(&region, (0, 0).into()));
        assert!(region_contains(&region, (500, 500).into()));
        assert!(region_contains(&region, (-10, -10).into()));
    }

    #[test]
    fn empty_region_rejects_all_points() {
        let region = Some(RegionAttributes { rects: vec![] });
        assert!(!region_contains(&region, (0, 0).into()));
        assert!(!region_contains(&region, (100, 15).into()));
    }

    #[test]
    fn single_rect_region_clips_subsurface_area() {
        // Simulates a compact music pill: 220x30 centered in a 480px-wide layer
        // that has subsurfaces extending to 460x140.  Without the input-region
        // gate the subsurface area (y > 30) would still intercept pointer events.
        let x = 130; // (480 - 220) / 2
        let region = Some(RegionAttributes {
            rects: vec![(
                smithay::wayland::compositor::RectangleKind::Add,
                Rectangle::new((x, 0).into(), (220, 30).into()),
            )],
        });

        // Inside the pill — should pass
        assert!(region_contains(&region, (200, 15).into()));
        assert!(region_contains(&region, (130, 0).into()));
        assert!(region_contains(&region, (349, 29).into()));

        // Outside the pill — should be rejected so subsurfaces don't intercept
        assert!(!region_contains(&region, (0, 15).into())); // left of pill
        assert!(!region_contains(&region, (400, 15).into())); // right of pill
        assert!(!region_contains(&region, (200, 50).into())); // below pill
        assert!(!region_contains(&region, (200, 200).into())); // far below (subsurface buffer)
    }

    #[test]
    fn expanded_region_covers_pill_and_cards() {
        // Simulates expanded notification island: pill + card stack
        let pill = (
            smithay::wayland::compositor::RectangleKind::Add,
            Rectangle::new((130, 0).into(), (220, 30).into()),
        );
        let cards = (
            smithay::wayland::compositor::RectangleKind::Add,
            Rectangle::new((90, 34).into(), (300, 200).into()),
        );
        let region = Some(RegionAttributes {
            rects: vec![pill, cards],
        });

        // In pill
        assert!(region_contains(&region, (200, 15).into()));
        // In cards
        assert!(region_contains(&region, (200, 100).into()));
        // Gap between pill and cards
        assert!(!region_contains(&region, (200, 32).into()));
        // Outside both
        assert!(!region_contains(&region, (50, 15).into()));
    }

    #[test]
    fn region_with_subtract_creates_hole() {
        let region = Some(RegionAttributes {
            rects: vec![
                (
                    smithay::wayland::compositor::RectangleKind::Add,
                    Rectangle::new((0, 0).into(), (480, 400).into()),
                ),
                (
                    smithay::wayland::compositor::RectangleKind::Subtract,
                    Rectangle::new((100, 100).into(), (100, 100).into()),
                ),
            ],
        });

        assert!(region_contains(&region, (50, 50).into()));
        assert!(!region_contains(&region, (150, 150).into())); // in the hole
        assert!(region_contains(&region, (250, 250).into()));
    }
}
