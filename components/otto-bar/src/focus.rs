//! Foreign toplevel focus tracker — delegates to otto_kit::utils::focus_watcher.

pub use otto_kit::utils::focus_watcher::{
    current_focused_app, generation, spawn_focus_watcher, FocusedApp,
};
