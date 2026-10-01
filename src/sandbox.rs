//! Telling sandboxed Wayland clients apart from the user's own programs.
//!
//! A sandbox engine connects its apps through a `wp_security_context_v1`
//! listener (Flatpak does), and every client that came in that way is
//! sandboxed. Otto keeps the interfaces that read the screen or inject input
//! away from them, so the compositor is not a way out of the sandbox. Programs
//! running as the user, outside a sandbox, already have the user's rights and
//! are not gated.

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
