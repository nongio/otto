//! Simple sound effect player for UI feedback
//!
//! Plays short sound samples (e.g., volume adjustment clicks) using PipeWire.
//! Follows XDG Sound Theme specification for sound lookup.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

/// Global sound path cache
static SOUND_CACHE: OnceLock<RwLock<HashMap<String, Option<PathBuf>>>> = OnceLock::new();

/// Global last play time cache for rate limiting
static LAST_PLAY: OnceLock<RwLock<HashMap<String, Instant>>> = OnceLock::new();

fn sound_cache() -> &'static RwLock<HashMap<String, Option<PathBuf>>> {
    SOUND_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn last_play_cache() -> &'static RwLock<HashMap<String, Instant>> {
    LAST_PLAY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Simple sound player for UI feedback
#[derive(Default)]
pub struct SoundPlayer {
    // Placeholder for future PipeWire state
}

impl SoundPlayer {
    /// Create a new sound player
    pub fn new() -> Result<Self, String> {
        debug!("Initializing sound player");

        // Pre-warm the sound cache for common events (async to avoid blocking startup)
        // Only if sounds are enabled
        let sound_enabled = crate::config::Config::with(|c| c.audio.sound_enabled);
        if sound_enabled {
            std::thread::spawn(|| {
                debug!("Pre-warming sound cache...");
                let common_events = ["audio-volume-change"];
                for event in &common_events {
                    let _ = find_sound_for_event(event);
                }
                debug!("Sound cache pre-warming complete");
            });
        } else {
            debug!("Sound effects disabled - skipping cache pre-warming");
        }

        Ok(Self {})
    }

    /// Play a sound file (non-blocking)
    pub fn play(&self, sound_path: &str) {
        let path = PathBuf::from(sound_path);

        if !path.exists() {
            error!("Sound file not found: {}", sound_path);
            return;
        }

        // Use pw-cat for now (simple, reliable, non-blocking)
        // TODO: Replace with native PipeWire stream for better control
        let path_clone = path.clone();
        std::thread::spawn(move || {
            let result = std::process::Command::new("pw-cat")
                .arg("--playback")
                .arg(&path_clone)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status(); // Changed from spawn() to status() to wait for completion

            match result {
                Ok(status) => {
                    if status.success() {
                        debug!("Played sound: {:?}", path_clone);
                    } else {
                        error!("pw-cat exited with error for {:?}", path_clone);
                    }
                }
                Err(e) => {
                    error!("Failed to play sound {:?}: {}", path_clone, e);
                }
            }
        });
    }

    /// Play a sound event by name using XDG Sound Theme
    ///
    /// Search order:
    /// 1. Custom sound in resources/{event}.{oga,ogg,wav}
    /// 2. Configured theme (if set)
    /// 3. Auto-detected system theme
    /// 4. "freedesktop" fallback theme
    ///
    /// Sound paths are cached after first lookup.
    /// Rate limited to prevent spam (min 100ms between same event).
    pub fn play_event(&self, event_name: &str) {
        // Check if sounds are enabled
        if !crate::config::Config::with(|c| c.audio.sound_enabled) {
            debug!("Sound effects disabled in config");
            return;
        }

        // Rate limiting: Don't play the same sound more than once per 100ms
        const MIN_INTERVAL: Duration = Duration::from_millis(100);
        {
            let mut last_play = last_play_cache().write().unwrap();
            if let Some(last_time) = last_play.get(event_name) {
                if last_time.elapsed() < MIN_INTERVAL {
                    debug!("Sound '{}' rate limited", event_name);
                    return;
                }
            }
            last_play.insert(event_name.to_string(), Instant::now());
        }

        // Check cache first
        let cache = sound_cache();
        {
            let cache_read = cache.read().unwrap();
            if let Some(cached_path) = cache_read.get(event_name) {
                if let Some(path) = cached_path {
                    if let Some(path_str) = path.to_str() {
                        self.play(path_str);
                        return;
                    }
                } else {
                    // Cached as "not found"
                    debug!("Sound event '{}' previously not found", event_name);
                    return;
                }
            }
        }

        // Not in cache - do lookup
        let sound_path = find_sound_for_event(event_name);

        // Cache the result (even if None)
        {
            let mut cache_write = cache.write().unwrap();
            cache_write.insert(event_name.to_string(), sound_path.clone());
        }

        if let Some(path) = sound_path {
            if let Some(path_str) = path.to_str() {
                self.play(path_str);
            }
        } else {
            debug!("No sound file found for event: {}", event_name);
        }
    }

    /// Play volume adjustment sound
    pub fn play_volume_sound(&self) {
        self.play_event("audio-volume-change");
    }
}

/// Find a sound file for a given event name
///
/// Implements XDG Sound Theme specification lookup
fn find_sound_for_event(event_name: &str) -> Option<PathBuf> {
    // 1. Check resources directory for custom sounds
    for ext in &["oga", "ogg", "wav", "flac"] {
        let resource_name = format!("{}.{}", event_name, ext);
        if let Some(path) = crate::utils::resource_path(&resource_name) {
            debug!("Found custom sound in resources: {:?}", path);
            return Some(path);
        }
    }

    // 2. Use XDG Sound Theme lookup
    crate::config::Config::with(|config| {
        if let Some(theme_name) = &config.audio.sound_theme {
            // Use specified theme
            if let Some(path) = find_sound_in_theme(event_name, theme_name) {
                debug!(
                    "Found sound in configured theme '{}': {:?}",
                    theme_name, path
                );
                return Some(path);
            }
            warn!(
                "Sound theme '{}' specified but event '{}' not found",
                theme_name, event_name
            );
        }

        // 3. Try auto-detection (same as icon theme or common defaults)
        let auto_themes = detect_sound_themes();
        for theme in &auto_themes {
            if let Some(path) = find_sound_in_theme(event_name, theme) {
                debug!("Found sound in auto-detected theme '{}': {:?}", theme, path);
                return Some(path);
            }
        }

        // 4. Fallback to "freedesktop" theme
        if let Some(path) = find_sound_in_theme(event_name, "freedesktop") {
            debug!("Found sound in freedesktop fallback theme: {:?}", path);
            return Some(path);
        }

        None
    })
}

/// Find a sound file in a specific theme
///
/// Search paths following XDG Sound Theme spec:
/// - /usr/share/sounds/{theme}/stereo/{event}.{oga,ogg,wav}
/// - /usr/local/share/sounds/{theme}/stereo/{event}.{oga,ogg,wav}
/// - ~/.local/share/sounds/{theme}/stereo/{event}.{oga,ogg,wav}
///
/// Also handles theme-specific directories (e.g., Pop uses stereo/action/)
fn find_sound_in_theme(event_name: &str, theme_name: &str) -> Option<PathBuf> {
    let base_dirs = [
        PathBuf::from("/usr/share/sounds"),
        PathBuf::from("/usr/local/share/sounds"),
    ];

    // Add user directory if available
    let mut search_dirs = base_dirs.to_vec();
    if let Some(home) = std::env::var_os("HOME") {
        let user_sounds = PathBuf::from(home).join(".local/share/sounds");
        search_dirs.push(user_sounds);
    }

    // Try each base directory
    for base_dir in search_dirs {
        let theme_dir = base_dir.join(theme_name);

        // Try multiple subdirectories (some themes organize differently)
        let subdirs = ["stereo", "stereo/action", ""];

        for subdir in &subdirs {
            let search_dir = if subdir.is_empty() {
                theme_dir.clone()
            } else {
                theme_dir.join(subdir)
            };

            for ext in &["oga", "ogg", "wav", "flac"] {
                let sound_path = search_dir.join(format!("{}.{}", event_name, ext));
                if sound_path.exists() {
                    return Some(sound_path);
                }
            }
        }
    }

    None
}

/// Auto-detect available sound themes
///
/// Returns a prioritized list of theme names to try based on desktop environment
fn detect_sound_themes() -> Vec<String> {
    let mut themes = Vec::new();

    // Check desktop environment
    if let Ok(de) = std::env::var("XDG_CURRENT_DESKTOP") {
        match de.to_lowercase().as_str() {
            "gnome" => themes.push("Yaru".to_string()),
            "kde" | "plasma" => themes.push("ocean".to_string()),
            "pop" => themes.push("Pop".to_string()),
            _ => {}
        }
    }

    // Common fallback themes
    themes.push("Pop".to_string());
    themes.push("Yaru".to_string());
    themes.push("ocean".to_string());

    themes
}
