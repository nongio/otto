//! Whether the desktop frosts its chrome.
//!
//! Frosting is the translucent, blurred material behind the dock, the top bar,
//! the launcher and the desktop's own panels. The blur is the compositor's,
//! but the bar and the launcher have to *ask* for it through the surface-style
//! protocol, and only the compositor reads Otto's configuration — so, like
//! [`crate::corners`], the answer travels in the environment and, for a
//! change while a process runs, over the Settings portal under Otto's own
//! namespace ([`crate::desktop_appearance`] stores it here with [`set`]).

use std::sync::atomic::{AtomicU8, Ordering};

/// The variable the compositor publishes. `0` turns the frosting off;
/// anything else, or nothing at all, leaves it on.
pub const ENV: &str = "OTTO_FROSTING";

/// 0 = not resolved yet, 1 = off, 2 = on.
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Whether chrome should be frosted.
pub fn enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        0 => {
            let value = match std::env::var(ENV) {
                Ok(text) => !matches!(text.trim(), "0" | "false" | "no" | "off"),
                Err(_) => true,
            };
            ENABLED.store(if value { 2 } else { 1 }, Ordering::Relaxed);
            value
        }
        1 => false,
        _ => true,
    }
}

/// Update the answer without touching the environment — see
/// [`crate::corners::set`] for why.
pub fn set(value: bool) {
    ENABLED.store(if value { 2 } else { 1 }, Ordering::Relaxed);
}

/// The least opaque a surface gets without its frost: a hint of what is
/// behind it, no more — a see-through panel with nothing blurred behind it
/// reads as a glitch, not as a style.
pub const UNFROSTED_MIN_ALPHA: u8 = 0xEB;

/// `colour` as a frosted surface should paint it: as given while frosting is
/// on, taken up to at least [`UNFROSTED_MIN_ALPHA`] when it is off.
pub fn material(colour: skia_safe::Color) -> skia_safe::Color {
    if enabled() {
        colour
    } else {
        skia_safe::Color::from_argb(
            colour.a().max(UNFROSTED_MIN_ALPHA),
            colour.r(),
            colour.g(),
            colour.b(),
        )
    }
}

/// Publish `value` to this process and everything it starts — see
/// [`crate::corners::export`].
pub fn export(value: bool) -> String {
    set(value);
    let text = if value { "1" } else { "0" };
    std::env::set_var(ENV, text);
    format!("{ENV}={text}")
}
