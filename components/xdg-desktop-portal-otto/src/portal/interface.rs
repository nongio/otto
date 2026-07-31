//! D-Bus interface implementation for `org.freedesktop.impl.portal.ScreenCast`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_io::Timer;
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use zbus::fdo;
use zbus::interface;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedFd, OwnedObjectPath, OwnedValue};

use crate::otto_client::screencast::WindowSource;
use crate::otto_client::OttoClient;
use crate::portal::{
    build_streams_value_from_descriptors, make_output_mapping_id, PortalState, Request,
    SelectedWindow, Session, SessionState, StreamDescriptor, CURSOR_MODE_EMBEDDED,
    SOURCE_TYPE_MONITOR, SOURCE_TYPE_WINDOW, SUPPORTED_CURSOR_MODES,
};
use zbus::zvariant::Str;

/// Maximum attempts when polling for PipeWire node ID.
const NODE_ID_MAX_RETRIES: u32 = 100;
/// Delay between retries when polling for PipeWire node ID.
const NODE_ID_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Creates a fallback mapping ID when output name is empty or unavailable.
pub fn fallback_mapping_id(output: &str) -> String {
    if output.is_empty() {
        "screencomposer:output-default".to_string()
    } else {
        make_output_mapping_id(output)
    }
}

/// Validates that the cursor mode is supported.
pub fn validate_cursor_mode(mode: u32) -> Result<u32, fdo::Error> {
    if (SUPPORTED_CURSOR_MODES & mode) != 0 {
        Ok(mode)
    } else {
        Err(fdo::Error::InvalidArgs(format!(
            "Unsupported cursor_mode value {mode}"
        )))
    }
}

/// Validates that the persist mode is within spec range (0-2).
pub fn validate_persist_mode(mode: u32) -> Result<u32, fdo::Error> {
    match mode {
        0..=2 => Ok(mode),
        other => Err(fdo::Error::InvalidArgs(format!(
            "Unsupported persist_mode value {other}"
        ))),
    }
}

/// Reads the user's preferred screencast output from
/// `$XDG_CONFIG_HOME/otto/screencast-output` (one connector name, e.g.
/// `virtual-1`). Read per SelectSources call so it can be changed between
/// sessions without restarting the portal. Stopgap until a proper source
/// picker exists.
fn preferred_output_override() -> Option<String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    let content = std::fs::read_to_string(base.join("otto/screencast-output")).ok()?;
    let name = content.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Best-effort human-readable name for a requesting app, for the dialog copy.
fn display_app_name(app_id: &str) -> String {
    match app_id.rsplit('.').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "An application".to_string(),
    }
}

/// Picks the output to capture when no picker dialog is reachable: the
/// `screencast-output` override if it names a present output, else the first.
fn fallback_output(available: &[String]) -> Option<String> {
    if available.is_empty() {
        return None;
    }
    match preferred_output_override() {
        Some(preferred) if available.contains(&preferred) => {
            info!(output = %preferred, "Using screencast-output override");
            Some(preferred)
        }
        Some(preferred) => {
            warn!(
                output = %preferred,
                ?available,
                "screencast-output override not among available outputs; using first"
            );
            available.first().cloned()
        }
        None => available.first().cloned(),
    }
}

/// What the user picked in the source dialog.
#[derive(Clone, Debug)]
pub enum SourceSelection {
    Monitor(String),
    Window(WindowSource),
}

#[derive(Clone)]
pub struct ScreenCastPortal {
    state: Arc<Mutex<PortalState>>,
    sc_client: Arc<OttoClient>,
}

impl ScreenCastPortal {
    pub fn new(sc_client: OttoClient) -> Self {
        Self {
            state: Arc::new(Mutex::new(PortalState::default())),
            sc_client: Arc::new(sc_client),
        }
    }

    /// Ask the user which source to share.
    ///
    /// Monitors and windows are offered as one radio list — the portal spec
    /// lets an app request both types, and the user shouldn't have to care
    /// which bit was set. Returns `Ok(None)` if the user dismissed the dialog,
    /// and `Err` only when no dialog renderer answered (see the caller's
    /// fallback).
    async fn pick_source(
        &self,
        app_id: &str,
        outputs: &[String],
        windows: &[WindowSource],
    ) -> zbus::Result<Option<SourceSelection>> {
        // Option ids are prefixed so one flat list can carry both kinds.
        let mut options: Vec<(String, String, String)> = Vec::new();

        for connector in outputs {
            let label = if outputs.len() == 1 {
                "Entire screen".to_string()
            } else {
                format!("Screen — {connector}")
            };
            options.push((
                format!("monitor:{connector}"),
                label,
                "video-display".into(),
            ));
        }

        for window in windows {
            let label = if window.title.is_empty() {
                window.app_id.clone()
            } else {
                window.title.clone()
            };
            options.push((
                format!("window:{}", window.id),
                label,
                window.app_id.clone(),
            ));
        }

        let Some((default_id, _, _)) = options.first().cloned() else {
            return Ok(None);
        };

        let proxy = self.sc_client.dialog_proxy().await?;
        let (response, results) = proxy
            .present_access(
                app_id,
                "Share your screen",
                &format!("{} wants to share your screen", display_app_name(app_id)),
                "The selected source will be visible to the application.",
                "video-display",
                "Share",
                "Cancel",
                true,
                vec![(
                    "source".to_string(),
                    "Choose what to share".to_string(),
                    options,
                    default_id,
                )],
            )
            .await?;

        if response != 0 {
            return Ok(None);
        }

        let Some((_, choice)) = results.into_iter().find(|(group, _)| group == "source") else {
            warn!("Picker returned no selection for the source group");
            return Ok(None);
        };

        Ok(match choice.split_once(':') {
            Some(("monitor", connector)) => Some(SourceSelection::Monitor(connector.to_string())),
            Some(("window", id)) => windows
                .iter()
                .find(|w| w.id == id)
                .cloned()
                .map(SourceSelection::Window),
            _ => {
                warn!(%choice, "Picker returned an unrecognised option id");
                None
            }
        })
    }

    /// Export a temporary Request object in dbus so the frontend can listen for the response signal.
    async fn register_request(
        &self,
        object_server: &ObjectServer,
        path: &OwnedObjectPath,
    ) -> fdo::Result<()> {
        object_server
            .at(path.clone(), Request::new(path.clone()))
            .await
            .map(|_| ())
            .map_err(|err| fdo::Error::Failed(err.to_string()))
    }

    async fn unregister_request(&self, object_server: &ObjectServer, path: &OwnedObjectPath) {
        if let Err(err) = object_server.remove::<Request, _>(path).await {
            warn!(request = %path.as_str(), ?err, "Failed to unregister request object");
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCastPortal {
    async fn create_session(
        &self,
        request_handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] object_server: &ObjectServer,
    ) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        info!(?app_id, ?options, "CreateSession called");

        self.register_request(object_server, &request_handle)
            .await?;

        let result = async {
            info!(session = %session_handle, "Registering session object");

            object_server
                .at(
                    session_handle.clone(),
                    Session::new(
                        session_handle.clone(),
                        self.sc_client.clone(),
                        self.state.clone(),
                    ),
                )
                .await
                .map_err(|err| fdo::Error::Failed(format!("Failed to export Session: {err}")))?;

            let default_cursor_mode = CURSOR_MODE_EMBEDDED;
            let sc_session_path = self
                .sc_client
                .create_session(default_cursor_mode)
                .await
                .map_err(|err| {
                    fdo::Error::Failed(format!("Failed to create ScreenComposer session: {err}"))
                })?;

            let sc_session_obj_path = OwnedObjectPath::try_from(sc_session_path.clone())
                .map_err(|err| fdo::Error::Failed(format!("Invalid session path: {err}")))?;

            {
                let mut state = self.state.lock().await;
                state.sessions.insert(
                    session_handle.to_string(),
                    SessionState {
                        sc_session: sc_session_obj_path.clone(),
                        selected_outputs: Vec::new(),
                        selected_window: None,
                        cursor_mode: default_cursor_mode,
                        persist_mode: None,
                        next_stream_id: 0,
                    },
                );
            }

            info!(
                portal_session = %session_handle,
                sc_session = %sc_session_path,
                "Created ScreenComposer session"
            );

            // Return session_handle in response as per XDG Desktop Portal spec
            let mut results = HashMap::new();
            results.insert(
                "session_id".to_string(),
                OwnedValue::from(Str::from(session_handle.to_string())),
            );

            Ok((0, results))
        }
        .await;

        self.unregister_request(object_server, &request_handle)
            .await;

        result
    }

    async fn select_sources(
        &self,
        request_handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] object_server: &ObjectServer,
    ) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        info!(session = %session_handle, ?app_id, ?options, "SelectSources called");

        self.register_request(object_server, &request_handle)
            .await?;

        let result = async {
            let requested_types = options
                .get("types")
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(SOURCE_TYPE_MONITOR);

            if requested_types & (SOURCE_TYPE_MONITOR | SOURCE_TYPE_WINDOW) == 0 {
                warn!(
                    session = %session_handle,
                    requested_types,
                    "Application requested unsupported source type"
                );
                return Ok((2, HashMap::new()));
            }

            // Get cursor_mode from options. If unsupported, fall back to EMBEDDED.
            let cursor_mode = options
                .get("cursor_mode")
                .and_then(|value| u32::try_from(value).ok())
                .and_then(|mode| validate_cursor_mode(mode).ok())
                .unwrap_or(CURSOR_MODE_EMBEDDED);

            let persist_mode = options
                .get("persist_mode")
                .and_then(|value| u32::try_from(value).ok())
                .map(validate_persist_mode)
                .transpose()?;

            let multiple = options
                .get("multiple")
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(false);

            let monitors_allowed = requested_types & SOURCE_TYPE_MONITOR != 0;
            let windows_allowed = requested_types & SOURCE_TYPE_WINDOW != 0;

            let available_outputs = if monitors_allowed {
                self.sc_client.list_outputs().await.map_err(|err| {
                    error!(session = %session_handle, ?err, "Failed to enumerate outputs");
                    fdo::Error::Failed(format!("Failed to enumerate outputs: {err}"))
                })?
            } else {
                Vec::new()
            };

            // A compositor too old to know ListWindows just yields no windows;
            // the monitor half of the picker still works.
            let available_windows = if windows_allowed {
                self.sc_client.list_windows().await.unwrap_or_else(|err| {
                    warn!(session = %session_handle, ?err, "Failed to enumerate windows");
                    Vec::new()
                })
            } else {
                Vec::new()
            };

            info!(
                session = %session_handle,
                outputs = available_outputs.len(),
                windows = available_windows.len(),
                "Received sources from compositor"
            );

            if available_outputs.is_empty() && available_windows.is_empty() {
                warn!(session = %session_handle, "No sources available for screencast");
                return Ok((3, HashMap::new()));
            }

            if multiple {
                info!(
                    session = %session_handle,
                    "Multiple selection requested; limiting to a single source"
                );
            }

            let selection = match self
                .pick_source(&app_id, &available_outputs, &available_windows)
                .await
            {
                Ok(Some(selection)) => selection,
                // User dismissed the picker.
                Ok(None) => {
                    info!(session = %session_handle, "User cancelled source selection");
                    return Ok((1, HashMap::new()));
                }
                // No dialog renderer on the bus. Monitor capture keeps its
                // pre-picker behaviour so an islands-less session still works;
                // a window is never picked on the user's behalf.
                Err(err) => {
                    warn!(session = %session_handle, ?err, "Source picker unavailable");
                    let Some(fallback) = fallback_output(&available_outputs) else {
                        return Ok((2, HashMap::new()));
                    };
                    SourceSelection::Monitor(fallback)
                }
            };

            {
                let mut state = self.state.lock().await;
                let entry = state
                    .sessions
                    .get_mut(session_handle.as_str())
                    .ok_or_else(|| fdo::Error::Failed("Session not found".to_string()))?;

                match &selection {
                    SourceSelection::Monitor(connector) => {
                        entry.selected_outputs = vec![connector.clone()];
                        entry.selected_window = None;
                    }
                    SourceSelection::Window(window) => {
                        entry.selected_outputs = Vec::new();
                        entry.selected_window = Some(SelectedWindow {
                            id: window.id.clone(),
                            title: window.title.clone(),
                        });
                    }
                }
                entry.cursor_mode = cursor_mode;
                entry.persist_mode = persist_mode;
                entry.next_stream_id = 0;
            }

            info!(
                session = %session_handle,
                selection = ?selection,
                cursor_mode,
                persist_mode = ?persist_mode,
                "Stored source selection"
            );

            let mut results = HashMap::new();
            if let Some(pm) = persist_mode {
                results.insert("persist_mode".to_string(), OwnedValue::from(pm));
            }

            Ok((0, results))
        }
        .await;

        self.unregister_request(object_server, &request_handle)
            .await;

        result
    }

    async fn start(
        &self,
        request_handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        parent_window: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] object_server: &ObjectServer,
    ) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        info!(session = %session_handle, ?app_id, parent_window, ?options, "Start called");

        self.register_request(object_server, &request_handle)
            .await?;

        let result = async {
            let (sc_session_path, selected_source, cursor_mode, stream_index, persist_mode) = {
                let mut state = self.state.lock().await;
                let entry = state
                    .sessions
                    .get_mut(session_handle.as_str())
                    .ok_or_else(|| fdo::Error::Failed("Session not found".to_string()))?;

                let selected_source = match (&entry.selected_window, entry.selected_outputs.first())
                {
                    (Some(window), _) => SourceSelection::Window(WindowSource {
                        id: window.id.clone(),
                        app_id: String::new(),
                        title: window.title.clone(),
                    }),
                    (None, Some(connector)) => SourceSelection::Monitor(connector.clone()),
                    (None, None) => {
                        return Err(fdo::Error::Failed(
                            "No source selected for session".to_string(),
                        ))
                    }
                };

                entry.next_stream_id += 1;

                (
                    entry.sc_session.clone(),
                    selected_source,
                    entry.cursor_mode,
                    entry.next_stream_id,
                    entry.persist_mode,
                )
            };

            let stream_identifier = format!("screen-{stream_index}");

            // Name kept for the mapping-id fallback below, which is
            // output-shaped; a window stream carries its own mapping id.
            let selected_output = match &selected_source {
                SourceSelection::Monitor(connector) => connector.clone(),
                SourceSelection::Window(window) => window.id.clone(),
            };

            info!(
                sc_session = %sc_session_path,
                source = ?selected_source,
                cursor_mode,
                "Recording selected source"
            );

            let sc_stream_path = match &selected_source {
                SourceSelection::Monitor(connector) => self
                    .sc_client
                    .record_monitor(&sc_session_path, connector.as_str(), cursor_mode)
                    .await
                    .map_err(|err| {
                        fdo::Error::Failed(format!("Failed to record monitor '{connector}': {err}"))
                    })?,
                SourceSelection::Window(window) => self
                    .sc_client
                    .record_window(&sc_session_path, window.id.as_str(), cursor_mode)
                    .await
                    .map_err(|err| {
                        fdo::Error::Failed(format!(
                            "Failed to record window '{}': {err}",
                            window.title
                        ))
                    })?,
            };

            info!(sc_stream = %sc_stream_path, "Got stream path, starting session");

            self.sc_client
                .start_session(&sc_session_path)
                .await
                .map_err(|err| fdo::Error::Failed(format!("Failed to start session: {err}")))?;

            let stream_metadata = self
                .sc_client
                .get_stream_metadata(&sc_stream_path)
                .await
                .map_err(|err| {
                    fdo::Error::Failed(format!("Failed to get stream metadata: {err}"))
                })?;

            let mapping_id = stream_metadata
                .get("mapping-id")
                .and_then(|value| value.try_clone().ok())
                .and_then(|owned| TryInto::<String>::try_into(owned).ok())
                .unwrap_or_else(|| fallback_mapping_id(&selected_output));

            let logical_position = stream_metadata
                .get("position")
                .and_then(|value| value.try_clone().ok())
                .and_then(|owned| TryInto::<(i32, i32)>::try_into(owned).ok());

            let logical_size = stream_metadata
                .get("size")
                .and_then(|value| value.try_clone().ok())
                .and_then(|owned| TryInto::<(i32, i32)>::try_into(owned).ok())
                .map(|(w, h)| (w.max(0) as u32, h.max(0) as u32));

            let mut pipewire_node_id = None;

            for attempt in 0..NODE_ID_MAX_RETRIES {
                match self.sc_client.get_pipewire_node_id(&sc_stream_path).await {
                    Ok(id) if id != u32::MAX => {
                        pipewire_node_id = Some(id);
                        info!(
                            node_id = id,
                            attempts = attempt + 1,
                            "Got valid PipeWire node ID"
                        );
                        break;
                    }
                    Ok(_) => {} // Invalid node ID, retry
                    Err(err) => {
                        warn!(attempt = attempt + 1, ?err, "Failed to query node ID");
                    }
                }
                Timer::after(NODE_ID_RETRY_DELAY).await;
            }

            let pipewire_node_id = pipewire_node_id.ok_or_else(|| {
                fdo::Error::Failed("PipeWire node ID not available after retries".to_string())
            })?;

            info!(
                node_id = pipewire_node_id,
                "Stream started with PipeWire node"
            );

            let node_info = self
                .sc_client
                .get_pipewire_node(&sc_stream_path)
                .await
                .map_err(|err| fdo::Error::Failed(format!("Failed to get node info: {err}")))?;

            let width = node_info
                .get("format-width")
                .and_then(|v| <u32>::try_from(v).ok())
                .or_else(|| logical_size.map(|(w, _)| w));
            let height = node_info
                .get("format-height")
                .and_then(|v| <u32>::try_from(v).ok())
                .or_else(|| logical_size.map(|(_, h)| h));

            let position = logical_position;
            let scale_factor = stream_metadata
                .get("scale-factor")
                .and_then(|value| value.try_clone().ok())
                .and_then(|owned| TryInto::<f64>::try_into(owned).ok());
            let refresh_millihz = node_info
                .get("format-refresh-millihz")
                .and_then(|value| <u32>::try_from(value).ok());
            let stride = node_info
                .get("format-stride")
                .and_then(|value| <u32>::try_from(value).ok());
            let fourcc = node_info
                .get("format-fourcc")
                .and_then(|value| <u32>::try_from(value).ok());
            let modifier = node_info
                .get("format-modifier")
                .and_then(|value| <u64>::try_from(value).ok());
            let buffer_kind = node_info
                .get("format-buffer-kind")
                .and_then(|value| value.try_clone().ok())
                .and_then(|owned| String::try_from(owned).ok());

            let descriptor = StreamDescriptor {
                node_id: pipewire_node_id,
                stream_id: stream_identifier.clone(),
                mapping_id: Some(mapping_id.clone()),
                width,
                height,
                position,
                scale_factor,
                refresh_millihz,
                stride,
                fourcc,
                modifier,
                buffer_kind,
                source_type: match &selected_source {
                    SourceSelection::Monitor(_) => SOURCE_TYPE_MONITOR,
                    SourceSelection::Window(_) => SOURCE_TYPE_WINDOW,
                },
            };

            let streams_value =
                build_streams_value_from_descriptors(&[descriptor]).map_err(|err| {
                    fdo::Error::Failed(format!("Failed to encode stream metadata: {err}"))
                })?;

            let mut results = HashMap::new();
            results.insert("streams".to_string(), streams_value);

            if let Some(pm) = persist_mode {
                results.insert("persist_mode".to_string(), OwnedValue::from(pm));
            }

            Ok((0, results))
        }
        .await;

        self.unregister_request(object_server, &request_handle)
            .await;

        result
    }

    async fn open_pipe_wire_remote(
        &self,
        session: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] object_server: &ObjectServer,
    ) -> fdo::Result<OwnedFd> {
        info!(session = %session, ?options, "OpenPipeWireRemote called");

        if object_server
            .interface::<_, Session>(&session)
            .await
            .is_err()
        {
            warn!(session = %session, "OpenPipeWireRemote called with unknown session");
            return Err(fdo::Error::InvalidArgs("Unknown session".to_string()));
        }

        let sc_session_path = {
            let state = self.state.lock().await;
            state
                .sessions
                .get(session.as_str())
                .ok_or_else(|| fdo::Error::Failed("Session not found".to_string()))?
                .sc_session
                .clone()
        };

        info!(sc_session = %sc_session_path, "Getting PipeWire remote FD from ScreenComposer session");

        self.sc_client
            .open_pipewire_remote(&sc_session_path)
            .await
            .map_err(|err| fdo::Error::Failed(format!("Failed to open PipeWire remote: {}", err)))
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        SOURCE_TYPE_MONITOR
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        SUPPORTED_CURSOR_MODES
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        5
    }
}
