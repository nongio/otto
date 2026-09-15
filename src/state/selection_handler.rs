#[cfg(feature = "xwayland")]
use std::os::fd::OwnedFd;

use smithay::{
    delegate_primary_selection,
    wayland::selection::{
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        SelectionHandler,
    },
};
#[cfg(feature = "xwayland")]
use smithay::{
    input::Seat,
    wayland::selection::{SelectionSource, SelectionTarget},
};
#[cfg(feature = "xwayland")]
use tracing::warn;

use super::{Backend, Otto};

impl<BackendData: Backend> SelectionHandler for Otto<BackendData> {
    type SelectionUserData = ();

    #[cfg(feature = "xwayland")]
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types())) {
                warn!(?err, ?ty, "Failed to set Xwayland selection");
            }
        }
    }

    #[cfg(feature = "xwayland")]
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.send_selection(ty, mime_type, fd) {
                warn!(?err, "Failed to send primary (X11 -> Wayland)");
            }
        }
    }
}

impl<BackendData: Backend> PrimarySelectionHandler for Otto<BackendData> {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

delegate_primary_selection!(@<BackendData: Backend + 'static> Otto<BackendData>);
