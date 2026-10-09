//! The shared parts of Otto's agent UIs.
//!
//! The launcher's agents mode and the side canvas both list otto-agents'
//! sessions under a search field. This crate holds what they have in common:
//! the row model and its ranking ([`item`]), the feed of sessions ([`sessions`])
//! on the connection thread every agent UI runs ([`link`]),
//! the keys that walk the rows and edit the field ([`keys`]), and the painters
//! for the rows and the field ([`rows`]).
//!
//! The Ask chat lives here too, so any app can host it: the conversation and
//! its connection ([`chat`]), and its log, laid out and painted at the
//! host's width ([`log`]).

pub mod chat;
pub mod keys;
pub mod link;
pub mod log;
pub mod opener;
pub mod sessions;

// The rows and their ranking are otto-kit's, so an app with no agent in it —
// the settings sidebar's search — can list matches the same way. Re-exported
// under their old names so the agent UIs keep reaching them here.
pub use otto_kit::components::item_list::{item, rows};
