//! The workspace crates and their declared dependencies, from `cargo metadata`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub manifest_path: PathBuf,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
pub struct Dependency {
    /// The package name, whatever the dependency is renamed to.
    pub name: String,
    /// `None` for a normal dependency, `"dev"` or `"build"` otherwise.
    pub kind: Option<String>,
}

impl Dependency {
    pub fn is_dev(&self) -> bool {
        self.kind.as_deref() == Some("dev")
    }
}

/// Every workspace member, sorted by name. `--no-deps` reads the manifests
/// only, so this needs neither the network nor a build.
pub fn crates(root: &Path) -> Result<Vec<Package>, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("running cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let md: Metadata =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("parsing cargo metadata: {e}"))?;
    let mut packages = md.packages;
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}
