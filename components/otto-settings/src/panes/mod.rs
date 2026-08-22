//! One module per settings pane.
//!
//! Each owns its own rows and their bindings, so panes can be worked on
//! independently.

pub mod displays;
pub mod dock;
pub mod general;
pub mod keyboard;
pub mod lock_and_login;
pub mod pointing;
pub mod power;
pub mod sound;
