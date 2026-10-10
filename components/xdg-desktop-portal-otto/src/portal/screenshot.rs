//! `org.freedesktop.impl.portal.Screenshot` backend.
//!
//! Capture goes through `grim`, which already talks to Otto's wlr-screencopy
//! support directly — no new compositor-side capture path needed. Interactive
//! requests are gated by a confirmation dialog, reusing the same renderer
//! [`AccessPortal`](crate::portal::AccessPortal) brokers to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use otto_foundations::uri::path_to_uri;
use otto_foundations::xdg;
use tokio::process::Command;
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
                let uri = path_to_uri(&path);
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

/// Runs `grim` to capture the full output set to a fresh PNG under the
/// user's Pictures folder, in `Screenshots/`, returning the absolute path.
///
/// zbus runs on Tokio here, so `grim` is awaited as a child process rather
/// than blocking the executor for the ~100ms it takes.
async fn capture_to_file() -> anyhow::Result<PathBuf> {
    let dir = pictures_dir()?.join("Screenshots");
    tokio::fs::create_dir_all(&dir).await?;

    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = dir.join(format!("screenshot-{ts}.png"));

    let status = Command::new("grim").arg(&path).status().await?;
    if !status.success() {
        anyhow::bail!("grim exited with {status}");
    }
    Ok(path)
}

/// `XDG_PICTURES_DIR` from `user-dirs.dirs`, or `~/Pictures`.
fn pictures_dir() -> anyhow::Result<PathBuf> {
    let home = xdg::home().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(xdg::user_dir("XDG_PICTURES_DIR").unwrap_or_else(|| home.join("Pictures")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The escaping is the toolkit's, so the URI an app is handed opens
    /// the file it names.
    #[test]
    fn a_path_with_spaces_and_non_ascii_is_percent_encoded() {
        assert_eq!(
            path_to_uri(Path::new("/home/me/Immagini/Schermate 2026/caffè.png")),
            "file:///home/me/Immagini/Schermate%202026/caff%C3%A8.png"
        );
    }
}
