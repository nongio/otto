use anyhow::{Context, Result};
use std::collections::HashMap;
use tracing::info;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const SERVICE: &str = "org.otto.ScreenCast";
const PATH: &str = "/org/otto/ScreenCast";

type Proxy<'a> = zbus::Proxy<'a>;

pub struct ScreenshareSession {
    connection: Connection,
    session_path: OwnedObjectPath,
    pub node_id: u32,
}

impl ScreenshareSession {
    pub async fn stop(&self) -> Result<()> {
        let proxy: Proxy<'_> = zbus::proxy::Builder::new(&self.connection)
            .interface("org.otto.ScreenCast.Session")?
            .path(&self.session_path)?
            .destination(SERVICE)?
            .build()
            .await?;

        proxy.call_method("Stop", &()).await?;
        info!("Screenshare session stopped");
        Ok(())
    }
}

pub async fn list_outputs() -> Result<Vec<String>> {
    let connection = Connection::session().await?;

    let proxy: Proxy<'_> = zbus::proxy::Builder::new(&connection)
        .interface(SERVICE)?
        .path(PATH)?
        .destination(SERVICE)?
        .build()
        .await?;

    let reply = proxy.call_method("ListOutputs", &()).await?;
    let outputs: Vec<String> = reply.body().deserialize()?;
    Ok(outputs)
}

pub async fn start_recording(connector: &str) -> Result<ScreenshareSession> {
    let connection = Connection::session().await?;

    let screencast: Proxy<'_> = zbus::proxy::Builder::new(&connection)
        .interface(SERVICE)?
        .path(PATH)?
        .destination(SERVICE)?
        .build()
        .await?;

    // Create session
    let props: HashMap<&str, Value<'_>> = HashMap::new();
    let reply = screencast
        .call_method("CreateSession", &(props,))
        .await?;
    let session_path: OwnedObjectPath = reply.body().deserialize()?;
    info!("Session: {}", session_path);

    // Record monitor
    let session_proxy: Proxy<'_> = zbus::proxy::Builder::new(&connection)
        .interface("org.otto.ScreenCast.Session")?
        .path(&session_path)?
        .destination(SERVICE)?
        .build()
        .await?;

    let stream_props: HashMap<&str, Value<'_>> = HashMap::new();
    let reply = session_proxy
        .call_method("RecordMonitor", &(connector, stream_props))
        .await?;
    let stream_path: OwnedObjectPath = reply.body().deserialize()?;
    info!("Stream: {}", stream_path);

    // Start session (creates PipeWire streams)
    session_proxy.call_method("Start", &()).await?;
    info!("Session started");

    // Get PipeWire node ID
    let stream_proxy: Proxy<'_> = zbus::proxy::Builder::new(&connection)
        .interface("org.otto.ScreenCast.Stream")?
        .path(&stream_path)?
        .destination(SERVICE)?
        .build()
        .await?;

    let reply = stream_proxy.call_method("PipeWireNode", &()).await?;
    let node_info: HashMap<String, OwnedValue> = reply.body().deserialize()?;
    let node_id: u32 = node_info
        .get("node-id")
        .context("No node-id in PipeWire node info")?
        .try_into()
        .map_err(|e| anyhow::anyhow!("Invalid node-id: {}", e))?;

    info!("PipeWire node ID: {}", node_id);

    Ok(ScreenshareSession {
        connection,
        session_path,
        node_id,
    })
}
