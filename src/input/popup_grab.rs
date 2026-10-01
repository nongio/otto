//! Who may take the keyboard with a popup grab, and keeping every other grab
//! off the lock screen and the password panel.
//!
//! `xdg_popup.grab` hands a client the keyboard: smithay's popup keyboard
//! grab ignores later focus changes, so whatever Otto focuses next — the lock
//! surface, the password panel — types into the popup instead. The protocol
//! ties the grab to "the serial of the user event" that opened the menu; Otto
//! checks that, and refuses popup grabs outright while the session is locked
//! or the panel is up.
//!
//! **The serial rule.** Each press (button, key, touch down, tablet tip or
//! button) delivered to a surface is noted per seat, with the client that got
//! it. A popup grab is honoured when the seat's latest press went to the
//! popup's own client and the serial is no older than that press and not one
//! Otto has yet to hand out. So the grab must follow the user's latest
//! interaction with that client, and a menu opened on release, or a submenu
//! grabbed with a later enter or motion serial, still works — those serials
//! came after the press. A client the user last pressed somewhere else, or one
//! making up a serial, is sent `popup_done`.

use smithay::{
    input::Seat,
    reexports::wayland_server::{
        backend::ClientId, protocol::wl_surface::WlSurface, Client, Resource,
    },
    utils::{Serial, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};

use crate::state::{Backend, ClientState, Otto, OttoComponent};

/// The latest press on a seat, and the client it was delivered to (`None`
/// for Otto's own views).
#[derive(Debug, Clone)]
pub struct LastPress {
    pub serial: Serial,
    pub client: Option<ClientId>,
}

/// Why a popup grab was turned down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupGrabRefusal {
    SessionLocked,
    PasswordPanelUp,
    /// The seat's latest press went to another client, or nowhere.
    NotTheLatestPress,
    /// Older than the client's latest press, or never handed out.
    BogusSerial,
}

impl<BackendData: Backend> Otto<BackendData> {
    /// Note a press delivered to `surface` (none: one of Otto's views) on
    /// `seat`.
    pub(crate) fn note_seat_press(
        &mut self,
        seat: &Seat<Self>,
        serial: Serial,
        surface: Option<&WlSurface>,
    ) {
        let client = surface.and_then(|s| s.client()).map(|c| c.id());
        self.seat_last_press
            .insert(seat.name().to_string(), LastPress { serial, client });
    }

    /// Whether a popup of `client` may grab `seat` with `serial`; the reason
    /// it may not otherwise.
    ///
    /// While locked only the locker Otto started may grab, and while the
    /// password panel is up only the polkit agent: their keys are theirs.
    pub fn popup_grab_refusal(
        &self,
        seat: &Seat<Self>,
        client: Option<&Client>,
        serial: Serial,
    ) -> Option<PopupGrabRefusal> {
        let component = client.and_then(ClientState::component_of);
        if self.is_session_locked() && component != Some(OttoComponent::Locker) {
            return Some(PopupGrabRefusal::SessionLocked);
        }
        if self.authorize_panel_up() && component != Some(OttoComponent::Authorize) {
            return Some(PopupGrabRefusal::PasswordPanelUp);
        }
        let client = client.map(|c| c.id());
        let client = client.as_ref();
        let Some(press) = self.seat_last_press.get(seat.name()) else {
            return Some(PopupGrabRefusal::NotTheLatestPress);
        };
        if client.is_none() || press.client.as_ref() != client {
            return Some(PopupGrabRefusal::NotTheLatestPress);
        }
        // One taken from the counter now is newer than any serial a client
        // was ever sent.
        let next = SERIAL_COUNTER.next_serial();
        if serial < press.serial || serial >= next {
            return Some(PopupGrabRefusal::BogusSerial);
        }
        None
    }

    /// Drop every keyboard grab on `seat` that `owner`'s surfaces do not
    /// hold: a popup grab of another client, or an input method's (whose
    /// grab has no focus, so it never counts as `owner`'s). With `pointer`,
    /// the pointer's grab too. Keys then follow the keyboard focus again.
    ///
    /// Used where the keys belong to one client and nobody else: the lock
    /// surface, the password panel.
    pub fn release_grabs_not_held_by(
        &mut self,
        seat: &Seat<Self>,
        owner: Option<&ClientId>,
        pointer: bool,
    ) {
        if let Some(keyboard) = seat.get_keyboard() {
            let holder = keyboard.grab_start_data().map(|data| {
                data.focus
                    .and_then(|focus| focus.wl_surface().and_then(|s| s.client()))
                    .map(|c| c.id())
            });
            if let Some(holder) = holder {
                if owner.is_none() || holder.as_ref() != owner {
                    tracing::info!(
                        ?holder,
                        "releasing a keyboard grab the key owner does not hold"
                    );
                    keyboard.unset_grab(self);
                }
            }
        }
        if pointer {
            if let Some(handle) = seat.get_pointer() {
                let holder = handle.grab_start_data().map(|data| {
                    data.focus
                        .and_then(|(focus, _)| focus.wl_surface().and_then(|s| s.client()))
                        .map(|c| c.id())
                });
                if let Some(holder) = holder {
                    if owner.is_none() || holder.as_ref() != owner {
                        handle.unset_grab(
                            self,
                            SERIAL_COUNTER.next_serial(),
                            smithay::backend::input::InputTime::now(),
                        );
                    }
                }
            }
        }
    }
}
