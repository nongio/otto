//! Utility functions for otto-kit.

pub mod color_extraction;
pub mod focus_watcher;
pub use color_extraction::extract_accent_color;
pub use focus_watcher::{current_focused_app, generation as focus_generation, spawn_focus_watcher, FocusedApp};
