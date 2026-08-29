//! Whether the desktop rounds the corners of its chrome.
//!
//! The dock's background, the top bar and window decorations are drawn by
//! three different processes, and only the compositor reads Otto's
//! configuration. It publishes the answer in the environment — the one channel
//! every child and every bus-activated helper already inherits — and this
//! module is where the rest of otto-kit reads it back.
//!
//! Read once, on first use: the value comes from the configuration file and
//! only takes effect at startup, so nothing here has to follow a change.

use std::sync::atomic::{AtomicU8, Ordering};

/// The variable the compositor publishes. `0` squares the corners off;
/// anything else, or nothing at all, rounds them.
pub const ENV: &str = "OTTO_ROUNDED_CORNERS";

/// 0 = not resolved yet, 1 = square, 2 = rounded.
static ROUNDED: AtomicU8 = AtomicU8::new(0);

/// Whether chrome should be drawn with rounded corners.
pub fn rounded() -> bool {
    match ROUNDED.load(Ordering::Relaxed) {
        0 => {
            let value = match std::env::var(ENV) {
                Ok(text) => !matches!(text.trim(), "0" | "false" | "no" | "off"),
                Err(_) => true,
            };
            ROUNDED.store(if value { 2 } else { 1 }, Ordering::Relaxed);
            value
        }
        1 => false,
        _ => true,
    }
}

/// `radius` where the desktop rounds its corners, and 0 where it does not.
///
/// Every drawing routine keeps its own radius — the numbers differ, and the
/// setting is a yes or no, not a size. This is the one place that turns the
/// answer into a value a draw call can take.
pub fn radius(radius: f32) -> f32 {
    if rounded() {
        radius
    } else {
        0.0
    }
}

/// Publish `value` to this process and everything it starts.
///
/// The compositor calls this once, before it spawns anything: it holds the
/// configuration, so it never reads the variable, but its own drawing goes
/// through [`radius`] like everyone else's.
///
/// Returns the assignment in `KEY=value` form, for the activation environments
/// that have to be told separately — a bus-activated helper is not a child of
/// the compositor and inherits nothing from it.
pub fn export(value: bool) -> String {
    ROUNDED.store(if value { 2 } else { 1 }, Ordering::Relaxed);
    let text = if value { "1" } else { "0" };
    std::env::set_var(ENV, text);
    format!("{ENV}={text}")
}
