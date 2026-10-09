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

mod otto_canvas_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_surface};

    wayland_scanner::generate_interfaces!("../../protocols/otto-canvas-v1.xml");
    wayland_scanner::generate_client_code!("../../protocols/otto-canvas-v1.xml");
}

mod otto_text_cursor_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_seat};

    wayland_scanner::generate_interfaces!("../../protocols/otto-text-cursor-v1.xml");
    wayland_scanner::generate_client_code!("../../protocols/otto-text-cursor-v1.xml");
}

mod kde_appmenu_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_surface};

    wayland_scanner::generate_interfaces!("../../protocols/kde-appmenu.xml");
    wayland_scanner::generate_client_code!("../../protocols/kde-appmenu.xml");
}

pub use kde_appmenu_protocol::{org_kde_kwin_appmenu, org_kde_kwin_appmenu_manager};

pub use sc_layer_protocol::{
    otto_style_transaction_v1, otto_surface_style_manager_v1, otto_surface_style_v1,
    otto_timing_function_v1,
};

pub use otto_canvas_protocol::{otto_canvas_item_v1, otto_canvas_manager_v1};
pub use otto_dock_protocol::{otto_dock_item_v1, otto_dock_manager_v1};
pub use otto_text_cursor_protocol::{otto_text_cursor_manager_v1, otto_text_cursor_v1};
