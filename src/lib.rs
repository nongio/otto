// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(feature = "winit", feature = "x11", feature = "udev")),
    allow(dead_code, unused_imports)
)]

#[cfg(any(feature = "udev", feature = "xwayland"))]
pub mod cursor;
pub mod drawing;
pub mod focus;
pub mod input;
pub mod input_handler;
pub mod interactive_view;
pub mod render;
pub mod render_elements;
pub mod render_metrics;
pub mod sc_layer_shell;
pub mod screenshare;
pub mod settings_service;
pub mod shell;
pub mod skia_renderer;
pub mod state;
pub mod textures_storage;
#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;
#[cfg(feature = "x11")]
pub mod x11;

pub use state::{CalloopData, ClientState, Otto};
mod workspaces;

mod config;
mod theme;
mod utils;
