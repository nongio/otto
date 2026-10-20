//! What the running compositor says about itself, over D-Bus.
//!
//! Reads `org.otto.Compositor.DebugInfo` (the contract is
//! `docs/developer/compositor-dbus-api.md`). A report is still written when
//! Otto is not running: the sections that need it say so.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::time::Duration;

use otto_dbus::compositor::CompositorProxyBlocking;
use serde_json::Value;

/// How long to wait for the compositor to answer.
///
/// A healthy compositor answers in milliseconds. A wedged one never does, and
/// a bug report about a frozen session is exactly when this tool runs, so the
/// wait stays short rather than zbus's default of 25 seconds.
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// The renderer the session draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renderer {
    /// `gl` or `vulkan`.
    pub api: String,
    /// Driver vendor string.
    pub vendor: String,
    /// Device or renderer name.
    pub renderer: String,
    /// API and driver version.
    pub version: String,
}

/// One output as the compositor sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// Connector name, such as `eDP-1`.
    pub name: String,
    /// Manufacturer from the EDID.
    pub make: String,
    /// Model from the EDID.
    pub model: String,
    /// Current mode width in physical pixels.
    pub width: i64,
    /// Current mode height in physical pixels.
    pub height: i64,
    /// Refresh rate in millihertz.
    pub refresh_mhz: i64,
    /// Fractional scale.
    pub scale: f64,
    /// Position in the layout, in logical points.
    pub x: i64,
    /// Position in the layout, in logical points.
    pub y: i64,
    /// A PipeWire-backed output rather than a monitor.
    pub is_virtual: bool,
}

/// The compositor's answer to `DebugInfo`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugInfo {
    /// Otto's version.
    pub version: String,
    /// The commit it was built from, when the build recorded one.
    pub git_commit: Option<String>,
    /// `udev`, `winit`, `x11` or `headless`.
    pub backend: String,
    /// The renderer, once one has come up.
    pub renderer: Option<Renderer>,
    /// Why Vulkan was asked for but OpenGL is in use.
    pub vulkan_fallback: Option<String>,
    /// The configuration files the compositor reads, lowest priority first.
    pub config_files: Vec<PathBuf>,
    /// The file the compositor logs to, if it logs to one.
    pub log_path: Option<PathBuf>,
    /// Every output, sorted by name.
    pub outputs: Vec<Output>,
}

impl DebugInfo {
    /// Parses the JSON `DebugInfo` returns.
    ///
    /// Fields that are missing or of the wrong type take their defaults, so a
    /// newer or older compositor still yields what it has.
    ///
    /// # Errors
    ///
    /// Fails when `json` is not a JSON object.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if !value.is_object() {
            return Err("not a JSON object".into());
        }
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        let renderer = value["renderer"].as_object().map(|r| Renderer {
            api: r.get("api").map(text).unwrap_or_default(),
            vendor: r.get("vendor").map(text).unwrap_or_default(),
            renderer: r.get("renderer").map(text).unwrap_or_default(),
            version: r.get("version").map(text).unwrap_or_default(),
        });
        let outputs = value["outputs"]
            .as_array()
            .map(|outputs| {
                outputs
                    .iter()
                    .map(|o| Output {
                        name: text(&o["name"]),
                        make: text(&o["make"]),
                        model: text(&o["model"]),
                        width: o["width"].as_i64().unwrap_or_default(),
                        height: o["height"].as_i64().unwrap_or_default(),
                        refresh_mhz: o["refresh_mhz"].as_i64().unwrap_or_default(),
                        scale: o["scale"].as_f64().unwrap_or(1.0),
                        x: o["x"].as_i64().unwrap_or_default(),
                        y: o["y"].as_i64().unwrap_or_default(),
                        is_virtual: o["virtual"].as_bool().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let config_files = value["config_files"]
            .as_array()
            .map(|files| {
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            version: text(&value["version"]),
            git_commit: value["git_commit"].as_str().map(str::to_owned),
            backend: text(&value["backend"]),
            renderer,
            vulkan_fallback: value["vulkan_fallback"].as_str().map(str::to_owned),
            config_files,
            log_path: value["log_path"].as_str().map(PathBuf::from),
            outputs,
        })
    }
}

/// Asks the compositor on the session bus for its debug info.
///
/// # Errors
///
/// Returns the reason, worded for the report, when there is no session bus,
/// Otto is not running, it does not answer in time, or it predates
/// `DebugInfo`.
pub fn query() -> Result<DebugInfo, String> {
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.method_timeout(REPLY_TIMEOUT).build())
        .map_err(|e| format!("not reachable, no session bus ({e})"))?;
    let proxy =
        CompositorProxyBlocking::new(&connection).map_err(|e| format!("not reachable ({e})"))?;
    match proxy.debug_info() {
        Ok(json) => {
            DebugInfo::from_json(&json).map_err(|e| format!("gave an unreadable answer ({e})"))
        }
        Err(e) if is_unknown_method(&e) => {
            Err("is running but too old to report this (no DebugInfo method)".into())
        }
        Err(e) => Err(format!("not reachable ({e})")),
    }
}

/// Whether `error` says the method does not exist: an Otto from before
/// `DebugInfo`.
fn is_unknown_method(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => {
            name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
        }
        zbus::Error::FDO(fdo) => matches!(**fdo, zbus::fdo::Error::UnknownMethod(_)),
        _ => false,
    }
}
