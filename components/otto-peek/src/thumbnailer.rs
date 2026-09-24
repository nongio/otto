//! Thumbnails made off the UI thread, for apps that show files as Files does.
//!
//! [`crate::thumbnail`] blocks: it reads the shared cache and may run the
//! sandboxed decoder. A [`Thumbnailer`] runs it on a thread of its own, one
//! file at a time, and reports back through a socket an app's poll loop
//! watches, the way Ask reports to the launcher.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;

use skia_safe::Image;

use crate::thumbcache::Size;

/// A file's thumbnail, or `None` when it has none.
pub type Thumbnail = (PathBuf, Option<Image>);

/// Makes thumbnails in the background, one at a time.
pub struct Thumbnailer {
    jobs: mpsc::Sender<PathBuf>,
    done: mpsc::Receiver<Thumbnail>,
    wake: UnixStream,
}

impl Thumbnailer {
    /// Start a thread that makes thumbnails at `size`.
    ///
    /// # Errors
    ///
    /// When the wake-up socket or the thread cannot be made.
    pub fn new(size: Size) -> std::io::Result<Self> {
        let (jobs, queued) = mpsc::channel::<PathBuf>();
        let (finished, done) = mpsc::channel();
        let (wake, wake_tx) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        wake_tx.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                // Ends when the app drops its side.
                for path in queued {
                    let modified = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok();
                    let image = crate::thumbnail(&path, modified, size);
                    if finished.send((path, image)).is_err() {
                        return;
                    }
                    // A full socket already has a wake-up in it.
                    let _ = (&wake_tx).write(&[1]);
                }
            })?;
        Ok(Self { jobs, done, wake })
    }

    /// Make a thumbnail of `path`; it comes back from [`Self::take`].
    pub fn request(&self, path: PathBuf) {
        let _ = self.jobs.send(path);
    }

    /// The socket that becomes readable when thumbnails are ready.
    pub fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// A handle on the same socket, for a loop that wants to own what it
    /// watches.
    ///
    /// # Errors
    ///
    /// When the socket cannot be duplicated.
    pub fn wake_handle(&self) -> std::io::Result<UnixStream> {
        self.wake.try_clone()
    }

    /// The thumbnails made since the last call. Never blocks.
    pub fn take(&mut self) -> Vec<Thumbnail> {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.done.try_iter().collect()
    }
}
