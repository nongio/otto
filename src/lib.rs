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

/// Publish `rounded_corners` to the components, and to the compositor's own
/// drawing routines.
///
/// The dock is drawn here, the top bar and window decorations are drawn in
/// other processes, and only the compositor reads the configuration file —
/// [`otto_kit::corners`] is where all three of them agree on the answer.
///
/// Returns the assignment to hand to the session's activation environments,
/// alongside the locale's.
pub fn export_rounded_corners() -> String {
    otto_kit::corners::export(config::Config::with(|c| c.rounded_corners))
}

/// Publish `window_controls_side` the same way, and for the same reason: an
/// otto-kit client draws its own titlebar, and only the compositor reads the
/// configuration file.
///
/// An unparseable value keeps the default rather than failing the session —
/// the schema rejects one before it can ever be written.
pub fn export_window_controls_side() -> String {
    let side = config::Config::with(|c| {
        otto_kit::controls_side::ControlsSide::parse(&c.window_controls_side).unwrap_or_default()
    });
    otto_kit::controls_side::export(side)
}

/// Publish `theme_scheme` the same way.
///
/// otto-kit apps normally learn light-versus-dark from the freedesktop
/// Settings portal, which Otto serves — but the portal backend is optional,
/// and without it every app fell back to light on a dark desktop. The
/// environment reaches them whether or not it is running; the portal still
/// wins wherever it answers, and it is still the only channel that carries a
/// live change.
pub fn export_color_scheme() -> String {
    let scheme = config::Config::with(|c| match c.theme_scheme {
        theme::ThemeScheme::Dark => otto_kit::theme::ColorScheme::Dark,
        theme::ThemeScheme::Light => otto_kit::theme::ColorScheme::Light,
    });
    otto_kit::color_scheme::export(scheme)
}
mod utils;
