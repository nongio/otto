//! Which of the user's programs may capture the screen, inject input on the
//! user's seat, read the clipboard or control other apps' windows.
//!
//! A sandboxed client never may (see [`crate::sandbox`]), and an agent's own
//! connection drives its own seat only. This is for every other program
//! running as the user: it is named by its executable when it connects, and
//! asked about once, through otto-islands' dialog, the first time it tries.
//! The answer is kept in xdg-permission-store's `otto-wayland` table, one
//! entry per capability, keyed by the executable's path.
//!
//! Trusted without asking: Otto's own components (by the socket Otto handed
//! them, or by executable — see [`is_trusted`]), clients inside Otto's own
//! process, XWayland, and the programs `[privacy] trusted_programs` lists.
//!
//! This keeps a program that does not know the rules from capturing the
//! screen or typing as the user unnoticed. It is not a boundary: a program
//! running as the user can impersonate one that is allowed. The boundary is
//! the sandbox (`otto-sandbox`). See `specs/agent-seats.md`, Phase 5c.

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use smithay::reexports::wayland_server::Client;

use crate::state::ClientState;

/// The permission store table the answers are kept in.
pub const TABLE: &str = "otto-wayland";

/// What a program asks to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Capture the screen (wlr-screencopy).
    ScreenCapture,
    /// Move the user's pointer and type as the user (virtual pointer and
    /// keyboard on the user's seat).
    Input,
    /// Read and set the clipboard without focus (data-control).
    Clipboard,
    /// See and control every app's windows (wlr-foreign-toplevel).
    WindowControl,
}

impl Capability {
    /// The entry in [`TABLE`] its answers are kept under.
    pub fn id(self) -> &'static str {
        match self {
            Self::ScreenCapture => "screen-capture",
            Self::Input => "input",
            Self::Clipboard => "clipboard",
            Self::WindowControl => "window-control",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::ScreenCapture,
            Self::Input,
            Self::Clipboard,
            Self::WindowControl,
        ]
        .into_iter()
        .find(|cap| cap.id() == id)
    }

    /// Whether a program is asked about it. The others are only for
    /// programs trusted or allowed beforehand: their interfaces are hidden
    /// from the rest, which fall back to what every app has (the clipboard
    /// of the focused window).
    pub fn is_asked(self) -> bool {
        matches!(self, Self::ScreenCapture | Self::Input)
    }
}

/// A program connected to Otto, as it was when it connected.
#[derive(Debug, Clone)]
pub struct Program {
    pub pid: u32,
    /// Its executable, as `/proc/<pid>/exe` named it.
    pub exe: PathBuf,
    pub trusted: bool,
}

/// What a client may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Allowed,
    Denied,
    /// Nobody has answered for the program yet: ask, by its executable.
    Ask(PathBuf),
}

/// The answers, by capability and executable. Global, because a global's
/// visibility is decided with nothing but the client at hand.
static ANSWERS: LazyLock<RwLock<HashMap<(Capability, PathBuf), bool>>> =
    LazyLock::new(Default::default);

/// Name the program on the other end of a new client's socket. `None` when
/// the kernel does not say, which is treated as a program nobody may trust.
pub fn identify(
    stream: &std::os::unix::net::UnixStream,
    trust_own_process: bool,
) -> Option<Program> {
    let pid = peer_pid(stream)?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let trusted = (trust_own_process && pid == std::process::id()) || is_trusted(&exe);
    Some(Program { pid, exe, trusted })
}

fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for the kernel to write to, and the
    // descriptor is the stream's own.
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    } == 0;
    (ok && cred.pid > 0).then_some(cred.pid as u32)
}

/// Whether the executable at `exe` is trusted without asking: listed in
/// `[privacy] trusted_programs`, or one of Otto's own programs installed
/// where only root can change it (or, in a `dev` build, beside Otto).
pub fn is_trusted(exe: &Path) -> bool {
    let listed = crate::config::Config::with(|c| {
        c.privacy
            .trusted_programs
            .iter()
            .any(|path| Path::new(path) == exe)
    });
    listed || is_ottos_own(exe)
}

fn is_ottos_own(exe: &Path) -> bool {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let ottos_name = name.starts_with("otto");
    if !ottos_name {
        return false;
    }
    // A `dev` build trusts its own uninstalled components beside it, as it
    // runs its own polkit agent (`crate::polkit_agent`).
    let beside_otto = cfg!(feature = "dev")
        && std::env::current_exe()
            .ok()
            .and_then(|own| own.parent().map(Path::to_path_buf))
            .is_some_and(|dir| exe.parent() == Some(dir.as_path()));
    beside_otto || root_owned(exe)
}

/// The file and its directory belong to root, and nobody else may write
/// either.
fn root_owned(exe: &Path) -> bool {
    let safe =
        |path: &Path| std::fs::metadata(path).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0);
    safe(exe) && exe.parent().is_some_and(safe)
}

/// What `client` may do about `capability`.
pub fn access(client: &Client, capability: Capability) -> Access {
    // XWayland, and anything else Otto connected without client data, is
    // Otto's own.
    let Some(state) = client.get_data::<ClientState>() else {
        return Access::Allowed;
    };
    if state.component.is_some() {
        return Access::Allowed;
    }
    if state.security_context.is_some() || state.agent_seat.is_some() {
        return Access::Denied;
    }
    let Some(program) = &state.program else {
        return Access::Denied;
    };
    if program.trusted {
        return Access::Allowed;
    }
    match answer(capability, &program.exe) {
        Some(true) => Access::Allowed,
        Some(false) => Access::Denied,
        None if capability.is_asked() => Access::Ask(program.exe.clone()),
        None => Access::Denied,
    }
}

/// Whether `client` is offered the interfaces behind `capability`.
pub fn offered(client: &Client, capability: Capability) -> bool {
    access(client, capability) == Access::Allowed
}

pub fn answer(capability: Capability, exe: &Path) -> Option<bool> {
    ANSWERS
        .read()
        .unwrap()
        .get(&(capability, exe.to_path_buf()))
        .copied()
}

/// Keep an answer.
pub fn record(capability: Capability, exe: &Path, allowed: bool) {
    ANSWERS
        .write()
        .unwrap()
        .insert((capability, exe.to_path_buf()), allowed);
}

/// Replace every answer with what the store holds.
pub fn replace_all(answers: HashMap<(Capability, PathBuf), bool>) {
    *ANSWERS.write().unwrap() = answers;
}

/// Talking to xdg-permission-store and otto-islands, off the compositor's
/// thread.
pub mod store {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use zbus::blocking::Connection;
    use zbus::zvariant::OwnedValue;

    use super::{Capability, TABLE};

    const STORE_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
    const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
    const STORE_INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";

    /// Every answer the store holds. `None` when it cannot be read.
    pub fn load(conn: &Connection) -> Option<HashMap<(Capability, PathBuf), bool>> {
        let mut answers = HashMap::new();
        for capability in [
            Capability::ScreenCapture,
            Capability::Input,
            Capability::Clipboard,
            Capability::WindowControl,
        ] {
            let reply = conn.call_method(
                Some(STORE_NAME),
                STORE_PATH,
                Some(STORE_INTERFACE),
                "Lookup",
                &(TABLE, capability.id()),
            );
            let apps: HashMap<String, Vec<String>> = match reply {
                Ok(reply) => {
                    reply
                        .body()
                        .deserialize::<(HashMap<String, Vec<String>>, OwnedValue)>()
                        .ok()?
                        .0
                }
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str().ends_with(".NotFound") =>
                {
                    continue
                }
                Err(err) => {
                    tracing::warn!(%err, "cannot read which programs may control the desktop");
                    return None;
                }
            };
            for (exe, permissions) in apps {
                let allowed = permissions.iter().any(|p| p == "yes");
                answers.insert((capability, PathBuf::from(exe)), allowed);
            }
        }
        Some(answers)
    }

    /// Keep an answer in the store.
    pub fn save(conn: &Connection, capability: Capability, exe: &Path, allowed: bool) {
        let permission = if allowed { "yes" } else { "no" };
        let saved = conn.call_method(
            Some(STORE_NAME),
            STORE_PATH,
            Some(STORE_INTERFACE),
            "SetPermission",
            &(
                TABLE,
                true,
                capability.id(),
                exe.to_string_lossy().as_ref(),
                vec![permission],
            ),
        );
        if let Err(err) = saved {
            tracing::warn!(%err, exe = %exe.display(), "cannot keep the answer");
        }
    }

    /// Call `changed` whenever the store's table changes, until the bus
    /// goes away.
    pub fn watch(conn: &Connection, mut changed: impl FnMut()) {
        let rule = match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(STORE_INTERFACE)
            .and_then(|r| r.member("Changed"))
            .map(|r| r.build())
        {
            Ok(rule) => rule,
            Err(err) => {
                tracing::warn!(%err, "cannot watch the permission store");
                return;
            }
        };
        let messages = match zbus::blocking::MessageIterator::for_match_rule(rule, conn, None) {
            Ok(messages) => messages,
            Err(err) => {
                tracing::warn!(%err, "cannot watch the permission store");
                return;
            }
        };
        for message in messages.flatten() {
            let table = message
                .body()
                .deserialize::<(
                    String,
                    String,
                    bool,
                    OwnedValue,
                    HashMap<String, Vec<String>>,
                )>()
                .map(|(table, ..)| table);
            if table.is_ok_and(|t| t == TABLE) {
                changed();
            }
        }
    }

    /// Ask the user whether the program at `exe` may do what `capability`
    /// allows. `None` when no dialog could be shown or it was withdrawn.
    pub fn ask(conn: &Connection, capability: Capability, exe: &Path) -> Option<bool> {
        let binary = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let app = otto_kit::desktop_entry::lookup_app_by_binary(&binary)
            .map(|info| info.name)
            .unwrap_or_else(|| binary.clone());
        let (title, body, icon) = match capability {
            Capability::ScreenCapture => (
                otto_kit::t_owned!("access-capture-title", app = app.clone()),
                otto_kit::t_owned!("access-capture-body", app = app.clone()),
                "video-display",
            ),
            _ => (
                otto_kit::t_owned!("access-input-title", app = app.clone()),
                otto_kit::t_owned!("access-input-body", app = app.clone()),
                "input-keyboard",
            ),
        };
        let reply = conn.call_method(
            Some("org.otto.Island"),
            "/org/otto/Dialog",
            Some("org.otto.Dialog1"),
            "PresentAccess",
            &(
                binary.as_str(),
                title.as_str(),
                exe.to_string_lossy().as_ref(),
                body.as_str(),
                icon,
                otto_kit::t!("access-allow"),
                otto_kit::t!("access-deny"),
                true,
                Vec::<(String, String, Vec<(String, String, String)>, String)>::new(),
            ),
        );
        match reply.and_then(|r| r.body().deserialize::<(u32, Vec<(String, String)>)>()) {
            Ok((0, _)) => Some(true),
            Ok((1, _)) => Some(false),
            Ok(_) => None,
            Err(err) => {
                tracing::warn!(%err, "cannot ask about a program");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_round_trip_their_ids() {
        for cap in [
            Capability::ScreenCapture,
            Capability::Input,
            Capability::Clipboard,
            Capability::WindowControl,
        ] {
            assert_eq!(Capability::from_id(cap.id()), Some(cap));
        }
        assert_eq!(Capability::from_id("nope"), None);
    }

    #[test]
    fn only_ottos_programs_are_its_own() {
        assert!(!is_ottos_own(Path::new("/usr/bin/grim")));
        assert!(
            !is_ottos_own(Path::new("/tmp/otto-bar")),
            "/tmp is not root's alone"
        );
        let own = std::env::current_exe().unwrap();
        let beside = own.parent().unwrap().join("otto-bar");
        assert_eq!(is_ottos_own(&beside), cfg!(feature = "dev"));
        assert!(!is_ottos_own(&own.parent().unwrap().join("grim")));
    }

    #[test]
    fn a_peer_in_this_process_is_named() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let program = identify(&a, true).expect("named");
        assert_eq!(program.pid, std::process::id());
        assert!(program.trusted);
        assert!(!identify(&a, false).unwrap().trusted || is_trusted(&program.exe));
    }
}
