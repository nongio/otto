//! Files dropped on the card join the stash.
//!
//! A drag enters only the surface that takes the card's pointer: the
//! floating balloon's overlay, which takes the pointer only over the card,
//! the card itself in the side canvas, or, with no card yet, the drop
//! invitation (see [`crate::invite`]). Whatever it carries as
//! `text/uri-list` is taken.

// Rust guideline compliant 2026-02-21

use wayland_client::protocol::{
    wl_data_device::{self, WlDataDevice},
    wl_data_device_manager::{DndAction, WlDataDeviceManager},
    wl_data_offer::{self, WlDataOffer},
    wl_seat::WlSeat,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

use otto_kit::clipboard::URI_LIST;

use crate::primary::OfferTypes;
use crate::State;

/// The drag over the card, if it carries files.
#[derive(Debug)]
pub struct Drops {
    _device: WlDataDevice,
    offer: Option<WlDataOffer>,
}

impl Drops {
    /// Take drops on `seat`.
    pub fn new(manager: &WlDataDeviceManager, seat: &WlSeat, qh: &QueueHandle<State>) -> Self {
        Self {
            _device: manager.get_data_device(seat, qh, ()),
            offer: None,
        }
    }

    fn forget(&mut self) {
        if let Some(offer) = self.offer.take() {
            offer.destroy();
        }
    }
}

/// Whether `offer` carries files.
fn carries_files(offer: &WlDataOffer) -> bool {
    offer
        .data::<OfferTypes>()
        .and_then(|types| {
            types
                .0
                .lock()
                .ok()
                .map(|types| types.iter().any(|t| t == URI_LIST))
        })
        .unwrap_or(false)
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id: Some(offer),
                ..
            } => {
                state.drops.forget();
                let over_invite = state
                    .invite
                    .as_ref()
                    .is_some_and(|invite| invite.surface() == &surface);
                let over_card = over_invite
                    || state
                        .panel
                        .as_ref()
                        .is_some_and(|panel| panel.takes_pointer(&surface));
                if over_card && carries_files(&offer) {
                    if over_invite {
                        if let Some(invite) = state.invite.as_mut() {
                            invite.set_hovered(true, &mut state.pool);
                        }
                    }
                    offer.accept(serial, Some(URI_LIST.into()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    state.drops.offer = Some(offer);
                } else {
                    offer.accept(serial, None);
                    offer.destroy();
                }
            }
            wl_data_device::Event::Leave => {
                state.drops.forget();
                if let Some(invite) = state.invite.as_mut() {
                    invite.set_hovered(false, &mut state.pool);
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = state.drops.offer.take() {
                    if let Some(invite) = state.invite.as_mut() {
                        invite.set_dropped();
                    }
                    state.receive_drop(offer);
                }
            }
            // Selections aren't stashed from here: see `primary`.
            wl_data_device::Event::Selection { id: Some(offer) }
                if state.drops.offer.as_ref() != Some(&offer) =>
            {
                offer.destroy();
            }
            _ => {}
        }
    }

    // A `data_offer` event creates the offer the following `enter` names;
    // its types arrive on the offer itself.
    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferTypes::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferTypes> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        types: &OfferTypes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if let Ok(mut types) = types.0.lock() {
                types.push(mime_type);
            }
        }
    }
}
