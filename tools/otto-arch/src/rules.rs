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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    pub description: String,
    pub crates: Vec<String>,
    pub may_use: Vec<String>,
    #[serde(default)]
    pub forbid_deps: Vec<String>,
    /// Lints every crate's `src/lib.rs` turns on (`#![warn(..)]`, `deny` or
    /// `forbid`).
    #[serde(default)]
    pub lib_lints: Vec<String>,
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
    pub files: Vec<String>,
    pub reason: String,
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
                .map(|f| Exception::Source { rule: r.clone(), file: f.clone() })
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
    Ok(rules)
}
