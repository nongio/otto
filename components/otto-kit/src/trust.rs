//! Who is on the other end of a bus call or a socket, and whether it is one
//! of Otto's own programs.
//!
//! A peer is named by the executable of its process, read from the kernel
//! (`/proc/<pid>/exe`) while a pidfd to that process is held, so the pid
//! cannot be reused under us. On the session bus the pidfd comes from
//! `GetConnectionCredentials` (`ProcessFD`; dbus 1.15 and dbus-broker), on a
//! Unix socket from `SO_PEERPIDFD` (Linux 6.5). Without either the pid is
//! used as it is, and [`Peer::pinned`] says so.
//!
//! An executable is one of Otto's own when it sits in the same directory as
//! the running program, or when it has one of the names asked for and root
//! owns it and its directory with no group or world write. A build run from
//! a checkout thereby trusts the components built beside it exactly as far
//! as it trusts itself: whoever can write there can replace the running
//! program too. A name alone proves nothing; a user-writable file called
//! `otto-islands` is anybody's.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// A process on the other end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub pid: u32,
    /// Its executable, as the kernel names it.
    pub exe: PathBuf,
    /// Whether `exe` was read while a pidfd pinned the process, so it is
    /// certainly this process's and not a successor's with the same pid.
    pub pinned: bool,
}

impl Peer {
    /// The executable's file name.
    pub fn program(&self) -> &str {
        self.exe
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }
}

// Not in every libc yet.
const SO_PEERPIDFD: libc::c_int = 77;

/// The process a pidfd refers to, if it is still running.
pub fn peer_of_pidfd(pidfd: BorrowedFd<'_>) -> Option<Peer> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok()?;
    // `Pid: -1` once the process is gone.
    let pid: u32 = info
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|pid| pid.trim().parse().ok())
        .filter(|&pid: &u32| pid > 0)?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(Peer {
        pid,
        exe,
        pinned: true,
    })
}

/// The process on the other end of a connected Unix socket.
pub fn peer_of_socket(stream: &UnixStream) -> Option<Peer> {
    let mut fd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `fd` and `len` are valid for the kernel to write to, and the
    // descriptor is the stream's own.
    let got_pidfd = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_PEERPIDFD,
            (&mut fd as *mut libc::c_int).cast(),
            &mut len,
        )
    } == 0;
    if got_pidfd && fd >= 0 {
        // SAFETY: the kernel just handed us this descriptor and nothing else
        // owns it.
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };
        return peer_of_pidfd(pidfd.as_fd());
    }
    // An older kernel: the pid as it was when the socket was accepted.
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: as above.
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    } == 0;
    if !ok || cred.pid <= 0 {
        return None;
    }
    let pid = cred.pid as u32;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(Peer {
        pid,
        exe,
        pinned: false,
    })
}

/// The process behind a bus name: the unique name of a message's sender, or
/// a well-known name's current owner.
pub async fn bus_peer(connection: &zbus::Connection, name: &str) -> Option<Peer> {
    let name = zbus::names::BusName::try_from(name.to_string()).ok()?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionCredentials",
            &(name,),
        )
        .await
        .ok()?;
    let body = reply.body();
    let credentials: HashMap<String, zbus::zvariant::OwnedValue> = body.deserialize().ok()?;
    if let Some(zbus::zvariant::Value::Fd(pidfd)) = credentials.get("ProcessFD").map(|v| &**v) {
        if let Ok(pidfd) = pidfd.as_fd().try_clone_to_owned() {
            return peer_of_pidfd(pidfd.as_fd());
        }
    }
    let pid = match credentials.get("ProcessID").map(|v| &**v) {
        Some(zbus::zvariant::Value::U32(pid)) => *pid,
        _ => return None,
    };
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(Peer {
        pid,
        exe,
        pinned: false,
    })
}

/// The process that sent the message `header` belongs to.
pub async fn bus_caller(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> Option<Peer> {
    let sender = header.sender()?.to_string();
    bus_peer(connection, &sender).await
}

/// Whether the executable at `exe` is one of Otto's own programs: beside the
/// running one, or called one of `names` and installed where only root can
/// change it.
pub fn is_component(exe: &Path, names: &[&str]) -> bool {
    if beside_self(exe) {
        return true;
    }
    let named = exe
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| names.contains(&name));
    named && root_owned(exe)
}

fn beside_self(exe: &Path) -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|own| own.parent().map(Path::to_path_buf))
        .is_some_and(|dir| exe.parent() == Some(dir.as_path()))
}

/// The file and its directory belong to root, and nobody else may write
/// either.
fn root_owned(exe: &Path) -> bool {
    let safe =
        |path: &Path| std::fs::metadata(path).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0);
    safe(exe) && exe.parent().is_some_and(safe)
}

/// Refuse a bus call unless it comes from one of Otto's own programs, as
/// [`is_component`] judges them. `what` names the interface, for the error
/// and the log.
pub async fn require_component(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
    names: &[&str],
    what: &str,
) -> zbus::fdo::Result<Peer> {
    let peer = bus_caller(connection, header).await.ok_or_else(|| {
        zbus::fdo::Error::AccessDenied(format!("{what}: cannot tell which program is calling"))
    })?;
    if is_component(&peer.exe, names) {
        return Ok(peer);
    }
    tracing::warn!(
        exe = %peer.exe.display(),
        pid = peer.pid,
        what,
        "refused: not one of Otto's own programs"
    );
    Err(zbus::fdo::Error::AccessDenied(format!(
        "{what} is for Otto's own programs; {} is not one",
        peer.exe.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_peer_in_this_process_is_named_and_pinned() {
        let (a, _b) = UnixStream::pair().unwrap();
        let peer = peer_of_socket(&a).expect("named");
        assert_eq!(peer.pid, std::process::id());
        assert_eq!(peer.exe, std::env::current_exe().unwrap());
        assert!(
            peer.pinned,
            "SO_PEERPIDFD should pin the peer on this kernel"
        );
    }

    #[test]
    fn components_are_beside_us_or_roots() {
        let own = std::env::current_exe().unwrap();
        let beside = own.parent().unwrap().join("anything");
        assert!(is_component(&beside, &[]), "beside the running program");
        assert!(!is_component(
            Path::new("/tmp/otto-islands"),
            &["otto-islands"]
        ));
        assert!(!is_component(Path::new("/usr/bin/true"), &["otto-islands"]));
        // Only when the system's /usr/bin is root's, as it is on any sane host.
        if root_owned(Path::new("/usr/bin/true")) {
            assert!(is_component(Path::new("/usr/bin/true"), &["true"]));
        }
    }
}
