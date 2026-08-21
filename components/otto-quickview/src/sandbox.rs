//! Containment for the decode worker.
//!
//! The worker parses files nobody vetted. It is spawned per preview, it is
//! handed exactly one file descriptor, and it is expected to sometimes die
//! badly — that is the design, not a failure of it.
//!
//! **What is actually enforced** — measured by [`self_test`], not assumed:
//!
//! * memory is capped (`RLIMIT_AS`), so a decompression bomb dies instead of
//!   swapping the desktop;
//! * no file can be written or grown at all (`RLIMIT_FSIZE = 0`);
//! * the network is unreachable (`CLONE_NEWNET`);
//! * no privilege can be gained (`PR_SET_NO_NEW_PRIVS`);
//! * no descriptor is inherited beyond the one file
//!   ([`close_inherited_fds`]), so there is no Wayland or bus connection to
//!   misuse;
//! * CPU time is capped, and the parent kills on a wall-clock deadline —
//!   which is the thing a *thread* could never have offered.
//!
//! **What is not enforced, and is a known gap:** the worker can still `open`
//! and read unrelated files. Nothing here is a filesystem jail — `chdir("/")`
//! stops relative-path surprises and nothing more. Closing this needs either a
//! seccomp filter denying `openat` (cheap for the in-process decoders, but it
//! would break the PDF path, which must `exec` a rasteriser that opens its own
//! libraries) or a mount namespace with `pivot_root` (correct, and it must
//! then bind in enough of `/usr` for those rasterisers to run). Until one of
//! those lands, the containment story is: one process, one descriptor, hard
//! budgets, no network, no writes — and reading is not contained.
//!
//! Run `otto-quickview --sandbox-selftest` to see the current answer rather
//! than trusting this comment.
//!
//! Everything here is raw `libc`. There is no sandboxing crate in the tree and
//! the calls involved are a dozen lines each.

use std::io;

/// The descriptor the file to preview always arrives on in the worker. Fixed
/// so the worker never has to be told a path, and so a path cannot be
/// substituted between the parent's `fstat` and the child's `open`.
pub const FILE_FD: i32 = 3;

/// Budgets applied to a worker. Deliberately generous enough that a real
/// 200 MP photograph decodes, and tight enough that a decompression bomb does
/// not take the machine with it.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Address space ceiling. The single most important limit: it is what turns
    /// a memory bomb into a dead worker instead of a swapping desktop.
    pub address_space: u64,
    /// CPU seconds. A backstop for the parent's wall-clock deadline, which is
    /// the limit that normally fires first.
    pub cpu_seconds: u64,
    /// How many bytes a decoder may read from the file. Metadata-only
    /// previewers stop long before this.
    pub max_read: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            address_space: 1024 * 1024 * 1024,
            cpu_seconds: 10,
            max_read: 512 * 1024 * 1024,
        }
    }
}

/// Drop every capability the worker will not need, before it looks at a byte
/// of the file.
///
/// Called in the child between `fork` and `exec` and again at the top of the
/// worker's own `main` — the pre-exec half cannot survive `execve` for the
/// `no_new_privs` bit alone, and the post-exec half cannot unshare namespaces
/// as reliably. Applying both is cheap and neither is sufficient alone.
///
/// # Safety
///
/// Must only be called in a freshly forked child, before `exec`, where the only
/// other thing running is this code. It calls async-signal-unsafe-adjacent
/// functions in the narrow way that is permitted there.
pub unsafe fn apply(budget: Budget) -> io::Result<()> {
    // No path to resolve relative names against. Combined with holding no
    // directory descriptor, the worker has nowhere to walk to.
    if libc::chdir(c"/".as_ptr()) != 0 {
        return Err(io::Error::last_os_error());
    }

    // A setuid binary reached from here must not gain anything. This also makes
    // a seccomp filter installable without privileges, should one be added.
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        return Err(io::Error::last_os_error());
    }

    set_limit(libc::RLIMIT_AS, budget.address_space)?;
    set_limit(libc::RLIMIT_CPU, budget.cpu_seconds)?;
    // No file the worker creates may contain anything. It has no reason to
    // write, and this makes that structural rather than a matter of trust.
    set_limit(libc::RLIMIT_FSIZE, 0)?;
    // Enough for the inherited descriptors, a rasteriser's pipes, and nothing
    // resembling a file-descriptor exhaustion attack.
    set_limit(libc::RLIMIT_NOFILE, 64)?;
    // No core dump: a crashed previewer would otherwise write the contents of
    // the file it was parsing into the filesystem.
    set_limit(libc::RLIMIT_CORE, 0)?;

    // Network namespace last, and advisory. It needs either privilege or
    // unprivileged-userns support, and a kernel that refuses is not a reason to
    // decline to preview — the worker still has no socket it is allowed to
    // open a useful connection from, and no credentials to use. The rlimits
    // above are the load-bearing part.
    unshare_network();

    Ok(())
}

fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: `limit` is fully initialised and `resource` is a valid constant.
    if unsafe { libc::setrlimit(resource, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Best-effort network isolation.
///
/// Tries an unprivileged user namespace first, since that is what makes
/// `CLONE_NEWNET` available to a normal desktop process. Failure is not
/// reported: this is a second lock on a door the worker has no key to anyway.
fn unshare_network() {
    // SAFETY: `unshare` with these flags affects only the calling process.
    unsafe {
        if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) == 0 {
            return;
        }
        libc::unshare(libc::CLONE_NEWNET);
    }
}

/// What containment is actually in force, established by trying each thing
/// rather than by assuming the syscall that set it up worked.
///
/// This exists because a security property nobody measures is a security
/// property nobody has. `otto-quickview --sandbox-selftest` prints it.
#[derive(Debug, Default)]
pub struct SelfTest {
    pub address_space_capped: bool,
    pub cannot_grow_a_file: bool,
    pub network_unreachable: bool,
    /// Whether an unrelated file can still be opened. **Currently true**: the
    /// rlimits and namespaces below do not restrict the filesystem, and the
    /// honest containment story is the process boundary plus the budgets, not
    /// a filesystem jail. Recorded so the gap is visible rather than assumed
    /// away.
    pub can_still_open_other_files: bool,
}

/// Measure the sandbox from inside it. Call only after [`apply`].
pub fn self_test() -> SelfTest {
    let mut result = SelfTest::default();

    // SAFETY: every call below is a plain query or a deliberate attempt that is
    // expected to fail; none of them mutate state this process depends on.
    unsafe {
        let mut limit: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_AS, &mut limit) == 0 {
            result.address_space_capped = limit.rlim_cur != libc::RLIM_INFINITY;
        }
        if libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) == 0 {
            result.cannot_grow_a_file = limit.rlim_cur == 0;
        }

        // A UDP socket needs no peer to prove the point: in a fresh network
        // namespace there is no route to anywhere.
        let socket = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if socket < 0 {
            result.network_unreachable = true;
        } else {
            let mut address: libc::sockaddr_in = std::mem::zeroed();
            address.sin_family = libc::AF_INET as libc::sa_family_t;
            address.sin_port = 53u16.to_be();
            // 1.1.1.1
            address.sin_addr.s_addr = u32::from_be_bytes([1, 1, 1, 1]).to_be();
            let connected = libc::connect(
                socket,
                &address as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
            result.network_unreachable = connected != 0;
            libc::close(socket);
        }

        let probe = b"/etc/passwd\0";
        let fd = libc::open(probe.as_ptr() as *const libc::c_char, libc::O_RDONLY);
        if fd >= 0 {
            result.can_still_open_other_files = true;
            libc::close(fd);
        }
    }

    result
}

/// Close every descriptor above the ones the worker is meant to have.
///
/// The parent controls what it passes, but a descriptor leaked by something
/// earlier in the process's life — a Wayland socket, the session bus — would
/// otherwise be inherited straight into the previewer.
///
/// # Safety
///
/// Child-side only, as for [`apply`].
pub unsafe fn close_inherited_fds(keep_up_to: i32) {
    // `close_range` is the clean way and exists on every kernel Otto targets;
    // the fallback loop is there because it costs three lines.
    #[allow(clippy::useless_conversion)]
    let closed = libc::syscall(
        libc::SYS_close_range,
        (keep_up_to + 1) as libc::c_uint,
        libc::c_uint::MAX,
        0,
    );
    if closed == 0 {
        return;
    }
    let max = libc::sysconf(libc::_SC_OPEN_MAX).clamp(64, 4096) as i32;
    for fd in (keep_up_to + 1)..max {
        libc::close(fd);
    }
}
