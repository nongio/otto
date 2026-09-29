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

use super::{CompositorCommand, StreamTarget, CURSOR_MODE_EMBEDDED};

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
    /// Session owners whose departure is being watched for.
    watched_owners: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

/// Internal state for a session.
#[derive(Clone)]
struct SessionState {
    /// The unique bus name that created it; see [`SessionInterface`].
    owner: String,
    /// The session's streams, shared with its [`SessionInterface`], so a
    /// session whose owner left can be torn down from outside it.
    streams: Arc<RwLock<HashMap<String, StreamState>>>,
    started: bool,
}

impl ScreenCastInterface {
    fn new(compositor_tx: Sender<CompositorCommand>, connection: Connection) -> Self {
        Self {
            compositor_tx,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            connection,
            watched_owners: Arc::default(),
        }
    }

    /// Tear down every session `owner` created once its name leaves the
    /// bus: a portal or otto-rdp that crashed cannot call `Stop`, and its
    /// PipeWire streams would otherwise keep the screen recorded.
    async fn watch_owner(&self, owner: String) -> zbus::Result<()> {
        if !self.watched_owners.lock().unwrap().insert(owner.clone()) {
            return Ok(());
        }
        let departure = match name_departure(&self.connection, &owner).await {
            Ok(departure) => departure,
            Err(err) => {
                self.watched_owners.lock().unwrap().remove(&owner);
                return Err(err);
            }
        };
        let connection = self.connection.clone();
        let compositor_tx = self.compositor_tx.clone();
        let sessions = self.sessions.clone();
        let watched = self.watched_owners.clone();
        tokio::spawn(async move {
            departure.await;
            watched.lock().unwrap().remove(&owner);
            let owned: Vec<String> = sessions
                .read()
                .await
                .iter()
                .filter(|(_, session)| session.owner == owner)
                .map(|(path, _)| path.clone())
                .collect();
            if !owned.is_empty() {
                info!(
                    owner,
                    sessions = owned.len(),
                    "Screencast client left the bus; ending its sessions"
                );
            }
            for path in owned {
                teardown_session(&connection, &compositor_tx, &sessions, &path).await;
            }
        });
        Ok(())
    }
}

/// A future that finishes once `owner` has left the bus (at once, if it
/// already has).
async fn name_departure(
    connection: &Connection,
    owner: &str,
) -> zbus::Result<impl std::future::Future<Output = ()>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let mut changes = dbus
        .receive_name_owner_changed_with_args(&[(0, owner)])
        .await?;
    // Subscribed before this check, so a name that goes in between is still
    // seen: either here, or as a change.
    let gone_already = !dbus
        .name_has_owner(owner.try_into()?)
        .await
        .unwrap_or(false);
    Ok(async move {
        use zbus::export::futures_util::StreamExt;
        if gone_already {
            return;
        }
        while let Some(change) = changes.next().await {
            if change.args().is_ok_and(|args| args.new_owner().is_none()) {
                break;
            }
        }
    })
}

/// Takes interface `I` at `path` off the bus.
///
/// Teardown is best-effort: a path that has already gone is the outcome we
/// wanted, and failing to unregister is no reason to fail the caller's
/// `Stop`.
async fn unregister<I: zbus::object_server::Interface>(connection: &Connection, path: &str) {
    let object_path = match ObjectPath::try_from(path) {
        Ok(path) => path,
        Err(e) => {
            warn!(%path, ?e, "Invalid object path, not unregistering");
            return;
        }
    };

    match connection
        .object_server()
        .remove::<I, _>(&object_path)
        .await
    {
        Ok(_) => debug!(%path, "Unregistered object"),
        Err(e) => warn!(%path, ?e, "Failed to unregister object"),
    }
}

/// End session `session_path`: stop its streams, drop it and its PipeWire
/// streams in the compositor, and take its objects off the bus. Does nothing
/// for a session already gone, so `Stop` and an owner leaving can race.
async fn teardown_session(
    connection: &Connection,
    compositor_tx: &Sender<CompositorCommand>,
    sessions: &RwLock<HashMap<String, SessionState>>,
    session_path: &str,
) {
    let Some(session) = sessions.write().await.remove(session_path) else {
        return;
    };
    // Take the streams: nothing may start them again from here.
    let streams = std::mem::take(&mut *session.streams.write().await);

    info!(session = %session_path, stream_count = streams.len(), "Stopping {} streams", streams.len());
    for (path, stream) in &streams {
        if stream.started {
            info!(session = %session_path, stream_path = %path, target = %stream.target.key(), "Stopping started stream");
            if let Err(e) = compositor_tx.send(CompositorCommand::StopRecording {
                session_id: session_path.to_string(),
                target: stream.target.clone(),
            }) {
                warn!(?e, "Failed to stop recording");
            }
        } else {
            info!(session = %session_path, stream_path = %path, target = %stream.target.key(), "Skipping non-started stream");
        }
        unregister::<StreamInterface>(connection, path).await;
    }

    // Drop the compositor-side session, along with any stream the
    // bookkeeping above missed.
    if let Err(e) = compositor_tx.send(CompositorCommand::DestroySession {
        session_id: session_path.to_string(),
    }) {
        warn!(?e, "Failed to destroy session");
    }

    // Last, since it may take the very object calling this off the bus. zbus
    // releases the root lock before dispatching, so removing ourselves from
    // inside a method is fine.
    unregister::<SessionInterface>(connection, session_path).await;
}

#[interface(name = "org.otto.ScreenCast")]
impl ScreenCastInterface {
    /// Creates a new screencast session.
    ///
    /// Properties may include:
    /// - `cursor-mode`: u32 (1 = hidden, 2 = embedded, 4 = metadata)
    async fn create_session(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let owner = sender_of(&header)?;
        let cursor_mode = properties
            .get("cursor-mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(CURSOR_MODE_EMBEDDED);

        let session_id = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session_path = format!("/org/otto/ScreenCast/session/{session_id}");

        info!(
            session_id,
            cursor_mode, "Creating screencast session at {session_path}"
        );

        // Register the session D-Bus object
        let session_iface = SessionInterface::new(
            session_path.clone(),
            owner.clone(),
            self.compositor_tx.clone(),
            self.sessions.clone(),
            self.connection.clone(),
        );

        // Store session state
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                session_path.clone(),
                SessionState {
                    owner: owner.clone(),
                    streams: session_iface.streams.clone(),
                    started: false,
                },
            );
        }

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

        if let Err(err) = self.watch_owner(owner.clone()).await {
            // Without the watch a crashed client would leave its session
            // recording: refuse rather than hand out one that may leak.
            warn!(
                owner,
                "Cannot watch the screencast client's bus name: {err}"
            );
            teardown_session(
                &self.connection,
                &self.compositor_tx,
                &self.sessions,
                &session_path,
            )
            .await;
            return Err(zbus::fdo::Error::Failed(format!(
                "cannot watch the caller's bus name: {err}"
            )));
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

    /// Lists capturable windows as `(identifier, app_id, title)`.
    ///
    /// The identifier is the window's `ext-foreign-toplevel-list-v1` handle
    /// identifier; pass it back as the `window-id` property of `RecordWindow`.
    async fn list_windows(&self) -> zbus::fdo::Result<Vec<(String, String, String)>> {
        debug!("Listing windows (D-Bus handler)");

        let (tx, rx) = tokio::sync::oneshot::channel();

        self.compositor_tx
            .send(CompositorCommand::ListWindows { response_tx: tx })
            .map_err(|e| {
                error!("Failed to send ListWindows command: {}", e);
                zbus::fdo::Error::Failed(format!("Channel send error: {e}"))
            })?;

        let windows = rx.await.map_err(|e| {
            error!("Failed to receive ListWindows response: {}", e);
            zbus::fdo::Error::Failed(format!("Response channel error: {e}"))
        })?;

        debug!("Received {} windows", windows.len());
        Ok(windows
            .into_iter()
            .map(|w| (w.id, w.app_id, w.title))
            .collect())
    }
}

/// Session D-Bus interface.
///
/// Implements `org.otto.ScreenCast.Session` at dynamic paths.
pub struct SessionInterface {
    /// The session's object path.
    session_path: String,
    /// The unique bus name that created the session; the only one that may
    /// drive it. A unique name is never handed to another connection.
    owner: String,
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
    target: StreamTarget,
    cursor_mode: u32,
    node_id: Option<u32>,
    width: u32,
    height: u32,
    started: bool,
}

impl SessionInterface {
    fn new(
        session_path: String,
        owner: String,
        compositor_tx: Sender<CompositorCommand>,
        sessions: Arc<RwLock<HashMap<String, SessionState>>>,
        connection: Connection,
    ) -> Self {
        Self {
            session_path,
            owner,
            compositor_tx,
            sessions,
            connection,
            streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Allocate a stream object for `target` and export it on the bus.
    ///
    /// The PipeWire stream itself is not created until `Session.Start`; until
    /// then `node_id` stays `None`.
    async fn register_stream(
        &self,
        target: StreamTarget,
        cursor_mode: u32,
        width: u32,
        height: u32,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let stream_id = STREAM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stream_path = format!("{}/stream/{stream_id}", self.session_path);

        debug!(target = %target.key(), width, height, "Registering stream at {stream_path}");

        // Store stream state (node_id will be set when PipeWire stream starts)
        {
            let mut streams = self.streams.write().await;
            streams.insert(
                stream_path.clone(),
                StreamState {
                    target,
                    cursor_mode,
                    node_id: None,
                    width,
                    height,
                    started: false,
                },
            );
        }

        // Register the stream D-Bus object
        let stream_iface = StreamInterface::new(
            stream_path.clone(),
            self.owner.clone(),
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
}

#[interface(name = "org.otto.ScreenCast.Session")]
impl SessionInterface {
    /// Starts recording a monitor by connector name.
    async fn record_monitor(
        &mut self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connector: &str,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        deny_unless_owner(&header, &self.owner, "RecordMonitor")?;
        let cursor_mode = properties
            .get("cursor-mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(CURSOR_MODE_EMBEDDED);

        info!(%connector, cursor_mode, "Recording monitor");

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

        self.register_stream(
            StreamTarget::Output(connector.to_string()),
            cursor_mode,
            output.width,
            output.height,
        )
        .await
    }

    /// Starts recording a single window.
    ///
    /// Properties:
    /// - `window-id`: s — the window's `ext-foreign-toplevel-list-v1`
    ///   identifier, as returned by `org.otto.ScreenCast.ListWindows`.
    /// - `cursor-mode`: u32 — optional, defaults to embedded.
    async fn record_window(
        &mut self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        deny_unless_owner(&header, &self.owner, "RecordWindow")?;
        let window_id: String = properties
            .get("window-id")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs("RecordWindow requires a window-id".to_string())
            })?;

        let cursor_mode = properties
            .get("cursor-mode")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(CURSOR_MODE_EMBEDDED);

        info!(%window_id, cursor_mode, "Recording window");

        // Resolve the identifier to current window dimensions.
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ListWindows { response_tx: tx })
            .map_err(|e| zbus::fdo::Error::Failed(format!("Channel send error: {e}")))?;

        let windows = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("Response channel error: {e}")))?;

        let window = windows
            .iter()
            .find(|w| w.id == window_id)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("Window {window_id} not found")))?;

        self.register_stream(
            StreamTarget::Window(window_id.clone()),
            cursor_mode,
            window.width,
            window.height,
        )
        .await
    }

    /// Starts all streams in the session.
    async fn start(
        &mut self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        deny_unless_owner(&header, &self.owner, "Start")?;
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
            let Some((target, cursor_mode)) = ({
                let streams = self.streams.read().await;
                streams
                    .get(&stream_path)
                    .map(|s| (s.target.clone(), s.cursor_mode))
            }) else {
                // Stream vanished between listing and starting
                continue;
            };
            let connector = target.key();

            // Create response channel for node_id
            let (tx, rx) = tokio::sync::oneshot::channel();

            // Notify compositor to start recording
            if let Err(e) = self.compositor_tx.send(CompositorCommand::StartRecording {
                session_id: self.session_path.clone(),
                target,
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

    /// Stops the session, tears down its streams and unregisters both.
    ///
    /// This is terminal: the portal calls it from `Session.Close`, and a
    /// client that comes back gets a fresh session rather than inheriting a
    /// half-dead one. Leaving the objects on the bus and the recording state
    /// in the compositor would strand a PipeWire node per cast.
    async fn stop(
        &mut self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        deny_unless_owner(&header, &self.owner, "Stop")?;
        info!(session = %self.session_path, "Stopping session");
        teardown_session(
            &self.connection,
            &self.compositor_tx,
            &self.sessions,
            &self.session_path,
        )
        .await;
        Ok(())
    }

    /// Opens a PipeWire remote file descriptor.
    ///
    /// This returns an FD that can be used with `pw_context_connect_fd()`.
    async fn open_pipe_wire_remote(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        _options: HashMap<&str, Value<'_>>,
    ) -> zbus::fdo::Result<OwnedFd> {
        deny_unless_owner(&header, &self.owner, "OpenPipeWireRemote")?;
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
    /// The session owner's unique bus name; see [`SessionInterface`].
    owner: String,
    /// Shared stream state.
    streams: Arc<RwLock<HashMap<String, StreamState>>>,
}

impl StreamInterface {
    fn new(
        stream_path: String,
        owner: String,
        streams: Arc<RwLock<HashMap<String, StreamState>>>,
    ) -> Self {
        Self {
            stream_path,
            owner,
            streams,
        }
    }
}

#[interface(name = "org.otto.ScreenCast.Stream")]
impl StreamInterface {
    /// Starts this individual stream.
    async fn start(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        deny_unless_owner(&header, &self.owner, "Stream.Start")?;
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
    async fn stop(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        deny_unless_owner(&header, &self.owner, "Stream.Stop")?;
        debug!(stream = %self.stream_path, "Stopping stream");

        let mut streams = self.streams.write().await;
        if let Some(stream) = streams.get_mut(&self.stream_path) {
            stream.started = false;
        }

        Ok(())
    }

    /// Returns PipeWire node metadata including `node-id`.
    async fn pipe_wire_node(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        deny_unless_owner(&header, &self.owner, "PipeWireNode")?;
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
    async fn metadata(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        deny_unless_owner(&header, &self.owner, "Metadata")?;
        let streams = self.streams.read().await;
        let stream = streams
            .get(&self.stream_path)
            .ok_or_else(|| zbus::fdo::Error::Failed("Stream not found".to_string()))?;

        let mut result = HashMap::new();
        // `source-type` mirrors the portal's SOURCE_TYPE_* bits so the portal
        // can build the right stream properties without tracking targets itself.
        match &stream.target {
            StreamTarget::Output(connector) => {
                result.insert(
                    "connector".to_string(),
                    Value::from(connector.as_str()).try_into().unwrap(),
                );
                result.insert("source-type".to_string(), OwnedValue::from(1u32));
            }
            StreamTarget::Window(window_id) => {
                result.insert(
                    "window-id".to_string(),
                    Value::from(window_id.as_str()).try_into().unwrap(),
                );
                result.insert("source-type".to_string(), OwnedValue::from(2u32));
            }
        }
        result.insert("width".to_string(), OwnedValue::from(stream.width));
        result.insert("height".to_string(), OwnedValue::from(stream.height));
        result.insert(
            "cursor-mode".to_string(),
            OwnedValue::from(stream.cursor_mode),
        );

        Ok(result)
    }
}

/// Compositor D-Bus interface for health checks, app management and agent
/// seats.
pub struct CompositorInterface {
    compositor_tx: Sender<CompositorCommand>,
    /// Bus names holding agent seats whose departure is being watched for.
    watched_agents: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl CompositorInterface {
    fn new(compositor_tx: Sender<CompositorCommand>) -> Self {
        Self {
            compositor_tx,
            watched_agents: Arc::default(),
        }
    }

    /// Remove `owner`'s seats once its name leaves the bus — a crashed agent
    /// cannot release them itself.
    async fn watch_agent(&self, connection: &Connection, owner: String) -> zbus::Result<()> {
        if !self.watched_agents.lock().unwrap().insert(owner.clone()) {
            return Ok(());
        }
        let dbus = zbus::fdo::DBusProxy::new(connection).await?;
        let mut changes = dbus
            .receive_name_owner_changed_with_args(&[(0, owner.as_str())])
            .await?;
        let compositor_tx = self.compositor_tx.clone();
        let watched = self.watched_agents.clone();
        // Subscribed before this check, so a name that goes in between is
        // still seen: either here, or as a change.
        let gone_already = !dbus
            .name_has_owner(owner.as_str().try_into()?)
            .await
            .unwrap_or(false);
        tokio::spawn(async move {
            use zbus::export::futures_util::StreamExt;
            if !gone_already {
                while let Some(change) = changes.next().await {
                    if change.args().is_ok_and(|args| args.new_owner().is_none()) {
                        break;
                    }
                }
            }
            info!(owner, "Agent left the bus; removing its seats");
            watched.lock().unwrap().remove(&owner);
            let _ = compositor_tx.send(CompositorCommand::ReleaseAgentSeats {
                owner,
                response_tx: None,
            });
        });
        Ok(())
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
    /// Sends a focus command to the compositor for the given app_id.
    /// Returns true if the command was dispatched (not whether a window was found).
    async fn focus_app(&self, app_id: &str) -> zbus::fdo::Result<bool> {
        info!(app_id, "focus_app requested via D-Bus");
        self.compositor_tx
            .send(CompositorCommand::FocusApp {
                app_id: app_id.to_string(),
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        Ok(true)
    }

    /// Give the calling agent a seat of its own: a pointer, keyboard and
    /// cursor, beside the user's. `name` is shown next to its cursor.
    ///
    /// Returns the `wl_seat` name to create virtual input on, and the
    /// cursor's colour as `#RRGGBB`. The seat is removed when the caller
    /// calls `ReleaseAgentSeat` or leaves the bus; asking again under the
    /// same name later in the session gives the same seat name and colour.
    async fn request_agent_seat(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
        name: &str,
    ) -> zbus::fdo::Result<(String, String)> {
        let owner = sender_of(&header)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::RequestAgentSeat {
                agent_name: name.to_string(),
                owner: owner.clone(),
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        let granted = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))?
            .map_err(zbus::fdo::Error::AccessDenied)?;
        if let Err(err) = self.watch_agent(connection, owner.clone()).await {
            warn!(owner, "Cannot watch the agent's bus name: {err}");
        }
        Ok((granted.seat, granted.color))
    }

    /// Give the calling agent a workspace of its own, on which its seat
    /// acts. The workspace is new, named after the agent, and not switched
    /// to: the user goes there when they want to watch. The agent's input
    /// reaches that workspace's windows whether it is on screen or not.
    ///
    /// Returns the output it is on and that output's logical geometry and
    /// scale: `(output, x, y, width, height, scale)`. Absolute pointer
    /// motion addresses that rectangle.
    async fn request_own_workspace(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<(String, i32, i32, i32, i32, f64)> {
        let owner = sender_of(&header)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::RequestOwnWorkspace {
                owner,
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        let workspace = rx
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))?
            .map_err(zbus::fdo::Error::AccessDenied)?;
        Ok((
            workspace.output,
            workspace.x,
            workspace.y,
            workspace.width,
            workspace.height,
            workspace.scale,
        ))
    }

    /// Start a program for the calling agent: `argv[0]` with the rest as
    /// its arguments, in Otto's session environment. Its windows open on
    /// the agent's own workspace, and take the agent's keyboard, not the
    /// user's. Returns its process id.
    async fn launch_on_own_workspace(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        argv: Vec<String>,
    ) -> zbus::fdo::Result<u32> {
        let owner = sender_of(&header)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::LaunchOnOwnWorkspace {
                owner,
                argv,
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        rx.await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))?
            .map_err(zbus::fdo::Error::AccessDenied)
    }

    /// End the caller's own-workspace grant. The workspace and its windows
    /// stay, for the user. Returns whether there was one.
    async fn release_own_workspace(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<bool> {
        let owner = sender_of(&header)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ReleaseOwnWorkspace {
                owner,
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        rx.await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))
    }

    /// Capture a workspace to a PNG, whether it is on screen or not: its
    /// wallpaper and windows, at its output's resolution. `workspace` is
    /// its id as `org.otto.Shell1.GetWorkspaces` lists it, or its name (any
    /// case). Returns the path of the PNG, under
    /// `$XDG_RUNTIME_DIR/otto/captures`.
    async fn capture_workspace(&self, workspace: &str) -> zbus::fdo::Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::CaptureWorkspace {
                workspace: workspace.to_string(),
                response_tx: tx,
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        rx.await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))?
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Give back every seat the caller holds. Returns whether it held any.
    async fn release_agent_seat(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<bool> {
        let owner = sender_of(&header)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.compositor_tx
            .send(CompositorCommand::ReleaseAgentSeats {
                owner,
                response_tx: Some(tx),
            })
            .map_err(|e| zbus::fdo::Error::Failed(format!("channel send failed: {e}")))?;
        rx.await
            .map_err(|e| zbus::fdo::Error::Failed(format!("no answer: {e}")))
    }
}

/// A session, and its streams, answer only the connection that created it.
fn deny_unless_owner(
    header: &zbus::message::Header<'_>,
    owner: &str,
    what: &str,
) -> zbus::fdo::Result<()> {
    if header
        .sender()
        .is_some_and(|sender| sender.as_str() == owner)
    {
        return Ok(());
    }
    warn!(what, sender = ?header.sender(), owner, "Refusing a screencast call from another connection");
    Err(zbus::fdo::Error::AccessDenied(format!(
        "{what}: this screencast session belongs to another client"
    )))
}

/// The unique bus name a call came from.
fn sender_of(header: &zbus::message::Header<'_>) -> zbus::fdo::Result<String> {
    header
        .sender()
        .map(|sender| sender.to_string())
        .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))
}

/// Starts the D-Bus service on the session bus.
///
/// `a11y` is present only for a session that owns the screen and has
/// accessibility enabled; see [`crate::a11y`].
pub async fn run_dbus_service(
    compositor_tx: Sender<CompositorCommand>,
    a11y: Option<crate::a11y::A11yDbusParts>,
) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let settings_tx = compositor_tx.clone();
    let shell_tx = compositor_tx.clone();

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
    crate::settings_service::register_settings_interface(&connection, settings_tx).await?;

    // The scripting surface: the command language, the tree, and the two
    // events a status bar watches (docs/developer/shell-dbus-api.md).
    crate::shell_service::register_shell_interface(&connection, shell_tx).await?;

    // Accessibility: assistive technologies watch and grab keys through this.
    // The well-known name comes last, so an AT that sees the name appear finds
    // the interface already there.
    if let Some(a11y) = a11y {
        match crate::a11y::keyboard_monitor::register(&connection, a11y.keyboard, a11y.key_events)
            .await
        {
            Ok(()) => match connection
                .request_name(crate::a11y::keyboard_monitor::BUS_NAME)
                .await
            {
                Ok(()) => {
                    // The pointer half of the same object. at-spi2-core builds
                    // one device from both interfaces, so a manager that
                    // serves only the keyboard is not one Orca can use.
                    if let Err(err) = crate::a11y::pointer_locator::register(
                        &connection,
                        a11y.pointer,
                        a11y.pointer_moves,
                    )
                    .await
                    {
                        tracing::warn!("Could not register the a11y pointer locator: {err}");
                    }
                    info!("Accessibility manager started at org.freedesktop.a11y.Manager")
                }
                // Another compositor, or a stale one, already owns it. Otto
                // keeps running; only assistive technologies are affected.
                Err(err) => tracing::warn!("Could not own the a11y manager name: {err}"),
            },
            Err(err) => tracing::warn!("Could not register the a11y manager: {err}"),
        }
    }

    info!("D-Bus service started at org.otto.ScreenCast");

    // Keep the service running
    std::future::pending::<()>().await;

    Ok(())
}
