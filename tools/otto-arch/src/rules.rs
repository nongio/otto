//! The shape of `architecture.toml`.

use std::path::Path;
use std::sync::OnceLock;

use globset::{Glob, GlobSet, GlobSetBuilder};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    #[serde(rename = "layer")]
    pub layers: Vec<Layer>,
    #[serde(default, rename = "module_rule")]
    pub module_rules: Vec<ModuleRule>,
    #[serde(default, rename = "source_rule")]
    pub source_rules: Vec<SourceRule>,
    #[serde(default, rename = "allow")]
    pub allows: Vec<Allow>,
    #[serde(default, rename = "affects")]
    pub affects: Vec<Affects>,
}

/// Files a crate's directory does not hold, and the crates (or CI areas)
/// they reach. `otto-arch affected` translates these into determinator path
/// rules (see `affected::determinator_rules`) to decide what a change needs
/// checked.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Affects {
    /// Repository paths, anchored at the root: `dir/` is everything under a
    /// directory, anything else a globset glob, matched as determinator
    /// matches it: `*` matches any run of characters, `/` included, so
    /// `docs/*.md` also matches `docs/a/b.md`.
    pub paths: Vec<String>,
    /// Workspace members a change to one of these files affects, before the
    /// closure over their dependents; `"*"` is every member, `[]` none.
    pub crates: Vec<String>,
    /// CI areas outside the crate graph the files reach (see [`AREAS`]).
    #[serde(default)]
    pub areas: Vec<String>,
    pub reason: String,
    #[serde(skip)]
    set: OnceLock<GlobSet>,
}

/// The CI areas an `[[affects]]` rule may name.
pub const AREAS: &[&str] = &["packaging"];

impl Affects {
    /// The rule's paths as globset globs: `dir/` becomes `dir/**`.
    pub fn globs(&self) -> Vec<String> {
        self.paths.iter().map(|p| glob(p)).collect()
    }

    pub fn matches(&self, path: &str) -> bool {
        self.set
            .get_or_init(|| glob_set(&self.globs()).unwrap_or_else(|_| GlobSet::empty()))
            .is_match(path)
    }

    pub fn names_every_crate(&self) -> bool {
        self.crates.iter().any(|c| c == "*")
    }
}

/// An `[[affects]]` path as a globset glob.
pub fn glob(path: &str) -> String {
    if path.ends_with('/') {
        format!("{path}**")
    } else {
        path.to_owned()
    }
}

/// `globs` as one set, built the way determinator builds a path rule's
/// (`Glob::new`, default options), so a pattern matches the same files here
/// as in the selection.
pub fn glob_set(globs: &[String]) -> Result<GlobSet, String> {
    let mut b = GlobSetBuilder::new();
    for g in globs {
        b.add(Glob::new(g).map_err(|e| format!("glob {g:?}: {e}"))?);
    }
    b.build().map_err(|e| e.to_string())
}

/// One `[[affects]]` path pattern against a repository-relative `path`.
#[cfg(test)]
pub fn path_matches(pattern: &str, path: &str) -> bool {
    Glob::new(&glob(pattern)).is_ok_and(|g| g.compile_matcher().is_match(path))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    pub description: String,
    pub crates: Vec<String>,
    pub may_use: Vec<String>,
    /// Third-party crates no crate in the layer may pull in, directly or
    /// through any other dependency.
    #[serde(default)]
    pub forbid_deps: Vec<String>,
    /// Lints every crate's `src/lib.rs` turns on (`#![warn(..)]`, `deny` or
    /// `forbid`).
    #[serde(default)]
    pub lib_lints: Vec<String>,
    /// Drawn in the generated crate graph.
    #[serde(default = "yes")]
    pub graph: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleRule {
    pub name: String,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRule {
    pub name: String,
    pub pattern: String,
    pub allowed_in: Vec<String>,
    pub reason: String,
}

/// One known violation. Exactly one of the three shapes:
/// `crate` + `dep`, `module` + `uses`, or `rule` + `files`.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Allow {
    #[serde(rename = "crate")]
    pub krate: Option<String>,
    pub dep: Option<String>,
    pub module: Option<String>,
    pub uses: Option<String>,
    pub rule: Option<String>,
    #[serde(default)]
    pub files: Vec<FileCount>,
    pub reason: String,
}

/// A file that still holds `count` matches of a source rule. The count makes
/// a new match in the same file fail, and a fixed one shrink the entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileCount {
    pub path: String,
    pub count: usize,
}

/// What an [`Allow`] excuses, as a key the checks compare against.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Exception {
    Crate { krate: String, dep: String },
    Module { module: String, uses: String },
    Source { rule: String, file: String },
}

impl std::fmt::Display for Exception {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exception::Crate { krate, dep } => write!(f, "crate = {krate:?}, dep = {dep:?}"),
            Exception::Module { module, uses } => {
                write!(f, "module = {module:?}, uses = {uses:?}")
            }
            Exception::Source { rule, file } => write!(f, "rule = {rule:?}, file = {file:?}"),
        }
    }
}

impl Allow {
    fn exceptions(&self) -> Result<Vec<Exception>, String> {
        let shape = (
            &self.krate,
            &self.dep,
            &self.module,
            &self.uses,
            &self.rule,
            self.files.is_empty(),
        );
        match shape {
            (Some(k), Some(d), None, None, None, true) => {
                Ok(vec![Exception::Crate { krate: k.clone(), dep: d.clone() }])
            }
            (None, None, Some(m), Some(u), None, true) => {
                Ok(vec![Exception::Module { module: m.clone(), uses: u.clone() }])
            }
            (None, None, None, None, Some(r), false) => Ok(self
                .files
                .iter()
                .map(|f| Exception::Source { rule: r.clone(), file: f.path.clone() })
                .collect()),
            _ => Err(format!(
                "an [[allow]] entry needs exactly one of crate+dep, module+uses or rule+files: {self:?}"
            )),
        }
    }
}

impl Rules {
    pub fn exceptions(&self) -> Result<Vec<Exception>, String> {
        let mut out = Vec::new();
        for a in &self.allows {
            if a.reason.trim().is_empty() {
                return Err(format!("an [[allow]] entry has no reason: {a:?}"));
            }
            for e in a.exceptions()? {
                if out.contains(&e) {
                    return Err(format!("[[allow]] {e} is listed twice"));
                }
                out.push(e);
            }
        }
        Ok(out)
    }

    /// How many matches of `rule` an [[allow]] entry grants `file`.
    pub fn allowed_count(&self, rule: &str, file: &str) -> Option<usize> {
        self.allows
            .iter()
            .filter(|a| a.rule.as_deref() == Some(rule))
            .flat_map(|a| &a.files)
            .find(|f| f.path == file)
            .map(|f| f.count)
    }

    pub fn layer_of(&self, krate: &str) -> Option<&Layer> {
        self.layers
            .iter()
            .find(|l| l.crates.iter().any(|c| c == krate))
    }
}

pub fn load(path: &Path) -> Result<Rules, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let rules: Rules = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for l in &rules.layers {
        for used in &l.may_use {
            if !rules.layers.iter().any(|o| &o.name == used) {
                return Err(format!("layer {:?} may_use unknown layer {used:?}", l.name));
            }
        }
    }
    for a in &rules.affects {
        if a.reason.trim().is_empty() || a.paths.is_empty() {
            return Err(format!(
                "an [[affects]] entry needs paths and a reason: {:?}",
                a.paths
            ));
        }
        glob_set(&a.globs()).map_err(|e| format!("[[affects]] {:?}: {e}", a.paths))?;
        if let Some(area) = a.areas.iter().find(|x| !AREAS.contains(&x.as_str())) {
            return Err(format!(
                "[[affects]] {:?} names unknown area {area:?}; known: {AREAS:?}",
                a.paths
            ));
        }
    }
    Ok(rules)
}
