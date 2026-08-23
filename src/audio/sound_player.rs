//! UI feedback sounds for the compositor.
//!
//! The lookup, the cache and the playing all live in
//! [`otto_kit::sound`](otto_kit::sound), shared with every otto-kit app so the
//! desktop sounds like one desktop. What is left here is the compositor's own
//! part of it: pushing its configuration into that player, and naming the
//! events the compositor itself has.

use tracing::debug;

/// Plays the compositor's UI sounds.
///
/// Holds nothing — the player's state is process-wide — but is kept as a type
/// so the compositor can carry "sound is available" in its state the way it
/// carries every other subsystem, and so a build without sound is a `None`
/// rather than a scattering of checks.
#[derive(Default)]
pub struct SoundPlayer;

impl SoundPlayer {
    /// Configure the shared player from Otto's config and pre-warm the events
    /// the compositor plays most.
    pub fn new() -> Result<Self, String> {
        let (enabled, theme) =
            crate::config::Config::with(|c| (c.audio.sound_enabled, c.audio.sound_theme.clone()));
        otto_kit::sound::set_enabled(enabled);
        otto_kit::sound::set_theme(theme);
        // Sounds shipped with Otto override the theme's, which is how a
        // bundled click can replace one nobody likes.
        otto_kit::sound::set_extra_search_dirs(vec![
            std::path::PathBuf::from("resources"),
            std::path::PathBuf::from("/etc/otto/share"),
        ]);

        if enabled {
            // The first lookup walks every theme directory; doing it on the
            // first volume key would put that walk on the keystroke.
            debug!("pre-warming the sound cache");
            otto_kit::sound::prewarm(&["audio-volume-change", "desktop-screen-lock"]);
        }

        Ok(Self)
    }

    /// Play a sound file directly, off the theme.
    pub fn play(&self, path: &str) {
        otto_kit::sound::play_file(std::path::Path::new(path));
    }

    /// Play a themed sound by its sound-naming-spec event name.
    pub fn play_event(&self, event: &str) {
        otto_kit::sound::play_event(event);
    }

    /// Play volume adjustment sound
    pub fn play_volume_sound(&self) {
        self.play_event("audio-volume-change");
    }

    /// Play the session-lock sound.
    ///
    /// `desktop-screen-lock` is the sound-naming spec's event for it. Not every
    /// theme ships one — `freedesktop` does not — in which case the lookup
    /// falls through the other installed themes and, finding nothing, the lock
    /// is simply silent.
    pub fn play_lock_sound(&self) {
        self.play_event("desktop-screen-lock");
    }
}
