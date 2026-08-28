// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(
        feature = "winit",
        feature = "x11",
        feature = "udev",
        feature = "headless"
    )),
    allow(dead_code, unused_imports)
)]

pub mod a11y;
pub mod audio;
pub mod background_effect;
#[cfg(any(feature = "udev", feature = "xwayland", feature = "headless"))]
pub mod cursor;
pub mod debug_gesture;
pub mod drawing;
pub mod focus;
#[cfg(feature = "headless")]
pub mod headless;
pub mod input;
pub mod input_handler;
pub mod interactive_view;
pub mod locale_env;
pub mod lock;
pub mod login;
pub mod otto_dock;
pub mod render;
pub mod render_elements;
#[cfg(feature = "metrics")]
pub mod render_metrics;
pub mod render_phase_stats;
pub mod renderer;
pub mod screenshare;
pub mod settings;
pub mod settings_service;
pub mod shell;
pub mod skia_renderer;
pub mod state;
pub mod surface_config_cache;
pub mod surface_style;
pub mod textures_storage;
#[cfg(feature = "udev")]
pub mod udev;
pub mod virtual_output;
#[cfg(feature = "winit")]
pub mod winit;
#[cfg(feature = "x11")]
pub mod x11;

pub use state::{CalloopData, ClientState, Otto};
mod workspaces;

mod config;
mod theme;

/// The user's preferred locales, most preferred first.
///
/// Exposed so `main` can load the string catalogues before any chrome is
/// built, without opening up the whole config module.
pub fn configured_locales() -> Vec<String> {
    config::Config::with(|c| c.locales.clone())
}
mod utils;
