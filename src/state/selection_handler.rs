use std::os::fd::OwnedFd;

use smithay::{
    delegate_primary_selection,
    wayland::selection::{
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        SelectionHandler,
    },
};

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
            if let Err(err) = xwm.send_selection(ty, mime_type, fd, self.handle.clone()) {
                warn!(?err, "Failed to send primary (X11 -> Wayland)");
            }
        }
    }
    fn new_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: smithay::input::Seat<Self>,
    ) {
        println!("new_selection {:?} {:?}", ty, source);
    }
    fn send_selection(
        &mut self,
        ty: smithay::wayland::selection::SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: smithay::input::Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        println!("send_selection {:?} {:?} {:?}", ty, mime_type, fd);
    }
}

impl<BackendData: Backend> PrimarySelectionHandler for Otto<BackendData> {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

delegate_primary_selection!(@<BackendData: Backend + 'static> Otto<BackendData>);
