//! Audio control via PipeWire and MPRIS
//!
//! This module provides native PipeWire integration for audio volume control,
//! and MPRIS D-Bus integration for media player control.

pub mod media_control;
pub mod sound_player;
pub mod volume;

pub use media_control::{MediaController, MediaError};
pub use sound_player::SoundPlayer;
pub use volume::{AudioManager, AudioState, VolumeError};
