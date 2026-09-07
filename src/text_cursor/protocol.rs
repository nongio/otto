//! Generated bindings for `otto-text-cursor-v1`.

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;
    wayland_scanner::generate_interfaces!("./protocols/otto-text-cursor-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/otto-text-cursor-v1.xml");
}

pub use gen::otto_text_cursor_manager_v1::OttoTextCursorManagerV1;
pub use gen::otto_text_cursor_v1::OttoTextCursorV1;
