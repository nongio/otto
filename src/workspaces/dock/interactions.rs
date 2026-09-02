use layers::prelude::Transition;
use otto_kit::components::context_menu::ContextMenuRenderer;
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::{
        keyboard::Keysym,
        pointer::{CursorIcon, CursorImageStatus},
    },
    utils::IsAlive,
};

use crate::{
    config::Config,
    interactive_view::{InteractiveView, ViewInteractions},
    settings::value::SettingValue,
};

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
        data: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::MotionEvent,
    ) {
        // The grip runs across the dock, so it resizes along the other axis.
        let resize_cursor = if self.position().is_vertical() {
            CursorIcon::EwResize
        } else {
            CursorIcon::NsResize
        };
        // A resize drag owns the pointer until it is released.
        if self.resize_drag_update((event.location.x, event.location.y)) {
            data.set_cursor(&CursorImageStatus::Named(resize_cursor));
            return;
        }
        // The handle is the drag-to-resize grip — say so on hover.
        let over_handle = data
            .layers_engine
            .current_hover()
            .is_some_and(|layer| self.is_handle_layer(&layer));
        if over_handle {
            data.set_cursor(&CursorImageStatus::Named(resize_cursor));
        } else {
            data.set_cursor(&CursorImageStatus::Named(CursorIcon::default()));
        }
        // A drag on an icon owns the pointer just as a resize drag does.
        if self.icon_drag_update((event.location.x, event.location.y)) {
            return;
        }
        let scale = Config::with(|c| c.screen_scale);
        if let Some(menu) = self
            .context_menu
            .read()
            .unwrap()
            .as_ref()
            .filter(|m| m.is_active())
        {
            let mut menu_state = menu.view.get_state();
            let items = menu_state.items();
            let style = &menu_state.style;
            // render_bounds_transformed returns physical pixels; event.location is logical.
            let menu_bounds = menu.view_layer.render_bounds_transformed();
            let x = event.location.x as f32 - menu_bounds.left / scale as f32;
            let y = event.location.y as f32 - menu_bounds.top / scale as f32;
            let item_index = ContextMenuRenderer::hit_test_items(items, style, x, y, 0.0);
            menu_state.select_at_depth(0, item_index);
            menu.view.update_state(&menu_state);
        }

        // Magnification follows the pointer along the dock's long axis.
        let along = if self.position().is_vertical() {
            event.location.y
        } else {
            event.location.x
        };
        self.update_magnification_position((along * scale) as f32);

        // Update label visibility: show tooltip for the hovered dock item only.
        // Skip while a context menu is open.
        if !self.has_menu_open() {
            self.set_active_label(self.hovered_label());
        }
    }
    fn on_leave(&self, _serial: smithay::utils::Serial, _time: u32) {
        // A drag in flight keeps the dock flat and the icon lifted until the
        // button comes back up, wherever the pointer has wandered off to.
        if self.is_icon_dragging() {
            return;
        }
        self.demagnify_elements();
        self.set_active_label(None);
        // Autohide is managed exclusively by check_dock_hot_zone via cached_dock_bounds.
    }
    fn on_enter(&self, _event: &smithay::input::pointer::MotionEvent) {
        self.show_autohide();
        self.magnify_elements_animated();
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
                if let Some(layer_id) = state.layers_engine.current_hover() {
                    // Left-press on the handle starts a resize drag.
                    if event.button != BTN_RIGHT && self.is_handle_layer(&layer_id) {
                        self.begin_resize_drag(state.last_pointer_location);
                        return;
                    }
                    if let Some((target, label)) = self.darkening_target_for_hover(&layer_id) {
                        self.darken_pressed(&target);
                        if let Some(l) = label {
                            l.set_opacity(1.0_f32, Some(Transition::ease_in_quad(0.05)));
                        }
                    }
                    // A left press on an app icon may turn into a reorder; it
                    // only becomes one once the pointer has moved far enough.
                    if event.button != BTN_RIGHT {
                        if let Some((_, match_id)) = self.get_app_from_layer(&layer_id) {
                            self.begin_icon_drag(&match_id, state.last_pointer_location);
                        }
                    }
                }
            }
            ButtonState::Released => {
                // Finishing a resize drag persists the new size and eats the click.
                if self.end_resize_drag(state) {
                    self.clear_pressed();
                    let still_on_handle = state
                        .layers_engine
                        .current_hover()
                        .is_some_and(|layer| self.is_handle_layer(&layer));
                    if !still_on_handle {
                        state.set_cursor(&CursorImageStatus::Named(CursorIcon::default()));
                    }
                    return;
                }
                // Finishing an icon drag drops it in its new place and eats
                // the click, so the app is not launched by being moved.
                if self.end_icon_drag() {
                    self.clear_pressed();
                    return;
                }
                self.cancel_icon_drag();
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
                        let item_index =
                            ContextMenuRenderer::hit_test_items(items, style, x, y, 0.0);
                        drop(menu_lock);
                        if let Some(idx) = item_index {
                            // Get action_id and app_id synchronously before closing the menu
                            let action_id = {
                                let menu_lock = self.context_menu.read().unwrap();
                                menu_lock.as_ref().and_then(|m| {
                                    m.view
                                        .get_state()
                                        .items_at_depth(0)
                                        .get(idx)
                                        .and_then(|i| i.action_id())
                                        .map(|s| s.to_string())
                                })
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
                        let view = InteractiveView {
                            view: Box::new(self.clone()),
                        };
                        if let Some(keyboard) = seat.get_keyboard() {
                            keyboard.set_focus(
                                state,
                                Some(crate::focus::KeyboardFocusTarget::View(view)),
                                event.serial,
                            );
                        }
                        self.clear_pressed();
                        return;
                    }

                    // Only execute the click action when released on the same
                    // element that was initially pressed.
                    if self.is_released_on_pressed(&layer_id) {
                        if let Some((identifier, match_id)) = self.get_app_from_layer(&layer_id) {
                            // Check for right-click on protocol layer item
                            if event.button == BTN_RIGHT {
                                tracing::info!(
                                    "🖱️ Right-click detected on protocol layer app: {}",
                                    identifier
                                );

                                let pos = state.last_pointer_location;
                                let pos = layers::prelude::Point::new(pos.0 as f32, pos.1 as f32);
                                state
                                    .workspaces
                                    .dock
                                    .open_context_menu(pos, identifier.clone());
                                let view = InteractiveView {
                                    view: Box::new(self.clone()),
                                };
                                if let Some(keyboard) = seat.get_keyboard() {
                                    keyboard.set_focus(
                                        state,
                                        Some(crate::focus::KeyboardFocusTarget::View(view)),
                                        event.serial,
                                    );
                                }
                            } else {
                                // Normal left-click: focus or launch app
                                if !state.focus_app(&identifier) {
                                    if let Some(bookmark) = self.bookmark_config_for(&match_id) {
                                        if let Some(app) = self.bookmark_application(&match_id) {
                                            if let Some((cmd, args)) =
                                                app.command(&bookmark.exec_args)
                                            {
                                                state.launch_program(cmd, args);
                                                // Bounce the icon until the app's window shows up.
                                                self.start_bounce(&match_id);
                                            } else {
                                                warn!(
                                                    "bookmark {} has no executable command",
                                                    identifier
                                                );
                                            }
                                        } else {
                                            warn!("bookmark {} not loaded into dock", identifier);
                                        }
                                    }
                                }
                            }
                        } else if let Some(wid) = self.get_window_from_layer(&layer_id) {
                            // if we click on a minimized window, unminimize it
                            if let Some(wid) = state.workspaces.unminimize_window(&wid) {
                                state.activate_window(&wid);
                            }
                        }
                    }
                }
                // Always clear the pressed darkening — handles release outside too.
                self.clear_pressed();
                self.dragging
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
    fn on_key(
        &self,
        event: &smithay::input::keyboard::KeysymHandle<'_>,
        state: smithay::backend::input::KeyState,
    ) {
        if state != KeyState::Released {
            return;
        }

        enum MenuAction {
            None,
            Navigate,
            Close,
        }
        let action = {
            let menu_lock = self.context_menu.read().unwrap();
            let Some(menu) = menu_lock.as_ref() else {
                return;
            };

            match event.modified_sym() {
                Keysym::Up => {
                    menu.select_previous();
                    MenuAction::Navigate
                }
                Keysym::Down => {
                    menu.select_next();
                    MenuAction::Navigate
                }
                Keysym::Right => {
                    menu.open_submenu();
                    MenuAction::Navigate
                }
                Keysym::Left => {
                    menu.close_submenu();
                    MenuAction::Navigate
                }
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
            let Some(menu) = menu_lock.as_ref().filter(|m| m.is_active()) else {
                return;
            };
            match event.modified_sym() {
                Keysym::Return | Keysym::KP_Enter => {
                    let state = menu.view.get_state();
                    let depth = state.depth();
                    let idx = state.selected_index(None);
                    let action_id = idx.and_then(|i| {
                        state
                            .items_at_depth(depth)
                            .get(i)
                            .and_then(|item| item.action_id())
                            .map(|s| s.to_string())
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
    /// Change one `dock.*` setting the way any other client would, so an
    /// in-compositor interaction and a settings app are indistinguishable to
    /// everyone watching.
    fn set_dock_setting<Backend: crate::state::Backend + 'static>(
        &self,
        state: &mut crate::Otto<Backend>,
        id: &str,
        value: SettingValue,
    ) {
        if let Err(err) = crate::settings::set(state, id, value) {
            warn!("Could not change {id}: {err}");
        }
    }

    /// Execute the named context-menu action for the given app identifier.
    pub(super) fn execute_context_menu_action<Backend: crate::state::Backend>(
        &self,
        action_id: &str,
        app_id: &str,
        state: &mut crate::Otto<Backend>,
    ) {
        tracing::info!("Context menu action '{}' for app '{}'", action_id, app_id);
        // An entry the app's own desktop file contributed: run its command.
        if let Some(action) = action_id.strip_prefix("action:") {
            if let Some(match_id) = self.match_id_for(app_id) {
                if let Some(app) = self.bookmark_application(&match_id) {
                    match app.action_command(action) {
                        Some((cmd, args)) => state.launch_program(cmd, args),
                        None => warn!("desktop action {action} of {app_id} has no command"),
                    }
                }
            }
            return;
        }
        match action_id {
            "open" | "new_window" => {
                // Focus if running, otherwise launch
                if self.is_app_running(app_id) {
                    state.focus_app(app_id);
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
                    if let Some(app) = dock_state
                        .running_apps
                        .iter()
                        .find(|a| a.match_id == match_id)
                        .cloned()
                    {
                        let bookmark = crate::config::DockBookmark {
                            desktop_id: match_id.clone(),
                            label: None,
                            exec_args: vec![],
                        };
                        self.update_bookmarks(|bookmarks| {
                            if !bookmarks.iter().any(|b| b.desktop_id == match_id) {
                                bookmarks.push(bookmark);
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
                    self.update_bookmarks(|bookmarks| {
                        bookmarks.retain(|b| {
                            let id = b
                                .desktop_id
                                .strip_suffix(".desktop")
                                .unwrap_or(&b.desktop_id);
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
                let autohide = Config::with(|c| c.dock.autohide);
                // Through the settings service like any other writer, so the
                // change is validated, applied, persisted and announced once.
                self.set_dock_setting(state, "dock.autohide", SettingValue::Bool(!autohide));
                tracing::info!(
                    "Dock auto-hide {}",
                    if !autohide { "enabled" } else { "disabled" }
                );
            }
            "position_bottom" | "position_left" | "position_right" => {
                let position = match action_id {
                    "position_left" => crate::config::DockPosition::Left,
                    "position_right" => crate::config::DockPosition::Right,
                    _ => crate::config::DockPosition::Bottom,
                };
                let name = match position {
                    crate::config::DockPosition::Left => "left",
                    crate::config::DockPosition::Right => "right",
                    crate::config::DockPosition::Bottom => "bottom",
                };
                self.set_dock_setting(state, "dock.position", SettingValue::Str(name.to_string()));
                tracing::info!("Dock moved to {:?}", position);
            }
            "toggle_magnification" => {
                let magnification = Config::with(|c| c.dock.magnification);
                self.set_dock_setting(
                    state,
                    "dock.magnification",
                    SettingValue::Bool(!magnification),
                );
                tracing::info!(
                    "Dock magnification {}",
                    if !magnification {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
            _ => {}
        }
    }
}
