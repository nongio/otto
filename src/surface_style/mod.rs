mod handlers;
mod protocol;

use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::DisplayHandle;

use crate::state::Backend;

pub use handlers::create_style_manager_global;
pub use protocol::{gen, SurfaceStyle, SurfaceStyleHandler, OttoSurfaceStyleZOrder, StyleTransaction};

/// Shell global state
#[derive(Clone)]
pub struct OttoSurfaceStyleState {
    shell_global: GlobalId,
}

impl OttoSurfaceStyleState {
    /// Create a new surface style global
    pub fn new<BackendData: Backend + 'static>(display: &DisplayHandle) -> OttoSurfaceStyleState {
        let shell_global = create_style_manager_global::<BackendData>(display);

        OttoSurfaceStyleState { shell_global }
    }

    /// Get shell global id
    pub fn shell_global(&self) -> GlobalId {
        self.shell_global.clone()
    }
}
