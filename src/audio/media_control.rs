//! Media player control via MPRIS D-Bus interface
//!
//! Provides control over media players (Spotify, VLC, Firefox, Chrome, etc.)
//! using the MPRIS2 D-Bus protocol. Supports play/pause, next, previous, and stop.

use mpris::PlayerFinder;
use tracing::{debug, error, info};

#[derive(Debug)]
pub enum MediaError {
    NoPlayerFound,
    ConnectionFailed(String),
    OperationFailed(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::NoPlayerFound => write!(f, "No media player found"),
            MediaError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            MediaError::OperationFailed(msg) => write!(f, "Operation failed: {}", msg),
        }
    }
}

impl std::error::Error for MediaError {}

/// Media controller for MPRIS-compatible players
pub struct MediaController;

impl MediaController {
    /// Find the currently active media player
    /// Returns the first playing player, or the first paused player, or the first player found
    fn find_active_player() -> Result<mpris::Player, MediaError> {
        let player_finder = PlayerFinder::new().map_err(|e| {
            MediaError::ConnectionFailed(format!("Failed to connect to D-Bus: {}", e))
        })?;

        // Try to find active player (playing or paused)
        if let Ok(player) = player_finder.find_active() {
            debug!("Found active player: {}", player.identity());
            return Ok(player);
        }

        // Fall back to first available player
        let players = player_finder.find_all().map_err(|e| {
            MediaError::ConnectionFailed(format!("Failed to enumerate players: {}", e))
        })?;

        if let Some(player) = players.into_iter().next() {
            debug!("Using first available player: {}", player.identity());
            Ok(player)
        } else {
            Err(MediaError::NoPlayerFound)
        }
    }

    /// Toggle play/pause on the active player
    pub fn play_pause() -> Result<(), MediaError> {
        std::thread::spawn(|| match Self::find_active_player() {
            Ok(player) => {
                let identity = player.identity();
                match player.play_pause() {
                    Ok(_) => {
                        info!(player = %identity, "Toggled play/pause");
                    }
                    Err(e) => {
                        error!(player = %identity, error = %e, "Failed to toggle play/pause");
                    }
                }
            }
            Err(MediaError::NoPlayerFound) => {
                debug!("No media player found for play/pause");
            }
            Err(e) => {
                error!(error = %e, "Failed to find media player");
            }
        });
        Ok(())
    }

    /// Skip to next track
    pub fn next() -> Result<(), MediaError> {
        std::thread::spawn(|| match Self::find_active_player() {
            Ok(player) => {
                let identity = player.identity();
                match player.next() {
                    Ok(_) => {
                        info!(player = %identity, "Skipped to next track");
                    }
                    Err(e) => {
                        error!(player = %identity, error = %e, "Failed to skip to next");
                    }
                }
            }
            Err(MediaError::NoPlayerFound) => {
                debug!("No media player found for next track");
            }
            Err(e) => {
                error!(error = %e, "Failed to find media player");
            }
        });
        Ok(())
    }

    /// Go to previous track
    pub fn previous() -> Result<(), MediaError> {
        std::thread::spawn(|| match Self::find_active_player() {
            Ok(player) => {
                let identity = player.identity();
                match player.previous() {
                    Ok(_) => {
                        info!(player = %identity, "Skipped to previous track");
                    }
                    Err(e) => {
                        error!(player = %identity, error = %e, "Failed to skip to previous");
                    }
                }
            }
            Err(MediaError::NoPlayerFound) => {
                debug!("No media player found for previous track");
            }
            Err(e) => {
                error!(error = %e, "Failed to find media player");
            }
        });
        Ok(())
    }

    /// Stop playback
    pub fn stop() -> Result<(), MediaError> {
        std::thread::spawn(|| match Self::find_active_player() {
            Ok(player) => {
                let identity = player.identity();
                match player.stop() {
                    Ok(_) => {
                        info!(player = %identity, "Stopped playback");
                    }
                    Err(e) => {
                        error!(player = %identity, error = %e, "Failed to stop playback");
                    }
                }
            }
            Err(MediaError::NoPlayerFound) => {
                debug!("No media player found for stop");
            }
            Err(e) => {
                error!(error = %e, "Failed to find media player");
            }
        });
        Ok(())
    }

    /// Get current playback metadata (for future OSD)
    #[allow(dead_code)]
    pub fn get_metadata() -> Option<MediaMetadata> {
        match Self::find_active_player() {
            Ok(player) => {
                if let Ok(metadata) = player.get_metadata() {
                    Some(MediaMetadata {
                        title: metadata.title().map(|s| s.to_string()),
                        artist: metadata
                            .artists()
                            .and_then(|a| a.first().map(|s| s.to_string())),
                        album: metadata.album_name().map(|s| s.to_string()),
                    })
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}

/// Media metadata for OSD display
#[derive(Debug, Clone)]
pub struct MediaMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}
