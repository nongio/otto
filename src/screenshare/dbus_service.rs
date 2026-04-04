//! D-Bus service implementation for `org.otto.ScreenCast`.
//!
//! Implements the backend D-Bus API that the portal expects, as defined in
//! the portal's otto_client module.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use smithay::reexports::calloop::channel::Sender;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

use super::CompositorCommand;

/// Global session counter for unique IDs.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Global stream counter for unique IDs.
static STREAM_COUNTER: AtomicU64 = AtomicU64::new(1);

/// The main ScreenCast D-Bus interface.
///
/// Implements `org.otto.ScreenCast` at `/org/otto/ScreenCast`.
pub struct ScreenCastInterface {
    /// Channel to send commands to the compositor's main loop.
    compositor_tx: Sender<CompositorCommand>,
    /// Active sessions indexed by their object path.
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    /// D-Bus connection for registering session objects.
    connection: Connection,
}

/// Internal state for a session.
#[derive(Clone)]
struct SessionState {
    #[allow(dead_code)]
    cursor_mode: u32,
    streams: Vec<String>, // Stream object paths
    started: bool,
}

impl ScreenCastInterface {
    fn new(compositor_tx: Sender<CompositorCommand>, connection: Connection) -> Self {
        Self {
            compositor_tx,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            connection,
        }
    }
}

#[interface(name = "org.otto.ScreenCast")]
impl ScreenCastInterface {
    /// Creates a new screencast session.
    ///
    /// Properties may include:
    /// - `cursor-mode`: u32 (0 = hidden, 1 = embedded, 2 = metadata)
    async fn create_session(
        &self,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let cursor_mode = properties
            .get("cursor-mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1); // Default to embedded cursor

        let session_id = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session_path = format!("/org/otto/ScreenCast/session/{session_id}");

        info!(
            session_id,
            cursor_mode, "Creating screencast session at {session_path}"
        );

        // Store session state
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                session_path.clone(),
                SessionState {
                    cursor_mode,
                    streams: Vec::new(),
                    started: false,
                },
            );
        }

        // Register the session D-Bus object
        let session_iface = SessionInterface::new(
            session_path.clone(),
            self.compositor_tx.clone(),
            self.sessions.clone(),
            self.connection.clone(),
        );

        let path = ObjectPath::try_from(session_path.as_str())
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid session path: {e}")))?;

        self.connection
            .object_server()
            .at(path, session_iface)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("Failed to register session: {e}")))?;

        debug!("Registered session interface at {session_path}");

        // Notify compositor
        if let Err(e) = self.compositor_tx.send(CompositorCommand::CreateSession {
            session_id: session_path.clone(),
            cursor_mode,
        }) {
            error!(?e, "Failed to notify compositor of session creation");
        }

        OwnedObjectPath::try_from(session_path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid path: {e}")))
    }

    /// Lists available output connectors.
    async fn list_outputs(&self) -> zbus::fdo::Result<Vec<String>> {
        debug!("Listing outputs (D-Bus handler)");

        let (tx, rx) = tokio::sync::oneshot::channel();

        self.compositor_tx
            .send(CompositorCommand::ListOutputs { response_tx: tx })
            .map_err(|e| {
                error!("Failed to send ListOutputs command: {}", e);
                zbus::fdo::Error::Failed(format!("Channel send error: {e}"))
            })?;

        debug!("ListOutputs command sent, waiting for response");
        let outputs = rx.await.map_err(|e| {
            error!("Failed to receive ListOutputs response: {}", e);
            zbus::fdo::Error::Failed(format!("Response channel error: {e}"))
        })?;

        let connectors: Vec<String> = outputs.into_iter().map(|o| o.connector).collect();
        debug!("Received {} outputs: {:?}", connectors.len(), connectors);
        Ok(connectors)
    }
}

/// Session D-Bus interface.
///
/// Implements `org.otto.ScreenCast.Session` at dynamic paths.
pub struct SessionInterface {
    /// The session's object path.
    session_path: String,
    /// Channel to send commands to the compositor's main loop.
    compositor_tx: Sender<CompositorCommand>,
    /// Shared session state.
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
    /// D-Bus connection for registering stream objects.
    connection: Connection,
    /// Streams owned by this session.
    streams: Arc<RwLock<HashMap<String, StreamState>>>,
}

/// Internal state for a stream.
#[derive(Clone)]
struct StreamState {
    connector: String,
    cursor_mode: u32,
    node_id: Option<u32>,
    width: u32,
    height: u32,
    started: bool,
}

impl SessionInterface {
    fn new(
        session_path: String,
        compositor_tx: Sender<CompositorCommand>,
        sessions: Arc<RwLock<HashMap<String, SessionState>>>,
        connection: Connection,
    ) -> Self {
        Self {
            session_path,
            compositor_tx,
            sessions,
            connection,
            streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[interface(name = "org.otto.ScreenCast.Session")]
impl SessionInterface {
    /// Starts recording a monitor by connector name.
    async fn record_monitor(
        &mut self,
        connector: &str,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let cursor_mode = properties
            .get("cursor-mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1);

        let stream_id = STREAM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stream_path = format!("{}/stream/{stream_id}", self.session_path);

        info!(
            %connector,
            cursor_mode,
            "Recording monitor, stream at {stream_path}"
        );

        // Get output info from compositor to know dimensions
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ListOutputs { response_tx: tx })
            .map_err(|e| zbus::fdo::Error::Failed(format!("Channel send error: {e}")))?;

        let outputs = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("Response channel error: {e}")))?;

        let output = outputs
            .iter()
            .find(|o| o.connector == connector)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("Output {connector} not found")))?;

        // Store stream state (node_id will be set when PipeWire stream starts)
        {
            let mut streams = self.streams.write().await;
            streams.insert(
                stream_path.clone(),
                StreamState {
                    connector: connector.to_string(),
                    cursor_mode,
                    node_id: None,
                    width: output.width,
                    height: output.height,
                    started: false,
                },
            );
        }

        // Update session state
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&self.session_path) {
                session.streams.push(stream_path.clone());
            }
        }

        // Register the stream D-Bus object
        let stream_iface = StreamInterface::new(
            stream_path.clone(),
            self.compositor_tx.clone(),
            self.streams.clone(),
        );

        let path = ObjectPath::try_from(stream_path.as_str())
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid stream path: {e}")))?;

        self.connection
            .object_server()
            .at(path, stream_iface)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("Failed to register stream: {e}")))?;

        OwnedObjectPath::try_from(stream_path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid path: {e}")))
    }

    /// Starts recording a window (not implemented).
    async fn record_window(
        &self,
        _properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        Err(zbus::fdo::Error::NotSupported(
            "Window recording not yet implemented".to_string(),
        ))
    }

    /// Starts all streams in the session.
    async fn start(&mut self) -> zbus::fdo::Result<()> {
        info!(session = %self.session_path, "Starting session");

        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&self.session_path) {
                session.started = true;
            }
        }

        // Start all streams
        let stream_paths: Vec<String> = {
            let streams = self.streams.read().await;
            streams.keys().cloned().collect()
        };

        for stream_path in stream_paths {
            let (connector, cursor_mode) = {
                let streams = self.streams.read().await;
                streams
                    .get(&stream_path)
                    .map(|s| (s.connector.clone(), s.cursor_mode))
                    .unwrap_or_else(|| {
                        // Skip if stream not found
                        (String::new(), 0)
                    })
            };

            if connector.is_empty() {
                continue;
            }

            // Create response channel for node_id
            let (tx, rx) = tokio::sync::oneshot::channel();

            // Notify compositor to start recording
            if let Err(e) = self.compositor_tx.send(CompositorCommand::StartRecording {
                session_id: self.session_path.clone(),
                output_connector: connector.clone(),
                cursor_mode,
                response_tx: tx,
            }) {
                error!(?e, "Failed to start recording");
                return Err(zbus::fdo::Error::Failed(format!(
                    "Failed to start recording: {e}"
                )));
            }

            // Wait for response with node_id
            match rx.await {
                Ok(Ok(node_id)) => {
                    info!(%connector, node_id, "Recording started, got PipeWire node");
                    let mut streams = self.streams.write().await;
                    if let Some(stream) = streams.get_mut(&stream_path) {
                        info!(session = %self.session_path, stream_path = %stream_path, connector = %connector, node_id, "Marking stream as started in session");
                        stream.started = true;
                        stream.node_id = Some(node_id);
                    }
                }
                Ok(Err(e)) => {
                    error!(%connector, %e, "Failed to start recording");
                    return Err(zbus::fdo::Error::Failed(format!(
                        "Failed to start recording: {e}"
                    )));
                }
                Err(e) => {
                    error!(%connector, ?e, "Response channel error");
                    return Err(zbus::fdo::Error::Failed(format!(
                        "Response channel error: {e}"
                    )));
                }
            }
        }

        Ok(())
    }

    /// Stops the session and all its streams.
    async fn stop(&mut self) -> zbus::fdo::Result<()> {
        info!(session = %self.session_path, "Stopping session");

        // Stop all streams
        let mut streams = self.streams.write().await;
        info!(session = %self.session_path, stream_count = streams.len(), "Stopping {} streams", streams.len());
        for (path, stream) in streams.iter_mut() {
            if stream.started {
                info!(session = %self.session_path, stream_path = %path, connector = %stream.connector, "Stopping started stream");
                stream.started = false;

                if let Err(e) = self.compositor_tx.send(CompositorCommand::StopRecording {
                    session_id: self.session_path.clone(),
                    output_connector: stream.connector.clone(),
                }) {
                    warn!(?e, "Failed to stop recording");
                }
            } else {
                info!(session = %self.session_path, stream_path = %path, connector = %stream.connector, "Skipping non-started stream");
            }
        }

        // Mark session as stopped
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&self.session_path) {
                session.started = false;
            }
        }

        Ok(())
    }

    /// Opens a PipeWire remote file descriptor.
    ///
    /// This returns an FD that can be used with `pw_context_connect_fd()`.
    async fn open_pipe_wire_remote(
        &self,
        _options: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedFd> {
        debug!(session = %self.session_path, "Opening PipeWire remote");

        // Request PipeWire FD from compositor
        let (tx, rx) = tokio::sync::oneshot::channel();

        self.compositor_tx
            .send(CompositorCommand::GetPipeWireFd {
                session_id: self.session_path.clone(),
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("Channel send error: {e}")))?;

        let fd = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("Response channel error: {e}")))?
            .map_err(|e| zbus::fdo::Error::Failed(format!("PipeWire error: {e}")))?;

        Ok(fd)
    }
}

/// Stream D-Bus interface.
///
/// Implements `org.otto.ScreenCast.Stream` at dynamic paths.
pub struct StreamInterface {
    /// The stream's object path.
    stream_path: String,
    /// Channel to send commands to the compositor's main loop.
    #[allow(dead_code)]
    compositor_tx: Sender<CompositorCommand>,
    /// Shared stream state.
    streams: Arc<RwLock<HashMap<String, StreamState>>>,
}

impl StreamInterface {
    fn new(
        stream_path: String,
        compositor_tx: Sender<CompositorCommand>,
        streams: Arc<RwLock<HashMap<String, StreamState>>>,
    ) -> Self {
        Self {
            stream_path,
            compositor_tx,
            streams,
        }
    }
}

#[interface(name = "org.otto.ScreenCast.Stream")]
impl StreamInterface {
    /// Starts this individual stream.
    async fn start(&self) -> zbus::fdo::Result<()> {
        debug!(stream = %self.stream_path, "Starting stream");

        let mut streams = self.streams.write().await;
        if let Some(stream) = streams.get_mut(&self.stream_path) {
            stream.started = true;

            // Note: actual recording start happens when session.start() is called
            // This is here for individual stream control if needed
        }

        Ok(())
    }

    /// Stops this individual stream.
    async fn stop(&self) -> zbus::fdo::Result<()> {
        debug!(stream = %self.stream_path, "Stopping stream");

        let mut streams = self.streams.write().await;
        if let Some(stream) = streams.get_mut(&self.stream_path) {
            stream.started = false;
        }

        Ok(())
    }

    /// Returns PipeWire node metadata including `node-id`.
    async fn pipe_wire_node(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let streams = self.streams.read().await;
        let stream = streams
            .get(&self.stream_path)
            .ok_or_else(|| zbus::fdo::Error::Failed("Stream not found".to_string()))?;

        let node_id = stream.node_id.ok_or_else(|| {
            zbus::fdo::Error::Failed("PipeWire stream not yet started".to_string())
        })?;

        let mut result = HashMap::new();
        result.insert("node-id".to_string(), OwnedValue::from(node_id));

        Ok(result)
    }

    /// Returns static stream metadata.
    async fn metadata(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let streams = self.streams.read().await;
        let stream = streams
            .get(&self.stream_path)
            .ok_or_else(|| zbus::fdo::Error::Failed("Stream not found".to_string()))?;

        let mut result = HashMap::new();
        result.insert(
            "connector".to_string(),
            Value::from(stream.connector.as_str()).try_into().unwrap(),
        );
        result.insert("width".to_string(), OwnedValue::from(stream.width));
        result.insert("height".to_string(), OwnedValue::from(stream.height));
        result.insert(
            "cursor-mode".to_string(),
            OwnedValue::from(stream.cursor_mode),
        );

        Ok(result)
    }
}

/// Compositor D-Bus interface for health checks and app management.
pub struct CompositorInterface {
    compositor_tx: Sender<CompositorCommand>,
}

impl CompositorInterface {
    fn new(compositor_tx: Sender<CompositorCommand>) -> Self {
        Self { compositor_tx }
    }
}

#[interface(name = "org.otto.Compositor")]
impl CompositorInterface {
    /// Ping method for watchdog health checks.
    ///
    /// Returns "pong" if the compositor is responsive.
    async fn ping(&self) -> zbus::fdo::Result<String> {
        debug!("Ping received from watchdog");
        Ok("pong".to_string())
    }

    /// Focus an application window by app_id.
    ///
    /// Raises the most recent window belonging to the given app_id and
    /// gives it keyboard focus. Returns true if a matching window was found.
    async fn focus_app(&self, app_id: &str) -> zbus::fdo::Result<bool> {
        info!(app_id, "focus_app requested via D-Bus");
        self.compositor_tx
            .send(CompositorCommand::FocusApp {
                app_id: app_id.to_string(),
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        Ok(true)
    }
}

/// Starts the D-Bus service on the session bus.
pub async fn run_dbus_service(compositor_tx: Sender<CompositorCommand>) -> zbus::Result<()> {
    let connection = Connection::session().await?;

    let screencast = ScreenCastInterface::new(compositor_tx.clone(), connection.clone());

    connection
        .object_server()
        .at("/org/otto/ScreenCast", screencast)
        .await?;

    connection.request_name("org.otto.ScreenCast").await?;

    // Register the compositor interface (health + app management)
    let compositor = CompositorInterface::new(compositor_tx);
    connection
        .object_server()
        .at("/org/otto/Compositor", compositor)
        .await?;

    connection.request_name("org.otto.Compositor").await?;

    // Register the Settings interface
    crate::settings_service::register_settings_interface(&connection).await?;

    info!("D-Bus service started at org.otto.ScreenCast");

    // Keep the service running
    std::future::pending::<()>().await;

    Ok(())
}
