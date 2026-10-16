//! The desktop wallpaper's geometry: where an image lands on a screen.
//!
//! Canvas-pure and connection-free, like everything the compositor takes from
//! this crate: the compositor paints the wallpaper with it, and the settings
//! app's preview uses the same arithmetic, so what the preview shows is what
//! the desktop gets.

mod layout;
mod paint;
mod placement;
mod source;

pub use layout::{arrange, Arrangement, Layout, Mode, SpanView, MAX_ZOOM};
pub use paint::{paint, sampling};
pub use placement::{cover, cover_size, Placement};
pub use source::{open, render_frame, FrameStyle, OpenError, Source};
