//! Whether a newer Otto has been released.
//!
//! Asks the version endpoint a minute after start and every few hours after.
//! Quiet by design: a newer release only adds an entry to the Otto menu, with
//! no notification and nothing drawn in the bar.
//!
//! Installs are counted the way Fedora's "countme" does it: the first check of
//! each week adds `count=1` and a bucket for how old the install is, and the
//! endpoint sums those per week. Which week was last counted, and when the
//! install was first seen, stay in `$XDG_STATE_HOME/otto/update-check`.
//!
//! An empty `update_check_url` in `otto-bar.toml` turns the check, and the
//! request, off.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};

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
    let today = chrono::Utc::now().date_naive();
    let mut state = CountState::load();
    let first_seen = *state.first_seen.get_or_insert(today);
    let week = week_start(today);
    let counting = state.counted_week != Some(week);
    if counting {
        request = request
            .query("count", "1")
            .query("age", age_bucket(first_seen, today).to_string());
    }
    let mut response = request.call().map_err(|e| e.to_string())?;
    // Only a request that arrived counts, so a failed one is retried later.
    if counting {
        state.counted_week = Some(week);
    }
    state.save();
    let body = response
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

/// What the weekly count needs to remember, kept locally as `key=date` lines.
#[derive(Debug, Default, PartialEq)]
struct CountState {
    first_seen: Option<NaiveDate>,
    counted_week: Option<NaiveDate>,
}

impl CountState {
    fn path() -> Option<std::path::PathBuf> {
        Some(
            otto_kit::xdg::state_home()?
                .join("otto")
                .join("update-check"),
        )
    }

    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let mut state = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok();
            match key.trim() {
                "first_seen" => state.first_seen = date,
                "counted_week" => state.counted_week = date,
                _ => {}
            }
        }
        state
    }

    fn save(&self) {
        let Some(path) = Self::path() else { return };
        let mut text = String::new();
        if let Some(d) = self.first_seen {
            text.push_str(&format!("first_seen={d}\n"));
        }
        if let Some(d) = self.counted_week {
            text.push_str(&format!("counted_week={d}\n"));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, text) {
            tracing::debug!("{}: {e}", path.display());
        }
    }
}

/// Monday of the week, the same day the endpoint files the count under.
fn week_start(day: NaiveDate) -> NaiveDate {
    day - chrono::Days::new(u64::from(day.weekday().num_days_from_monday()))
}

/// How long this install has existed: 1 its first week, 2 its first month,
/// 3 its first six months, 4 after that. Fedora's buckets.
fn age_bucket(first_seen: NaiveDate, today: NaiveDate) -> u8 {
    match (today - first_seen).num_days() {
        ..7 => 1,
        7..31 => 2,
        31..183 => 3,
        _ => 4,
    }
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
    use super::{age_bucket, is_newer, week_start, CountState, NaiveDate};

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(week_start(day("2026-10-17")), day("2026-10-12"));
        assert_eq!(week_start(day("2026-10-12")), day("2026-10-12"));
        assert_eq!(week_start(day("2026-10-18")), day("2026-10-12"));
    }

    #[test]
    fn age_follows_fedoras_buckets() {
        let first = day("2026-01-01");
        assert_eq!(age_bucket(first, day("2026-01-07")), 1);
        assert_eq!(age_bucket(first, day("2026-01-08")), 2);
        assert_eq!(age_bucket(first, day("2026-03-01")), 3);
        assert_eq!(age_bucket(first, day("2026-12-01")), 4);
    }

    #[test]
    fn state_reads_back_and_ignores_junk() {
        let state =
            CountState::parse("first_seen=2026-10-01\ncounted_week=2026-10-12\nnoise\nx=y\n");
        assert_eq!(state.first_seen, Some(day("2026-10-01")));
        assert_eq!(state.counted_week, Some(day("2026-10-12")));
        assert_eq!(CountState::parse("garbage"), CountState::default());
    }

    #[test]
    fn compares_numerically() {
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(is_newer("v1.7.2", "1.7.1"));
        assert!(!is_newer("1.7.1", "1.7.1"));
        assert!(!is_newer("1.7.0", "1.7.1"));
        assert!(!is_newer("1.7.1-rc1", "1.7.1"));
    }
}
