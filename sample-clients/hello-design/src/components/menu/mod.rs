mod data;
mod drawing;
mod surface;

// Include generated protocol code
mod sc_layer_protocol {
    use wayland_client;

    pub use wayland_client::protocol::{__interfaces::*, wl_surface};

    wayland_scanner::generate_interfaces!("../../protocols/sc-layer-v1.xml");
    wayland_scanner::generate_client_code!("../../protocols/sc-layer-v1.xml");
}

pub use data::{MenuItem, MenuItemId, Position};
pub use sc_layer_protocol::{sc_layer_shell_v1, sc_layer_v1};
pub use surface::Menu;

use std::fmt;

#[derive(Debug)]
pub enum MenuError {
    SurfaceCreationFailed,
    NotConfigured,
    NoParent,
    NotImplemented,
    WaylandError(String),
}

impl fmt::Display for MenuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MenuError::SurfaceCreationFailed => write!(f, "Failed to create menu surface"),
            MenuError::NotConfigured => write!(f, "Menu surface not configured"),
            MenuError::NoParent => write!(f, "No parent surface provided"),
            MenuError::NotImplemented => write!(f, "Feature not yet implemented"),
            MenuError::WaylandError(e) => write!(f, "Wayland error: {}", e),
        }
    }
}

impl std::error::Error for MenuError {}
