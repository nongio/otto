//! Utility functions for otto-kit.

pub mod color_extraction;
pub mod focus_watcher;
pub mod sampling;
pub use color_extraction::{extract_accent_color, extract_palette};
pub use sampling::icon_sampling;
