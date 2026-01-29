//! Input handling subsystem
//!
//! This module provides a modular input handling system split by device type:
//! - `actions`: Key action definitions and processing
//! - `keyboard`: Keyboard event handling and shortcuts
//! - `pointer`: Mouse/pointer event handling
//! - `gestures`: Gesture processing (swipe, pinch, hold)
//! - `tablet`: Tablet input support

pub mod actions;
pub mod keyboard;
pub mod pointer;

#[cfg(feature = "udev")]
pub mod gestures;
#[cfg(feature = "udev")]
pub mod tablet;

// Re-export commonly used types
pub use actions::{resolve_shortcut_action, KeyAction};
pub use keyboard::{
    app_switcher_hold_is_active, capture_app_switcher_hold_modifiers, process_keyboard_shortcut,
};
