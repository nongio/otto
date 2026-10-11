//! The checks themselves. Each returns problems as ready-to-print text; an
//! [`Exception`] the code no longer needs is a problem too, so the debt list in
//! `architecture.toml` can only shrink.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

use crate::rules::{Exception, Rules};
use crate::workspace::Package;

pub fn all(root: &Path, rules: &Rules, crates: &[Package]) -> Result<Vec<String>, String> {
    let exceptions = rules.exceptions()?;
    let mut used = BTreeSet::new();
    let mut problems = Vec::new();

    placement(rules, crates, &mut problems);
    lib_lints(rules, crates, &mut problems)?;
    crate_deps(rules, crates, &exceptions, &mut used, &mut problems);
    modules(root, rules, &exceptions, &mut used, &mut problems)?;
    sources(root, rules, crates, &exceptions, &mut used, &mut problems)?;

    for e in exceptions.iter().filter(|e| !used.contains(*e)) {
        problems.push(format!(
            "stale [[allow]] {e}\n  The code no longer needs this exception: delete the entry."
        ));
    }
    Ok(problems)
}

/// Every member sits in exactly one layer, and every listed crate exists.
fn placement(rules: &Rules, crates: &[Package], problems: &mut Vec<String>) {
    for c in crates {
        let layers: Vec<_> = rules
            .layers
            .iter()
            .filter(|l| l.crates.contains(&c.name))
            .map(|l| &l.name)
            .collect();
        match layers.len() {
            1 => {}
            0 => problems.push(format!(
                "crate {} is in no layer\n  Add it to the [[layer]] it belongs to in architecture.toml.",
                c.name
            )),
            _ => problems.push(format!("crate {} is in several layers: {layers:?}", c.name)),
        }
    }
    for l in &rules.layers {
        for name in &l.crates {
            if !crates.iter().any(|c| &c.name == name) {
                problems.push(format!(
                    "layer {} lists {name}, which is not a workspace member",
                    l.name
                ));
            }
        }
    }
}

fn crate_deps(
    rules: &Rules,
    crates: &[Package],
    exceptions: &[Exception],
    used: &mut BTreeSet<Exception>,
    problems: &mut Vec<String>,
) {
    for c in crates {
        let Some(layer) = rules.layer_of(&c.name) else {
            continue;
        };
        // A crate can list one dependency several times (per target, per
        // kind); judge each name once.
        let deps: BTreeSet<&str> = c
            .dependencies
            .iter()
            .filter(|d| !d.is_dev())
            .map(|d| d.name.as_str())
            .collect();
        for dep in deps {
            let problem = if let Some(dep_layer) = rules.layer_of(dep) {
                (!layer.may_use.contains(&dep_layer.name)).then(|| {
                    format!(
                        "crate {} ({}) depends on {dep} ({})\n  Layer {} may use: {}.",
                        c.name,
                        layer.name,
                        dep_layer.name,
                        layer.name,
                        list(&layer.may_use)
                    )
                })
            } else {
                layer.forbid_deps.iter().any(|f| f == dep).then(|| {
                    format!(
                        "crate {} ({}) depends on {dep}, which layer {} must stay free of\n  {}",
                        c.name, layer.name, layer.name, layer.description
                    )
                })
            };
            let Some(problem) = problem else { continue };
            let key = Exception::Crate {
                krate: c.name.clone(),
                dep: dep.to_owned(),
            };
            if exceptions.contains(&key) {
                used.insert(key);
            } else {
                problems.push(problem);
            }
        }
    }
}

/// Library layers keep their public API deliberate by turning lints on in
/// lib.rs; `[lints]` cannot, because members inherit the workspace table.
fn lib_lints(rules: &Rules, crates: &[Package], problems: &mut Vec<String>) -> Result<(), String> {
    for c in crates {
        let Some(layer) = rules.layer_of(&c.name) else {
            continue;
        };
        if layer.lib_lints.is_empty() {
            continue;
        }
        let lib = c.manifest_path.parent().unwrap().join("src/lib.rs");
        let text = std::fs::read_to_string(&lib).unwrap_or_default();
        let ast = syn::parse_file(&text).map_err(|e| format!("parsing {}: {e}", lib.display()))?;
        for lint in &layer.lib_lints {
            let on =
                ast.attrs.iter().any(|a| {
                    ["warn", "deny", "forbid"].iter().any(|l| a.path().is_ident(l))
                    && a.parse_args_with(
                        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                    )
                    .is_ok_and(|lints| lints.iter().any(|p| p.is_ident(lint)))
                });
            if !on {
                problems.push(format!(
                    "crate {} ({}) does not turn on {lint}\n  Add `#![warn({lint})]` to its src/lib.rs.",
                    c.name, layer.name
                ));
            }
        }
    }
    Ok(())
}

fn list(items: &[String]) -> String {
    if items.is_empty() {
        "nothing in the workspace".into()
    } else {
        items.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Compositor modules
// ---------------------------------------------------------------------------

/// For each top-level module of `src/`, the top-level modules it names
/// through `crate::`, with one place each is named from.
pub fn module_graph(src: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let mut graph: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for file in rust_files(src)? {
        let rel = file.strip_prefix(src).unwrap();
        let top = rel
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy();
        let top = top.strip_suffix(".rs").unwrap_or(&top).to_owned();
        if top == "lib" || top == "main" {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("reading {}: {e}", file.display()))?;
        let ast = syn::parse_file(&text).map_err(|e| format!("parsing {}: {e}", file.display()))?;
        let mut v = CratePaths::default();
        v.visit_file(&ast);
        let entry = graph.entry(top.clone()).or_default();
        for (target, line) in v.found {
            if target != top {
                entry
                    .entry(target)
                    .or_insert_with(|| format!("src/{}:{line}", rel.display()));
            }
        }
    }
    Ok(graph)
}

/// Collects the first segment after `crate::` in paths and `use` trees.
#[derive(Default)]
struct CratePaths {
    found: Vec<(String, usize)>,
}

impl CratePaths {
    fn use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.push(&p.ident),
            syn::UseTree::Name(n) => self.push(&n.ident),
            syn::UseTree::Rename(r) => self.push(&r.ident),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| self.use_tree(t)),
            syn::UseTree::Glob(_) => {}
        }
    }

    fn push(&mut self, ident: &syn::Ident) {
        self.found
            .push((ident.to_string(), ident.span().start().line));
    }
}

impl<'ast> Visit<'ast> for CratePaths {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if let syn::UseTree::Path(p) = &item.tree {
            if p.ident == "crate" {
                self.use_tree(&p.tree);
            }
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let mut segs = path.segments.iter();
        if let (Some(first), Some(second)) = (segs.next(), segs.next()) {
            if first.ident == "crate" {
                self.push(&second.ident);
            }
        }
        syn::visit::visit_path(self, path);
    }
}

fn modules(
    root: &Path,
    rules: &Rules,
    exceptions: &[Exception],
    used: &mut BTreeSet<Exception>,
    problems: &mut Vec<String>,
) -> Result<(), String> {
    let graph = module_graph(&root.join("src"))?;
    for rule in &rules.module_rules {
        for name in rule.from.iter().chain(&rule.to) {
            if name != "*" && !graph.contains_key(name) {
                problems.push(format!(
                    "module rule {} names {name}, which is not a top-level module of src/",
                    rule.name
                ));
            }
        }
        let wildcard = rule.from.iter().any(|f| f == "*");
        for (module, uses) in &graph {
            let covered = if wildcard {
                !rule.to.contains(module)
            } else {
                rule.from.contains(module)
            };
            if !covered {
                continue;
            }
            for (target, at) in uses {
                if !rule.to.contains(target) {
                    continue;
                }
                let key = Exception::Module {
                    module: module.clone(),
                    uses: target.clone(),
                };
                if exceptions.contains(&key) {
                    used.insert(key);
                } else {
                    problems.push(format!(
                        "module {module} uses {target} ({at}), breaking {}\n  {}",
                        rule.name, rule.reason
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Source patterns
// ---------------------------------------------------------------------------

fn sources(
    root: &Path,
    rules: &Rules,
    crates: &[Package],
    exceptions: &[Exception],
    used: &mut BTreeSet<Exception>,
    problems: &mut Vec<String>,
) -> Result<(), String> {
    if rules.source_rules.is_empty() {
        return Ok(());
    }
    // The compositor's src/ and every component's; sample clients and tools
    // are outside the product.
    let mut files = Vec::new();
    for c in crates {
        let dir = c.manifest_path.parent().unwrap();
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        if rel.as_os_str().is_empty() || rel.starts_with("components") {
            files.extend(rust_files(&dir.join("src"))?);
        }
    }
    for rule in &rules.source_rules {
        let re = regex::Regex::new(&rule.pattern)
            .map_err(|e| format!("source rule {}: {e}", rule.name))?;
        for file in &files {
            let rel = file
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rule.allowed_in.iter().any(|p| rel.starts_with(p.as_str())) {
                continue;
            }
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("reading {}: {e}", file.display()))?;
            let Some(m) = re.find(&text) else { continue };
            let key = Exception::Source {
                rule: rule.name.clone(),
                file: rel.clone(),
            };
            if exceptions.contains(&key) {
                used.insert(key);
            } else {
                let line = text[..m.start()].lines().count().max(1);
                problems.push(format!(
                    "{rel}:{line} matches {}: `{}`\n  {}",
                    rule.name,
                    m.as_str(),
                    rule.reason
                ));
            }
        }
    }
    Ok(())
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("reading {}: {e}", d.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(src: &str) -> Vec<String> {
        let mut v = CratePaths::default();
        v.visit_file(&syn::parse_file(src).unwrap());
        v.found.into_iter().map(|(m, _)| m).collect()
    }

    #[test]
    fn finds_crate_paths_in_uses_and_expressions() {
        let found = paths(
            "use crate::{a::X, b, c as d, e::{f, g}};
             use crate::h::*;
             use std::fmt;
             use super::i;
             fn f() { crate::j::k(); let _: crate::l::M; }",
        );
        assert_eq!(found, ["a", "b", "c", "e", "h", "j", "l"]);
    }

    fn package(root: &Path, name: &str, dir: &str, deps: &[(&str, Option<&str>)]) -> Package {
        Package {
            name: name.into(),
            manifest_path: root.join(dir).join("Cargo.toml"),
            dependencies: deps
                .iter()
                .map(|(n, kind)| crate::workspace::Dependency {
                    name: (*n).into(),
                    kind: kind.map(Into::into),
                })
                .collect(),
        }
    }

    /// One problem of every kind, in a throwaway workspace.
    #[test]
    fn reports_each_kind_of_violation() {
        let root = std::env::temp_dir().join(format!("otto-arch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("src/lib.rs", "use crate::udev::X;");
        write("src/udev.rs", "pub struct X;");
        write("src/render.rs", "fn f() { let _ = crate::udev::X; }");
        write("src/input.rs", "use crate::udev::X;");
        write(
            "components/app/src/main.rs",
            r#"fn f() { std::env::var("HOME"); }"#,
        );

        let rules: Rules = toml::from_str(
            r#"
            [[layer]]
            name = "base"
            description = "no runtime"
            crates = ["base"]
            may_use = []
            forbid_deps = ["tokio"]

            [[layer]]
            name = "app"
            description = "programs"
            crates = ["app", "other-app"]
            may_use = ["base"]

            [[module_rule]]
            name = "backends-are-leaves"
            from = ["*"]
            to = ["udev"]
            reason = "r"

            [[source_rule]]
            name = "xdg"
            pattern = 'env::var\("HOME"\)'
            allowed_in = ["components/base/"]
            reason = "r"

            [[allow]]
            module = "input"
            uses = "udev"
            reason = "known"

            [[allow]]
            crate = "app"
            dep = "base"
            reason = "never needed"
            "#,
        )
        .unwrap();
        // Sorted by name, as cargo metadata returns them.
        let crates = [
            package(
                &root,
                "app",
                "components/app",
                &[("base", None), ("other-app", None)],
            ),
            package(
                &root,
                "base",
                "components/base",
                &[("tokio", None), ("app", Some("dev"))],
            ),
            package(&root, "other-app", "components/other-app", &[]),
            package(&root, "stray", "components/stray", &[]),
        ];

        let problems = all(&root, &rules, &crates).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let starts: Vec<_> = problems.iter().map(|p| p.lines().next().unwrap()).collect();
        assert_eq!(
            starts,
            [
                "crate stray is in no layer",
                "crate app (app) depends on other-app (app)",
                "crate base (base) depends on tokio, which layer base must stay free of",
                "module render uses udev (src/render.rs:1), breaking backends-are-leaves",
                "components/app/src/main.rs:1 matches xdg: `env::var(\"HOME\")`",
                "stale [[allow]] crate = \"app\", dep = \"base\"",
            ]
        );
    }
}
