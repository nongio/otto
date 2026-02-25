//! Keyboard input handling utilities
//!
//! Provides key code constants for handling keyboard events in HelloDesign components.
//!
//! ## Key Codes vs Keysyms
//!
//! This module provides **Linux kernel keycodes** (evdev scancodes), which are:
//! - Hardware-level scan codes from the Linux input subsystem
//! - What you receive from raw Wayland `wl_keyboard::key` events
//! - Layout-dependent (same physical key, different codes on different keyboards)
//!
//! ### When to Use Keycodes
//! Use these constants when handling `wl_keyboard::key` events where you receive
//! a raw `u32` keycode before XKB translation.
//!
//! ### Better Alternative: XKB Keysyms
//! For more robust keyboard handling, prefer **XKB keysyms** from `xkbcommon::xkb::keysyms`:
//! - Layout-independent (Ctrl+C works on any keyboard layout)
//! - Semantic (KEY_Return, KEY_Escape, not numbers)
//! - Cross-platform standard
//! - Already in dependency tree via `smithay-client-toolkit`
//!
//! ### Migration Path
//! When XKB context is available in your event handler:
//! ```rust,ignore
//! use xkbcommon::xkb::keysyms;
//!
//! match keysym {
//!     keysyms::KEY_Up => { /* ... */ }
//!     keysyms::KEY_Down => { /* ... */ }
//!     keysyms::KEY_Return => { /* ... */ }
//!     keysyms::KEY_Escape => { /* ... */ }
//!     _ => {}
//! }
//! ```
//!
//! ## Reference
//!
//! Keycode source: Linux kernel `<linux/input-event-codes.h>`
//! - Full list: https://github.com/torvalds/linux/blob/master/include/uapi/linux/input-event-codes.h

/// Linux kernel keycode constants (evdev scancodes)
pub mod keycodes {
    /// Escape key (scancode 1)
    pub const ESC: u32 = 1;

    /// Enter/Return key (scancode 28)
    pub const ENTER: u32 = 28;

    /// Backspace key (scancode 14)
    pub const BACKSPACE: u32 = 14;

    /// Tab key (scancode 15)
    pub const TAB: u32 = 15;

    /// Space bar (scancode 57)
    pub const SPACE: u32 = 57;

    // Arrow keys
    /// Up arrow (scancode 103)
    pub const UP: u32 = 103;

    /// Down arrow (scancode 108)
    pub const DOWN: u32 = 108;

    /// Left arrow (scancode 105)
    pub const LEFT: u32 = 105;

    /// Right arrow (scancode 106)
    pub const RIGHT: u32 = 106;

    // Common letter keys (QWERTY)
    /// M key (scancode 50)
    pub const M: u32 = 50;

    /// Q key (scancode 16)
    pub const Q: u32 = 16;

    // Function keys
    /// F1 key
    pub const F1: u32 = 59;

    /// F2 key
    pub const F2: u32 = 60;

    /// F3 key
    pub const F3: u32 = 61;

    /// F4 key
    pub const F4: u32 = 62;

    /// F5 key
    pub const F5: u32 = 63;

    /// F6 key
    pub const F6: u32 = 64;

    /// F7 key
    pub const F7: u32 = 65;

    /// F8 key
    pub const F8: u32 = 66;

    /// F9 key
    pub const F9: u32 = 67;

    /// F10 key
    pub const F10: u32 = 68;

    /// F11 key
    pub const F11: u32 = 87;

    /// F12 key
    pub const F12: u32 = 88;

    // Navigation keys
    /// Home key
    pub const HOME: u32 = 102;

    /// End key
    pub const END: u32 = 107;

    /// Page Up
    pub const PAGE_UP: u32 = 104;

    /// Page Down
    pub const PAGE_DOWN: u32 = 109;

    /// Delete key
    pub const DELETE: u32 = 111;

    /// Insert key
    pub const INSERT: u32 = 110;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_navigation_keys() {
        assert_eq!(keycodes::UP, 103);
        assert_eq!(keycodes::DOWN, 108);
        assert_eq!(keycodes::LEFT, 105);
        assert_eq!(keycodes::RIGHT, 106);
    }

    #[test]
    fn test_action_keys() {
        assert_eq!(keycodes::ESC, 1);
        assert_eq!(keycodes::ENTER, 28);
        assert_eq!(keycodes::BACKSPACE, 14);
    }
}
