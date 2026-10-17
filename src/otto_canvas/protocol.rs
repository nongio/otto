//! Generated bindings for `otto-canvas-v1`.

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;
    wayland_scanner::generate_interfaces!("./protocols/otto-canvas-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/otto-canvas-v1.xml");
}

pub use gen::otto_canvas_item_v1::OttoCanvasItemV1;
pub use gen::otto_canvas_manager_v1::OttoCanvasManagerV1;
