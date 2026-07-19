//! Drive Otto's `org.otto.ScreenCast` D-Bus service to capture a physical
//! output (connector), returning a PipeWire node id the capture thread can
//! consume — the same node the built-in screenshare portal hands to clients.
//!
//! Flow: CreateSession → Session.RecordMonitor(connector) → Session.Start
//! (starts PipeWire and resolves the node id) → Stream.PipeWireNode.

use std::collections::HashMap;

use anyhow::{anyhow, Context};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const SERVICE: &str = "org.otto.ScreenCast";
const ROOT: &str = "/org/otto/ScreenCast";

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
    let session: OwnedObjectPath = conn
        .call_method(
            Some(SERVICE),
            ROOT,
            Some(SERVICE),
            "CreateSession",
            &(session_props,),
        )
        .await
        .context("CreateSession — is Otto running with screenshare?")?
        .body()
        .deserialize()?;

    // Session.RecordMonitor(s, a{sv}) → stream object path. The stream has
    // its own cursor-mode that defaults to hidden — and it, not the session
    // one, is what the compositor's blit consults. Ask embedded here too.
    let mut mon_props: HashMap<&str, Value> = HashMap::new();
    mon_props.insert("cursor-mode", Value::U32(2));
    let stream: OwnedObjectPath = conn
        .call_method(
            Some(SERVICE),
            session.as_str(),
            Some("org.otto.ScreenCast.Session"),
            "RecordMonitor",
            &(connector, mon_props),
        )
        .await
        .with_context(|| format!("RecordMonitor({connector}) — unknown connector?"))?
        .body()
        .deserialize()?;

    // Session.Start() blocks until the PipeWire node id is resolved.
    conn.call_method(
        Some(SERVICE),
        session.as_str(),
        Some("org.otto.ScreenCast.Session"),
        "Start",
        &(),
    )
    .await
    .context("Session.Start")?;

    // Stream.PipeWireNode() → { "node-id": u32, ... }.
    let meta: HashMap<String, OwnedValue> = conn
        .call_method(
            Some(SERVICE),
            stream.as_str(),
            Some("org.otto.ScreenCast.Stream"),
            "PipeWireNode",
            &(),
        )
        .await
        .context("Stream.PipeWireNode")?
        .body()
        .deserialize()?;

    let node = meta
        .get("node-id")
        .ok_or_else(|| anyhow!("PipeWireNode reply missing node-id"))?;
    u32::try_from(node).context("node-id was not a u32")
}
