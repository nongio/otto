// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(feature = "winit", feature = "x11", feature = "udev")),
    allow(dead_code, unused_imports)
)]

pub mod audio;
#[cfg(any(feature = "udev", feature = "xwayland"))]
pub mod cursor;
pub mod drawing;
pub mod focus;
pub mod input;
pub mod input_handler;
pub mod interactive_view;
pub mod otto_dock;
pub mod render;
pub mod render_elements;
#[cfg(feature = "metrics")]
pub mod render_metrics;
pub mod renderer;
pub mod screenshare;
pub mod settings_service;
pub mod shell;
pub mod skia_renderer;
pub mod state;
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
mod utils;
