//! The parts of otto-mixer that need no window: what the service decides to
//! link, where the window remembers its cards, and the Graph canvas's layout,
//! hit-testing and drawing onto a bare canvas, which streams the service
//! moves onto effects and to the outputs app rules name, and what it does
//! while an app records from a microphone.
//!
//! Both binaries link this: `otto-mixer` (the patchbay window) and
//! `otto-mixer-service` (the daemon that keeps saved links in force).

pub mod app_rules;
pub mod calls;
pub mod effect_streams;
pub mod graph_view;
pub mod keeper;
pub mod layout;

/// The `application.name` the service connects to PipeWire with. The window
/// looks for a client by this name to tell whether the service is running.
pub const SERVICE_APP: &str = "otto-mixer-service";

/// The `application.name` the window connects with.
pub const WINDOW_APP: &str = "otto-mixer";
