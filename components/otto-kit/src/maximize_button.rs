//! Whether the titlebar shows the zoom (maximize) dot.
//!
//! Otto ships with two traffic lights — close and minimize — because a window
//! is zoomed by double-clicking its bar. A desktop that wants the third dot
//! turns it on in the configuration, and the answer travels the way corner
//! rounding and the controls' side do (see [`crate::corners`]): the compositor
//! reads the file and publishes it in the environment, and everything that
//! draws a titlebar reads it back from here.

use std::sync::atomic::{AtomicU8, Ordering};

/// The variable the compositor publishes. `1` shows the dot; anything else, or
/// nothing at all, hides it.
pub const ENV: &str = "OTTO_SHOW_MAXIMIZE_BUTTON";

/// 0 = not resolved yet, 1 = hidden, 2 = shown.
static SHOWN: AtomicU8 = AtomicU8::new(0);

/// Whether a titlebar should draw the zoom dot.
pub fn enabled() -> bool {
    match SHOWN.load(Ordering::Relaxed) {
        0 => {
            let value = match std::env::var(ENV) {
                Ok(text) => matches!(text.trim(), "1" | "true" | "yes" | "on"),
                Err(_) => false,
            };
            store(value);
            value
        }
        2 => true,
        _ => false,
    }
}

fn store(value: bool) {
    SHOWN.store(if value { 2 } else { 1 }, Ordering::Relaxed);
}

/// Publish `value` to this process and everything it starts, and return the
/// assignment for the session's activation environments. See
/// [`crate::corners::export`].
pub fn export(value: bool) -> String {
    store(value);
    let text = if value { "1" } else { "0" };
    std::env::set_var(ENV, text);
    format!("{ENV}={text}")
}
