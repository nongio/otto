//! Telling sandboxed Wayland clients apart from the user's own programs.
//!
//! A sandbox engine connects its apps through a `wp_security_context_v1`
//! listener (Flatpak does), and every client that came in that way is
//! sandboxed. Otto keeps the interfaces that read the screen or inject input
//! away from them, so the compositor is not a way out of the sandbox. Programs
//! running as the user, outside a sandbox, already have the user's rights and
//! are not gated.
//!
//! An agent's own Wayland connection, which Otto hands it over D-Bus
//! (`specs/agent-seats.md`, Phase 5), is kept off the same interfaces: it is
//! [confined](is_confined_client), and may drive its own seat only.

use smithay::reexports::wayland_server::Client;

use crate::state::ClientState;

/// Whether a Wayland client came in through a security context — a Flatpak,
/// or anything else a sandbox engine connected on the app's behalf.
///
/// Clients without Otto's client data (XWayland) are not sandboxed by this
/// measure; the X server is the user's.
pub fn is_sandboxed_client(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.security_context.is_some())
}

/// Whether `client` is kept off the interfaces that read the screen or
/// inject input: a sandboxed client, or an agent's own connection. The
/// virtual input globals are offered to agents' connections all the same —
/// see [`may_drive_seat`].
pub fn is_confined_client(client: &Client) -> bool {
    is_sandboxed_client(client) || ClientState::agent_seat_of(client).is_some()
}

/// Whether `client` may create virtual input on the seat named `seat`,
/// where `user_seat` is the user's and `owned_agent_seat` says whether
/// `seat` is an agent seat an agent asked for (rather than the static one).
///
/// An agent's connection drives its own seat and no other. A seat an agent
/// asked for is driven only through that agent's connection. The user's seat
/// and the static agent seat, an explicit opt-in for stock tools, are open to
/// every other unsandboxed client.
pub fn may_drive_seat(
    client: &Client,
    seat: &str,
    user_seat: &str,
    owned_agent_seat: bool,
) -> bool {
    if is_sandboxed_client(client) {
        return false;
    }
    match ClientState::agent_seat_of(client) {
        Some(own) => own == seat,
        None => seat == user_seat || !owned_agent_seat,
    }
}
