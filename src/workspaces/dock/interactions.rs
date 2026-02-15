use smithay::{backend::input::ButtonState, utils::IsAlive};

use crate::{config::Config, interactive_view::ViewInteractions};

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

        self.update_magnification_position((event.location.x * scale) as f32);
    }
    fn on_leave(&self, _serial: smithay::utils::Serial, _time: u32) {
        self.update_magnification_position(-500.0);
    }
    fn on_button(
        &self,
        _seat: &smithay::input::Seat<crate::Otto<Backend>>,
        state: &mut crate::Otto<Backend>,
        event: &smithay::input::pointer::ButtonEvent,
    ) {
        const BTN_RIGHT: u32 = 0x111; // 273
        
        match event.state {
            ButtonState::Pressed => {
                // println!("dock Button pressed");
                if let Some(layer_id) = state.layers_engine.current_hover() {
                    if let Some((_identifier, _match_id)) = self.get_app_from_layer(&layer_id) {
                        self.dragging
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
            ButtonState::Released => {
                if let Some(layer_id) = state.layers_engine.current_hover() {
                    if let Some((identifier, match_id)) = self.get_app_from_layer(&layer_id) {
                        println!("Button released on layer {}, identifier={}, has_protocol_layer={}", layer_id.0, identifier, self.has_protocol_layer(&identifier));
                        // Check for right-click on protocol layer item
                        if event.button == BTN_RIGHT && self.has_protocol_layer(&identifier) {
                            tracing::info!("Right-click on protocol layer app: {}", identifier);
                            
                            // Look up dock item resource and send menu_requested event
                            if let Some(dock_item_resource) = state.otto_dock.app_id_to_resource.get(&identifier) {
                                let pointer_loc = state.pointer.current_location();
                                tracing::info!("Sending menu_requested event: app_id={}, x={}, y={}", 
                                    identifier, pointer_loc.x as i32, pointer_loc.y as i32);
                                dock_item_resource.menu_requested(pointer_loc.x as i32, pointer_loc.y as i32);
                            } else {
                                warn!("No dock item resource found for app_id={}", identifier);
                            }
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
                self.dragging
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}
