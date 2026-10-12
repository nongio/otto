//! The shape of `architecture.toml`.

use std::path::Path;

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
/// they reach. `otto-arch affected` reads these to decide what a change
/// needs checked.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Affects {
    /// Repository paths, anchored at the root: `dir/` is everything under a
    /// directory, anything else a file. `*` matches within one path segment
    /// and `**/` any number of directories.
    pub paths: Vec<String>,
    /// Paths the rule leaves out, in the same syntax.
    #[serde(default)]
    pub except: Vec<String>,
    /// Workspace members a change to one of these files affects, before the
    /// closure over their dependents; `"*"` is every member, `[]` none.
    pub crates: Vec<String>,
    /// CI areas outside the crate graph the files reach (see [`AREAS`]).
    #[serde(default)]
    pub areas: Vec<String>,
    pub reason: String,
}

/// The CI areas an `[[affects]]` rule may name.
pub const AREAS: &[&str] = &["packaging"];

impl Affects {
    pub fn matches(&self, path: &str) -> bool {
        self.paths.iter().any(|p| path_matches(p, path))
            && !self.except.iter().any(|p| path_matches(p, path))
    }
}

/// `pattern` against a repository-relative `path`; see [`Affects::paths`].
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let mut re = String::from("^");
    let mut rest = pattern;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**/") {
            re.push_str("(?:[^/]+/)*");
            rest = r;
        } else if let Some(r) = rest.strip_prefix('*') {
            re.push_str("[^/]*");
            rest = r;
        } else {
            let c = rest.chars().next().unwrap();
            re.push_str(&regex::escape(c.encode_utf8(&mut [0; 4])));
            rest = &rest[c.len_utf8()..];
        }
    }
    if pattern.ends_with('/') {
        re.push_str(".*");
    }
    re.push('$');
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(path))
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
                "an [[affects]] entry needs paths and a reason: {a:?}"
            ));
        }
        if let Some(area) = a.areas.iter().find(|x| !AREAS.contains(&x.as_str())) {
            return Err(format!(
                "[[affects]] {:?} names unknown area {area:?}; known: {AREAS:?}",
                a.paths
            ));
        }
    }
    Ok(rules)
}
