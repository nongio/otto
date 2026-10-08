//! The chat's log: the conversation laid out as lines at a width the host
//! gives ([`layout`]), painted a band at a time and hit-tested
//! ([`LogPainter`]), and its text selected ([`selection`]).
//!
//! The log draws onto a canvas, never into a scene, so it paints the same
//! into the launcher's card and into a plain otto-kit window.

mod layout;
pub mod paint;
pub mod selection;

pub use layout::*;
pub use paint::{AttachmentHit, LogPainter};
