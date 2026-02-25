use smithay::reexports::wayland_server::protocol::*;

pub mod gen {
    pub use smithay::reexports::wayland_server;
    pub use smithay::reexports::wayland_server::protocol::__interfaces::*;
    pub use smithay::reexports::wayland_server::protocol::*;
    pub use smithay::reexports::wayland_server::*;
    wayland_scanner::generate_interfaces!("./protocols/otto-dock-v1.xml");
    wayland_scanner::generate_server_code!("./protocols/otto-dock-v1.xml");
}

pub use gen::otto_dock_item_v1::OttoDockItemV1;
pub use gen::otto_dock_manager_v1::OttoDockManagerV1;

/// Type of dock item for layout purposes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockItemType {
    AppElement,
    PlaceElement,
}

impl Default for DockItemType {
    fn default() -> Self {
        Self::AppElement
    }
}

/// Compositor-side dock item state
#[derive(Debug, Clone)]
pub struct DockItem {
    /// The wl_surface (with layer_surface role) being placed in the dock
    pub wl_surface: Option<wl_surface::WlSurface>,

    /// Type for layout purposes
    pub item_type: DockItemType,

    /// Application identifier (for app elements)
    pub app_id: Option<String>,

    /// Optional badge text
    pub badge: Option<String>,

    /// Optional progress value (0.0 to 1.0, negative = hidden)
    pub progress: Option<f64>,

    /// Optional preview subsurface
    pub preview_subsurface: Option<wl_subsurface::WlSubsurface>,

    pub width: u32,
    pub height: u32,
}
