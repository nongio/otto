//! Connecting to a running service, over its socket or over TCP.
//!
//! Lives in `otto-agents-client`, which the launcher depends on too: both ends
//! of this socket have to agree about where it is and how it is framed.

pub use otto_agents_client::transport::{
    LOOPBACK, SOCKET_PATH, connect, default_socket_path, default_url, socket_path,
};
