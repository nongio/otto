//! Interface layer towards Otto's backend-facing APIs.
//!
//! The proxies themselves are declared once, in the `otto-dbus` crate; this
//! module wraps them in the calls the portal makes, one submodule per backend
//! API.

use zbus::{Connection, Result};

/// Client wrapper for Otto's D-Bus APIs.
#[derive(Clone)]
pub struct OttoClient {
    pub(crate) connection: Connection,
}

impl OttoClient {
    /// Creates a new client using the given D-Bus connection.
    pub async fn new(connection: Connection) -> Result<Self> {
        Ok(Self { connection })
    }
}

pub mod dialog;
pub mod file_picker;
pub mod screencast;
