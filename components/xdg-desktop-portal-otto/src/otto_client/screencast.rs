//! ScreenCast D-Bus client for ScreenComposer's backend API.
//!
//! This module speaks to `org.otto.ScreenCast` (the backend interface
//! described in `ScreenCast-backend-spec.md`) so the portal can fulfill frontend
//! requests.

use std::collections::HashMap;

use tracing::{debug, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Result;

use crate::otto_client::OttoClient;

/// D-Bus proxy for `org.otto.ScreenCast` service.
#[zbus::proxy(
    interface = "org.otto.ScreenCast",
    default_service = "org.otto.ScreenCast",
    default_path = "/org/otto/ScreenCast"
)]
trait ScreenCast {
    /// Creates a new screencast session with the given properties.
    async fn create_session(&self, properties: HashMap<&str, Value<'_>>)
        -> Result<OwnedObjectPath>;

    /// Lists available output connectors.
    async fn list_outputs(&self) -> Result<Vec<String>>;

    /// Lists capturable windows as `(identifier, app_id, title)`.
    async fn list_windows(&self) -> Result<Vec<(String, String, String)>>;
}

/// A capturable window as reported by the compositor.
#[derive(Clone, Debug)]
pub struct WindowSource {
    /// `ext-foreign-toplevel-list-v1` identifier — opaque, pass back verbatim.
    pub id: String,
    pub app_id: String,
    pub title: String,
}

/// D-Bus proxy for `org.otto.ScreenCast.Session`.
#[zbus::proxy(
    interface = "org.otto.ScreenCast.Session",
    default_service = "org.otto.ScreenCast"
)]
trait ScreenCastSession {
    /// Starts recording a monitor by connector name.
    async fn record_monitor(
        &self,
        connector: &str,
        properties: HashMap<&str, Value<'_>>,
    ) -> Result<OwnedObjectPath>;

    /// Starts recording a window (not yet implemented in compositor).
    async fn record_window(&self, properties: HashMap<&str, Value<'_>>) -> Result<OwnedObjectPath>;

    /// Starts all streams in the session.
    async fn start(&self) -> Result<()>;

    /// Stops the session and all its streams.
    async fn stop(&self) -> Result<()>;
}

/// D-Bus proxy for `org.otto.ScreenCast.Stream`.
#[zbus::proxy(
    interface = "org.otto.ScreenCast.Stream",
    default_service = "org.otto.ScreenCast"
)]
trait ScreenCastStream {
    /// Starts this individual stream.
    async fn start(&self) -> Result<()>;

    /// Stops this individual stream.
    async fn stop(&self) -> Result<()>;

    /// Returns PipeWire node metadata including `node-id`.
    async fn pipe_wire_node(&self) -> Result<HashMap<String, OwnedValue>>;

    /// Returns static stream metadata (mapping id, geometry, etc.).
    async fn metadata(&self) -> Result<HashMap<String, OwnedValue>>;
}

impl OttoClient {
    /// Creates a new screencast session with the specified cursor mode.
    pub async fn create_session(&self, cursor_mode: u32) -> Result<String> {
        let proxy = ScreenCastProxy::builder(&self.connection).build().await?;

        let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
        properties.insert("cursor-mode", Value::U32(cursor_mode));

        debug!(cursor_mode, "Creating ScreenComposer session");
        let session_path: OwnedObjectPath = proxy.create_session(properties).await?;
        debug!(%session_path, "ScreenComposer session created");

        Ok(session_path.to_string())
    }

    /// Lists available output connectors from the compositor.
    pub async fn list_outputs(&self) -> Result<Vec<String>> {
        debug!("Requesting list_outputs from compositor");
        let proxy = ScreenCastProxy::builder(&self.connection).build().await?;
        let outputs = proxy.list_outputs().await?;
        debug!(
            "Received {} outputs from compositor: {:?}",
            outputs.len(),
            outputs
        );
        Ok(outputs)
    }

    /// Lists capturable windows from the compositor.
    pub async fn list_windows(&self) -> Result<Vec<WindowSource>> {
        debug!("Requesting list_windows from compositor");
        let proxy = ScreenCastProxy::builder(&self.connection).build().await?;
        let windows = proxy.list_windows().await?;
        debug!("Received {} windows from compositor", windows.len());
        Ok(windows
            .into_iter()
            .map(|(id, app_id, title)| WindowSource { id, app_id, title })
            .collect())
    }

    /// Starts recording a single window by its foreign-toplevel identifier.
    pub async fn record_window(
        &self,
        session_path: &OwnedObjectPath,
        window_id: &str,
        cursor_mode: u32,
    ) -> Result<OwnedObjectPath> {
        let proxy = ScreenCastSessionProxy::builder(&self.connection)
            .path(session_path)?
            .build()
            .await?;

        let mut properties = HashMap::new();
        properties.insert("window-id", Value::from(window_id));
        properties.insert("cursor-mode", Value::U32(cursor_mode));
        debug!(window_id, "Recording window");
        let stream_path = proxy.record_window(properties).await?;
        debug!(%stream_path, "Stream created");

        Ok(stream_path)
    }

    /// Starts recording a monitor identified by connector name.
    pub async fn record_monitor(
        &self,
        session_path: &OwnedObjectPath,
        connector: &str,
        cursor_mode: u32,
    ) -> Result<OwnedObjectPath> {
        let proxy = ScreenCastSessionProxy::builder(&self.connection)
            .path(session_path)?
            .build()
            .await?;

        let mut properties = HashMap::new();
        properties.insert("cursor-mode", Value::U32(cursor_mode));
        debug!(connector, "Recording monitor");
        let stream_path = proxy.record_monitor(connector, properties).await?;
        debug!(%stream_path, "Stream created");

        Ok(stream_path)
    }

    /// Starts all streams in the session.
    pub async fn start_session(&self, session_path: &OwnedObjectPath) -> Result<()> {
        let proxy = ScreenCastSessionProxy::builder(&self.connection)
            .path(session_path)?
            .build()
            .await?;

        debug!(%session_path, "Starting session");
        proxy.start().await
    }

    /// Stops the session and all its streams.
    pub async fn stop_session(&self, session_path: &OwnedObjectPath) -> Result<()> {
        let proxy = ScreenCastSessionProxy::builder(&self.connection)
            .path(session_path)?
            .build()
            .await?;

        debug!(%session_path, "Stopping session");
        proxy.stop().await
    }

    /// Starts an individual stream (usually called via session start).
    #[allow(dead_code)]
    pub async fn start_stream(&self, stream_path: &OwnedObjectPath) -> Result<()> {
        let proxy = ScreenCastStreamProxy::builder(&self.connection)
            .path(stream_path)?
            .build()
            .await?;

        debug!(%stream_path, "Starting stream");
        proxy.start().await
    }

    /// Gets the PipeWire node ID for a stream.
    pub async fn get_pipewire_node_id(&self, stream_path: &OwnedObjectPath) -> Result<u32> {
        let proxy = ScreenCastStreamProxy::builder(&self.connection)
            .path(stream_path)?
            .build()
            .await?;

        let node_info = proxy.pipe_wire_node().await?;
        node_info
            .get("node-id")
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| zbus::Error::Failure("No node-id in PipeWire node info".to_string()))
    }

    /// Gets full PipeWire node metadata for a stream.
    pub async fn get_pipewire_node(
        &self,
        stream_path: &OwnedObjectPath,
    ) -> Result<HashMap<String, OwnedValue>> {
        let proxy = ScreenCastStreamProxy::builder(&self.connection)
            .path(stream_path)?
            .build()
            .await?;

        proxy.pipe_wire_node().await
    }

    /// Gets compositor-provided stream metadata (mapping id, geometry, etc.).
    pub async fn get_stream_metadata(
        &self,
        stream_path: &OwnedObjectPath,
    ) -> Result<HashMap<String, OwnedValue>> {
        let proxy = ScreenCastStreamProxy::builder(&self.connection)
            .path(stream_path)?
            .build()
            .await?;

        proxy.metadata().await
    }

    /// Opens a PipeWire remote file descriptor for the session.
    pub async fn open_pipewire_remote(
        &self,
        session_path: &OwnedObjectPath,
    ) -> Result<zbus::zvariant::OwnedFd> {
        let result: Result<zbus::zvariant::OwnedFd> = self
            .connection
            .call_method(
                Some("org.otto.ScreenCast"),
                session_path,
                Some("org.otto.ScreenCast.Session"),
                "OpenPipeWireRemote",
                &HashMap::<String, Value>::new(),
            )
            .await?
            .body()
            .deserialize();

        match result {
            Ok(fd) => {
                debug!("Received PipeWire FD from Otto");
                Ok(fd)
            }
            Err(e) => {
                warn!(?e, "Failed to get PipeWire FD");
                Err(zbus::Error::Failure(format!(
                    "Failed to get PipeWire FD: {e:?}"
                )))
            }
        }
    }
}
