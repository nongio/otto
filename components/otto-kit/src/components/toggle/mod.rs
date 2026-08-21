#![allow(clippy::module_inception)]
//! On/off switch: a rounded track and a knob that slides between two ends.
//!
//! Promoted from the throwaway prototype in `otto-settings`' `widgets.rs` —
//! same look, but with a hit-test helper and interaction states added. No
//! state of its own: the caller owns the boolean value and, for an animated
//! flip, the knob's fractional position along the track.

mod toggle;

pub use toggle::{
    draw, hit_test, knob_fraction_for, Flip, ToggleInteraction, FLIP_DURATION, HEIGHT, WIDTH,
};
