//! Conversion between `file://` URIs and local paths.
//!
//! Lives in `otto-agents-client` so the service and its clients cannot end up
//! disagreeing about what folder a URI names.

pub use otto_agents_client::uri::{from_path, to_path};
