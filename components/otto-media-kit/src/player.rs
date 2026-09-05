//! The host-side handle on a playback.
//!
//! A [`Player`] is one worker process, one file, and the frames coming out of
//! it. The host opens the file itself — it never hands the worker a path —
//! creates the frame ring, spawns `otto-media-worker` with exactly those two
//! descriptors, and reads its events on a thread of its own. Every event
//! updates [`State`], a fresh frame is copied out of the ring, and the host's
//! `wake` closure is called so its loop paints.
//!
//! Nothing here blocks the caller: opening spawns and returns, commands are a
//! `write` to a pipe, and [`Player::frame`] hands back whatever last arrived.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::{self, Command as Cmd, Event};

/// One decoded frame, RGBx, rows tightly packed.
#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Frames from one. A host that keys its repaint on this repaints once
    /// per frame and not otherwise.
    pub seq: u64,
    pub position: Duration,
    pub data: Arc<Vec<u8>>,
}

impl Frame {
    /// The frame as a Skia image. Copies once; the image outlives the borrow
    /// in every caller.
    pub fn to_image(&self) -> Option<skia_safe::Image> {
        let info = skia_safe::ImageInfo::new(
            (self.width as i32, self.height as i32),
            skia_safe::ColorType::RGB888x,
            skia_safe::AlphaType::Opaque,
            None,
        );
        let row_bytes = self.width as usize * protocol::BYTES_PER_PIXEL as usize;
        if self.data.len() != row_bytes * self.height as usize {
            return None;
        }
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&self.data), row_bytes)
    }
}

/// Where the playback is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// The worker is starting up, or the stream is not yet known.
    Loading,
    Playing,
    Paused,
    Ended,
    /// The worker gave up or died. The reason is in [`State::error`].
    Failed,
}

/// Everything the host may want to draw or decide on, as of the last event.
#[derive(Debug, Clone)]
pub struct State {
    pub playback: Playback,
    /// Frame size as it arrives, after the worker's own scaling.
    pub size: Option<(u32, u32)>,
    pub duration: Option<Duration>,
    /// The last position the worker reported, and when. Read it through
    /// [`State::position`], which advances it while playing.
    reported: Duration,
    reported_at: Instant,
    pub volume: f64,
    pub error: Option<String>,
    /// Sequence number of the newest frame in [`Player::frame`]; 0 before
    /// any has arrived.
    pub frame_seq: u64,
}

impl State {
    fn new() -> Self {
        Self {
            playback: Playback::Loading,
            size: None,
            duration: None,
            reported: Duration::ZERO,
            reported_at: Instant::now(),
            volume: 1.0,
            error: None,
            frame_seq: 0,
        }
    }

    /// The current position, interpolated from the last report while
    /// playing so a scrubber moves smoothly between the worker's ticks.
    pub fn position(&self) -> Duration {
        let mut position = self.reported;
        if self.playback == Playback::Playing {
            position += self.reported_at.elapsed();
        }
        match self.duration {
            Some(duration) => position.min(duration),
            None => position,
        }
    }

    fn report(&mut self, position: Duration) {
        self.reported = position;
        self.reported_at = Instant::now();
    }
}

struct Shared {
    state: Mutex<State>,
    frame: Mutex<Option<Frame>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

/// A playback in progress. Dropping it ends the worker.
pub struct Player {
    child: Child,
    commands: Option<ChildStdin>,
    shared: Arc<Shared>,
}

/// How large a frame the worker may hand back, in pixels. The worker scales
/// the video down to fit, keeping its aspect; it never scales up.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_width: 1920,
            max_height: 1080,
        }
    }
}

impl Player {
    /// Start playing `path`.
    ///
    /// Returns as soon as the worker is spawned; the stream's size and
    /// duration arrive through [`Player::state`] once it has prerolled. The
    /// worker is asked to play immediately.
    ///
    /// `wake` is called from another thread whenever there is something new
    /// to draw — a frame, a state change — and must be cheap: typically it
    /// pokes the host's event loop.
    pub fn open(
        path: &Path,
        limits: Limits,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<Player> {
        // `O_NONBLOCK` so a FIFO cannot block the open; the check below then
        // refuses anything that is not a regular file, since a device or a
        // socket could block a read.
        let file = File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }

        let ring = memfd("otto-media-frames")?;
        let executable = worker_executable()?;

        let mut command = Command::new(executable);
        command
            .arg("--max-width")
            .arg(limits.max_width.to_string())
            .arg("--max-height")
            .arg(limits.max_height.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("OTTO_MEDIA_TRACE").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .env_clear();
        for (key, value) in std::env::vars_os() {
            if inherits(&key.to_string_lossy()) {
                command.env(key, value);
            }
        }

        let file_fd = file.into_raw_fd();
        let ring_fd = ring.as_raw_fd();
        // SAFETY: runs in the forked child between `fork` and `exec`; only
        // raw syscalls are made.
        unsafe {
            command.pre_exec(move || {
                place(file_fd, protocol::FILE_FD)?;
                place(ring_fd, protocol::FRAMES_FD)?;
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        // The parent's copy of the file is no longer needed; the ring stays
        // open here, since the host maps it.
        // SAFETY: `file_fd` was handed to us by `into_raw_fd` and is not
        // used again.
        unsafe { libc::close(file_fd) };

        let stdout = child.stdout.take().expect("piped stdout");
        let mut commands = child.stdin.take().expect("piped stdin");

        let shared = Arc::new(Shared {
            state: Mutex::new(State::new()),
            frame: Mutex::new(None),
            wake: Box::new(wake),
        });
        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("otto-media-events".into())
                .spawn(move || read_events(stdout, ring, shared))
                .map_err(|err| io::Error::other(format!("cannot start event reader: {err}")))?;
        }

        let _ = commands.write_all(Cmd::Play.encode().as_bytes());
        Ok(Player {
            child,
            commands: Some(commands),
            shared,
        })
    }

    /// A snapshot of the playback state.
    pub fn state(&self) -> State {
        self.shared.state.lock().unwrap().clone()
    }

    /// The newest frame, if one has arrived.
    pub fn frame(&self) -> Option<Frame> {
        self.shared.frame.lock().unwrap().clone()
    }

    pub fn play(&mut self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&mut self) {
        self.send(Cmd::Pause);
    }

    /// Play if paused or ended, pause if playing.
    pub fn toggle(&mut self) {
        match self.state().playback {
            Playback::Playing => self.pause(),
            _ => self.play(),
        }
    }

    /// Jump to `position`. `accurate` lands on the exact frame; a scrub in
    /// flight passes `false` and gets the nearest keyframe, which is what
    /// keeps dragging responsive.
    pub fn seek(&mut self, position: Duration, accurate: bool) {
        {
            // Show the target at once rather than the last report, so the
            // scrubber does not jump back while the worker catches up.
            let mut state = self.shared.state.lock().unwrap();
            state.report(position);
        }
        self.send(Cmd::Seek { position, accurate });
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.shared.state.lock().unwrap().volume = volume.clamp(0.0, 1.0);
        self.send(Cmd::Volume(volume));
    }

    fn send(&mut self, command: Cmd) {
        if let Some(commands) = self.commands.as_mut() {
            if commands.write_all(command.encode().as_bytes()).is_err() {
                // The worker is gone; the event reader reports it.
                self.commands = None;
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Ask nicely, then do not wait to find out: a Drop that blocks on a
        // stuck demuxer would stall the host's frame loop.
        self.send(Cmd::Quit);
        self.commands = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The event reader: runs until the worker's stdout closes.
fn read_events(stdout: std::process::ChildStdout, ring: File, shared: Arc<Shared>) {
    let mut lines = BufReader::new(stdout).lines();
    let mut mapping: Option<Mapping> = None;
    let mut size = (0u32, 0u32);

    while let Some(Ok(line)) = lines.next() {
        let Some(event) = Event::parse(&line) else {
            tracing::debug!("otto-media-worker said something unexpected: {line}");
            continue;
        };
        match event {
            Event::Ready {
                width,
                height,
                duration,
            } => {
                size = (width, height);
                mapping = Mapping::new(&ring, protocol::ring_bytes(width, height)).ok();
                let mut state = shared.state.lock().unwrap();
                state.size = Some((width, height));
                state.duration = duration;
                if state.playback == Playback::Loading {
                    state.playback = Playback::Paused;
                }
            }
            Event::Frame {
                slot,
                seq,
                position,
            } => {
                let (width, height) = size;
                let Some(mapping) = mapping.as_ref() else {
                    continue;
                };
                let offset = protocol::slot_offset(slot, width, height) as usize;
                let bytes = protocol::slot_bytes(width, height) as usize;
                let Some(pixels) = mapping.slice(offset, bytes) else {
                    continue;
                };
                // Copied out at once: the worker will be back to this slot
                // two frames from now, and the host draws whenever it likes.
                let data = Arc::new(pixels.to_vec());
                *shared.frame.lock().unwrap() = Some(Frame {
                    width,
                    height,
                    seq,
                    position,
                    data,
                });
                let mut state = shared.state.lock().unwrap();
                state.frame_seq = seq;
                // A frame is the most precise position report there is.
                if state.playback != Playback::Playing {
                    state.report(position);
                }
            }
            Event::Position(position) => shared.state.lock().unwrap().report(position),
            Event::Playing => {
                let mut state = shared.state.lock().unwrap();
                let position = state.position();
                state.report(position);
                state.playback = Playback::Playing;
            }
            Event::Paused => {
                let mut state = shared.state.lock().unwrap();
                let position = state.position();
                state.report(position);
                state.playback = Playback::Paused;
            }
            Event::Ended => {
                let mut state = shared.state.lock().unwrap();
                if let Some(duration) = state.duration {
                    state.report(duration);
                }
                state.playback = Playback::Ended;
            }
            Event::Error(reason) => {
                let mut state = shared.state.lock().unwrap();
                state.error = Some(reason);
                state.playback = Playback::Failed;
            }
        }
        (shared.wake)();
    }

    // The pipe closed: the worker exited, cleanly or not. A playback that
    // was still going has failed; one that had ended or was never started
    // stays as it was.
    let mut state = shared.state.lock().unwrap();
    if matches!(
        state.playback,
        Playback::Loading | Playback::Playing | Playback::Paused
    ) {
        if state.error.is_none() {
            state.error = Some("the media worker exited".into());
        }
        state.playback = Playback::Failed;
        drop(state);
        (shared.wake)();
    }
}

/// A read-only view of the frame ring.
struct Mapping {
    ptr: *const u8,
    len: usize,
}

// SAFETY: the mapping is only ever read, and only from the event thread.
unsafe impl Send for Mapping {}

impl Mapping {
    fn new(ring: &File, len: u64) -> io::Result<Self> {
        let len = len as usize;
        // SAFETY: mapping a descriptor we own, read-only, shared; the worker
        // has sized it before announcing the size mapped here.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                ring.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            ptr: ptr as *const u8,
            len,
        })
    }

    fn slice(&self, offset: usize, len: usize) -> Option<&[u8]> {
        if offset.checked_add(len)? > self.len {
            return None;
        }
        // SAFETY: bounds checked above; the mapping is readable for its
        // whole length. The worker may write concurrently, which yields a
        // torn frame at worst — never undefined memory.
        Some(unsafe { std::slice::from_raw_parts(self.ptr.add(offset), len) })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: unmapping what `new` mapped.
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

/// A sealed-free memfd; the worker sizes it.
fn memfd(name: &str) -> io::Result<File> {
    let name = std::ffi::CString::new(name).expect("no NUL in name");
    // SAFETY: a valid C string; the descriptor is owned by the returned File.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh descriptor nobody else holds.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Put `fd` on `target` for the child, clearing close-on-exec as `dup2`
/// does. When it already is `target`, the flag is cleared by hand — Rust
/// opens everything close-on-exec, and `dup2` onto itself is a no-op.
fn place(fd: RawFd, target: RawFd) -> io::Result<()> {
    // SAFETY: raw descriptor calls on descriptors this process owns.
    unsafe {
        if fd == target {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(io::Error::last_os_error());
            }
        } else if libc::dup2(fd, target) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Which of the host's environment the worker gets.
///
/// Not the Wayland or bus addresses: those are capabilities. What it does
/// need is the audio server's socket directory, GStreamer's own settings and
/// its registry cache, and the locale for any string it reports.
fn inherits(key: &str) -> bool {
    matches!(
        key,
        "PATH"
            | "HOME"
            | "XDG_RUNTIME_DIR"
            | "XDG_CACHE_HOME"
            | "RUST_LOG"
            | "LANG"
            | "LANGUAGE"
            | "PULSE_SERVER"
            | "PIPEWIRE_REMOTE"
            | "LD_LIBRARY_PATH"
            | "OTTO_MEDIA_TRACE"
    ) || key.starts_with("GST_")
        || key.starts_with("LIBVA_")
}

/// Where the worker binary is.
///
/// `OTTO_MEDIA_WORKER` names it outright — a development checkout building
/// the worker into a different target directory. Otherwise it is looked for
/// next to the host's own executable, which is where an install puts both,
/// and finally on `PATH`.
fn worker_executable() -> io::Result<PathBuf> {
    const NAME: &str = "otto-media-worker";
    if let Some(path) = std::env::var_os("OTTO_MEDIA_WORKER") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(own) = std::env::current_exe() {
        let sibling = own.with_file_name(NAME);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    let on_path = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(NAME))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_default();
    on_path.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "otto-media-worker is not installed next to this program or on PATH",
        )
    })
}

/// Whether playback is available at all on this system: the worker can be
/// found. Cheap, so a host may ask before deciding what to draw.
pub fn available() -> bool {
    worker_executable().is_ok()
}
