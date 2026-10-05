//! Drive Otto's `org.otto.ScreenCast` D-Bus service to capture a physical
//! output (connector), returning a PipeWire node id the capture thread can
//! consume — the same node the built-in screenshare portal hands to clients.
//!
//! Flow: CreateSession → Session.RecordMonitor(connector) → Session.Start
//! (starts PipeWire and resolves the node id) → Stream.PipeWireNode.

use std::collections::HashMap;

use anyhow::{anyhow, Context};
use otto_dbus::screencast::{ScreenCastProxy, ScreenCastSessionProxy, ScreenCastStreamProxy};
use zbus::zvariant::Value;
use zbus::Connection;

/// Resolve a PipeWire node id for the given output connector (e.g. "eDP-1").
pub async fn node_for_connector(connector: &str) -> anyhow::Result<u32> {
    let conn = Connection::session()
        .await
        .context("connecting to the session bus")?;

    // CreateSession(a{sv}) → session object path. Cursor modes follow the
    // xdg portal values: 1 = hidden, 2 = embedded, 4 = metadata. RDP has no
    // cursor side-channel wired, so ask for the cursor baked into the video.
    let mut session_props: HashMap<&str, Value> = HashMap::new();
    session_props.insert("cursor-mode", Value::U32(2));
    let session = ScreenCastProxy::new(&conn)
        .await?
        .create_session(session_props)
        .await
        .context("CreateSession — is Otto running with screenshare?")?;
    let session = ScreenCastSessionProxy::builder(&conn)
        .path(session)?
        .build()
        .await?;

    // Session.RecordMonitor(s, a{sv}) → stream object path. The stream has
    // its own cursor-mode that defaults to hidden — and it, not the session
    // one, is what the compositor's blit consults. Ask embedded here too.
    let mut mon_props: HashMap<&str, Value> = HashMap::new();
    mon_props.insert("cursor-mode", Value::U32(2));
    let stream = session
        .record_monitor(connector, mon_props)
        .await
        .with_context(|| format!("RecordMonitor({connector}) — unknown connector?"))?;

    // Session.Start() blocks until the PipeWire node id is resolved.
    session.start().await.context("Session.Start")?;

    // Stream.PipeWireNode() → { "node-id": u32, ... }.
    let meta = ScreenCastStreamProxy::builder(&conn)
        .path(stream)?
        .build()
        .await?
        .pipe_wire_node()
        .await
        .context("Stream.PipeWireNode")?;

    let node = meta
        .get("node-id")
        .ok_or_else(|| anyhow!("PipeWireNode reply missing node-id"))?;
    u32::try_from(node).context("node-id was not a u32")
}
