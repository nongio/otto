//! A screen region, picked with `slurp` and captured with `grim`, as the
//! `shot` helper does.

// Rust guideline compliant 2026-02-21

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

/// Let the user drag out a region and save it as a PNG in `dir`.
///
/// Blocks until the region is picked and captured. Returns `None` when the
/// pick is cancelled (Escape, or a click without a drag).
///
/// # Errors
///
/// When `slurp` or `grim` is missing or fails, or `dir` can't be made.
pub fn capture(dir: &Path) -> anyhow::Result<Option<PathBuf>> {
    let picked = Command::new("slurp")
        .stderr(Stdio::null())
        .output()
        .context("cannot run slurp")?;
    // slurp exits with an error when the pick is cancelled.
    if !picked.status.success() {
        return Ok(None);
    }
    let geometry = String::from_utf8_lossy(&picked.stdout).trim().to_owned();
    if geometry.is_empty() {
        return Ok(None);
    }

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot make {}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("region-{stamp}.png"));
    let status = Command::new("grim")
        .args(["-t", "png", "-g", &geometry])
        .arg(&path)
        .status()
        .context("cannot run grim")?;
    anyhow::ensure!(status.success(), "grim failed with {status}");
    Ok(Some(path))
}
