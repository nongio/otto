//! Agent Host Protocol (AHP) server.
//!
//! The normative specification is vendored under `spec/upstream/` at the
//! repository root; wire types come from the `ahp-types` crate pinned to the
//! same protocol version.

pub mod acp;
pub mod agent;
pub mod cli;
pub mod config;
pub mod dialog;
pub mod host;
pub mod rpc;
pub mod server;
pub mod store;
pub mod uri;

pub use server::Server;
