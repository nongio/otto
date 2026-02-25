use layers::{prelude::Transition, skia};
use otto_kit::components::context_menu::ContextMenuRenderer;
use smithay::{backend::input::{ButtonState, KeyState}, input::keyboard::Keysym, utils::IsAlive};

use crate::{config::Config, interactive_view::{InteractiveView, ViewInteractions}};

use tracing::warn;

use super::DockView;

// Dock view interactions
impl<Backend: crate::state::Backend> ViewInteractions<Backend> for DockView {
    fn id(&self) -> Option<usize> {
        Some(self.wrap_layer.id.0.into())
    }
    fn is_alive(&self) -> bool {
        self.alive()
    }
    fn on_motion(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        _data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        if self.dragging.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let scale = Config::with(|c| c.screen_scale);
        if let Some(menu) = self.context_menu.read().unwrap().as_ref().filter(|m| m.is_active()) {
            let mut menu_state = menu.view.get_state();
            let items = menu_state.items();
            let style = &menu_state.style;
            // render_bounds_transformed returns physical pixels; event.location is logical.
            let menu_bounds = menu.view_layer.render_bounds_transformed();
            let x = event.location.x as f32 - menu_bounds.left / scale as f32;
            let y = event.location.y as f32 - menu_bounds.top / scale as f32;
            let item_index = ContextMenuRenderer::hit_test_items(items, style, x, y);
            menu_state.select_at_depth(0, item_index);
            menu.view.update_state(&menu_state);
        }

        self.update_magnification_position((event.location.x * scale) as f32);
    }
    fn on_leave(&self, _serial: smithay::utils::Serial, _time: u32) {
        self.schedule_autohide();
    }
    fn on_enter(&self, _event: &smithay::input::pointer::MotionEvent) {
        self.show_autohide();
    }
    fn on_button(
        &self,
        seat: &smithay::input::Seat<crate::Otto<Backend>>,
        state: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        const BTN_RIGHT: u32 = 0x111; // 273

        match event.state {
            ButtonState::Pressed => {
                // println!("dock Button pressed");
                if let Some(layer_id) = state.layers_engine.current_hover() {
                    if let Some((_identifier, match_id)) = self.get_app_from_layer(&layer_id) {
                        let apps_layers = self.app_layers.read().unwrap();
                        if let Some(entry) = apps_layers.get(&match_id) {
                            let darken_color = skia::Color::from_argb(100, 100, 100, 100);
                            let add = skia::Color::from_argb(0, 0, 0, 0);
                            let filter = skia::color_filters::lighting(darken_color, add);
                            entry.icon_scaler.set_color_filter(filter);
                            entry.icon_scaler.set_opacity(1.0, None);
                            entry.label_layer.set_opacity(1.0, Some(Transition::ease_in_quad(0.05)));
                        }
                    }
                }
            }
            ButtonState::Released => {
                // If context menu is open, forward the click to it
                {
                    use crate::config::Config;
                    use otto_kit::components::context_menu::ContextMenuRenderer;
                    let scale = Config::with(|c| c.screen_scale) as f32;
                    let menu_lock = self.context_menu.read().unwrap();
                    if let Some(menu) = menu_lock.as_ref().filter(|m| m.is_active()) {
                        let menu_state = menu.view.get_state();
                        let items = menu_state.items();
                        let style = &menu_state.style;
                        let menu_bounds = menu.view_layer.render_bounds_transformed();
                        let ptr = state.last_pointer_location;
                        let x = ptr.0 as f32 - menu_bounds.left / scale;
                        let y = ptr.1 as f32 - menu_bounds.top / scale;
                        let item_index = ContextMenuRenderer::hit_test_items(items, style, x, y);
                        drop(menu_lock);
                        if let Some(idx) = item_index {
                            // Get action_id and app_id synchronously before closing the menu
                            let action_id = {
                                let menu_lock = self.context_menu.read().unwrap();
                                menu_lock.as_ref()
                                    .and_then(|m| m.view.get_state().items_at_depth(0).get(idx).and_then(|i| i.action_id()).map(|s| s.to_string()))
                            };
                            let app_id = self.context_menu_app_id.read().unwrap().clone();
                            // Execute action immediately (while we have &mut state)
                            if let (Some(action_id), Some(app_id)) = (action_id, app_id) {
                                self.execute_context_menu_action(&action_id, &app_id, state);
                            }
                            // Pulse animation plays on the still-visible menu, then closes it
                            {
                                let menu_lock = self.context_menu.read().unwrap();
                                if let Some(menu) = menu_lock.as_ref() {
                                    menu.pulse_then_close(0, idx, self.clone());
                                }
                            }
                        } else {
                            // Click outside menu — close it
                            self.close_context_menu();
                        }
                        return;
                    }
                }

                if let Some(layer_id) = state.layers_engine.current_hover() {
                    // Right-click on the dock handle → settings menu
                    if event.button == BTN_RIGHT && self.is_handle_layer(&layer_id) {
                        state.workspaces.dock.open_handle_context_menu();
                        let view = InteractiveView { view: Box::new(self.clone()) };
                        seat.get_keyboard().map(|keyboard| {
                            keyboard.set_focus(state, Some(crate::focus::KeyboardFocusTarget::View(view)), event.serial);
                        });
                        return;
                    }

                    if let Some((identifier, match_id)) = self.get_app_from_layer(&layer_id) {
                        // Check for right-click on protocol layer item
                        if event.button == BTN_RIGHT {
                            tracing::info!("🖱️ Right-click detected on protocol layer app: {}", identifier);

                            let pos = state.last_pointer_location;
                            let pos = layers::prelude::Point::new(pos.0 as f32, pos.1 as f32);
                            state.workspaces.dock.open_context_menu(pos, identifier.clone());
                            let view = InteractiveView { view: Box::new(self.clone()) };
                            seat.get_keyboard().map(|keyboard| {
                                keyboard.set_focus(state, Some(crate::focus::KeyboardFocusTarget::View(view)), event.serial);
                            });
                            return;
                        } else {
                            // Normal left-click: focus or launch app
                            if let Some(wid) = state.workspaces.focus_app(&identifier) {
                                state.set_keyboard_focus_on_surface(&wid);
                            } else if let Some(bookmark) = self.bookmark_config_for(&match_id) {
                                if let Some(app) = self.bookmark_application(&match_id) {
                                    if let Some((cmd, args)) = app.command(&bookmark.exec_args) {
                                        state.launch_program(cmd, args);
                                    } else {
                                        warn!("bookmark {} has no executable command", identifier);
                                    }
                                } else {
                                    warn!("bookmark {} not loaded into dock", identifier);
                                }
                            }
                        }
                    } else if let Some(wid) = self.get_window_from_layer(&layer_id) {
                        // if we click on a minimized window, unminimize it
                        if let Some(wid) = state.workspaces.unminimize_window(&wid) {
                            state.workspaces.focus_app_with_window(&wid);
                            state.set_keyboard_focus_on_surface(&wid);
                        }
                    }
                }
                // Clear darken filter on any pressed app icon
                if let Some(layer_id) = state.layers_engine.current_hover() {
                    if let Some((_identifier, match_id)) = self.get_app_from_layer(&layer_id) {
                        let apps_layers = self.app_layers.read().unwrap();
                        if let Some(entry) = apps_layers.get(&match_id) {
                            entry.icon_scaler.set_color_filter(None);
                            entry.icon_scaler.set_opacity(1.0, None);
                            entry.label_layer.set_opacity(1.0, Some(Transition::ease_in_quad(0.05)));
                        }
                    }
                }
                self.dragging
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
    fn on_key(&self, event: &smithay::input::keyboard::KeysymHandle<'_>, state: smithay::backend::input::KeyState) {
        if state != KeyState::Released {
            return;
        }

        enum MenuAction { None, Navigate, Close }
        let action = {
            let menu_lock = self.context_menu.read().unwrap();
            let Some(menu) = menu_lock.as_ref() else { return };

            match event.modified_sym() {
                Keysym::Up    => { menu.select_previous(); MenuAction::Navigate }
                Keysym::Down  => { menu.select_next();     MenuAction::Navigate }
                Keysym::Right => { menu.open_submenu();    MenuAction::Navigate }
                Keysym::Left  => { menu.close_submenu();   MenuAction::Navigate }
                Keysym::Escape => MenuAction::Close,
                _ => MenuAction::None,
            }
        }; // menu_lock dropped here

        if let MenuAction::Close = action {
            self.close_context_menu();
        }
    }

    fn on_key_with_data(
        &self,
        event: &smithay::input::keyboard::KeysymHandle<'_>,
        key_state: smithay::backend::input::KeyState,
        data: &mut crate::Otto<Backend>,
    ) {
        if key_state != KeyState::Released {
            return;
        }
        let (idx, depth, action_id) = {
            let menu_lock = self.context_menu.read().unwrap();
            let Some(menu) = menu_lock.as_ref().filter(|m| m.is_active()) else { return };
            match event.modified_sym() {
                Keysym::Return | Keysym::KP_Enter => {
                    let state = menu.view.get_state();
                    let depth = state.depth();
                    let idx = state.selected_index(None);
                    let action_id = idx.and_then(|i| {
                        state.items_at_depth(depth).get(i).and_then(|item| item.action_id()).map(|s| s.to_string())
                    });
                    (idx, depth, action_id)
                }
                _ => return,
            }
        };
        if let (Some(idx), Some(action_id)) = (idx, action_id) {
            let app_id = self.context_menu_app_id.read().unwrap().clone();
            // Execute action immediately (while we have &mut data)
            if let Some(app_id) = app_id {
                self.execute_context_menu_action(&action_id, &app_id, data);
            }
            // Pulse animation plays on the still-visible menu, then closes it
            {
                let menu_lock = self.context_menu.read().unwrap();
                if let Some(menu) = menu_lock.as_ref() {
                    menu.pulse_then_close(depth, idx, self.clone());
                }
            }
        }
    }

    fn on_keyboard_leave(&self) {
        self.close_context_menu();
    }

}

impl DockView {
    /// Execute the named context-menu action for the given app identifier.
    pub(super) fn execute_context_menu_action<Backend: crate::state::Backend>(
        &self,
        action_id: &str,
        app_id: &str,
        state: &mut crate::Otto<Backend>,
    ) {
        tracing::info!("Context menu action '{}' for app '{}'", action_id, app_id);
        match action_id {
            "open" | "new_window" => {
                // Focus if running, otherwise launch
                if self.is_app_running(app_id) {
                    state.workspaces.focus_app(app_id);
                } else if let Some(match_id) = self.match_id_for(app_id) {
                    if let Some(app) = self.bookmark_application(&match_id) {
                        if let Some((cmd, args)) = app.command(&[]) {
                            state.launch_program(cmd, args);
                        }
                    }
                }
            }
            "keep_in_dock" => {
                if let Some(match_id) = self.match_id_for(app_id) {
                    let mut dock_state = self.get_state();
                    if let Some(app) = dock_state.running_apps.iter().find(|a| a.match_id == match_id).cloned() {
                        let bookmark = crate::config::DockBookmark {
                            desktop_id: match_id.clone(),
                            label: None,
                            exec_args: vec![],
                        };
                        self.update_dock_config(|d| {
                            if !d.bookmarks.iter().any(|b| b.desktop_id == match_id) {
                                d.bookmarks.push(bookmark);
                            }
                        });
                        if !dock_state.launchers.iter().any(|a| a.match_id == match_id) {
                            dock_state.launchers.push(app);
                            self.update_state(&dock_state);
                        }
                        tracing::info!("Added '{}' to dock bookmarks", match_id);
                    }
                }
            }
            "remove_from_dock" => {
                if let Some(match_id) = self.match_id_for(app_id) {
                    self.update_dock_config(|d| {
                        d.bookmarks.retain(|b| {
                            let id = b.desktop_id.strip_suffix(".desktop").unwrap_or(&b.desktop_id);
                            id != match_id
                        });
                    });
                    let mut dock_state = self.get_state();
                    dock_state.launchers.retain(|a| a.match_id != match_id);
                    self.update_state(&dock_state);
                    tracing::info!("Removed '{}' from dock bookmarks", app_id);
                }
            }
            "quit" => {
                state.workspaces.quit_app(app_id);
            }
            "toggle_autohide" => {
                let autohide = self.dock_config.read().unwrap().autohide;
                self.update_dock_config(|d| d.autohide = !autohide);
                tracing::info!("Dock auto-hide {}", if !autohide { "enabled" } else { "disabled" });
                // When autohide is disabled the dock becomes permanently visible,
                // so any maximized windows must shrink to respect the dock height.
                if autohide {
                    state.remaximize_maximized_windows();
                }
            }
            "toggle_magnification" => {
                let magnification = self.dock_config.read().unwrap().magnification;
                self.set_magnification_enabled(!magnification);
                self.save_config();
                tracing::info!("Dock magnification {}", if !magnification { "enabled" } else { "disabled" });
            }
            _ => {}
        }
    }
}
