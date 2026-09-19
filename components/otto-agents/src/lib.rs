//! Agent Host Protocol (AHP) server.
//!
//! The normative specification is vendored under `spec/upstream/` at the
//! repository root; wire types come from the `ahp-types` crate pinned to the
//! same protocol version.

pub mod acp;
pub mod agent;
pub mod cli;
pub mod client;
pub mod config;
pub mod dialog;
pub mod elicitation;
pub mod host;
pub mod i18n;
pub mod images;
pub mod rpc;
pub mod server;
pub mod skills;
pub mod store;
pub mod uri;
pub mod vendors;
pub mod xdg;

pub use server::Server;
