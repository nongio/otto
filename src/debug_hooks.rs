//! File-driven debug toggles (`touch /tmp/otto-<name>`) and command files
//! (`echo … > /tmp/otto-action`), for driving and inspecting a live session
//! from a shell without a rebuild.
//!
//! Every check is a `stat()` or a `read()` on a hot path — per frame, per
//! commit, per event-loop turn — so the whole mechanism is compiled in only
//! with the `debug-hooks` feature (part of `dev`). Without it these helpers
//! are constants and the branches they guard fold away.

/// Whether the hooks are compiled in.
pub const ENABLED: bool = cfg!(feature = "debug-hooks");

/// A toggle is on while the file exists.
#[cfg(feature = "debug-hooks")]
#[inline]
pub fn toggle(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[cfg(not(feature = "debug-hooks"))]
#[inline(always)]
pub fn toggle(_path: &str) -> bool {
    false
}

/// Read and delete a command file, so one write runs once.
#[cfg(feature = "debug-hooks")]
pub fn take_file(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(text)
}

#[cfg(not(feature = "debug-hooks"))]
#[inline(always)]
pub fn take_file(_path: &str) -> Option<String> {
    None
}
