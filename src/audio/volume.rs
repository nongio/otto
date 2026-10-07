//! PipeWire-based audio volume control
//!
//! Provides native integration with PipeWire for volume management:
//! - Enumerate audio sink nodes via Registry
//! - Get/set volume via node parameters
//! - Track mute state
//! - Event-driven updates for OSD integration

use std::sync::{mpsc, Arc, Mutex, OnceLock};

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
    /// Channel to the wpctl worker thread, started lazily on first change
    wpctl_tx: OnceLock<mpsc::Sender<WpctlCommand>>,
}

impl AudioManager {
    /// Create a new audio manager
    pub fn new() -> Result<Self, VolumeError> {
        info!("Initializing PipeWire audio manager");

        Ok(Self {
            state: Arc::new(Mutex::new(AudioState::default())),
            wpctl_tx: OnceLock::new(),
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

    /// Queue a volume change for the wpctl worker (temporary implementation)
    fn set_volume_wpctl(&self, volume: u32) -> Result<(), VolumeError> {
        self.send_wpctl(WpctlCommand::Volume(volume))
    }

    /// Queue a mute change for the wpctl worker (temporary implementation)
    fn set_mute_wpctl(&self, muted: bool) -> Result<(), VolumeError> {
        self.send_wpctl(WpctlCommand::Mute(muted))
    }

    /// Send a command to the wpctl worker, starting it on first use.
    ///
    /// A single worker applies commands in order, so a held volume key can't
    /// race several `wpctl` processes and leave a stale volume applied.
    fn send_wpctl(&self, command: WpctlCommand) -> Result<(), VolumeError> {
        let sender = self.wpctl_tx.get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            if let Err(e) = std::thread::Builder::new()
                .name("otto-wpctl".into())
                .spawn(move || wpctl_worker(rx))
            {
                error!("Failed to spawn wpctl worker: {}", e);
            }
            tx
        });

        sender
            .send(command)
            .map_err(|e| VolumeError::OperationFailed(format!("wpctl worker gone: {}", e)))
    }
}

/// A change to apply to the default sink via `wpctl`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WpctlCommand {
    /// Volume in percent (0-100)
    Volume(u32),
    Mute(bool),
}

/// Collapse each run of consecutive volume changes to its last value, keeping
/// mute changes in their original order relative to volume.
fn coalesce_wpctl_commands(commands: Vec<WpctlCommand>) -> Vec<WpctlCommand> {
    let mut out: Vec<WpctlCommand> = Vec::with_capacity(commands.len());
    for command in commands {
        if let (WpctlCommand::Volume(_), Some(WpctlCommand::Volume(_))) = (command, out.last()) {
            out.pop();
        }
        out.push(command);
    }
    out
}

/// Worker loop: block for a command, drain whatever else is pending, and
/// apply the coalesced batch in order.
fn wpctl_worker(rx: mpsc::Receiver<WpctlCommand>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        for command in coalesce_wpctl_commands(batch) {
            run_wpctl(command);
        }
    }
}

fn run_wpctl(command: WpctlCommand) {
    let output = match command {
        WpctlCommand::Volume(volume) => {
            let volume_fraction = (volume as f32 / 100.0).clamp(0.0, 1.0);
            std::process::Command::new("wpctl")
                .args([
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{:.4}", volume_fraction),
                ])
                .output()
        }
        WpctlCommand::Mute(muted) => {
            let mute_arg = if muted { "1" } else { "0" };
            std::process::Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", mute_arg])
                .output()
        }
    };

    match output {
        Ok(out) if out.status.success() => {
            debug!("Applied {:?} via wpctl", command);
        }
        Ok(out) => {
            error!("wpctl failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Err(e) => {
            error!("Failed to execute wpctl: {}", e);
        }
    }
}

impl Default for AudioManager {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(AudioState::default())),
            wpctl_tx: OnceLock::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WpctlCommand::{Mute, Volume};

    #[test]
    fn coalesce_keeps_latest_volume() {
        let batch = vec![Volume(55), Volume(60), Volume(65)];
        assert_eq!(coalesce_wpctl_commands(batch), vec![Volume(65)]);
    }

    #[test]
    fn coalesce_keeps_mute_order() {
        let batch = vec![
            Volume(10),
            Volume(20),
            Mute(true),
            Volume(30),
            Volume(40),
            Mute(false),
        ];
        assert_eq!(
            coalesce_wpctl_commands(batch),
            vec![Volume(20), Mute(true), Volume(40), Mute(false)]
        );
    }

    #[test]
    fn coalesce_empty() {
        assert!(coalesce_wpctl_commands(Vec::new()).is_empty());
    }
}
