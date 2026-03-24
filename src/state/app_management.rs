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
        if let Some(top_wid) = self.workspaces.get_top_window_of_workspace(workspace_index) {
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
                .and_then(|wv| wv.window_selector_view.get_selected_window_id());
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
        keyboard.set_focus(self, Some(window.clone().into()), serial);
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
