//! PipeWire-based audio volume control
//!
//! Provides native integration with PipeWire for volume management:
//! - Enumerate audio sink nodes via Registry
//! - Get/set volume via node parameters
//! - Track mute state
//! - Event-driven updates for OSD integration

use std::sync::{Arc, Mutex};

use tracing::{debug, error, info};

#[derive(Debug)]
pub enum VolumeError {
    InitFailed(String),
    ConnectionFailed(String),
    NoSinkFound,
    OperationFailed(String),
}

impl std::fmt::Display for VolumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VolumeError::InitFailed(msg) => write!(f, "PipeWire init failed: {}", msg),
            VolumeError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            VolumeError::NoSinkFound => write!(f, "No audio sink found"),
            VolumeError::OperationFailed(msg) => write!(f, "Operation failed: {}", msg),
        }
    }
}

impl std::error::Error for VolumeError {}

/// Audio state tracked for OSD display
#[derive(Debug, Clone)]
pub struct AudioState {
    /// Current volume (0-100)
    pub volume: u32,
    /// Mute state
    pub muted: bool,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            volume: 50,
            muted: false,
        }
    }
}

/// Audio manager using PipeWire for volume control
pub struct AudioManager {
    /// Cached audio state
    state: Arc<Mutex<AudioState>>,
}

impl AudioManager {
    /// Create a new audio manager
    pub fn new() -> Result<Self, VolumeError> {
        info!("Initializing PipeWire audio manager");

        Ok(Self {
            state: Arc::new(Mutex::new(AudioState::default())),
        })
    }

    /// Get current audio state
    pub fn get_state(&self) -> AudioState {
        self.state.lock().unwrap().clone()
    }

    /// Increase volume by delta (clamped to 0-100)
    pub fn increase_volume(&self, delta: i32) -> Result<(), VolumeError> {
        let mut state = self.state.lock().unwrap();
        let new_volume = (state.volume as i32 + delta).clamp(0, 100) as u32;

        tracing::trace!(
            current = state.volume,
            delta = delta,
            new = new_volume,
            "Increasing volume"
        );

        // Apply via wpctl for now (will be replaced with native PipeWire)
        self.set_volume_wpctl(new_volume)?;
        state.volume = new_volume;

        Ok(())
    }

    /// Decrease volume by delta (clamped to 0-100)
    pub fn decrease_volume(&self, delta: i32) -> Result<(), VolumeError> {
        self.increase_volume(-delta)
    }

    /// Toggle mute state
    pub fn toggle_mute(&self) -> Result<(), VolumeError> {
        let mut state = self.state.lock().unwrap();
        let new_muted = !state.muted;

        tracing::trace!(current = state.muted, new = new_muted, "Toggling mute");

        // Apply via wpctl for now (will be replaced with native PipeWire)
        self.set_mute_wpctl(new_muted)?;
        state.muted = new_muted;

        Ok(())
    }

    /// Set volume via wpctl (temporary implementation)
    fn set_volume_wpctl(&self, volume: u32) -> Result<(), VolumeError> {
        let volume_fraction = (volume as f32 / 100.0).clamp(0.0, 1.0);

        std::thread::spawn(move || {
            let output = std::process::Command::new("wpctl")
                .args([
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{:.4}", volume_fraction),
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    tracing::trace!("Volume set to {}% via wpctl", volume);
                }
                Ok(out) => {
                    tracing::error!("wpctl failed: {}", String::from_utf8_lossy(&out.stderr));
                }
                Err(e) => {
                    error!("Failed to execute wpctl: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Set mute via wpctl (temporary implementation)
    fn set_mute_wpctl(&self, muted: bool) -> Result<(), VolumeError> {
        let mute_arg = if muted { "1" } else { "0" };

        std::thread::spawn(move || {
            let output = std::process::Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", mute_arg])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    debug!("Mute set to {} via wpctl", muted);
                }
                Ok(out) => {
                    error!("wpctl failed: {}", String::from_utf8_lossy(&out.stderr));
                }
                Err(e) => {
                    error!("Failed to execute wpctl: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Query current volume from PipeWire (future implementation)
    #[allow(dead_code)]
    fn query_volume_pipewire(&self) -> Result<AudioState, VolumeError> {
        // TODO: Use Registry to find default sink
        // TODO: Query node parameters for volume
        // TODO: Parse volume and mute state
        Err(VolumeError::OperationFailed(
            "Native PipeWire query not yet implemented".to_string(),
        ))
    }

    /// Set volume via PipeWire (future implementation)
    #[allow(dead_code)]
    fn set_volume_pipewire(&self, _volume: u32) -> Result<(), VolumeError> {
        // TODO: Use Registry to find default sink
        // TODO: Set node parameters
        Err(VolumeError::OperationFailed(
            "Native PipeWire control not yet implemented".to_string(),
        ))
    }
}

impl Default for AudioManager {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(AudioState::default())),
        }
    }
}
