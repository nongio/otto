// Protocol definitions module

mod sc_layer_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_surface};

    wayland_scanner::generate_interfaces!("../../protocols/otto-surface-style-unstable-v1.xml");
    wayland_scanner::generate_client_code!("../../protocols/otto-surface-style-unstable-v1.xml");
}

mod otto_dock_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_surface};

    wayland_scanner::generate_interfaces!("../../protocols/otto-dock-v1.xml");
    wayland_scanner::generate_client_code!("../../protocols/otto-dock-v1.xml");
}

pub use sc_layer_protocol::{
    otto_style_transaction_v1, otto_surface_style_manager_v1, otto_surface_style_v1,
    otto_timing_function_v1,
};

pub use otto_dock_protocol::{otto_dock_item_v1, otto_dock_manager_v1};
