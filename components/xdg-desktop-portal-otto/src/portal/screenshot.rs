//! `org.freedesktop.impl.portal.Screenshot` backend.
//!
//! Capture goes through `grim`, which already talks to Otto's wlr-screencopy
//! support directly — no new compositor-side capture path needed. Requests are
//! gated by a confirmation dialog, reusing the same renderer
//! [`AccessPortal`](crate::portal::AccessPortal) brokers to, unless the
//! frontend says it already checked the app's `screenshot` permission and the
//! app did not ask for an interactive capture (see [`needs_confirmation`]).

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};
use tokio::process::Command;
use tracing::{info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};
use zbus::{interface, ObjectServer};

use crate::otto_client::OttoClient;
use crate::portal::Request;

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
        handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] object_server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        info!(?app_id, "Screenshot requested");

        // Exported for the length of the call so the frontend can close it
        // when the app withdraws.
        let request = Request::new(handle.clone());
        let cancellation = request.cancellation();
        let exported = object_server
            .at(handle.clone(), request)
            .await
            .unwrap_or_else(|err| {
                warn!(?err, %handle, "could not export the request object");
                false
            });

        let result = cancellation
            .run(self.take(&app_id, &options))
            .await
            .unwrap_or_else(|| {
                info!(?app_id, "Screenshot request closed by the frontend");
                (1, HashMap::new())
            });

        if exported {
            if let Err(err) = object_server.remove::<Request, _>(&handle).await {
                warn!(?err, %handle, "could not remove the request object");
            }
        }
        result
    }
}

impl ScreenshotPortal {
    async fn take(
        &self,
        app_id: &str,
        options: &HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if needs_confirmation(options) {
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
                    app_id,
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

/// Whether the user has to confirm the capture in the dialog first.
///
/// Per the impl Screenshot spec (version 2), `permission_store_checked` says
/// the frontend already looked up the app's `screenshot` permission and it
/// was granted; absent, it means no. Without it a non-interactive request is
/// just as unvetted as an interactive one, so only a checked, non-interactive
/// request skips the dialog.
fn needs_confirmation(options: &HashMap<String, OwnedValue>) -> bool {
    let flag = |key: &str| {
        options
            .get(key)
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
    };
    flag("interactive") || !flag("permission_store_checked")
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
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    Ok(user_dir(&dirs, "XDG_PICTURES_DIR", &home).unwrap_or_else(|| home.join("Pictures")))
}

/// The folder `variable` names in the text of a `user-dirs.dirs` file,
/// `$HOME` expanded.
fn user_dir(dirs: &str, variable: &str, home: &Path) -> Option<PathBuf> {
    dirs.lines().find_map(|line| {
        let value = line.trim().strip_prefix(variable)?.strip_prefix('=')?;
        let value = value.trim().trim_matches('"');
        let path = match value.strip_prefix("$HOME") {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(value),
        };
        Some(path).filter(|path| path.is_absolute())
    })
}

/// Everything but RFC 3986's unreserved characters and the `/` separator.
const PATH: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// A path as a percent-encoded `file://` URI, escaped the way
/// `otto_kit::uri::path_to_uri` does (the portal does not link the toolkit).
fn path_to_uri(path: &Path) -> String {
    format!(
        "file://{}",
        percent_encode(path.as_os_str().as_bytes(), PATH)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(pairs: &[(&str, bool)]) -> HashMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), OwnedValue::from(*value)))
            .collect()
    }

    #[test]
    fn only_a_checked_non_interactive_request_skips_the_dialog() {
        assert!(needs_confirmation(&options(&[])));
        assert!(needs_confirmation(&options(&[("interactive", false)])));
        assert!(needs_confirmation(&options(&[("interactive", true)])));
        assert!(needs_confirmation(&options(&[(
            "permission_store_checked",
            false
        )])));
        assert!(needs_confirmation(&options(&[
            ("interactive", true),
            ("permission_store_checked", true),
        ])));
        assert!(!needs_confirmation(&options(&[(
            "permission_store_checked",
            true
        )])));
        assert!(!needs_confirmation(&options(&[
            ("interactive", false),
            ("permission_store_checked", true),
        ])));
    }

    #[test]
    fn a_path_with_spaces_and_non_ascii_is_percent_encoded() {
        assert_eq!(
            path_to_uri(Path::new("/home/me/Immagini/Schermate 2026/caffè.png")),
            "file:///home/me/Immagini/Schermate%202026/caff%C3%A8.png"
        );
    }

    #[test]
    fn the_pictures_folder_comes_from_user_dirs() {
        let home = Path::new("/home/me");
        let dirs = "# written by xdg-user-dirs-update\n\
                    XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n\
                    XDG_PICTURES_DIR=\"$HOME/Immagini\"\n";
        assert_eq!(
            user_dir(dirs, "XDG_PICTURES_DIR", home),
            Some(PathBuf::from("/home/me/Immagini"))
        );
        assert_eq!(user_dir("", "XDG_PICTURES_DIR", home), None);
    }
}
