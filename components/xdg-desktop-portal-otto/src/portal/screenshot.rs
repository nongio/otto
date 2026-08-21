//! `org.freedesktop.impl.portal.Screenshot` backend.
//!
//! Capture goes through `grim`, which already talks to Otto's wlr-screencopy
//! support directly — no new compositor-side capture path needed. Interactive
//! requests are gated by a confirmation dialog, reusing the same renderer
//! [`AccessPortal`](crate::portal::AccessPortal) brokers to.

use std::collections::HashMap;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{info, warn};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

use crate::otto_client::OttoClient;

pub struct ScreenshotPortal {
    client: OttoClient,
}

impl ScreenshotPortal {
    pub fn new(client: OttoClient) -> Self {
        Self { client }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl ScreenshotPortal {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    /// Capture the screen and return a `file://` URI in `results["uri"]`.
    /// `response`: `0` success, `1` cancelled, `2` failed.
    async fn screenshot(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, "Screenshot requested");

        let interactive = options
            .get("interactive")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false);

        if interactive {
            let proxy = match self.client.dialog_proxy().await {
                Ok(p) => p,
                Err(err) => {
                    warn!(?err, "no dialog renderer available; denying screenshot");
                    return (1, HashMap::new());
                }
            };
            let body = format!("{app_id} wants to take a screenshot of your screen.");
            match proxy
                .present_access(
                    &app_id,
                    "Take Screenshot",
                    "",
                    &body,
                    "",
                    "Take Screenshot",
                    "Cancel",
                    true,
                    Vec::new(),
                )
                .await
            {
                Ok((0, _)) => {}
                Ok((response, _)) => return (response.max(1), HashMap::new()),
                Err(err) => {
                    warn!(?err, "dialog renderer call failed; denying screenshot");
                    return (1, HashMap::new());
                }
            }
        }

        match capture_to_file().await {
            Ok(path) => {
                let uri = format!("file://{path}");
                let mut results = HashMap::new();
                if let Ok(v) = OwnedValue::try_from(Value::from(Str::from(uri))) {
                    results.insert("uri".to_string(), v);
                }
                info!(?app_id, "Screenshot captured");
                (0, results)
            }
            Err(err) => {
                warn!(?err, "screenshot capture failed");
                (2, HashMap::new())
            }
        }
    }
}

/// Runs `grim` to capture the full output set to a fresh PNG under
/// `~/Pictures/Screenshots/`, returning the absolute path.
///
/// Blocks the calling (zbus executor) thread for the ~100ms `grim` takes —
/// not a Tokio task, so `spawn_blocking` isn't available here.
async fn capture_to_file() -> anyhow::Result<String> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let dir = format!("{home}/Pictures/Screenshots");
    std::fs::create_dir_all(&dir)?;

    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = format!("{dir}/screenshot-{ts}.png");

    let status = Command::new("grim").arg(&path).status()?;
    if !status.success() {
        anyhow::bail!("grim exited with {status}");
    }
    Ok(path)
}
