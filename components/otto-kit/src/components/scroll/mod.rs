#![allow(clippy::module_inception)]
//! Scroll view: clips content to a viewport, offsets it along one [`Axis`],
//! and draws a proportional scrollbar over the top.
//!
//! Split the same way as [`text_input`](crate::components::text_input):
//! [`ScrollState`] is the model, [`ScrollRenderer`] the stateless drawing and
//! geometry, and [`ScrollView`] the widget that ties them together for a
//! host to drive. All three are pure canvas + data — no `AppContext`, no
//! client runtime — so the compositor can draw a scrolled pane exactly like
//! a client app can.

mod backing;
mod band;
mod renderer;
mod scroll;
mod state;

pub use backing::ScrollSurfaces;
pub use band::{Band, BandView};
pub use renderer::ScrollRenderer;
pub use scroll::{wheel_scale, ScrollView};
pub use state::{Axis, ScrollState};
