use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_seat, delegate_shm,
    delegate_xdg_popup, delegate_xdg_shell, delegate_xdg_window,
};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::components::menu::{sc_layer_shell_v1, sc_layer_v1};
use crate::AppData;

// Delegate macros
delegate_compositor!(AppData);
delegate_output!(AppData);
delegate_shm!(AppData);
delegate_seat!(AppData);
delegate_xdg_shell!(AppData);
delegate_xdg_window!(AppData);
delegate_xdg_popup!(AppData);
delegate_registry!(AppData);

// SC Layer protocol handlers
impl Dispatch<sc_layer_shell_v1::ScLayerShellV1, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &sc_layer_shell_v1::ScLayerShellV1,
        _event: sc_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<sc_layer_v1::ScLayerV1, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &sc_layer_v1::ScLayerV1,
        _event: sc_layer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
