//! Audio volume control via PipeWire
//!
//! This module provides native PipeWire integration for audio volume control,
//! allowing real-time volume adjustment and state tracking for OSD display.

pub mod volume;

pub use volume::{AudioManager, AudioState, VolumeError};
