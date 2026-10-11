//! The checks themselves. Each returns problems as ready-to-print text; an
//! [`Exception`] the code no longer needs is a problem too, so the debt list in
//! `architecture.toml` can only shrink.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::rules::{Exception, Rules};
use crate::workspace::{Graph, Package};

pub fn all(
    root: &Path,
    rules: &Rules,
    crates: &[Package],
    graph: &Graph,
) -> Result<Vec<String>, String> {
    let exceptions = rules.exceptions()?;
    let mut used = BTreeSet::new();
    let mut problems = Vec::new();

    placement(rules, crates, &mut problems);
    lib_lints(rules, crates, &mut problems)?;
    crate_deps(rules, crates, graph, &exceptions, &mut used, &mut problems);
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

/// Workspace dependencies against `may_use` (declared, direct), and
/// `forbid_deps` against everything a crate pulls in (resolved, transitive).
fn crate_deps(
    rules: &Rules,
    crates: &[Package],
    graph: &Graph,
    exceptions: &[Exception],
    used: &mut BTreeSet<Exception>,
    problems: &mut Vec<String>,
) {
    for c in crates {
        let Some(layer) = rules.layer_of(&c.name) else {
            continue;
        };
        let mut found: Vec<(String, String)> = Vec::new();
        // A crate can list one dependency several times (per target, per
        // kind); judge each name once.
        let deps: BTreeSet<&str> = c
            .dependencies
            .iter()
            .filter(|d| !d.is_dev())
            .map(|d| d.name.as_str())
            .collect();
        for dep in deps {
            let Some(dep_layer) = rules.layer_of(dep) else {
                continue;
            };
            if !layer.may_use.contains(&dep_layer.name) {
                found.push((
                    dep.to_owned(),
                    format!(
                        "crate {} ({}) depends on {dep} ({})\n  Layer {} may use: {}.",
                        c.name,
                        layer.name,
                        dep_layer.name,
                        layer.name,
                        list(&layer.may_use)
                    ),
                ));
            }
        }
        for banned in &layer.forbid_deps {
            if let Some(chain) = graph.path(&c.name, banned) {
                found.push((
                    banned.clone(),
                    format!(
                        "crate {} ({}) pulls in {banned}, which layer {} must stay free of\n  \
                         Through: {}\n  {}",
                        c.name,
                        layer.name,
                        layer.name,
                        chain.join(" -> "),
                        layer.description
                    ),
                ));
            }
        }
        for (dep, problem) in found {
            let key = Exception::Crate {
                krate: c.name.clone(),
                dep,
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
/// the crate root; `[lints]` cannot, because members inherit the workspace
/// table.
fn lib_lints(rules: &Rules, crates: &[Package], problems: &mut Vec<String>) -> Result<(), String> {
    for c in crates {
        let Some(layer) = rules.layer_of(&c.name) else {
            continue;
        };
        if layer.lib_lints.is_empty() {
            continue;
        }
        let Some(lib) = c.lib_root() else {
            problems.push(format!(
                "crate {} ({}) has no library target, but its layer is for libraries",
                c.name, layer.name
            ));
            continue;
        };
        let text =
            std::fs::read_to_string(lib).map_err(|e| format!("reading {}: {e}", lib.display()))?;
        let ast = syn::parse_file(&text).map_err(|e| format!("parsing {}: {e}", lib.display()))?;
        for lint in &layer.lib_lints {
            if !turns_on(&ast.attrs, lint) {
                problems.push(format!(
                    "crate {} ({}) does not turn on {lint}\n  Add `#![warn({lint})]` to {}.",
                    c.name,
                    layer.name,
                    lib.display()
                ));
            }
        }
    }
    Ok(())
}

/// Whether `#![warn(..)]`, `deny` or `forbid` among `attrs` names `lint`
/// (a plain lint or a tool lint such as `clippy::foo`).
fn turns_on(attrs: &[syn::Attribute], lint: &str) -> bool {
    attrs.iter().any(|a| {
        ["warn", "deny", "forbid"]
            .iter()
            .any(|l| a.path().is_ident(l))
            && a.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|lints| lints.iter().any(|p| path_string(p) == lint))
    })
}

fn path_string(p: &syn::Path) -> String {
    p.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
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
/// through `crate::`, with one place each is named from. A name the crate root
/// re-exports from one of its own modules (`crate::Otto`) counts as that
/// module; a module re-exported from another crate (`crate::renderer`, from
/// otto-render) keeps its own name, so a rule can still forbid it. Test code
/// counts too: a test reaching into a backend couples the same way.
pub fn module_graph(src: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let reexports = root_reexports(src)?;
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
        let ast = parse(&file)?;
        let mut v = CratePaths::rooted_at(&reexports.crates);
        v.visit_file(&ast);
        let entry = graph.entry(top.clone()).or_default();
        for (name, line) in v.found {
            let target = reexports.local.get(&name).cloned().unwrap_or(name);
            if target != top {
                entry
                    .entry(target)
                    .or_insert_with(|| format!("src/{}:{line}", rel.display()));
            }
        }
    }
    Ok(graph)
}

/// What the crate root re-exports.
#[derive(Default)]
struct Reexports {
    /// Names from the crate's own modules, as name -> module.
    local: BTreeMap<String, String>,
    /// Names from other crates (`renderer`, from `otto_render`).
    external: BTreeSet<String>,
    /// The crates those come from (`otto_render`): a path through one of
    /// them reaches the same module as `crate::`.
    crates: BTreeSet<String>,
}

/// The `pub use` items in the crate root.
fn root_reexports(src: &Path) -> Result<Reexports, String> {
    fn leaves(tree: &syn::UseTree, out: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(p) => leaves(&p.tree, out),
            syn::UseTree::Name(n) => out.push(n.ident.to_string()),
            syn::UseTree::Rename(r) => out.push(r.rename.to_string()),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| leaves(t, out)),
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = Reexports::default();
    let lib = src.join("lib.rs");
    if !lib.is_file() {
        return Ok(out);
    }
    for item in parse(&lib)?.items {
        let syn::Item::Use(u) = item else { continue };
        if !matches!(u.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let syn::UseTree::Path(mut p) = u.tree else {
            continue;
        };
        let in_crate = p.ident == "crate";
        if in_crate {
            let syn::UseTree::Path(inner) = *p.tree else {
                continue;
            };
            p = inner;
        }
        let module = p.ident.to_string();
        let mut names = Vec::new();
        leaves(&p.tree, &mut names);
        let is_local =
            in_crate || src.join(format!("{module}.rs")).is_file() || src.join(&module).is_dir();
        if !is_local {
            out.crates.insert(module.clone());
        }
        for name in names {
            if is_local {
                out.local.insert(name, module.clone());
            } else {
                out.external.insert(name);
            }
        }
    }
    Ok(out)
}

/// Collects the first segment after `crate::` (or after a crate the root
/// re-exports from, such as `otto_render::`) in paths, `use` trees and the
/// tokens of macro calls, which syn leaves unparsed.
struct CratePaths {
    roots: BTreeSet<String>,
    found: Vec<(String, usize)>,
}

impl Default for CratePaths {
    fn default() -> Self {
        Self::rooted_at(&BTreeSet::new())
    }
}

impl CratePaths {
    fn rooted_at(crates: &BTreeSet<String>) -> Self {
        let mut roots = crates.clone();
        roots.insert("crate".into());
        CratePaths {
            roots,
            found: Vec::new(),
        }
    }

    fn is_root(&self, ident: &impl std::fmt::Display) -> bool {
        self.roots.contains(&ident.to_string())
    }

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

    /// `crate :: name` anywhere in a token stream, nested groups included.
    fn tokens(&mut self, tokens: TokenStream) {
        let tts: Vec<TokenTree> = tokens.into_iter().collect();
        for (i, tt) in tts.iter().enumerate() {
            match tt {
                TokenTree::Group(g) => self.tokens(g.stream()),
                TokenTree::Ident(id) if self.is_root(id) => {
                    if let [TokenTree::Punct(a), TokenTree::Punct(b), TokenTree::Ident(name), ..] =
                        &tts[i + 1..]
                    {
                        if a.as_char() == ':' && b.as_char() == ':' {
                            self.push(name);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for CratePaths {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if let syn::UseTree::Path(p) = &item.tree {
            if self.is_root(&p.ident) {
                self.use_tree(&p.tree);
            }
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let mut segs = path.segments.iter();
        if let (Some(first), Some(second)) = (segs.next(), segs.next()) {
            if self.is_root(&first.ident) {
                self.push(&second.ident);
            }
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
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
    let external = root_reexports(&root.join("src"))?.external;
    for rule in &rules.module_rules {
        for name in &rule.from {
            if name != "*" && !graph.contains_key(name) {
                problems.push(format!(
                    "module rule {} names {name} in `from`, which is not a top-level module of src/",
                    rule.name
                ));
            }
        }
        for name in &rule.to {
            if !graph.contains_key(name) && !external.contains(name) {
                problems.push(format!(
                    "module rule {} names {name} in `to`, which is neither a top-level module of \
                     src/ nor a module the crate root re-exports",
                    rule.name
                ));
            }
        }
        // "*" covers every module, the targets too: one backend naming
        // another breaks backends-are-leaves as well. Self-references never
        // reach the graph.
        let wildcard = rule.from.iter().any(|f| f == "*");
        for (module, uses) in &graph {
            if !wildcard && !rule.from.contains(module) {
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
    // The compositor's src/, its crates/ and every component's; sample
    // clients and tools are outside the product.
    let mut files = Vec::new();
    for c in crates {
        let dir = c.manifest_path.parent().unwrap();
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        if rel.as_os_str().is_empty() || rel.starts_with("components") || rel.starts_with("crates")
        {
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
            if !re.is_match(&text) {
                continue;
            }
            // Tests that save and restore the environment are not lookups.
            let tests =
                test_lines(&syn::parse_file(&text).map_err(|e| format!("parsing {rel}: {e}"))?);
            let lines: Vec<usize> = re
                .find_iter(&text)
                .map(|m| text[..m.start()].matches('\n').count() + 1)
                .filter(|l| !tests.iter().any(|r| r.contains(l)))
                .collect();
            if lines.is_empty() {
                continue;
            }
            let key = Exception::Source {
                rule: rule.name.clone(),
                file: rel.clone(),
            };
            let at = lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            match rules.allowed_count(&rule.name, &rel) {
                Some(n) if exceptions.contains(&key) => {
                    used.insert(key);
                    if lines.len() > n {
                        problems.push(format!(
                            "{rel} has {} matches of {} (lines {at}); [[allow]] grants {n}\n  {}",
                            lines.len(),
                            rule.name,
                            rule.reason
                        ));
                    } else if lines.len() < n {
                        problems.push(format!(
                            "{rel} is down to {} matches of {}; lower its [[allow]] count from {n}",
                            lines.len(),
                            rule.name
                        ));
                    }
                }
                _ => problems.push(format!(
                    "{rel} matches {} (lines {at})\n  {}",
                    rule.name, rule.reason
                )),
            }
        }
    }
    Ok(())
}

/// Line ranges of `#[test]` functions and `#[cfg(test)]` items.
fn test_lines(file: &syn::File) -> Vec<RangeInclusive<usize>> {
    #[derive(Default)]
    struct Tests(Vec<RangeInclusive<usize>>);
    impl Tests {
        fn item(&mut self, attrs: &[syn::Attribute], node: &impl Spanned) -> bool {
            let test = attrs.iter().any(|a| {
                a.path().is_ident("test")
                    || (a.path().is_ident("cfg")
                        && a.parse_args::<syn::Ident>().is_ok_and(|i| i == "test"))
            });
            if test {
                let span = node.span();
                self.0.push(span.start().line..=span.end().line);
            }
            test
        }
    }
    impl<'ast> Visit<'ast> for Tests {
        fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
            if !self.item(&i.attrs, i) {
                syn::visit::visit_item_fn(self, i);
            }
        }
        fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
            if !self.item(&i.attrs, i) {
                syn::visit::visit_item_mod(self, i);
            }
        }
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            if !self.item(&i.attrs, i) {
                syn::visit::visit_item_impl(self, i);
            }
        }
    }
    let mut t = Tests::default();
    t.visit_file(file);
    t.0
}

fn parse(file: &Path) -> Result<syn::File, String> {
    let text =
        std::fs::read_to_string(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
    syn::parse_file(&text).map_err(|e| format!("parsing {}: {e}", file.display()))
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
    fn finds_crate_paths_in_uses_expressions_and_macros() {
        let found = paths(
            "use crate::{a::X, b, c as d, e::{f, g}};
             use crate::h::*;
             use std::fmt;
             use super::i;
             fn f() {
                 crate::j::k();
                 let _: crate::l::M;
                 tracing::info!(n = crate::m::count(), \"{}\", vec![crate::n::x()].len());
             }
             macro_rules! o { () => { crate::o::p() }; }",
        );
        assert_eq!(found, ["a", "b", "c", "e", "h", "j", "l", "m", "n", "o"]);
    }

    /// A unique scratch directory, created fresh so a leftover can't be
    /// mistaken for it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("otto-arch-test-{}-{nanos}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            Scratch(dir)
        }

        fn write(&self, path: &str, text: &str) {
            let p = self.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn package(root: &Path, name: &str, dir: &str, deps: &[(&str, Option<&str>)]) -> Package {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "manifest_path": root.join(dir).join("Cargo.toml"),
            "dependencies": deps
                .iter()
                .map(|(n, kind)| serde_json::json!({ "name": n, "kind": kind }))
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    /// One problem of every kind, in a throwaway workspace.
    #[test]
    fn reports_each_kind_of_violation() {
        let ws = Scratch::new();
        ws.write("src/lib.rs", "pub use state::Otto;\nuse crate::udev::X;");
        ws.write("src/state.rs", "pub struct Otto;");
        ws.write("src/udev.rs", "pub struct X;");
        ws.write("src/winit.rs", "use crate::udev::X;");
        ws.write(
            "src/render.rs",
            "fn f() { log!(crate::udev::X); }\nfn g(_: crate::Otto) {}",
        );
        ws.write("src/input.rs", "use crate::udev::X;");
        ws.write(
            "components/app/src/main.rs",
            "fn f() { std::env::var(\"HOME\"); }\n\
             #[cfg(test)]\nmod tests { fn t() { std::env::var(\"HOME\"); } }",
        );
        ws.write(
            "components/other-app/src/main.rs",
            "fn f() { std::env::var(\"HOME\"); std::env::var(\"HOME\"); }",
        );
        ws.write(
            "components/third-app/src/main.rs",
            "fn f() { std::env::var(\"HOME\"); }",
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
            crates = ["app", "other-app", "third-app"]
            may_use = ["base"]

            [[module_rule]]
            name = "backends-are-leaves"
            from = ["*"]
            to = ["udev", "winit"]
            reason = "r"

            [[module_rule]]
            name = "render-below-state"
            from = ["render"]
            to = ["state"]
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

            [[allow]]
            rule = "xdg"
            reason = "known"
            files = [
                { path = "components/other-app/src/main.rs", count = 1 },
                { path = "components/third-app/src/main.rs", count = 2 },
            ]
            "#,
        )
        .unwrap();
        // Sorted by name, as cargo metadata returns them.
        let root = &ws.0;
        let crates = [
            package(
                root,
                "app",
                "components/app",
                &[("base", None), ("other-app", None)],
            ),
            package(
                root,
                "base",
                "components/base",
                &[("zbus", None), ("app", Some("dev"))],
            ),
            package(root, "other-app", "components/other-app", &[]),
            package(root, "stray", "components/stray", &[]),
            package(root, "third-app", "components/third-app", &[]),
        ];
        let mut graph = Graph::default();
        graph.edges.insert("base".into(), ["zbus".into()].into());
        graph.edges.insert("zbus".into(), ["tokio".into()].into());

        let problems = all(root, &rules, &crates, &graph).unwrap();
        let starts: Vec<_> = problems.iter().map(|p| p.lines().next().unwrap()).collect();
        assert_eq!(
            starts,
            [
                "crate stray is in no layer",
                "crate app (app) depends on other-app (app)",
                "crate base (base) pulls in tokio, which layer base must stay free of",
                "module render uses udev (src/render.rs:1), breaking backends-are-leaves",
                "module winit uses udev (src/winit.rs:1), breaking backends-are-leaves",
                "module render uses state (src/render.rs:2), breaking render-below-state",
                "components/app/src/main.rs matches xdg (lines 1)",
                "components/other-app/src/main.rs has 2 matches of xdg (lines 1, 1); [[allow]] grants 1",
                "components/third-app/src/main.rs is down to 1 matches of xdg; lower its [[allow]] count from 2",
                "stale [[allow]] crate = \"app\", dep = \"base\"",
            ]
        );
        assert!(problems[2].contains("Through: base -> zbus -> tokio"));
    }

    /// A module re-exported from another crate keeps its name, through
    /// `crate::` and through the other crate's own path, while a name from a
    /// local module counts as that module.
    #[test]
    fn graph_follows_root_reexports() {
        let ws = Scratch::new();
        ws.write(
            "src/lib.rs",
            "pub use state::Otto;\npub use otto_render::{renderer, drawing};",
        );
        ws.write("src/state.rs", "pub struct Otto;");
        ws.write(
            "src/config.rs",
            "use crate::renderer::X;\nfn f(_: crate::Otto) { otto_render::drawing::y(); }",
        );
        let graph = module_graph(&ws.0.join("src")).unwrap();
        let uses: Vec<_> = graph["config"].keys().map(String::as_str).collect();
        assert_eq!(uses, ["drawing", "renderer", "state"]);
        let external = root_reexports(&ws.0.join("src")).unwrap().external;
        assert_eq!(
            external.iter().map(String::as_str).collect::<Vec<_>>(),
            ["drawing", "renderer"]
        );
    }

    #[test]
    fn lib_lints_accept_tool_lints() {
        let attrs = syn::parse_file("#![warn(unreachable_pub, clippy::pedantic)]")
            .unwrap()
            .attrs;
        assert!(turns_on(&attrs, "unreachable_pub"));
        assert!(turns_on(&attrs, "clippy::pedantic"));
        assert!(!turns_on(&attrs, "missing_docs"));
    }
}
