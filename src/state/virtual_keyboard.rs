//! zwp_virtual_keyboard_v1, on the seats a client may drive.
//!
//! smithay's virtual keyboard manager types a keyboard into whichever seat
//! the client names. Otto puts its own manager in front: a keyboard on a
//! seat the client may not drive (see [`crate::sandbox::may_drive_seat`]),
//! or on the user's seat by a program not allowed to
//! ([`crate::program_access`]), is refused with the protocol's
//! `unauthorized` error, and every other request
//! goes to smithay unchanged.
//!
//! An agent's connection types on its own seat whichever it names, as long
//! as it has bound that seat: stock tools take the first seat they see,
//! which is the user's.

use smithay::{
    input::Seat,
    reexports::{
        wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
            zwp_virtual_keyboard_manager_v1::{self, ZwpVirtualKeyboardManagerV1},
            zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
        },
        wayland_server::{Client, DataInit, DisplayHandle, New, Resource},
    },
    wayland::{Dispatch2, GlobalData, GlobalDispatch2},
};

use crate::state::{Backend, Otto};

/// The manager global's data, standing in for smithay's.
pub struct SeatCheckedKeyboards;

/// A bound manager: checks the seat, then hands the request to smithay.
pub struct SeatCheckedKeyboardManager;

/// A keyboard that was refused. The client is disconnected with the error;
/// until then its requests do nothing.
pub struct RefusedKeyboard;

impl<B: Backend + 'static> GlobalDispatch2<ZwpVirtualKeyboardManagerV1, Otto<B>>
    for SeatCheckedKeyboards
{
    fn bind(
        &self,
        _state: &mut Otto<B>,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, Otto<B>>,
    ) {
        data_init.init(resource, SeatCheckedKeyboardManager);
    }

    /// Never offered to sandboxed clients (see `src/sandbox.rs`). Agents'
    /// connections see it, to type on their own seat.
    fn can_view(&self, client: &Client) -> bool {
        !crate::sandbox::is_sandboxed_client(client)
    }
}

impl<B: Backend + 'static> Dispatch2<ZwpVirtualKeyboardManagerV1, Otto<B>>
    for SeatCheckedKeyboardManager
{
    fn request(
        &self,
        state: &mut Otto<B>,
        client: &Client,
        resource: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Otto<B>>,
    ) {
        let zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { seat, id } = request
        else {
            return;
        };
        // An agent's connection: its own seat, as the client bound it.
        let seat = crate::state::ClientState::agent_seat_of(client)
            .and_then(|own| state.agent_seat(own))
            .and_then(|agent| agent.seat.client_seats(client).into_iter().next())
            .unwrap_or(seat);
        let seat_name = Seat::<Otto<B>>::from_resource(&seat)
            .map(|seat| seat.name().to_string())
            .unwrap_or_default();
        let user_seat = state.seat.name();
        let owned = state
            .agent_seat(&seat_name)
            .is_some_and(|agent| agent.owner.is_some());
        let user_seat = user_seat.to_string();
        let refused = !crate::sandbox::may_drive_seat(client, &seat_name, &user_seat, owned)
            || (seat_name == user_seat
                && !crate::state::virtual_pointer::user_input_allowed(state, client));
        if refused {
            tracing::warn!(
                seat = seat_name,
                "virtual keyboard refused: this connection may not drive that seat"
            );
            data_init.init(id, RefusedKeyboard);
            resource.post_error(
                zwp_virtual_keyboard_manager_v1::Error::Unauthorized,
                format!("this connection may not type on seat {seat_name:?}"),
            );
            return;
        }
        <GlobalData as Dispatch2<ZwpVirtualKeyboardManagerV1, Otto<B>>>::request(
            &GlobalData,
            state,
            client,
            resource,
            zwp_virtual_keyboard_manager_v1::Request::CreateVirtualKeyboard { seat, id },
            handle,
            data_init,
        );
    }
}

impl<B: Backend + 'static> Dispatch2<ZwpVirtualKeyboardV1, Otto<B>> for RefusedKeyboard {
    fn request(
        &self,
        _state: &mut Otto<B>,
        _client: &Client,
        _resource: &ZwpVirtualKeyboardV1,
        _request: <ZwpVirtualKeyboardV1 as Resource>::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Otto<B>>,
    ) {
    }
}
