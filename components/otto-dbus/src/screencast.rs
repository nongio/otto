//! `org.otto.ScreenCast`, with its `Session` and `Stream` objects: the
//! backend the screencast portal and the RDP bridge capture through.
//!
//! Served by the compositor (`src/screenshare/dbus_service.rs`); see
//! `docs/developer/screenshare.md`. Session and stream proxies have no
//! default path: build them at the object path the parent answered with.

use std::collections::HashMap;

use zbus::zvariant::{OwnedFd, OwnedObjectPath, OwnedValue, Value};

/// The well-known bus name.
pub const SERVICE: &str = "org.otto.ScreenCast";
/// The object path of the root interface.
pub const PATH: &str = "/org/otto/ScreenCast";

#[zbus::proxy(
    interface = "org.otto.ScreenCast",
    default_service = "org.otto.ScreenCast",
    default_path = "/org/otto/ScreenCast"
)]
pub trait ScreenCast {
    /// Create a session; answers its object path. `cursor-mode` follows the
    /// portal's values: `1` hidden, `2` embedded, `4` metadata.
    fn create_session(&self, properties: HashMap<&str, Value<'_>>)
        -> zbus::Result<OwnedObjectPath>;

    /// The connectors a session may record.
    fn list_outputs(&self) -> zbus::Result<Vec<String>>;

    /// The capturable windows, as `(identifier, app_id, title)`.
    fn list_windows(&self) -> zbus::Result<Vec<(String, String, String)>>;
}

#[zbus::proxy(
    interface = "org.otto.ScreenCast.Session",
    default_service = "org.otto.ScreenCast"
)]
pub trait ScreenCastSession {
    /// Record a monitor by connector name; answers the stream's object path.
    fn record_monitor(
        &self,
        connector: &str,
        properties: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Record a window (`window-id` in `properties`); answers the stream's
    /// object path.
    fn record_window(&self, properties: HashMap<&str, Value<'_>>) -> zbus::Result<OwnedObjectPath>;

    /// Start every stream; returns once their PipeWire nodes exist.
    fn start(&self) -> zbus::Result<()>;

    /// Stop the session and its streams.
    fn stop(&self) -> zbus::Result<()>;

    /// A PipeWire remote for the session's streams.
    fn open_pipe_wire_remote(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<OwnedFd>;
}

#[zbus::proxy(
    interface = "org.otto.ScreenCast.Stream",
    default_service = "org.otto.ScreenCast"
)]
pub trait ScreenCastStream {
    /// Start this stream.
    fn start(&self) -> zbus::Result<()>;

    /// Stop this stream.
    fn stop(&self) -> zbus::Result<()>;

    /// The PipeWire node, including `node-id`.
    fn pipe_wire_node(&self) -> zbus::Result<HashMap<String, OwnedValue>>;

    /// Static stream metadata (mapping id, geometry, …).
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
}
