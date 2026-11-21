//! The parts of otto-audio that need no window: what the service decides to
//! link, where the window remembers its cards, and the Graph canvas's layout,
//! hit-testing and drawing onto a bare canvas, and which streams the service
//! moves onto effects.
//!
//! Both binaries link this: `otto-audio` (the patchbay window) and
//! `otto-audio-service` (the daemon that keeps saved links in force).

pub mod effect_streams;
pub mod graph_view;
pub mod keeper;
pub mod layout;

/// The `application.name` the service connects to PipeWire with. The window
/// looks for a client by this name to tell whether the service is running.
pub const SERVICE_APP: &str = "otto-audio-service";

/// The `application.name` the window connects with.
pub const WINDOW_APP: &str = "otto-audio";
