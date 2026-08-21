#![allow(clippy::module_inception)]
//! Continuous value control: a track, a knob, and an optional caller-supplied
//! readout string.
//!
//! Promoted from the throwaway prototype in `otto-settings`' `widgets.rs`.
//! Split the way [`text_input`](crate::components::text_input) is:
//! [`slider`] is stateless drawing plus the geometry hit-testing and value
//! mapping share, and [`SliderDrag`] is the small state struct a host owns
//! to turn pointer events into value changes.

mod slider;
mod state;

pub use slider::{
    draw, fraction, hit_test_knob, hit_test_track, knob_center, value_at, SliderInteraction,
    KNOB_RADIUS, TRACK_THICKNESS,
};
pub use state::{SliderDrag, SliderResponse};
