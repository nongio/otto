//! D-Bus Request object for async portal responses.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::Notify;
use tracing::{info, warn};
use zbus::fdo;
use zbus::interface;
use zbus::zvariant::OwnedObjectPath;
use zbus::ObjectServer;

/// Represents a pending portal request.
///
/// The frontend calls `Close` on it when the requesting application withdraws
/// (it exited, or the user closed the window that asked). The method still
/// running for the request waits on its [`Cancellation`] and answers `1`
/// (cancelled) instead of carrying on.
#[derive(Clone)]
pub struct Request {
    path: OwnedObjectPath,
    cancel: Cancellation,
}

impl Request {
    /// Creates a new request with the given D-Bus object path.
    pub fn new(path: OwnedObjectPath) -> Self {
        Self {
            path,
            cancel: Cancellation::default(),
        }
    }

    /// The handle the pending method waits on.
    pub fn cancellation(&self) -> Cancellation {
        self.cancel.clone()
    }

    /// Run `work` for the request at `path`, with the request object exported
    /// there for as long as it runs: `None` when the frontend closes the
    /// request first, and `work` is dropped where it stood.
    ///
    /// Whatever `work` has put up on screen must come down when it is
    /// dropped; the dialog helpers in `otto_client` withdraw theirs.
    ///
    /// A request that cannot be exported still runs, only without a way to
    /// close it.
    pub async fn serve<R>(
        object_server: &ObjectServer,
        path: &OwnedObjectPath,
        work: impl Future<Output = R>,
    ) -> Option<R> {
        let request = Self::new(path.clone());
        let cancellation = request.cancellation();
        let exported = object_server
            .at(path.clone(), request)
            .await
            .unwrap_or_else(|err| {
                warn!(?err, request = %path, "could not export the request object");
                false
            });
        let result = cancellation.run(work).await;
        if exported {
            if let Err(err) = object_server.remove::<Self, _>(path).await {
                warn!(?err, request = %path, "could not remove the request object");
            }
        }
        result
    }
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    /// Called by the frontend to cancel the request.
    async fn close(&self) -> fdo::Result<()> {
        info!(request = %self.path, "Request.Close called; cancelling");
        self.cancel.cancel();
        Ok(())
    }
}

/// Signalled once, when the frontend closes the request.
///
/// A `Close` that lands before the method starts waiting is not lost:
/// [`Notify::notify_one`] keeps the permit for the next waiter.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<Notify>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.notify_one();
    }

    /// Runs `work` unless the request is closed first: `None` then, and
    /// `work` is dropped where it stood.
    pub async fn run<R>(&self, work: impl Future<Output = R>) -> Option<R> {
        tokio::select! {
            result = work => Some(result),
            () = self.0.notified() => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn work_finishes_when_nobody_closes() {
        let cancel = Cancellation::default();
        assert_eq!(cancel.run(async { 7 }).await, Some(7));
    }

    #[tokio::test]
    async fn a_close_before_the_wait_still_cancels() {
        let cancel = Cancellation::default();
        cancel.cancel();
        let never = std::future::pending::<u32>();
        assert_eq!(cancel.run(never).await, None);
    }

    #[tokio::test]
    async fn a_close_during_the_wait_cancels() {
        let cancel = Cancellation::default();
        let closer = cancel.clone();
        tokio::spawn(async move { closer.cancel() });
        let never = std::future::pending::<u32>();
        assert_eq!(cancel.run(never).await, None);
    }
}
