//! Whether a newer Otto has been released.
//!
//! Asks the version endpoint a minute after start and every few hours after.
//! Quiet by design: a newer release only adds an entry to the Otto menu, with
//! no notification and nothing drawn in the bar.
//!
//! Each request carries a random install id kept in
//! `$XDG_STATE_HOME/otto/install-id` and the running version, so releases can
//! be counted by unique installs. An empty `update_check_url` in
//! `otto-bar.toml` turns the check, and the request, off.

use std::sync::Mutex;
use std::time::Duration;

use crate::config::update_check_url;

const FIRST_CHECK: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RELEASES_PAGE: &str = "https://github.com/nongio/otto/releases/latest";

#[derive(Debug, Clone)]
pub struct Release {
    pub url: String,
}

static AVAILABLE: Mutex<Option<Release>> = Mutex::new(None);

/// The newer release, if the last check found one.
pub fn available() -> Option<Release> {
    AVAILABLE.lock().ok()?.clone()
}

/// Start checking in the background. Does nothing when the URL is empty.
pub fn start() {
    let url = update_check_url();
    if url.is_empty() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            match tokio::task::spawn_blocking(move || check(url)).await {
                Ok(Ok(found)) => {
                    if let Ok(mut slot) = AVAILABLE.lock() {
                        *slot = found;
                    }
                }
                Ok(Err(e)) => tracing::debug!("update check: {e}"),
                Err(e) => tracing::debug!("update check task: {e}"),
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

/// Open the release page in the browser.
pub fn open(release: &Release) {
    let spawned = std::process::Command::new("xdg-open")
        .arg(&release.url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!("xdg-open {}: {e}", release.url),
    }
}

fn check(url: &str) -> Result<Option<Release>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut request = agent
        .get(url)
        .header(
            "user-agent",
            concat!("otto-bar/", env!("CARGO_PKG_VERSION")),
        )
        .query("v", current);
    if let Some(id) = install_id() {
        request = request.query("id", id);
    }
    let body = request
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(16 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or("no version in response")?;
    if !is_newer(version, current) {
        return Ok(None);
    }
    // Only ever hand xdg-open a GitHub page, whatever the endpoint says.
    let url = json
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or(RELEASES_PAGE);
    Ok(Some(Release {
        url: url.to_string(),
    }))
}

/// The install's random id, created on first use. None when the state
/// directory can't be written, and the check goes out without one.
fn install_id() -> Option<String> {
    let path = otto_kit::xdg::state_home()?.join("otto").join("install-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, &id).ok()?;
    Some(id)
}

/// Whether `latest` is a later release than `current`, comparing the dotted
/// numbers and ignoring any pre-release or build suffix.
fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    parts(latest) > parts(current)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_numerically() {
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(is_newer("v1.7.2", "1.7.1"));
        assert!(!is_newer("1.7.1", "1.7.1"));
        assert!(!is_newer("1.7.0", "1.7.1"));
        assert!(!is_newer("1.7.1-rc1", "1.7.1"));
    }
}
