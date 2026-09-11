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
    material_at_least(colour, UNFROSTED_MIN_ALPHA)
}

/// The floor for menus without their frost. Higher than
/// [`UNFROSTED_MIN_ALPHA`]: a menu opens over whatever is on screen, and its
/// rows of small text are read against that with nothing blurring it.
pub const POPUP_UNFROSTED_MIN_ALPHA: u8 = 0xF2;

/// [`material`] for menus: taken up to at least
/// [`POPUP_UNFROSTED_MIN_ALPHA`] when frosting is off.
pub fn popup_material(colour: skia_safe::Color) -> skia_safe::Color {
    material_at_least(colour, POPUP_UNFROSTED_MIN_ALPHA)
}

/// The floor for the dock and the top bar without their frost. Lower than
/// [`UNFROSTED_MIN_ALPHA`]: they sit on the desktop and nothing but the
/// wallpaper passes under them, so a see-through bar still reads as a bar.
/// Menus, cards and the rest float over arbitrary content and keep the
/// higher floor.
pub const BAR_UNFROSTED_MIN_ALPHA: u8 = 0xCC;

/// [`material`] for the dock and the top bar: taken up to at least
/// [`BAR_UNFROSTED_MIN_ALPHA`] when frosting is off.
pub fn bar_material(colour: skia_safe::Color) -> skia_safe::Color {
    material_at_least(colour, BAR_UNFROSTED_MIN_ALPHA)
}

fn material_at_least(colour: skia_safe::Color, floor: u8) -> skia_safe::Color {
    if enabled() {
        colour
    } else {
        skia_safe::Color::from_argb(colour.a().max(floor), colour.r(), colour.g(), colour.b())
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
