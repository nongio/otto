//! The connection thread every agent UI runs beside its own loop.
//!
//! A UI's loop is not async, and AHP's client is. [`Link`] starts a thread
//! with a current-thread runtime of its own, runs the connection there, and
//! hands what it learns back over a channel, waking the loop through a socket
//! the loop polls: [`Link::poll_fd`]. The session feed and the Ask chat both
//! talk to otto-agents this way.

// Rust guideline compliant 2026-02-21

use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use ahp::Client;

use crate::sessions::{connect, CONNECT_TIMEOUT};

/// The connection thread's side of the channel.
pub struct Reporter<U> {
    updates: mpsc::Sender<U>,
    wake: UnixStream,
}

impl<U> Reporter<U> {
    /// Hand `update` to the loop, and wake it.
    pub fn send(&self, update: U) {
        if self.updates.send(update).is_ok() {
            // A full socket already has a wake-up in it, which is all a byte
            // is for.
            let _ = (&self.wake).write(&[1]);
        }
    }
}

/// The loop's side: what the connection thread reports, and the socket that
/// says there is some.
pub struct Link<U> {
    updates: mpsc::Receiver<U>,
    wake: UnixStream,
}

impl<U: Send + 'static> Link<U> {
    /// Start a thread called `name` and run `run` on it, in a runtime of its
    /// own, with the [`Reporter`] it sends its updates through. A runtime
    /// that cannot be built is reported as `failed` says.
    ///
    /// # Panics
    ///
    /// Panics when the process cannot make a socket pair or start a thread,
    /// which only happens when it is out of descriptors or memory.
    pub fn start<F, Fut>(name: &str, failed: fn(String) -> U, run: F) -> Self
    where
        F: FnOnce(Reporter<U>) -> Fut + Send + 'static,
        Fut: Future<Output = ()>,
    {
        let (update_tx, updates) = mpsc::channel();
        let (wake, wake_tx) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => panic!("cannot create the {name} thread's wake-up socket: {err}"),
        };
        let _ = wake.set_nonblocking(true);
        let _ = wake_tx.set_nonblocking(true);
        let reporter = Reporter {
            updates: update_tx,
            wake: wake_tx,
        };

        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        reporter.send(failed(err.to_string()));
                        return;
                    }
                };
                runtime.block_on(run(reporter));
            })
            .unwrap_or_else(|err| panic!("cannot start the {name} thread: {err}"));

        Self { updates, wake }
    }

    /// The socket that becomes readable when there is news.
    pub fn poll_fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    /// Everything the thread has reported since the last call, in order.
    /// Never blocks.
    pub fn drain(&mut self) -> mpsc::TryIter<'_, U> {
        let mut buffer = [0u8; 64];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.updates.try_iter()
    }
}

/// Connect to the service at `url` as `name`, giving up after
/// [`CONNECT_TIMEOUT`].
///
/// # Errors
///
/// Fails with what to tell the person: the service could not be reached, or
/// did not answer in time.
pub async fn reach(url: &str, name: &str) -> Result<Client, String> {
    match tokio::time::timeout(CONNECT_TIMEOUT, connect(url, name)).await {
        Ok(Ok(client)) => Ok(client),
        Ok(Err(err)) => Err(format!("otto-agents at {url}: {err}")),
        Err(_) => Err(format!("otto-agents at {url} did not answer")),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn what_the_thread_reports_reaches_the_loop_in_order() {
        let mut link = Link::start(
            "link-test",
            |err| err,
            |reporter| async move {
                reporter.send("one".to_owned());
                reporter.send("two".to_owned());
            },
        );
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < 2 && Instant::now() < deadline {
            got.extend(link.drain());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, ["one", "two"]);
        assert!(link.drain().next().is_none(), "nothing is reported twice");
    }

    #[test]
    fn an_unreachable_service_is_an_error_to_show() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        // Nothing speaks WebSocket on the discard port.
        let err = runtime
            .block_on(reach("ws://127.0.0.1:9", "test"))
            .err()
            .expect("nothing to connect to");
        assert!(err.starts_with("otto-agents at ws://127.0.0.1:9"), "{err}");
    }
}
