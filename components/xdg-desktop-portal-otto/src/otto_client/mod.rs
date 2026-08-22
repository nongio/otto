//! Interface layer towards Otto's backend-facing APIs.
//!
//! This module owns the D-Bus bindings we use to talk to Otto.
//! Each backend API should live in its own submodule (e.g. Screencast,
//! RemoteDesktop). For now only the ScreenCast API is implemented.
//! See `ScreenCast-backend-spec.md` for the contract this module targets.

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
pub mod settings;
