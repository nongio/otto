use smithay::{
    delegate_data_control, delegate_data_device,
    reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    wayland::selection::{
        data_device::{DataDeviceHandler, DataDeviceState},
        wlr_data_control::{DataControlHandler, DataControlState},
    },
};

use super::{Backend, Otto};

impl<BackendData: Backend> DataDeviceHandler for Otto<BackendData> {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
    fn action_choice(
        &mut self,
        available: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
        preferred: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
    ) -> smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction {
        // println!("available {:?} preferred {:?}", available, preferred);
        // if the preferred action is valid (a single action) and in the available actions, use it
        // otherwise, follow a fallback stategy

        if [DndAction::Move, DndAction::Copy, DndAction::Ask].contains(&preferred)
            && available.contains(preferred)
        {
            self.load_cursor_for_action(preferred);
            preferred
        } else if available.contains(DndAction::Ask) {
            self.load_cursor_for_action(DndAction::Ask);
            DndAction::Ask
        } else if available.contains(DndAction::Copy) {
            self.load_cursor_for_action(DndAction::Copy);
            DndAction::Copy
        } else if available.contains(DndAction::Move) {
            self.load_cursor_for_action(DndAction::Move);
            DndAction::Move
        } else {
            self.load_cursor_for_action(DndAction::None);
            DndAction::empty()
        }
    }
}

impl<BackendData: Backend> DataControlHandler for Otto<BackendData> {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

delegate_data_device!(@<BackendData: Backend + 'static> Otto<BackendData>);
delegate_data_control!(@<BackendData: Backend + 'static> Otto<BackendData>);
