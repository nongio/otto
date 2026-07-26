use smithay::{
    desktop::space::SpaceElement, reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    reexports::wayland_server::backend::ObjectId, utils::SERIAL_COUNTER,
};

use crate::focus::KeyboardFocusTarget;

use super::{Backend, Otto};

impl<BackendData: Backend> Otto<BackendData> {
    pub fn quit_appswitcher_app(&mut self) {
        self.workspaces.quit_appswitcher_app();
        // FIXME focus the previous window
    }
    pub fn toggle_maximize_focused_window(&mut self) {
        let Some(window) = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| match focus {
                KeyboardFocusTarget::Window(window) => Some(window),
                _ => None,
            })
        else {
            return;
        };

        match window.underlying_surface() {
            smithay::desktop::WindowSurface::Wayland(_) => {
                if let Some(toplevel) = window.toplevel() {
                    let toplevel = toplevel.clone();
                    let is_maximized = toplevel.with_pending_state(|state| {
                        state.states.contains(xdg_toplevel::State::Maximized)
                    });
                    if is_maximized {
                        <Self as smithay::wayland::shell::xdg::XdgShellHandler>::unmaximize_request(
                            self, toplevel,
                        );
                    } else {
                        <Self as smithay::wayland::shell::xdg::XdgShellHandler>::maximize_request(
                            self, toplevel,
                        );
                    }
                }
            }
            #[cfg(feature = "xwayland")]
            smithay::desktop::WindowSurface::X11(surface) => {
                if surface.is_maximized() {
                    self.unmaximize_request_x11(surface);
                } else {
                    self.maximize_request_x11(surface);
                }
            }
        }
    }
    /// Re-run maximize_request on every currently-maximized window so that the
    /// new usable geometry (e.g. dock just became visible) is applied immediately.
    pub fn remaximize_maximized_windows(&mut self) {
        let windows: Vec<_> = self.workspaces.spaces_elements().cloned().collect();
        for window in windows {
            match window.underlying_surface() {
                smithay::desktop::WindowSurface::Wayland(_) => {
                    if let Some(toplevel) = window.toplevel() {
                        let is_maximized = toplevel.with_pending_state(|state| {
                            state.states.contains(xdg_toplevel::State::Maximized)
                        });
                        if is_maximized {
                            let toplevel = toplevel.clone();
                            <Self as smithay::wayland::shell::xdg::XdgShellHandler>::maximize_request(
                                self, toplevel,
                            );
                        }
                    }
                }
                #[cfg(feature = "xwayland")]
                smithay::desktop::WindowSurface::X11(surface) => {
                    if surface.is_maximized() {
                        self.maximize_request_x11(surface);
                    }
                }
            }
        }
    }

    pub fn close_focused_window(&mut self) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            if let Some(KeyboardFocusTarget::Window(window)) = keyboard.current_focus() {
                match window.underlying_surface() {
                    smithay::desktop::WindowSurface::Wayland(toplevel) => toplevel.send_close(),
                    #[cfg(feature = "xwayland")]
                    smithay::desktop::WindowSurface::X11(surface) => {
                        let _ = surface.close();
                    }
                }
            }
        }
    }

    pub fn raise_next_app_window(&mut self) {
        if let Some(wid) = self.workspaces.raise_next_app_window() {
            self.set_keyboard_focus_on_surface(&wid);
        }
    }

    pub fn focus_app(&mut self, app_id: &str) -> bool {
        if let Some(wid) = self.workspaces.focus_app(app_id) {
            self.set_keyboard_focus_on_surface(&wid);
            true
        } else {
            false
        }
    }

    pub fn activate_window(&mut self, wid: &ObjectId) {
        if let Some(focused) = self.workspaces.focus_app_with_window(wid) {
            self.set_keyboard_focus_on_surface(&focused);
        }
    }

    /// Focus the top (non-minimised) window of the given workspace, or clear
    /// keyboard focus when the workspace is empty.  Used by every code-path that
    /// lands on a workspace (gesture swipe, selector click, expose close, …).
    pub fn focus_top_window_or_clear(&mut self, workspace_index: usize) {
        // While a self-managing X11 game (Cuphead) is fullscreen, keep keyboard
        // focus ON IT across workspace switches. Moving focus to another window —
        // or clearing it to None on an empty workspace — makes XWayland send the
        // game an X11 `FocusOut`, and Unity games with "Run In Background = false"
        // PAUSE rendering on FocusOut (the workspace-switch freeze). Holding focus
        // on the game means it never sees FocusOut. Released once it unfullscreens.
        #[cfg(feature = "xwayland")]
        if self
            .workspaces
            .windows_map
            .values()
            .any(|w| w.is_fullscreen() && w.x11_self_manages_focus())
        {
            return;
        }
        // Read the focused output's space, not the primary output's — each
        // output has its own workspace stack.
        let top = self
            .workspaces
            .focused_output_workspaces()
            .and_then(|ows| ows.spaces.get(workspace_index))
            .and_then(|space| {
                space.elements().rev().find_map(|e| {
                    let id = e.id();
                    if let Some(w) = self.workspaces.windows_map.get(&id) {
                        if w.is_minimised() {
                            return None;
                        }
                    }
                    Some(id)
                })
            });
        if let Some(top_wid) = top {
            self.set_keyboard_focus_on_surface(&top_wid);
        } else {
            self.clear_keyboard_focus();
        }
    }

    pub fn set_current_workspace_index(&mut self, index: usize) {
        // Use the focused output from the model cache — safe to call from button handlers
        // (avoids re-acquiring the pointer lock, which would deadlock inside a Smithay handler).
        let target_output = self.workspaces.focused_output().cloned();
        if let Some(output) = target_output {
            self.workspaces
                .set_workspace_for_output(&output, index, None);
        } else {
            self.workspaces.set_current_workspace_index(index, None);
        }
        // Focus the top window of the new workspace, or clear focus if empty
        self.focus_top_window_or_clear(index);
    }

    pub fn close_expose_show_all_and_focus_top(&mut self) {
        tracing::debug!("close_expose_show_all_and_focus_top");
        let was_open = self.workspaces.get_show_all();
        tracing::debug!("close_expose_show_all_and_focus_top: was_open={}", was_open);
        // Read hovered window BEFORE expose_set_visible(false) clears the selection.
        let hovered = if was_open {
            let workspace_index = self.workspaces.get_current_workspace_index();
            let h = self
                .workspaces
                .get_workspace_at(workspace_index)
                .and_then(|wv| wv.window_selector_view.get_selected_window_id());
            tracing::debug!("close_expose_show_all_and_focus_top: hovered={:?}", h);
            h
        } else {
            None
        };
        self.workspaces.expose_set_visible(false);
        if was_open {
            let workspace_index = self.workspaces.get_current_workspace_index();
            self.workspaces
                .apply_window_selector_order_to_workspace(workspace_index);
            if let Some(wid) = hovered {
                tracing::debug!("close_expose_show_all_and_focus_top: focused={:?}", wid);
                self.activate_window(&wid);
            } else {
                tracing::debug!(
                    "close_expose_show_all_and_focus_top: no hover, focusing current workspace top"
                );
                self.focus_top_window_or_clear(workspace_index);
                // expose_set_visible animates the dock position but never updates dock.active.
                // When a window is clicked, focus_app_with_window → set_current_workspace_index
                // → dock.show()/hide() fixes this. For the empty-space click path we must sync
                // the active flag here so the dock becomes interactive after expose closes.
                if !self.workspaces.dock.is_autohide_enabled() {
                    let is_fullscreen = self
                        .workspaces
                        .get_workspace_at(workspace_index)
                        .map(|w| w.get_fullscreen_mode())
                        .unwrap_or(false);
                    self.workspaces.dock.set_active_flag(!is_fullscreen);
                }
            }
        }
    }

    pub fn expose_end_with_velocity_and_focus_top(&mut self, raw_velocity: f32) {
        tracing::debug!(
            "expose_end_with_velocity_and_focus_top: velocity={}",
            raw_velocity
        );
        let was_open = self.workspaces.get_show_all();
        // Read hovered window BEFORE expose_end_with_velocity clears the selection.
        let hovered = if was_open {
            let workspace_index = self.workspaces.get_current_workspace_index();
            let h = self
                .workspaces
                .get_workspace_at(workspace_index)
                .and_then(|wv| {
                    // The close gesture clears the selection before we get here,
                    // so fall back to the snapshot taken at gesture start.
                    wv.window_selector_view
                        .get_selected_window_id()
                        .or_else(|| wv.window_selector_view.take_pre_close_hovered())
                });
            tracing::debug!("expose_end_with_velocity_and_focus_top: hovered={:?}", h);
            h
        } else {
            None
        };
        self.workspaces.expose_end_with_velocity(raw_velocity);
        let is_open_after = self.workspaces.get_show_all();
        tracing::debug!(
            "expose_end_with_velocity_and_focus_top: was_open={} is_open_after={}",
            was_open,
            is_open_after
        );
        if was_open && !is_open_after {
            let workspace_index = self.workspaces.get_current_workspace_index();
            self.workspaces
                .apply_window_selector_order_to_workspace(workspace_index);
            if let Some(wid) = hovered {
                tracing::debug!("expose_end_with_velocity_and_focus_top: focused={:?}", wid);
                self.activate_window(&wid);
            } else {
                tracing::debug!(
                    "expose_end_with_velocity_and_focus_top: no hover, focusing current workspace top"
                );
                self.focus_top_window_or_clear(workspace_index);
            }
        }
    }

    pub fn set_keyboard_focus_on_surface(&mut self, wid: &ObjectId) {
        let window = self.workspaces.get_window_for_surface(wid).cloned();
        if let Some(window) = window {
            self.set_keyboard_focus_on_window(&window);
        }
    }

    /// Centralized keyboard focus change: deactivates old window, activates new one,
    /// sends xdg configure and foreign-toplevel state for both.
    pub fn set_keyboard_focus_on_window(&mut self, window: &crate::shell::WindowElement) {
        // While a self-managing X11 game (Cuphead) is fullscreen, never move focus
        // to a DIFFERENT window (pointer, dock, self-activation by Steam Big
        // Picture, …). XWayland would send the game an X11 `FocusOut` and Unity
        // ("Run In Background = false") pauses rendering on it. Focusing the game
        // itself is allowed (target == game).
        #[cfg(feature = "xwayland")]
        {
            let target_id = window.id();
            if self
                .workspaces
                .windows_map
                .values()
                .any(|w| w.is_fullscreen() && w.x11_self_manages_focus() && w.id() != target_id)
            {
                return;
            }
        }

        let keyboard = self.seat.get_keyboard().unwrap();
        let serial = SERIAL_COUNTER.next_serial();

        // Deactivate the previously focused window
        if let Some(crate::focus::KeyboardFocusTarget::Window(old_window)) =
            keyboard.current_focus()
        {
            if old_window.wl_surface() != window.wl_surface() {
                old_window.set_activate(false);
                if let Some(view) = self.workspaces.get_window_view(&old_window.id()) {
                    view.set_active(false);
                }
                if let Some(toplevel) = old_window.toplevel() {
                    toplevel.send_configure();
                }
                let old_id = old_window.id();
                self.send_foreign_toplevel_state(&old_id, false);
            }
        }

        // Activate the new window and send configure
        window.set_activate(true);
        if let Some(view) = self.workspaces.get_window_view(&window.id()) {
            view.set_active(true);
        }
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_configure();
        }
        let wid = window.id();
        self.send_foreign_toplevel_state(&wid, true);

        // Always give the window the seat keyboard focus, so its keys are routed.
        // The ICCCM input model is honoured one level down, in
        // `KeyboardFocusTarget`'s `KeyboardTarget` impl (src/focus.rs): for
        // self-managing X11 clients (Globally-Active / No-Input) we deliver the
        // wl_keyboard events straight to the underlying wl_surface and SKIP the X11
        // focus protocol (set_input_focus / WM_TAKE_FOCUS). Forcing that protocol
        // stalls the render loop of Globally-Active Proton/Unity games (e.g.
        // Cuphead) — but they still need to receive keys, hence focus-but-no-X11-
        // state. set_activate above sets _NET_WM_STATE_FOCUSED and the call below
        // sets _NET_ACTIVE_WINDOW, which is what they actually poll for.
        keyboard.set_focus(self, Some(window.clone().into()), serial);
        self.set_x11_active_window(window);
    }

    /// Mirror keyboard focus to the X11 world by setting `_NET_ACTIVE_WINDOW`.
    ///
    /// XWayland clients with a Globally Active input model (e.g. Unity games via
    /// Proton, which set `input=False` and list `WM_TAKE_FOCUS`) block their
    /// render loop until they observe activation. `WM_TAKE_FOCUS` + `_NET_WM_STATE_FOCUSED`
    /// alone are not enough — they also poll `_NET_ACTIVE_WINDOW` on the root window.
    /// Without this the window stays black with no frames (the Cuphead startup deadlock).
    pub fn set_x11_active_window(&mut self, window: &crate::shell::WindowElement) {
        use smithay::desktop::WindowSurface;

        // Never hand `_NET_ACTIVE_WINDOW` to another window while a self-managing
        // X11 game is fullscreen. Unity/Proton games (Cuphead) QUIT when they lose
        // active-window status while fullscreen — losing it to Steam Big Picture on
        // a focus flap, or to another workspace's top window on swipe, makes Cuphead
        // tear down its X11 window (observed: unmap + ReparentWindow BadWindow).
        // Keep the game `_NET_ACTIVE_WINDOW` until it's unfullscreened/closed.
        #[cfg(feature = "xwayland")]
        {
            let target_id = window.id();
            let game_is_fullscreen =
                self.workspaces.windows_map.values().any(|w| {
                    w.is_fullscreen() && w.x11_self_manages_focus() && w.id() != target_id
                });
            if game_is_fullscreen {
                return;
            }
        }

        let WindowSurface::X11(surface) = window.underlying_surface() else {
            return;
        };
        let window_id = surface.window_id();
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.set_active_window(window_id) {
                tracing::warn!(?err, "failed to set _NET_ACTIVE_WINDOW for X11 window");
            }
        }
    }

    pub fn clear_keyboard_focus(&mut self) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            let serial = SERIAL_COUNTER.next_serial();

            // Deactivate the currently focused window when clearing focus
            if let Some(crate::focus::KeyboardFocusTarget::Window(old_window)) =
                keyboard.current_focus()
            {
                old_window.set_activate(false);
                // Update shadow for deactivated window
                if let Some(view) = self.workspaces.get_window_view(&old_window.id()) {
                    view.set_active(false);
                }
                if let Some(toplevel) = old_window.toplevel() {
                    toplevel.send_configure();
                }
                let old_id = old_window.id();
                self.send_foreign_toplevel_state(&old_id, false);
            }

            keyboard.set_focus(self, None, serial);
        }
    }
}
