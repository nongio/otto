//! Which Wayland clients get the privileged interfaces: the ones that read
//! the screen, inject input, read the clipboard without focus or control
//! other apps' windows. See `specs/security-model.md`.
//!
//! Four kinds of client are told apart:
//!
//! - A **sandboxed app** came in through a `wp_security_context_v1` listener
//!   (Flatpak does that). It never gets them: the compositor is not a way out
//!   of the sandbox.
//! - An **agent's connection** was made by Otto for an agent holding a seat
//!   (`specs/agent-seats.md`). It is kept off them too, except the virtual
//!   input for its own seat.
//! - An **Otto component** connected on a socket Otto handed it, or runs an
//!   executable [`otto_kit::trust`] vouches for. It always gets them.
//! - Any **other program of the user's** gets them by default, as on every
//!   other desktop: it already has the user's rights. `[privacy] strict`
//!   takes them away, and `[privacy] allow` gives them back to the
//!   executables it lists.
//!
//! The policy is read from the configuration when Otto starts
//! ([`init_policy`]); tests set it directly ([`set_policy`]).

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use smithay::reexports::wayland_server::Client;

use crate::state::ClientState;

/// What `[privacy]` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// The user's programs are kept off the privileged interfaces unless
    /// listed in `allow`.
    pub strict: bool,
    /// Executables allowed them under `strict`, by full path.
    pub allow: Vec<PathBuf>,
    /// Clients inside Otto's own process count as components: the headless
    /// tests' clients are, and tests of the policy turn it off.
    pub trust_own_process: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            strict: false,
            allow: Vec::new(),
            trust_own_process: true,
        }
    }
}

static POLICY: RwLock<Policy> = RwLock::new(Policy {
    strict: false,
    allow: Vec::new(),
    trust_own_process: true,
});

/// Take the policy from the configuration.
pub fn init_policy() {
    let (strict, allow) = crate::config::Config::with(|c| {
        (
            c.privacy.strict,
            c.privacy.allow.iter().map(PathBuf::from).collect(),
        )
    });
    set_policy(Policy {
        strict,
        allow,
        trust_own_process: true,
    });
}

pub fn set_policy(policy: Policy) {
    *POLICY.write().unwrap() = policy;
}

pub fn policy() -> Policy {
    POLICY.read().unwrap().clone()
}

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

/// Whether `client` is kept off the privileged interfaces whatever the
/// policy says: a sandboxed client, or an agent's own connection, also once
/// handed to the user. The
/// virtual input globals are offered to agents' connections all the same —
/// see [`may_drive_seat`].
pub fn is_confined_client(client: &Client) -> bool {
    is_sandboxed_client(client) || ClientState::was_agent_client(client)
}

/// Whether the program at `exe` gets the privileged interfaces under the
/// policy: always unless `strict`, then only Otto's components and the
/// executables `allow` lists.
pub fn is_trusted_program(exe: &Path) -> bool {
    let policy = policy();
    !policy.strict
        || otto_kit::trust::is_component(exe, otto_kit::trust::COMPONENTS)
        || policy.allow.iter().any(|allowed| allowed == exe)
}

/// Whether `client` gets the privileged interfaces: not confined, and an
/// Otto component or a program the policy trusts. A client Otto cannot name
/// is trusted only while the policy is not strict.
pub fn is_privileged_client(client: &Client) -> bool {
    if is_confined_client(client) {
        return false;
    }
    let Some(state) = client.get_data::<ClientState>() else {
        // XWayland, and anything else Otto connected without client data.
        return true;
    };
    if state.component.is_some() {
        return true;
    }
    let policy = policy();
    match &state.peer {
        // The headless tests' clients: Otto's own unless a test says
        // otherwise, and then judged by the policy's lists alone, never as a
        // component beside the running program (which they are).
        Some(peer) if peer.pid == std::process::id() => {
            policy.trust_own_process
                || !policy.strict
                || policy.allow.iter().any(|allowed| allowed == &peer.exe)
        }
        Some(peer) => is_trusted_program(&peer.exe),
        None => !policy.strict,
    }
}

/// Whether `client` may create virtual input on the seat named `seat`, where
/// `user_seat` is the user's.
///
/// An agent's connection drives the agent's own seat and no other. An
/// agent's seat is driven only through that agent's connections. The user's
/// seat is open to the clients that get the privileged interfaces.
pub fn may_drive_seat(client: &Client, seat: &str, user_seat: &str) -> bool {
    match ClientState::agent_seat_of(client) {
        Some(own) => own == seat,
        None => seat == user_seat && is_privileged_client(client),
    }
}

/// Refuse a bus call under `strict` unless it comes from an Otto component
/// or an allowed executable. `what` names the interface.
pub async fn require_trusted_caller(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
    what: &str,
) -> zbus::fdo::Result<()> {
    if !policy().strict {
        return Ok(());
    }
    let peer = otto_kit::trust::bus_caller(connection, header)
        .await
        .ok_or_else(|| {
            zbus::fdo::Error::AccessDenied(format!("{what}: cannot tell which program is calling"))
        })?;
    if is_trusted_program(&peer.exe) {
        return Ok(());
    }
    tracing::warn!(exe = %peer.exe.display(), pid = peer.pid, what, "refused under [privacy] strict");
    Err(zbus::fdo::Error::AccessDenied(format!(
        "{what} is for Otto's own programs and those [privacy] allow lists; {} is neither",
        peer.exe.display()
    )))
}
