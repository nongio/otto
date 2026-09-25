//! The primary selection: the text most recently selected in any app.
//!
//! It is read through wlr-data-control, which sees the selection without
//! focus, so stash can take text from apps whose text-input support
//! doesn't report selections (GTK3, Qt, terminals).

// Rust guideline compliant 2026-02-21

use std::io::PipeReader;
use std::sync::Mutex;

use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

use crate::State;

/// Text types, best first. The X11 names cover apps running in XWayland.
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];

/// The types an offer announced, filled in as its `offer` events arrive.
#[derive(Debug, Default)]
pub struct OfferTypes(pub Mutex<Vec<String>>);

/// The seat's primary selection, as the compositor last announced it.
#[derive(Debug)]
pub struct Primary {
    _device: ZwlrDataControlDeviceV1,
    offer: Option<ZwlrDataControlOfferV1>,
}

impl Primary {
    /// Watches `seat`'s primary selection.
    pub fn new(manager: &ZwlrDataControlManagerV1, seat: &WlSeat, qh: &QueueHandle<State>) -> Self {
        Self {
            _device: manager.get_data_device(seat, qh, ()),
            offer: None,
        }
    }

    /// Asks the selecting app to write its text into a pipe, and returns the
    /// end to read it from. The app closes the pipe once it has written.
    ///
    /// Returns `None` when nothing is selected, the selection isn't text, or
    /// no pipe could be made.
    pub fn receive(&self) -> Option<PipeReader> {
        let offer = self.offer.as_ref()?;
        let mime_type = {
            let types = offer.data::<OfferTypes>()?.0.lock().ok()?;
            TEXT_TYPES
                .iter()
                .find(|wanted| types.iter().any(|t| t == *wanted))?
                .to_string()
        };
        let (reader, writer) = std::io::pipe()
            .inspect_err(|error| tracing::warn!(%error, "no pipe for the primary selection"))
            .ok()?;
        // The request carries its own copy of the descriptor, so the writer
        // can be dropped here; the app then holds the only one and its close
        // ends the read.
        offer.receive(mime_type, std::os::fd::AsFd::as_fd(&writer));
        Some(reader)
    }

    fn replace(&mut self, offer: Option<ZwlrDataControlOfferV1>) {
        if let Some(old) = std::mem::replace(&mut self.offer, offer) {
            old.destroy();
        }
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                state.primary.replace(id);
            }
            // The clipboard isn't stashed from.
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) } => offer.destroy(),
            zwlr_data_control_device_v1::Event::Finished => {
                tracing::warn!("the primary selection is no longer available");
                state.primary.replace(None);
            }
            _ => {}
        }
    }

    // A `data_offer` event creates the offer that a following selection
    // event names; its types arrive on the offer itself.
    wayland_client::event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, OfferTypes::default()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, OfferTypes> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        types: &OfferTypes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            if let Ok(mut types) = types.0.lock() {
                types.push(mime_type);
            }
        }
    }
}
