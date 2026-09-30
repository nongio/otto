//! The shared parts of Otto's agent UIs.
//!
//! The launcher's agents mode and the side canvas both list otto-agents'
//! sessions under a search field. This crate holds what they have in common:
//! the row model and its ranking ([`item`]), the feed of sessions ([`sessions`]),
//! the keys that walk the rows and edit the field ([`keys`]), and the painters
//! for the rows and the field ([`rows`]).

pub mod item;
pub mod keys;
pub mod rows;
pub mod sessions;
