//! The chat's log: the conversation laid out as lines at a width the host
//! gives ([`layout`]), painted a band at a time and hit-tested
//! ([`LogPainter`]), its text selected ([`selection`]), and all of it kept
//! between a host's events by [`ChatView`].
//!
//! The log draws onto a canvas, never into a scene, so it paints the same
//! into the launcher's card and into a plain otto-kit window.

mod layout;
pub mod paint;
pub mod selection;
pub mod view;

pub use layout::*;
pub use paint::{AttachmentHit, LogPainter};
pub use view::{ChatView, Key, Keyed, Pressed, Released};
