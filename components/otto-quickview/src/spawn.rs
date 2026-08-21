//! The parent half of decoding: open the file, spawn a contained worker, and
//! enforce the deadline the worker cannot enforce on itself.
//!
//! This is where the process boundary earns its keep. A runaway *thread* cannot
//! be killed; a runaway *process* can, which is what makes the wall-clock
//! budget real rather than aspirational.

use std::fs::File;
use std::io::Read;
use std::os::fd::IntoRawFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::decode::Request;
use crate::payload;
use crate::payload::PreviewPayload;
use crate::sandbox::{self, FILE_FD};

/// How long a preview may take before the worker is killed. Generous next to
/// the 100 ms interaction budget because it is a backstop, not a target: the
/// common case is served from the thumbnail cache long before this matters.
const DEADLINE: Duration = Duration::from_secs(8);

/// A file opened for preview, with what we learned by opening it.
///
/// The metadata is carried because the thumbnail cache needs it and the parent
/// is the only side that can use it: `thumbnails::lookup` takes the source
/// mtime rather than statting again, and storing is parent-side because the
/// worker runs under `RLIMIT_FSIZE = 0` and physically cannot write.
#[allow(
    dead_code,
    reason = "read by the thumbnail cache wiring, not yet built"
)]
pub struct Opened {
    pub file: File,
    pub len: u64,
    pub is_dir: bool,
    pub mtime: std::time::SystemTime,
}

/// Open a path for previewing, refusing everything that is not safely readable.
///
/// The refusal list is not paranoia: opening a FIFO blocks until a writer
/// appears, and reading a character device can block forever. A previewer that
/// hangs on a named pipe in a directory listing is a previewer that hangs.
pub fn open(path: &Path) -> Result<Opened, String> {
    // `O_NONBLOCK` so a FIFO that slipped through cannot block the open itself.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|err| format!("{err}"))?;

    let metadata = file.metadata().map_err(|err| format!("{err}"))?;
    let kind = metadata.file_type();
    if kind.is_fifo() || kind.is_socket() || kind.is_char_device() || kind.is_block_device() {
        return Err("this is not a file that can be previewed".into());
    }
    if !kind.is_file() && !kind.is_dir() {
        return Err("this is not a file that can be previewed".into());
    }

    Ok(Opened {
        len: metadata.len(),
        is_dir: kind.is_dir(),
        mtime: metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        file,
    })
}

/// Trait needed for `custom_flags` on the open above.
use std::os::unix::fs::OpenOptionsExt;

/// Run one decode. Blocks until the worker answers, dies, or overruns.
///
/// Always returns a payload: a worker that crashed, hung, or produced nonsense
/// becomes an `Unavailable` with a reason, because "cannot preview this, here
/// is why" is itself a preview and a blank window is not.
pub fn decode(opened: Opened, request: &Request) -> PreviewPayload {
    let started = Instant::now();
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => return payload::unavailable(format!("cannot find the previewer: {err}")),
    };

    let budget = request.budget;
    // The child receives the file on a fixed descriptor and nothing else.
    let file_fd = opened.file.into_raw_fd();

    let mut command = Command::new(executable);
    command
        .arg("--decode-worker")
        .arg("--width")
        .arg(request.width.to_string())
        .arg("--height")
        .arg(request.height.to_string())
        .arg("--page")
        .arg(request.page.to_string())
        .arg("--zoom")
        .arg(format!("{:.4}", request.zoom))
        .arg("--name")
        .arg(&request.name)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A previewer has no business inheriting the session's environment: the
        // Wayland and bus addresses in particular are capabilities.
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "error".into()),
        );

    // SAFETY: runs in the forked child between `fork` and `exec`. Everything
    // called here is either async-signal-safe or is a raw syscall wrapper.
    unsafe {
        command.pre_exec(move || {
            // Put the file where the worker will look for it.
            //
            // `dup2` is also what clears `FD_CLOEXEC`, which Rust sets on every
            // file it opens — so when the file already happens to *be* on
            // descriptor 3, skipping the `dup2` would leave the flag set and
            // the worker would exec into an empty descriptor. Clear it by hand
            // in that case rather than skipping the step.
            if file_fd == FILE_FD {
                if libc::fcntl(FILE_FD, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(file_fd, FILE_FD) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Drop anything else this process happened to be holding — a
            // leaked Wayland socket would otherwise be inherited straight into
            // the previewer.
            sandbox::close_inherited_fds(FILE_FD);
            sandbox::apply(budget)?;
            Ok(())
        });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            // SAFETY: the descriptor was not consumed by a successful spawn.
            unsafe { libc::close(file_fd) };
            return payload::unavailable(format!("cannot start the previewer: {err}"));
        }
    };
    // The child has its own copy now.
    unsafe { libc::close(file_fd) };

    // Read on a thread so the deadline is enforceable: reading inline would
    // block in `read_to_end` with no way to notice time passing.
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => return payload::unavailable("the previewer produced no output"),
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });

    let payload = match receiver.recv_timeout(DEADLINE) {
        Ok(Ok(bytes)) => payload::decode(&bytes)
            .unwrap_or_else(|| payload::unavailable("the previewer produced something unreadable")),
        Ok(Err(err)) => payload::unavailable(format!("the previewer failed: {err}")),
        Err(_) => {
            // Overran. This is the case a thread could not have recovered from.
            let _ = child.kill();
            payload::unavailable("this file took too long to preview")
        }
    };

    let _ = child.wait();
    tracing::debug!(
        ms = started.elapsed().as_millis(),
        name = %request.name,
        "decoded"
    );
    payload
}

/// One-shot decode from a path, for the command line and for tests.
pub fn decode_path(path: &Path, request: &Request) -> PreviewPayload {
    match open(path) {
        Ok(opened) => {
            let mut request = request.clone();
            if request.name.is_empty() {
                request.name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            decode(opened, &request)
        }
        Err(reason) => payload::unavailable(reason),
    }
}
