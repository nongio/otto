//! A searchable list of things to pick: the row model, how a query ranks the
//! rows, and how they are painted.
//!
//! The launcher, the side canvas and the settings sidebar's search all list
//! their matches this way, so a row reads the same — and typing finds the same
//! things — wherever it appears.

pub mod item;
pub mod rows;
