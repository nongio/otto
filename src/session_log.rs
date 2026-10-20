//! Otto's own log file, `$XDG_STATE_HOME/otto/otto.log`.
//!
//! The compositor's tracing output goes to stderr and, through this module,
//! to a file, so a session started by a display manager (whose stderr nobody
//! keeps) still leaves a log behind. The previous session's file is kept as
//! `otto.log.1`: after a crash, that is the one worth reading.
//!
//! Writing never blocks the thread that logs. Each event is formatted into a
//! buffer and handed to a writer thread over a bounded channel; when the
//! channel is full the event is counted and dropped, and the count is written
//! once there is room again.

// Rust guideline compliant 2026-02-21

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// How many formatted events may wait for the writer thread.
///
/// A burst at `debug` level (a few thousand events in a frame) fits; beyond
/// that, events are dropped rather than stalling the compositor on disk I/O.
const QUEUE_LEN: usize = 8192;

/// The most bytes gathered into one `write` call.
const BATCH_BYTES: usize = 64 * 1024;

/// The file the running compositor writes, once [`start`] has opened it.
static CURRENT: OnceLock<(PathBuf, SyncSender<Message>)> = OnceLock::new();

/// What the writer thread receives.
enum Message {
    /// One formatted event.
    Event(Vec<u8>),
    /// Answer once everything sent before has been written.
    Flush(SyncSender<()>),
}

/// A log file fed by a writer thread; hand it to
/// `tracing_subscriber::fmt::layer().with_writer`.
#[derive(Debug, Clone)]
pub struct LogFile {
    path: PathBuf,
    tx: SyncSender<Message>,
    dropped: Arc<AtomicUsize>,
}

impl LogFile {
    /// Opens `path` for this session, keeping the previous one as `<path>.1`.
    ///
    /// # Errors
    ///
    /// Fails when the folder cannot be created or the file cannot be opened.
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if path.exists() {
            let mut previous = path.as_os_str().to_owned();
            previous.push(".1");
            // A failed rotation only loses the previous session's log.
            let _ = fs::rename(path, previous);
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let (tx, rx) = mpsc::sync_channel(QUEUE_LEN);
        let dropped = Arc::new(AtomicUsize::new(0));
        let thread_dropped = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("otto-log".into())
            .spawn(move || write_loop(file, &rx, &thread_dropped))?;
        Ok(Self {
            path: path.to_owned(),
            tx,
            dropped,
        })
    }

    /// Where the log is written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Waits up to `timeout` for everything logged so far to reach the file.
    pub fn flush(&self, timeout: Duration) {
        flush_through(&self.tx, timeout);
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = EventWriter;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            buf: Vec::new(),
            tx: self.tx.clone(),
            dropped: Arc::clone(&self.dropped),
        }
    }
}

/// Collects one event and queues it for the writer thread when dropped.
#[derive(Debug)]
pub struct EventWriter {
    buf: Vec<u8>,
    tx: SyncSender<Message>,
    dropped: Arc<AtomicUsize>,
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let event = std::mem::take(&mut self.buf);
        if let Err(TrySendError::Full(_)) = self.tx.try_send(Message::Event(event)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The writer thread: batches queued events into few `write` calls.
fn write_loop(mut file: File, rx: &Receiver<Message>, dropped: &AtomicUsize) {
    let mut out = Vec::with_capacity(BATCH_BYTES);
    let mut acks = Vec::new();
    while let Ok(first) = rx.recv() {
        let mut next = Some(first);
        while let Some(message) = next.take() {
            match message {
                Message::Event(bytes) => out.extend_from_slice(&bytes),
                Message::Flush(ack) => acks.push(ack),
            }
            if out.len() < BATCH_BYTES {
                next = rx.try_recv().ok();
            }
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            out.extend_from_slice(format!("[otto.log: {lost} events dropped]\n").as_bytes());
        }
        // Nothing sensible to do when the disk refuses: the log is a
        // convenience and stderr still has everything.
        let _ = file.write_all(&out);
        out.clear();
        for ack in acks.drain(..) {
            let _ = ack.send(());
        }
    }
}

/// Sends a flush request down `tx` and waits up to `timeout` for the answer.
fn flush_through(tx: &SyncSender<Message>, timeout: Duration) {
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    if tx.try_send(Message::Flush(ack_tx)).is_ok() {
        let _ = ack_rx.recv_timeout(timeout);
    }
}

/// Opens `dir/name` as the session's log and records it for [`path`] and
/// [`flush`].
///
/// # Errors
///
/// As [`LogFile::create`].
pub fn start(dir: &Path, name: &str) -> io::Result<LogFile> {
    let log = LogFile::create(&dir.join(name))?;
    let _ = CURRENT.set((log.path.clone(), log.tx.clone()));
    Ok(log)
}

/// The file this session logs to, once [`start`] has opened it.
pub fn path() -> Option<&'static Path> {
    CURRENT.get().map(|(path, _)| path.as_path())
}

/// Waits up to `timeout` for everything logged so far to reach the file:
/// called when the process is about to die.
pub fn flush(timeout: Duration) {
    if let Some((_, tx)) = CURRENT.get() {
        flush_through(tx, timeout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn keeps_the_previous_session_and_writes_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/otto.log");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "older session\n").unwrap();
        fs::write(dir.path().join("state/otto.log.1"), "oldest\n").unwrap();

        let log = LogFile::create(&path).unwrap();
        for line in ["first\n", "second\n"] {
            let mut writer = log.make_writer();
            writer.write_all(line.as_bytes()).unwrap();
        }
        log.flush(Duration::from_secs(5));

        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        assert_eq!(
            fs::read_to_string(dir.path().join("state/otto.log.1")).unwrap(),
            "older session\n"
        );
    }
}
