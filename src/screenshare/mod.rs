//! Screenshare infrastructure for the compositor.
//!
//! This module provides the compositor-side support for screen casting via the
//! xdg-desktop-portal protocol. It exposes a D-Bus service that the portal
//! backend (`xdg-desktop-portal-otto`) communicates with to:
//!
//! - Enumerate available outputs
//! - Create screencast sessions
//! - Start/stop recording
//! - Provide PipeWire file descriptors for video streams
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  org.otto.ScreenCast (D-Bus)                      │
//! │       │                                                     │
//! │       ▼                                                     │
//! │  FrameTapManager ← receives frames from render loop         │
//! │       │                                                     │
//! │       ▼                                                     │
//! │  ScreencastSessionTap → PipeWire stream                     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Threading Model
//!
//! The compositor uses calloop for its main event loop (synchronous, ~16ms
//! dispatch). The D-Bus service requires async (zbus/tokio). We bridge these:
//!
//! - D-Bus server runs on a dedicated tokio runtime thread
//! - Commands flow from D-Bus → compositor via `calloop::channel`
//! - Responses flow from compositor → D-Bus via `tokio::sync::mpsc`
//!

use std::collections::HashMap;

mod dbus_service;
mod pipewire_stream;

pub use dbus_service::run_dbus_service;

pub use pipewire_stream::{AvailableBuffer, BackendCapabilities, PipeWireStream, StreamConfig};

use smithay::reexports::calloop::channel::{
    channel, Event as ChannelEvent, Sender as ChannelSender,
};
use zbus::zvariant::OwnedFd;

use crate::renderer::BlitCurrentFrame;

/// Cursor mode values, matching the xdg-desktop-portal ScreenCast bitmask —
/// the portal forwards its own value verbatim as the `cursor-mode` property.
pub const CURSOR_MODE_HIDDEN: u32 = 1;
/// The cursor is drawn into the streamed frames.
pub const CURSOR_MODE_EMBEDDED: u32 = 2;
/// The cursor is sent as PipeWire metadata — not implemented, treated as hidden.
pub const CURSOR_MODE_METADATA: u32 = 4;

/// Active screencast session state (compositor side).
///
/// Tracks all active streams for a D-Bus session.
pub struct ScreencastSession {
    /// The D-Bus session path (e.g., "/org/otto/ScreenCast/session/1").
    pub session_id: String,
    /// Cursor mode for this session (HIDDEN, EMBEDDED, or METADATA).
    pub cursor_mode: u32,
    /// Active streams indexed by output connector name.
    pub streams: HashMap<String, ActiveStream>,
}

/// What a stream captures.
///
/// Monitor capture blits the finished output framebuffer; window capture
/// re-renders one toplevel's surface tree into the PipeWire buffer. The two
/// live in the same session map, keyed by [`StreamTarget::key`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StreamTarget {
    /// An output, by connector name (e.g. "HDMI-A-1").
    Output(String),
    /// A toplevel, by its `ext-foreign-toplevel-list-v1` identifier.
    Window(String),
}

impl StreamTarget {
    /// Map key for `ScreencastSession::streams`. Prefixed so an output and a
    /// window can never collide.
    pub fn key(&self) -> String {
        match self {
            StreamTarget::Output(connector) => format!("output:{connector}"),
            StreamTarget::Window(id) => format!("window:{id}"),
        }
    }
}

/// Active stream for one capture target.
///
/// Contains the PipeWire stream.
pub struct ActiveStream {
    /// What this stream captures.
    pub target: StreamTarget,
    /// PipeWire stream instance.
    pub pipewire_stream: PipeWireStream,
    /// Frame rendered last cycle, awaiting its GPU fence before the dmabuf is
    /// queued to PipeWire. Resolved (non-blocking) at the next render tick so
    /// the screenshare fence wait never blocks the main loop / input dispatch.
    pub pending_frame: Option<smithay::backend::renderer::sync::SyncPoint>,
}

/// Commands sent from the D-Bus service to the compositor main loop.
#[derive(Debug)]
pub enum CompositorCommand {
    /// Create a new screencast session.
    CreateSession {
        session_id: String,
        cursor_mode: u32,
    },
    /// List available outputs for screen casting.
    ListOutputs {
        response_tx: tokio::sync::oneshot::Sender<Vec<OutputInfo>>,
    },
    /// List capturable toplevel windows.
    ListWindows {
        response_tx: tokio::sync::oneshot::Sender<Vec<WindowInfo>>,
    },
    /// Start recording a capture target.
    StartRecording {
        session_id: String,
        target: StreamTarget,
        cursor_mode: u32,
        /// Response channel for the PipeWire node ID.
        response_tx: tokio::sync::oneshot::Sender<Result<u32, String>>,
    },
    /// Stop recording a capture target.
    StopRecording {
        session_id: String,
        target: StreamTarget,
    },
    /// Get a PipeWire file descriptor for the session.
    GetPipeWireFd {
        session_id: String,
        response_tx: tokio::sync::oneshot::Sender<Result<OwnedFd, String>>,
    },
    /// Destroy a session.
    DestroySession { session_id: String },
    /// Focus an application by app_id (e.g. from notification click).
    FocusApp { app_id: String },
    /// Change one setting, on the thread that can apply it to the running
    /// system. See `docs/developer/settings-dbus-api.md`.
    SetSetting {
        id: String,
        value: crate::settings::value::SettingValue,
        response_tx: tokio::sync::oneshot::Sender<
            Result<crate::settings::Status, crate::settings::SetError>,
        >,
    },
    /// Drop one setting from the writable config file and re-read the layers.
    ResetSetting {
        id: String,
        response_tx: tokio::sync::oneshot::Sender<
            Result<crate::settings::Status, crate::settings::SetError>,
        >,
    },
    /// Create a virtual output now, without a restart. Answers with the
    /// PipeWire node the new output streams to.
    AddVirtualOutput {
        config: crate::config::VirtualOutputConfig,
        response_tx: tokio::sync::oneshot::Sender<Result<u32, String>>,
    },
    /// Tear a virtual output down and unmap it.
    RemoveVirtualOutput {
        name: String,
        response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

/// Information about an available output.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub connector: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
    /// A PipeWire-backed output rather than a physical one. A client showing
    /// an arrangement needs to tell them apart: only a virtual output can be
    /// removed, and only a physical one has a connector behind it.
    pub is_virtual: bool,
    /// Position in the global compositor layout, in logical points.
    pub x: i32,
    pub y: i32,
    pub scale: f64,
}

/// Information about a capturable window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    /// `ext-foreign-toplevel-list-v1` identifier — the handle the portal
    /// passes back to `RecordWindow`.
    pub id: String,
    pub app_id: String,
    pub title: String,
    /// Window geometry in physical pixels.
    pub width: u32,
    pub height: u32,
}

/// Information about an active stream.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
}

/// Manager for the screenshare subsystem.
///
/// Owns the D-Bus service handle and the channel for communicating with it.
pub struct ScreenshareManager {
    /// Sender for commands from the D-Bus thread.
    pub command_sender: ChannelSender<CompositorCommand>,
}

impl ScreenshareManager {
    /// Start the screenshare D-Bus service.
    ///
    /// This spawns a dedicated tokio runtime thread that runs the zbus server.
    /// Returns a manager that can be stored in the compositor state.
    pub fn start<B: crate::state::Backend + 'static>(
        loop_handle: &smithay::reexports::calloop::LoopHandle<'static, crate::state::Otto<B>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (cmd_sender, cmd_receiver) = channel::<CompositorCommand>();

        // Register the calloop channel to receive commands
        loop_handle
            .insert_source(cmd_receiver, |event, _, state| {
                if let ChannelEvent::Msg(cmd) = event {
                    handle_screenshare_command(state, cmd);
                }
            })
            .map_err(|e| format!("Failed to insert screenshare channel: {}", e))?;

        // Spawn the D-Bus service on a dedicated tokio thread
        let cmd_sender_clone = cmd_sender.clone();
        let _ = std::thread::Builder::new()
            .name("screenshare-dbus".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for screenshare");

                rt.block_on(async move {
                    if let Err(e) = dbus_service::run_dbus_service(cmd_sender_clone).await {
                        tracing::error!("Screenshare D-Bus service failed: {}", e);
                    }
                });
            })?;

        Ok(Self {
            command_sender: cmd_sender,
        })
    }
}

/// Resolve an `ext-foreign-toplevel-list-v1` identifier to a mapped window and
/// the output hosting it.
///
/// Returns `None` if the identifier is unknown or the window is no longer
/// mapped to any output (the portal may hold a stale identifier from before
/// the user answered the picker dialog).
///
/// Takes the two fields it needs rather than `&Otto` so callers inside the
/// render loop — which already hold a mutable borrow of `backend_data` — can
/// still use it under field-disjoint borrows.
#[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
pub fn window_for_identifier(
    workspaces: &crate::workspaces::Workspaces,
    foreign_toplevels: &HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        crate::state::foreign_toplevel_shared::ForeignToplevelHandles,
    >,
    identifier: &str,
) -> Option<(crate::shell::WindowElement, smithay::output::Output)> {
    let window = workspaces.spaces_elements().find(|window| {
        foreign_toplevels
            .get(&window.id())
            .and_then(|handles| handles.identifier())
            .is_some_and(|id| id == identifier)
    })?;
    let output = workspaces.outputs_for_element(window).first().cloned()?;
    Some((window.clone(), output))
}

/// Ids of windows with an active screencast stream.
///
/// Drives [`crate::state::window_throttle::WindowThrottleState::Captured`] so a
/// shared window keeps painting while occluded or on another workspace. Takes
/// individual fields for the same borrow reason as [`window_for_identifier`].
#[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
pub fn screencast_window_ids(
    sessions: &HashMap<String, ScreencastSession>,
    workspaces: &crate::workspaces::Workspaces,
    foreign_toplevels: &HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        crate::state::foreign_toplevel_shared::ForeignToplevelHandles,
    >,
) -> std::collections::HashSet<smithay::reexports::wayland_server::backend::ObjectId> {
    let mut ids = std::collections::HashSet::new();
    for session in sessions.values() {
        for stream in session.streams.values() {
            if let StreamTarget::Window(identifier) = &stream.target {
                if let Some((window, _)) =
                    window_for_identifier(workspaces, foreign_toplevels, identifier)
                {
                    ids.insert(window.id());
                }
            }
        }
    }
    ids
}

/// Whether this window is the target of an active screencast stream.
///
/// Drives the sharing badge on the server-side titlebar. Takes individual
/// fields for the same borrow reason as [`window_for_identifier`], and answers
/// `false` without resolving anything when nothing is being cast — the common
/// case, hit on every commit of a decorated window.
#[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
pub fn is_window_screencast(
    sessions: &HashMap<String, ScreencastSession>,
    workspaces: &crate::workspaces::Workspaces,
    foreign_toplevels: &HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        crate::state::foreign_toplevel_shared::ForeignToplevelHandles,
    >,
    window_id: &smithay::reexports::wayland_server::backend::ObjectId,
) -> bool {
    if sessions.is_empty() {
        return false;
    }
    screencast_window_ids(sessions, workspaces, foreign_toplevels).contains(window_id)
}

/// Push the current capture state onto every decorated window's titlebar.
///
/// The per-window flag is otherwise only recomputed when that window commits,
/// which is fine while a share is running (a captured window paints) but not
/// when one starts or stops: the windows that just gained or lost the badge may
/// be idle. Call this whenever the set of streams changes.
pub fn refresh_sharing_badges<B: crate::state::Backend + 'static>(state: &crate::state::Otto<B>) {
    #[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
    let captured = screencast_window_ids(
        &state.screenshare_sessions,
        &state.workspaces,
        &state.foreign_toplevels,
    );
    for window in state.workspaces.spaces_elements() {
        let Some(view) = state.workspaces.get_window_view(&window.id()) else {
            continue;
        };
        let mut model = view.decoration_state();
        let sharing = captured.contains(&window.id());
        if model.sharing != sharing {
            model.sharing = sharing;
            view.update_decoration(model);
        }
    }
}

/// Handle a command from the D-Bus service.
fn handle_screenshare_command<B: crate::state::Backend + 'static>(
    state: &mut crate::state::Otto<B>,
    cmd: CompositorCommand,
) {
    match cmd {
        CompositorCommand::CreateSession {
            session_id,
            cursor_mode,
        } => {
            tracing::info!("CreateSession: {}, cursor_mode={}", session_id, cursor_mode);

            // Create compositor-side session state
            state.screenshare_sessions.insert(
                session_id.clone(),
                ScreencastSession {
                    session_id,
                    cursor_mode,
                    streams: HashMap::new(),
                },
            );
        }
        CompositorCommand::ListOutputs { response_tx } => {
            tracing::info!("ListOutputs command received");
            let outputs: Vec<OutputInfo> = state
                .workspaces
                .outputs()
                .map(|output| {
                    let (width, height, refresh_rate) = output
                        .current_mode()
                        .map(|m| (m.size.w as u32, m.size.h as u32, m.refresh as u32))
                        .unwrap_or((0, 0, 0));
                    let position = state
                        .workspaces
                        .output_geometry(output)
                        .map(|geometry| geometry.loc)
                        .unwrap_or_default();
                    let info = OutputInfo {
                        is_virtual: crate::virtual_output::is_virtual_output(output),
                        x: position.x,
                        y: position.y,
                        scale: output.current_scale().fractional_scale(),
                        connector: output.name(),
                        name: output.name(),
                        width,
                        height,
                        refresh_rate,
                    };
                    tracing::debug!("Output: {:?}", info);
                    info
                })
                .collect();
            tracing::info!("Returning {} outputs", outputs.len());
            let _ = response_tx.send(outputs);
        }
        CompositorCommand::ListWindows { response_tx } => {
            let windows: Vec<WindowInfo> = state
                .workspaces
                .spaces_elements()
                .filter_map(|window| {
                    // Only toplevels the foreign-toplevel protocol knows about
                    // are addressable by a portal — that identifier is the
                    // whole contract with the picker.
                    let handles = state.foreign_toplevels.get(&window.id())?;
                    let id = handles.identifier()?;
                    let output = state
                        .workspaces
                        .outputs_for_element(window)
                        .first()
                        .cloned();
                    let scale = output
                        .map(|o| o.current_scale().fractional_scale())
                        .unwrap_or(1.0);
                    let size = smithay::desktop::space::SpaceElement::geometry(window).size;
                    Some(WindowInfo {
                        id,
                        app_id: handles.app_id(),
                        title: handles.title(),
                        width: ((size.w as f64) * scale).round().max(0.0) as u32,
                        height: ((size.h as f64) * scale).round().max(0.0) as u32,
                    })
                })
                .collect();
            tracing::info!("Returning {} windows", windows.len());
            let _ = response_tx.send(windows);
        }
        CompositorCommand::StartRecording {
            session_id,
            target,
            cursor_mode,
            response_tx,
        } => {
            tracing::debug!(
                "StartRecording: session={}, target={:?}, cursor_mode={}",
                session_id,
                target,
                cursor_mode
            );

            // Resolve the target to a capture size, plus the output whose
            // refresh rate paces the stream. A window is captured at its
            // current size; later resizes are letterboxed into these fixed
            // dimensions rather than renegotiating the PipeWire format.
            let resolved = match &target {
                StreamTarget::Output(connector) => state
                    .workspaces
                    .outputs()
                    .find(|o| o.name() == *connector)
                    .cloned()
                    .ok_or_else(|| format!("Output not found: {connector}"))
                    .map(|output| {
                        let (w, h, refresh) = output
                            .current_mode()
                            .map(|m| (m.size.w as u32, m.size.h as u32, m.refresh as u32))
                            .unwrap_or((1920, 1080, 60000));
                        (w, h, refresh)
                    }),
                StreamTarget::Window(id) => {
                    window_for_identifier(&state.workspaces, &state.foreign_toplevels, id)
                        .ok_or_else(|| format!("Window not found: {id}"))
                        .and_then(|(window, output)| {
                            let scale = output.current_scale().fractional_scale();
                            let size =
                                smithay::desktop::space::SpaceElement::geometry(&window).size;
                            // Even dimensions: several PipeWire consumers (and
                            // every YUV encoder downstream) reject odd sizes.
                            let w = (((size.w as f64) * scale).round() as u32) & !1;
                            let h = (((size.h as f64) * scale).round() as u32) & !1;
                            if w == 0 || h == 0 {
                                return Err(format!("Window {id} has zero size"));
                            }
                            let refresh = output
                                .current_mode()
                                .map(|m| m.refresh as u32)
                                .unwrap_or(60000);
                            Ok((w, h, refresh))
                        })
                }
            };

            let (width, height, refresh_rate) = match resolved {
                Ok(v) => v,
                Err(e) => {
                    let _ = response_tx.send(Err(e));
                    return;
                }
            };

            let stream_key = target.key();

            // Get the session and update cursor_mode
            let session = match state.screenshare_sessions.get_mut(&session_id) {
                Some(s) => s,
                None => {
                    let _ = response_tx.send(Err(format!("Session not found: {}", session_id)));
                    return;
                }
            };

            // Update cursor mode for this session
            session.cursor_mode = cursor_mode;

            // Check if already recording this target
            if session.streams.contains_key(&stream_key) {
                let _ = response_tx.send(Err(format!("Already recording: {}", stream_key)));
                return;
            }

            // Build backend capabilities
            let gbm_device = state.backend_data.gbm_device();
            let capabilities = if let Some(ref gbm) = gbm_device {
                use smithay::backend::allocator::Fourcc;

                let formats = vec![Fourcc::Argb8888];

                const DRM_FORMAT_MOD_LINEAR: u64 = 0;
                const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

                // Every modifier we advertise must be allocatable AND
                // single-plane: `send_buffer_params`/`add_buffer` describe one
                // plane per buffer, so aux-plane modifiers (Intel CCS) cannot
                // be represented even though EGL reports them.
                let mut modifiers: Vec<i64> = Vec::new();
                let mut probe = |modifier: u64| {
                    if modifiers.contains(&(modifier as i64)) {
                        return;
                    }
                    let ok = gbm
                        .create_buffer_object_with_modifiers2::<()>(
                            width,
                            height,
                            Fourcc::Argb8888,
                            std::iter::once(modifier.into()),
                            smithay::backend::allocator::gbm::GbmBufferFlags::RENDERING,
                        )
                        .map(|bo| bo.plane_count() == 1)
                        .unwrap_or(false);
                    if ok {
                        modifiers.push(modifier as i64);
                    }
                };
                // LINEAR first: keeps existing clients (OBS) negotiating
                // exactly what they did before; tiled modifiers follow for
                // clients whose importer cannot map linear (gst vapostproc).
                probe(DRM_FORMAT_MOD_LINEAR);
                for format in state.backend_data.get_format_modifiers(Fourcc::Argb8888) {
                    if format != DRM_FORMAT_MOD_INVALID {
                        probe(format);
                    }
                }
                tracing::info!("Screenshare dmabuf modifiers offered: {:x?}", modifiers);

                pipewire_stream::BackendCapabilities {
                    supports_dmabuf: !modifiers.is_empty(),
                    formats,
                    modifiers,
                }
            } else {
                // Fallback to SHM
                pipewire_stream::BackendCapabilities::default()
            };

            // Create PipeWire stream
            // TODO: Make screenshare FPS cap configurable (e.g., config.screenshare.max_fps)
            // Chrome/WebRTC don't support >60fps, so we cap here for compatibility
            let framerate_num = (refresh_rate / 1000).min(60); // Cap at 60fps for compatibility

            let config = StreamConfig {
                width,
                height,
                framerate_num,
                framerate_denom: 1,
                gbm_device,
                capabilities,
            };
            let mut pipewire_stream = PipeWireStream::new(config);

            // Start the PipeWire stream synchronously (spawns a thread and connects to PipeWire)
            let node_id = match pipewire_stream.start_sync() {
                Ok(id) => id,
                Err(e) => {
                    let _ =
                        response_tx.send(Err(format!("Failed to start PipeWire stream: {}", e)));
                    return;
                }
            };

            tracing::debug!(
                "PipeWire stream started: session={}, target={}, node_id={}, size={}x{}",
                session_id,
                stream_key,
                node_id,
                width,
                height
            );

            // Store the active stream
            session.streams.insert(
                stream_key,
                ActiveStream {
                    target,
                    pipewire_stream,
                    pending_frame: None,
                },
            );

            refresh_sharing_badges(state);

            // Send success response with node_id
            let _ = response_tx.send(Ok(node_id));
        }
        CompositorCommand::StopRecording { session_id, target } => {
            tracing::debug!("StopRecording: session={}, target={:?}", session_id, target);

            // Get the session
            let session = match state.screenshare_sessions.get_mut(&session_id) {
                Some(s) => s,
                None => {
                    tracing::error!("Session not found: {}", session_id);
                    return;
                }
            };

            // Remove and stop the stream
            let stream_key = target.key();
            if let Some(_stream) = session.streams.remove(&stream_key) {
                tracing::debug!(
                    "Stopped stream for session={}, target={}",
                    session_id,
                    stream_key
                );
                // PipeWire stream will be dropped here
            } else {
                tracing::warn!(
                    "No active stream for {} in session {}",
                    stream_key,
                    session_id
                );
            }
            refresh_sharing_badges(state);
        }
        CompositorCommand::GetPipeWireFd {
            session_id,
            response_tx,
        } => {
            tracing::debug!("GetPipeWireFd: session={}", session_id);
            // TODO: Return actual PipeWire FD once PipeWire integration is complete
            // For now, return an error indicating it's not yet implemented
            let _ = response_tx.send(Err("PipeWire integration not yet complete".into()));
        }
        CompositorCommand::DestroySession { session_id } => {
            tracing::info!("DestroySession: session={}", session_id);

            // Remove the session and clean up all streams
            if let Some(session) = state.screenshare_sessions.remove(&session_id) {
                tracing::debug!(
                    "Destroyed session {} with {} active streams",
                    session_id,
                    session.streams.len()
                );
                // Streams will be dropped here
            } else {
                tracing::warn!("Session not found for destruction: {}", session_id);
            }
            refresh_sharing_badges(state);
        }
        CompositorCommand::FocusApp { app_id } => {
            tracing::info!("FocusApp: {}", app_id);
            state.focus_app(&app_id);
        }
        CompositorCommand::SetSetting {
            id,
            value,
            response_tx,
        } => {
            let _ = response_tx.send(crate::settings::set(state, &id, value));
        }
        CompositorCommand::ResetSetting { id, response_tx } => {
            let _ = response_tx.send(crate::settings::reset(state, &id));
        }
        CompositorCommand::AddVirtualOutput {
            config,
            response_tx,
        } => {
            let _ = response_tx.send(add_virtual_output(state, &config));
        }
        CompositorCommand::RemoveVirtualOutput { name, response_tx } => {
            let _ = response_tx.send(remove_virtual_output(state, &name));
        }
    }
}

/// Bring a virtual output up on the running compositor.
///
/// The same work `udev::init` does at startup, minus the config read — kept
/// here rather than in the backend because `virtual_outputs` lives on `Otto`
/// and both the gbm device and the format modifiers are already on `Backend`,
/// so a backend without dmabuf simply gets a shared-memory stream.
fn add_virtual_output<B: crate::state::Backend + 'static>(
    state: &mut crate::state::Otto<B>,
    config: &crate::config::VirtualOutputConfig,
) -> Result<u32, String> {
    if state
        .virtual_outputs
        .iter()
        .any(|vout| vout.output.name() == config.name)
    {
        return Err(format!("a virtual output named `{}` exists", config.name));
    }
    // A virtual output shares a namespace with the physical ones: two outputs
    // answering to the same name would make every by-name lookup ambiguous.
    if state
        .workspaces
        .outputs()
        .any(|output| output.name() == config.name)
    {
        return Err(format!("`{}` is already an output name", config.name));
    }

    let output = crate::virtual_output::VirtualOutputState::build_output(config);
    let global = output.create_global::<crate::state::Otto<B>>(&state.display_handle);

    let position: smithay::utils::Point<i32, smithay::utils::Logical> = config
        .position
        .map(|p| (p.x, p.y).into())
        .unwrap_or_else(|| (0, 0).into());
    state.workspaces.map_output(&output, position);

    let gbm_device = state.backend_data.gbm_device();
    let format_modifiers = state
        .backend_data
        .get_format_modifiers(smithay::backend::allocator::Fourcc::Argb8888);

    match crate::virtual_output::VirtualOutputState::start(
        output.clone(),
        global,
        config,
        gbm_device,
        format_modifiers,
    ) {
        Ok((vout_state, node_id)) => {
            tracing::info!(
                "Virtual output '{}' started at runtime (PipeWire node {})",
                config.name,
                node_id
            );
            state.virtual_outputs.push(vout_state);
            Ok(node_id)
        }
        Err(e) => {
            // Leave nothing half-created: the output was mapped before the
            // stream could fail, and an output with no stream renders forever
            // into nothing.
            state.workspaces.unmap_output(&output);
            Err(e)
        }
    }
}

/// Take a virtual output back down. Physical outputs are refused rather than
/// silently ignored — unmapping one would black out a real screen.
fn remove_virtual_output<B: crate::state::Backend + 'static>(
    state: &mut crate::state::Otto<B>,
    name: &str,
) -> Result<(), String> {
    let Some(index) = state
        .virtual_outputs
        .iter()
        .position(|vout| vout.output.name() == name)
    else {
        return Err(format!("no virtual output named `{name}`"));
    };

    let vout = state.virtual_outputs.remove(index);
    state.workspaces.unmap_output(&vout.output);
    tracing::info!("Virtual output '{name}' removed");
    // Dropping `vout` closes the PipeWire stream and releases the global.
    Ok(())
}

/// Render render elements into a PipeWire dmabuf, over an opaque black clear.
///
/// This is the window-capture counterpart of [`fullscreen_to_dmabuf`]. Instead
/// of blitting the composited output it re-renders the window's own surface
/// tree, so the capture is unaffected by windows stacked on top of it, by the
/// dock/topbar, or by which workspace is currently on screen.
///
/// Elements are expected to be positioned by the caller so the window's
/// geometry origin lands at (0, 0). Content outside `size` is clipped: a window
/// that grew since recording started is cropped, one that shrank leaves the
/// remainder black.
pub fn window_to_dmabuf<R, E>(
    renderer: &mut R,
    dst_dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    elements: &[E],
    scale: smithay::utils::Scale<f64>,
) -> Result<(), String>
where
    R: smithay::backend::renderer::Renderer
        + smithay::backend::renderer::Bind<smithay::backend::allocator::dmabuf::Dmabuf>,
    E: smithay::backend::renderer::element::RenderElement<R>,
{
    use smithay::backend::renderer::{Color32F, Frame};
    use smithay::utils::{Physical, Rectangle};

    let full = Rectangle::<i32, Physical>::from_size(size);

    let mut dmabuf_fb = renderer
        .bind(dst_dmabuf)
        .map_err(|e| format!("Failed to bind dmabuf: {:?}", e))?;

    let mut frame = renderer
        .render(&mut dmabuf_fb, size, smithay::utils::Transform::Normal)
        .map_err(|e| format!("Failed to create frame: {:?}", e))?;

    // Opaque black: the stream is ARGB, and consumers that ignore alpha would
    // otherwise show whatever was left in the recycled buffer.
    frame
        .clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[full])
        .map_err(|e| format!("Failed to clear: {:?}", e))?;

    // Front-to-back element order is top-most first; draw in reverse so upper
    // elements land on top.
    for element in elements.iter().rev() {
        let src = element.src();
        let dst = element.geometry(scale);
        let Some(mut damage) = full.intersection(dst) else {
            continue;
        };
        damage.loc -= dst.loc;
        element
            .draw(&mut frame, src, dst, &[damage], &[])
            .map_err(|e| format!("Failed to draw element: {:?}", e))?;
    }

    std::mem::drop(frame);

    Ok(())
}

/// Copy compositor framebuffer to PipeWire buffer with cursor rendering
///
/// Blits the current frame to destination dmabuf, then renders cursor elements on top
pub fn fullscreen_to_dmabuf<R, E>(
    renderer: &mut R,
    dst_dmabuf: &mut smithay::backend::allocator::dmabuf::Dmabuf,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    damage: Option<&[smithay::utils::Rectangle<i32, smithay::utils::Physical>]>,
    cursor_elements: &[E],
    scale: smithay::utils::Scale<f64>,
) -> Result<(), String>
where
    R: smithay::backend::renderer::Renderer
        + smithay::backend::renderer::Bind<smithay::backend::allocator::dmabuf::Dmabuf>,
    R: BlitCurrentFrame,
    E: smithay::backend::renderer::element::RenderElement<R>,
{
    use smithay::utils::Physical;
    // Step 1: Blit from current frame to destination dmabuf
    match damage {
        Some(rects) if !rects.is_empty() => {
            for rect in rects {
                renderer
                    .blit_current_frame(dst_dmabuf, *rect, *rect)
                    .map_err(|e| format!("Blit failed: {:?}", e))?;
            }
        }
        _ => {
            let rect = smithay::utils::Rectangle::<i32, Physical>::from_size(size);
            renderer
                .blit_current_frame(dst_dmabuf, rect, rect)
                .map_err(|e| format!("Blit failed: {:?}", e))?;
        }
    }

    // Step 2: Render cursor elements on top of blitted content
    if !cursor_elements.is_empty() {
        // Bind the destination dmabuf to create a frame for rendering cursors
        let mut dmabuf_fb = renderer
            .bind(dst_dmabuf)
            .map_err(|e| format!("Failed to bind dmabuf: {:?}", e))?;

        let mut cursor_frame = renderer
            .render(&mut dmabuf_fb, size, smithay::utils::Transform::Normal)
            .map_err(|e| format!("Failed to create cursor frame: {:?}", e))?;

        // Render each cursor element
        for element in cursor_elements.iter() {
            let src = element.src();
            let dst = element.geometry(scale);

            // Calculate damage rect (entire element area)
            let output_rect = smithay::utils::Rectangle::<i32, Physical>::from_size(size);
            if let Some(mut damage) = output_rect.intersection(dst) {
                damage.loc -= dst.loc;
                element
                    .draw(&mut cursor_frame, src, dst, &[damage], &[])
                    .map_err(|e| format!("Failed to draw cursor element: {:?}", e))?;
            }
        }

        std::mem::drop(cursor_frame);
    }

    Ok(())
}
