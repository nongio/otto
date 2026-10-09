//! D-Bus Session object for managing screencast session lifecycle.

use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{info, warn};
use zbus::fdo;
use zbus::interface;
use zbus::object_server::ObjectServer;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use crate::otto_client::OttoClient;
use crate::portal::{PortalState, SessionState};

/// Represents an active screencast session.
///
/// Sessions are created by `CreateSession` and closed by `Close` or when
/// the client disconnects.
#[derive(Clone)]
pub struct Session {
    path: OwnedObjectPath,
    sc_client: Arc<OttoClient>,
    state: Arc<Mutex<PortalState>>,
}

impl Session {
    /// Creates a new session object.
    pub fn new(
        path: OwnedObjectPath,
        sc_client: Arc<OttoClient>,
        state: Arc<Mutex<PortalState>>,
    ) -> Self {
        Self {
            path,
            sc_client,
            state,
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    /// Closes the session and releases all resources.
    async fn close(
        &self,
        #[zbus(object_server)] object_server: &ObjectServer,
        #[zbus(signal_emitter)] ctx: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        info!(session = %self.path, "Session.Close called - client requested session termination");

        let removed_state = {
            let mut state = self.state.lock().await;
            state.sessions.remove(self.path.as_str())
        };

        if let Some(SessionState { sc_session, .. }) = removed_state {
            info!(sc_session = %sc_session, "Stopping compositor session");
            match self.sc_client.stop_session(&sc_session).await {
                Ok(()) => {
                    info!(sc_session = %sc_session, "Compositor session stopped successfully");
                }
                Err(err) => {
                    warn!(
                        sc_session = %sc_session,
                        ?err,
                        "Failed to stop compositor session (may already be stopped)"
                    );
                }
            }
        } else {
            warn!(session = %self.path, "No compositor session found for portal session");
        }

        info!(session = %self.path, "Emitting Session::Closed signal");
        Session::closed(&ctx)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;

        info!(session = %self.path, "Removing session object from D-Bus");
        object_server
            .remove::<Session, _>(&self.path)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;

        Ok(())
    }

    /// Signal emitted when the session is closed.
    #[zbus(signal)]
    async fn closed(ctx: &SignalEmitter<'_>) -> zbus::Result<()>;
}
