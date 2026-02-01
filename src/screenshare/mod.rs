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

/// Active stream for one output.
///
/// Contains the PipeWire stream.
pub struct ActiveStream {
    /// Output connector name (e.g., "HDMI-A-1").
    pub output_connector: String,
    /// PipeWire stream instance.
    pub pipewire_stream: PipeWireStream,
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
    /// Start recording on a specific output.
    StartRecording {
        session_id: String,
        output_connector: String,
        cursor_mode: u32,
        /// Response channel for the PipeWire node ID.
        response_tx: tokio::sync::oneshot::Sender<Result<u32, String>>,
    },
    /// Stop recording on a specific output.
    StopRecording {
        session_id: String,
        output_connector: String,
    },
    /// Get a PipeWire file descriptor for the session.
    GetPipeWireFd {
        session_id: String,
        response_tx: tokio::sync::oneshot::Sender<Result<OwnedFd, String>>,
    },
    /// Destroy a session.
    DestroySession { session_id: String },
}

/// Information about an available output.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub connector: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
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
                    let info = OutputInfo {
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
        CompositorCommand::StartRecording {
            session_id,
            output_connector,
            cursor_mode,
            response_tx,
        } => {
            tracing::debug!(
                "StartRecording: session={}, output={}, cursor_mode={}",
                session_id,
                output_connector,
                cursor_mode
            );

            // Find the output by connector name
            let output = state
                .workspaces
                .outputs()
                .find(|o| o.name() == output_connector);

            let output = match output {
                Some(o) => o.clone(),
                None => {
                    let _ =
                        response_tx.send(Err(format!("Output not found: {}", output_connector)));
                    return;
                }
            };

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

            // Check if already recording this output
            if session.streams.contains_key(&output_connector) {
                let _ = response_tx.send(Err(format!(
                    "Already recording output: {}",
                    output_connector
                )));
                return;
            }

            // Get output dimensions for stream config
            let (width, height, refresh_rate) = output
                .current_mode()
                .map(|m| (m.size.w as u32, m.size.h as u32, m.refresh as u32))
                .unwrap_or((1920, 1080, 60000));

            // Build backend capabilities
            let gbm_device = state.backend_data.gbm_device();
            let capabilities = if let Some(ref _gbm) = gbm_device {
                use smithay::backend::allocator::Fourcc;

                // For now, advertise ARGB8888 with common modifiers
                // In production, we'd query the actual supported formats from the backend
                let formats = vec![Fourcc::Argb8888];

                // Common DRM modifiers - LINEAR and INVALID (for implicit modifier)
                const DRM_FORMAT_MOD_LINEAR: i64 = 0;
                const DRM_FORMAT_MOD_INVALID: i64 = 0x00ffffffffffffff_u64 as i64;
                let modifiers = vec![DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR];

                pipewire_stream::BackendCapabilities {
                    supports_dmabuf: true,
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
                "PipeWire stream started: session={}, output={}, node_id={}",
                session_id,
                output_connector,
                node_id
            );

            tracing::debug!(
                "Started PipeWire stream for session={}, output={}",
                session_id,
                output_connector
            );

            // Store the active stream
            session.streams.insert(
                output_connector.clone(),
                ActiveStream {
                    output_connector,
                    pipewire_stream,
                },
            );

            // Send success response with node_id
            let _ = response_tx.send(Ok(node_id));
        }
        CompositorCommand::StopRecording {
            session_id,
            output_connector,
        } => {
            tracing::debug!(
                "StopRecording: session={}, output={}",
                session_id,
                output_connector
            );

            // Get the session
            let session = match state.screenshare_sessions.get_mut(&session_id) {
                Some(s) => s,
                None => {
                    tracing::error!("Session not found: {}", session_id);
                    return;
                }
            };

            // Remove and stop the stream
            if let Some(_stream) = session.streams.remove(&output_connector) {
                tracing::debug!(
                    "Stopped stream for session={}, output={}",
                    session_id,
                    output_connector
                );
                // PipeWire stream will be dropped here
            } else {
                tracing::warn!(
                    "No active stream for output {} in session {}",
                    output_connector,
                    session_id
                );
            }
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
        }
    }
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
