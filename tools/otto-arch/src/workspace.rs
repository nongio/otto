//! The workspace crates, their declared dependencies and the resolved
//! dependency graph, from `cargo metadata`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    #[serde(default)]
    id: String,
    pub name: String,
    pub manifest_path: PathBuf,
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub targets: Vec<Target>,
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

#[derive(Debug, Deserialize)]
pub struct Target {
    pub kind: Vec<String>,
    pub src_path: PathBuf,
}

impl Package {
    /// The library target's root file, wherever `[lib] path` puts it.
    pub fn lib_root(&self) -> Option<&Path> {
        self.targets
            .iter()
            .find(|t| t.kind.iter().any(|k| k == "lib"))
            .map(|t| t.src_path.as_path())
    }

    /// Whether the package builds a library, of any kind (`lib`, `rlib`,
    /// `cdylib`, `proc-macro`, …): `cargo test --lib` tests each of them.
    pub fn has_library(&self) -> bool {
        const LIBRARY: [&str; 6] = ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];
        self.targets
            .iter()
            .any(|t| t.kind.iter().any(|k| LIBRARY.contains(&k.as_str())))
    }

    /// The directory holding the manifest.
    pub fn dir(&self) -> &Path {
        self.manifest_path.parent().unwrap_or(Path::new(""))
    }
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Debug, Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Debug, Deserialize)]
struct DepKind {
    kind: Option<String>,
}

/// Normal and build edges of the resolved graph, by package name. Two
/// versions of one crate share a node, which can only over-report a path.
#[derive(Debug, Default)]
pub struct Graph {
    pub edges: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    /// The shortest chain of dependencies from `from` to `to`, both ends
    /// included.
    pub fn path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
        let mut queue = VecDeque::from([from]);
        while let Some(at) = queue.pop_front() {
            for next in self.edges.get(at).into_iter().flatten() {
                if next == from || parent.contains_key(next.as_str()) {
                    continue;
                }
                parent.insert(next, at);
                if next == to {
                    let mut chain = vec![to.to_owned()];
                    let mut cur = to;
                    while let Some(p) = parent.get(cur) {
                        chain.push((*p).to_owned());
                        cur = p;
                    }
                    chain.reverse();
                    return Some(chain);
                }
                queue.push_back(next);
            }
        }
        None
    }
}

/// Every workspace member, sorted by name, and the resolved graph. Resolving
/// reads Cargo.lock (`--locked`), so a stale lockfile fails here as it would
/// in any CI build.
pub fn load(root: &Path) -> Result<(Vec<Package>, Graph), String> {
    parse(&metadata_json(root)?)
}

/// [`load`] from `cargo metadata` output already in hand.
pub fn parse(json: &str) -> Result<(Vec<Package>, Graph), String> {
    let md: Metadata =
        serde_json::from_str(json).map_err(|e| format!("parsing cargo metadata: {e}"))?;
    let names: BTreeMap<&str, &str> = md
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();
    let mut graph = Graph::default();
    for node in &md.resolve.nodes {
        let edges = graph
            .edges
            .entry(names[node.id.as_str()].to_owned())
            .or_default();
        for dep in &node.deps {
            if dep
                .dep_kinds
                .iter()
                .any(|k| k.kind.as_deref() != Some("dev"))
            {
                edges.insert(names[dep.pkg.as_str()].to_owned());
            }
        }
    }
    let members: BTreeSet<&str> = md.workspace_members.iter().map(String::as_str).collect();
    let mut packages: Vec<Package> = md
        .packages
        .into_iter()
        .filter(|p| members.contains(p.id.as_str()))
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((packages, graph))
}

/// The full `cargo metadata` of the workspace at `root`, resolve graph
/// included. `--locked`: a lockfile out of step with the manifests fails.
pub fn metadata_json(root: &Path) -> Result<String, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--locked",
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
    String::from_utf8(out.stdout).map_err(|e| format!("cargo metadata output: {e}"))
}
