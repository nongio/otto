//! What otto-agents and its clients both have to agree on.
//!
//! The launcher used to carry its own copy of this — the transport, where the
//! socket is, how a `file://` URI decodes, what a session URI looks like — and
//! the copies drifted: one accepted a relative URI the other refused, and one
//! had lost a fallback the other kept. Neither crate could see the other's
//! version, so nothing failed until a folder meant two different things.
//!
//! It is deliberately a leaf: the AHP crates and a socket, nothing of the
//! service's own. That is what lets the launcher depend on it.

pub mod session;
pub mod transport;
pub mod uri;

pub use transport::{connect, default_url, runtime_dir, socket_path};
