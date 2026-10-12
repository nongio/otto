//! `otto-arch affected`: the workspace members a change reaches, so CI
//! checks those and skips the rest.
//!
//! A changed file belongs to the member whose directory holds it (the root
//! package owns only its target directories: `src/`, `tests/`, …). Files no
//! member holds reach the crates the `[[affects]]` rules in
//! `architecture.toml` name; a file neither covers reaches everything. A
//! `Cargo.lock` edit reaches every member whose dependency closure in the
//! lockfile changed. The result is then closed over dependents: a member is
//! affected when anything it depends on is. A dev-dependency affects the
//! member's own tests but not the members that depend on it, since its
//! library does not link the dev-dependency.
//!
//! Any failure (no base to diff against, a git error, an unreadable
//! lockfile) selects everything: the answer may be too large, never too
//! small.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::rules::Rules;
use crate::workspace::Package;

/// A workspace member as the selection sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: String,
    /// The manifest directory, relative to the repository (`""` for the root).
    pub dir: String,
    /// Repository paths it owns: `dir/` prefixes or exact files.
    pub owns: Vec<String>,
    pub lib: bool,
    /// Workspace members it depends on, and whether only as a
    /// dev-dependency.
    pub deps: BTreeMap<String, bool>,
}

/// Members from `cargo metadata`. A component owns its whole directory;
/// the root package's directory is the repository, so it owns only where
/// its targets live (`src/`, `tests/`, `examples/`, `build.rs`, …).
pub fn members(root: &Path, packages: &[Package]) -> Vec<Member> {
    let names: BTreeSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    packages
        .iter()
        .map(|p| {
            let dir = rel(root, p.dir());
            let owns = if dir.is_empty() {
                let mut owns: Vec<String> = p
                    .targets
                    .iter()
                    .map(|t| rel(root, &t.src_path))
                    .map(|f| match f.split_once('/') {
                        Some((first, _)) => format!("{first}/"),
                        None => f,
                    })
                    .collect();
                owns.sort();
                owns.dedup();
                owns
            } else {
                vec![format!("{dir}/")]
            };
            let mut deps: BTreeMap<String, bool> = BTreeMap::new();
            for d in &p.dependencies {
                if d.path.is_some() && names.contains(d.name.as_str()) {
                    let dev_only = deps.get(&d.name).copied().unwrap_or(true) && d.is_dev();
                    deps.insert(d.name.clone(), dev_only);
                }
            }
            Member {
                name: p.name.clone(),
                owns,
                dir,
                lib: p.has_target("lib"),
                deps,
            }
        })
        .collect()
}

/// Members read straight from the manifests, for when `cargo metadata`
/// fails: names and library targets only, which is all a selection of
/// everything needs.
pub fn members_from_manifests(root: &Path) -> Result<Vec<Member>, String> {
    let read = |dir: &Path| -> Result<toml::Table, String> {
        let path = dir.join("Cargo.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let manifest = read(root)?;
    let mut dirs = vec![String::new()];
    if let Some(list) = manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    {
        dirs.extend(list.iter().filter_map(|v| v.as_str()).map(str::to_owned));
    }
    let mut out = Vec::new();
    for dir in dirs {
        let path = root.join(&dir);
        let m = read(&path)?;
        let Some(name) = m
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        out.push(Member {
            name: name.to_owned(),
            owns: if dir.is_empty() {
                Vec::new()
            } else {
                vec![format!("{dir}/")]
            },
            dir: dir.clone(),
            lib: m.contains_key("lib") || path.join("src/lib.rs").is_file(),
            deps: BTreeMap::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Every member, for a change nothing can be diffed for.
pub fn everything(members: &[Member], reason: String) -> Selection {
    Selection::all(members, reason)
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The member owning `path`: the longest owned prefix wins, so a file under
/// `components/x/` is `x`'s even though the root package's directory holds
/// it too.
pub fn owner<'a>(members: &'a [Member], path: &str) -> Option<&'a str> {
    members
        .iter()
        .flat_map(|m| m.owns.iter().map(move |o| (o, m)))
        .filter(|(o, _)| {
            if o.ends_with('/') {
                path.starts_with(o.as_str())
            } else {
                path == o.as_str()
            }
        })
        .max_by_key(|(o, _)| o.len())
        .map(|(_, m)| m.name.as_str())
}

/// Why a member is affected.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// A changed file inside the member.
    File(String),
    /// A changed file an `[[affects]]` rule maps to the member.
    Rule { file: String, reason: String },
    /// A package in the member's lockfile closure changed.
    Lock(String),
    /// It depends on an affected member.
    Dep(String),
    /// Its tests depend on an affected member.
    DevDep(String),
}

#[derive(Debug, Default, PartialEq)]
pub struct Selection {
    /// Set when everything is affected, with the reason.
    pub everything: Option<String>,
    /// Affected members and the first reason found for each.
    pub packages: BTreeMap<String, Why>,
    /// The file that reaches the packaging jobs, if any.
    pub packaging: Option<String>,
}

impl Selection {
    fn all(members: &[Member], reason: String) -> Self {
        Selection {
            packages: members
                .iter()
                .map(|m| (m.name.clone(), Why::File("*".into())))
                .collect(),
            packaging: Some(reason.clone()),
            everything: Some(reason),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }

    /// `name`'s chain back to the change: `otto-bar <- otto-kit <- file`.
    pub fn chain(&self, name: &str) -> String {
        let mut out = name.to_owned();
        let mut at = name;
        for _ in 0..self.packages.len() + 1 {
            match self.packages.get(at) {
                Some(Why::Dep(d)) => {
                    let _ = write!(out, " <- {d}");
                    at = d;
                }
                Some(Why::DevDep(d)) => {
                    let _ = write!(out, " <- (dev) {d}");
                    at = d;
                }
                Some(Why::File(f)) => return format!("{out} <- {f}"),
                Some(Why::Rule { file, reason }) => {
                    return format!("{out} <- {file} ([[affects]]: {reason})")
                }
                Some(Why::Lock(p)) => return format!("{out} <- Cargo.lock ({p})"),
                None => break,
            }
        }
        out
    }
}

/// The lockfile, which is diffed rather than mapped to a crate.
pub const LOCKFILE: &str = "Cargo.lock";

/// What `files` reach. `lock` is asked for the members whose lockfile
/// closure changed, only when `Cargo.lock` is among the files.
pub fn select(
    members: &[Member],
    rules: &Rules,
    files: &[String],
    lock: impl FnOnce() -> Result<BTreeMap<String, String>, String>,
) -> Selection {
    let names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let mut sel = Selection::default();
    let mut seeds: BTreeMap<String, Why> = BTreeMap::new();
    let mut everything = None;
    let mut lock_changed = false;

    for f in files {
        if f == LOCKFILE {
            lock_changed = true;
            continue;
        }
        let mut known = false;
        if let Some(m) = owner(members, f) {
            known = true;
            seeds
                .entry(m.to_owned())
                .or_insert_with(|| Why::File(f.clone()));
        }
        for rule in rules.affects.iter().filter(|r| r.matches(f)) {
            known = true;
            if !rule.areas.is_empty() && sel.packaging.is_none() {
                sel.packaging = Some(f.clone());
            }
            for c in &rule.crates {
                if c == "*" {
                    everything.get_or_insert_with(|| format!("{f}: {}", rule.reason));
                } else if !names.contains(c.as_str()) {
                    everything.get_or_insert_with(|| {
                        format!("{f}: [[affects]] names {c}, which is not a workspace member")
                    });
                } else {
                    seeds.entry(c.clone()).or_insert_with(|| Why::Rule {
                        file: f.clone(),
                        reason: rule.reason.clone(),
                    });
                }
            }
        }
        if !known {
            everything.get_or_insert_with(|| {
                format!("{f} is in no crate and no [[affects]] rule in architecture.toml")
            });
        }
    }
    if let Some(reason) = everything {
        return Selection::all(members, reason);
    }
    if lock_changed {
        match lock() {
            Ok(changed) => {
                for (m, pkg) in changed {
                    if names.contains(m.as_str()) {
                        seeds.entry(m).or_insert(Why::Lock(pkg));
                    }
                }
            }
            Err(e) => return Selection::all(members, format!("{LOCKFILE}: {e}")),
        }
    }

    // Reverse edges: who depends on whom.
    let mut dependents: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
    for m in members {
        for (d, dev) in &m.deps {
            dependents
                .entry(d.as_str())
                .or_default()
                .push((m.name.as_str(), *dev));
        }
    }
    let mut queue: VecDeque<String> = seeds.keys().cloned().collect();
    let mut propagated: BTreeSet<String> = seeds.keys().cloned().collect();
    sel.packages = seeds;
    while let Some(at) = queue.pop_front() {
        for &(dep, dev) in dependents.get(at.as_str()).into_iter().flatten() {
            let why = if dev {
                Why::DevDep(at.clone())
            } else {
                Why::Dep(at.clone())
            };
            match sel.packages.get(dep) {
                None => {
                    sel.packages.insert(dep.to_owned(), why);
                }
                // A normal edge explains more than a dev edge found first.
                Some(Why::DevDep(_)) if !dev => {
                    sel.packages.insert(dep.to_owned(), why);
                }
                _ => {}
            }
            if !dev && propagated.insert(dep.to_owned()) {
                queue.push_back(dep.to_owned());
            }
        }
    }
    sel
}

// ---------------------------------------------------------------------------
// Cargo.lock
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Lock {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

/// A package with everything that can change about it.
type Entry<'a> = (
    &'a str,
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    Vec<&'a str>,
);

/// The members whose dependency closure differs between two lockfiles,
/// each with one package that changed in it. The lockfile records
/// dependencies across every feature, target and kind (dev included), so a
/// closure is a superset of what any one build links: it can report a
/// member whose build would not change, never miss one whose build would.
pub fn lock_changes(
    base: &str,
    head: &str,
    members: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let base: Lock = toml::from_str(base).map_err(|e| format!("base lockfile: {e}"))?;
    let head: Lock = toml::from_str(head).map_err(|e| format!("head lockfile: {e}"))?;
    let mut out = BTreeMap::new();
    for m in members {
        let (b, h) = (closure(&base, m), closure(&head, m));
        if b != h {
            let differs = b
                .symmetric_difference(&h)
                .next()
                .map(|e| format!("{} {}", e.0, e.1))
                .unwrap_or_default();
            out.insert(m.clone(), differs);
        }
    }
    Ok(out)
}

fn closure<'a>(lock: &'a Lock, root: &str) -> BTreeSet<Entry<'a>> {
    let mut by_name: BTreeMap<&str, Vec<&LockPackage>> = BTreeMap::new();
    for p in &lock.package {
        by_name.entry(p.name.as_str()).or_default().push(p);
    }
    // A dependency is "name", "name version" or "name version (source)".
    let resolve = |dep: &str| -> Vec<&'a LockPackage> {
        let mut parts = dep.split(' ');
        let name = parts.next().unwrap_or_default();
        let version = parts.next();
        by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter(|p| version.is_none_or(|v| p.version == v))
            .copied()
            .collect()
    };
    let mut seen: BTreeMap<(&str, &str, Option<&str>), Entry<'a>> = BTreeMap::new();
    let mut stack: Vec<&LockPackage> = by_name.get(root).cloned().unwrap_or_default();
    while let Some(p) = stack.pop() {
        let key = (p.name.as_str(), p.version.as_str(), p.source.as_deref());
        if seen.contains_key(&key) {
            continue;
        }
        let mut deps: Vec<&str> = p.dependencies.iter().map(String::as_str).collect();
        deps.sort_unstable();
        for d in &deps {
            stack.extend(resolve(d));
        }
        seen.insert(key, (key.0, key.1, key.2, p.checksum.as_deref(), deps));
    }
    seen.into_values().collect()
}

// ---------------------------------------------------------------------------
// CI outputs
// ---------------------------------------------------------------------------

/// Members linted by steps of their own in ci.yml (the compositor's feature
/// matrix, the RDP bridge, the portal), by the `architecture` job
/// (otto-arch), or by none: submenu-gtk needs GTK development libraries no
/// job installs.
const OWN_CLIPPY: &[&str] = &[
    "otto",
    "otto-render",
    "otto-rdp",
    "xdg-desktop-portal-otto",
    "otto-arch",
    "submenu-gtk",
];

/// Members whose tests run in steps of their own: target-wide runs for the
/// file browser, the agent service and the portal; otto-rdp has no library;
/// otto-arch is the `architecture` job's.
const OWN_TESTS: &[&str] = &[
    "otto-files",
    "otto-agents",
    "xdg-desktop-portal-otto",
    "otto-rdp",
    "otto-arch",
];

/// Apps whose binary targets carry the unit tests `cargo test --bins` runs:
/// `--lib` skips them.
const BIN_TESTS: &[&str] = &[
    "otto-bar",
    "otto-greeter",
    "otto-input-overlay",
    "otto-islands",
    "otto-lock",
    "otto-msg",
    "otto-preview",
    "otto-settings",
    "otto-stash",
    "otto-launcher",
    "otto-search",
];

/// Members outside `ui`: the areas with filters of their own, and the tool
/// the `architecture` job checks.
const NOT_UI: &[&str] = &[
    "otto",
    "otto-render",
    "otto-rdp",
    "xdg-desktop-portal-otto",
    "otto-arch",
];

/// `key=value` lines for `$GITHUB_OUTPUT`.
pub fn outputs(sel: &Selection, members: &[Member]) -> Vec<(&'static str, String)> {
    let affected = |n: &str| sel.has(n);
    let flag = |b: bool| if b { "true" } else { "false" }.to_owned();
    let args = |keep: &dyn Fn(&Member) -> bool| {
        sel.packages
            .keys()
            .filter_map(|n| members.iter().find(|m| &m.name == n))
            .filter(|m| keep(m))
            .map(|m| format!("-p {}", m.name))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let compositor = affected("otto") || affected("otto-render");
    let rdp = affected("otto-rdp");
    let portal = affected("xdg-desktop-portal-otto");
    let files = affected("otto-files");
    let ui = sel.packages.keys().any(|n| !NOT_UI.contains(&n.as_str()));
    vec![
        ("all", flag(sel.everything.is_some())),
        (
            "packages",
            sel.packages.keys().cloned().collect::<Vec<_>>().join(" "),
        ),
        ("compositor", flag(compositor)),
        ("rdp", flag(rdp)),
        ("portal", flag(portal)),
        ("files", flag(files)),
        ("ui", flag(ui)),
        ("packaging", flag(sel.packaging.is_some())),
        ("rust", flag(compositor || rdp || portal || files || ui)),
        (
            "clippy_args",
            args(&|m| !OWN_CLIPPY.contains(&m.name.as_str())),
        ),
        (
            "test_lib_args",
            args(&|m| m.lib && !OWN_TESTS.contains(&m.name.as_str())),
        ),
        (
            "test_bins_args",
            args(&|m| BIN_TESTS.contains(&m.name.as_str())),
        ),
    ]
}

/// `-p` for every affected member, for a local `cargo test`.
pub fn test_args(sel: &Selection) -> String {
    sel.packages
        .keys()
        .map(|n| format!("-p {n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Human-readable: why each member is affected.
pub fn explain(sel: &Selection) -> String {
    let mut out = String::new();
    if let Some(r) = &sel.everything {
        let _ = writeln!(out, "everything is affected: {r}");
        return out;
    }
    if sel.packages.is_empty() {
        out.push_str("no crate is affected\n");
    }
    for name in sel.packages.keys() {
        let _ = writeln!(out, "  {}", sel.chain(name));
    }
    if let Some(f) = &sel.packaging {
        let _ = writeln!(out, "  packaging <- {f}");
    }
    out
}

// ---------------------------------------------------------------------------
// git
// ---------------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("running git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn nul_separated(bytes: Vec<u8>) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// A usable diff base: set, not the all-zero sha GitHub sends for a new
/// branch, and a commit this clone has.
pub fn check_base(root: &Path, base: &str) -> Result<(), String> {
    if base.is_empty() || base.bytes().all(|b| b == b'0') {
        return Err(format!("no diff base ({base:?})"));
    }
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
    )
    .map(drop)
    .map_err(|_| format!("diff base {base} is not a commit in this clone"))
}

/// Files that differ between `base` and `head`, or the working tree
/// (untracked files included) when `head` is `None`. The tree diff, not
/// the merge-base one: it is what changed between the two trees, so it can
/// only list more than a branch's own changes, never fewer. Renames list
/// both paths, since the old one's crate lost a file.
pub fn changed_files(root: &Path, base: &str, head: Option<&str>) -> Result<Vec<String>, String> {
    let mut args = vec!["diff", "--name-only", "--no-renames", "-z", base];
    args.extend(head);
    let mut files = nul_separated(git(root, &args)?);
    if head.is_none() {
        files.extend(nul_separated(git(
            root,
            &["ls-files", "-z", "--others", "--exclude-standard"],
        )?));
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// `Cargo.lock` at `rev`, or in the working tree.
pub fn lockfile(root: &Path, rev: Option<&str>) -> Result<String, String> {
    match rev {
        Some(rev) => git(root, &["show", &format!("{rev}:{LOCKFILE}")])
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
        None => std::fs::read_to_string(root.join(LOCKFILE))
            .map_err(|e| format!("reading {LOCKFILE}: {e}")),
    }
}

/// Tracked files, for validating the `[[affects]]` rules.
pub fn tracked_files(root: &Path) -> Result<Vec<String>, String> {
    git(root, &["ls-files", "-z"]).map(nul_separated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, owns: &[&str], lib: bool, deps: &[(&str, bool)]) -> Member {
        Member {
            name: name.into(),
            dir: owns[0].trim_end_matches('/').into(),
            owns: owns.iter().map(|s| s.to_string()).collect(),
            lib,
            deps: deps.iter().map(|(n, d)| (n.to_string(), *d)).collect(),
        }
    }

    /// otto (root) -> kit -> base; app -> kit; emoji -> kit; leaf -> base;
    /// otto's tests use emoji (a dev edge).
    fn ws() -> Vec<Member> {
        vec![
            member("app", &["components/app/"], false, &[("kit", false)]),
            member("base", &["components/base/"], true, &[]),
            member("emoji", &["components/emoji/"], true, &[("kit", false)]),
            member("kit", &["components/kit/"], true, &[("base", false)]),
            member("leaf", &["components/leaf/"], false, &[("base", false)]),
            member(
                "otto",
                &["src/", "tests/", "build.rs"],
                true,
                &[("kit", false), ("emoji", true)],
            ),
        ]
    }

    fn rules(text: &str) -> Rules {
        toml::from_str(&format!("layer = []\n{text}")).unwrap()
    }

    fn sel(files: &[&str], rules_text: &str) -> Selection {
        let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        select(&ws(), &rules(rules_text), &files, || {
            Err("lockfile not expected".into())
        })
    }

    fn names(s: &Selection) -> Vec<&str> {
        s.packages.keys().map(String::as_str).collect()
    }

    #[test]
    fn files_map_to_the_longest_owning_prefix() {
        let ws = ws();
        assert_eq!(owner(&ws, "src/main.rs"), Some("otto"));
        assert_eq!(owner(&ws, "tests/x.rs"), Some("otto"));
        assert_eq!(owner(&ws, "build.rs"), Some("otto"));
        assert_eq!(owner(&ws, "components/kit/src/lib.rs"), Some("kit"));
        assert_eq!(owner(&ws, "components/kit-extra/x.rs"), None);
        // The root directory holds everything but owns only its targets.
        assert_eq!(owner(&ws, "README.md"), None);
        assert_eq!(owner(&ws, "resources/x.svg"), None);
    }

    #[test]
    fn dependents_close_over_normal_edges_and_stop_after_dev_edges() {
        let s = sel(&["components/base/src/lib.rs"], "");
        assert_eq!(names(&s), ["app", "base", "emoji", "kit", "leaf", "otto"]);
        assert_eq!(
            s.chain("app"),
            "app <- kit <- base <- components/base/src/lib.rs"
        );

        // otto's tests use emoji, so otto is affected; a dev edge goes no
        // further.
        let s = sel(&["components/emoji/src/lib.rs"], "");
        assert_eq!(names(&s), ["emoji", "otto"]);
        assert_eq!(s.packages["otto"], Why::DevDep("emoji".into()));

        let s = sel(&["components/leaf/src/main.rs", "src/lib.rs"], "");
        assert_eq!(names(&s), ["leaf", "otto"]);
        assert!(s.everything.is_none());
    }

    const RULES: &str = r#"
        [[affects]]
        paths = ["resources/"]
        except = ["resources/bin/"]
        crates = ["otto"]
        areas = ["packaging"]
        reason = "r"
        [[affects]]
        paths = ["resources/bin/", "**/*.md"]
        crates = []
        areas = ["packaging"]
        reason = "installed"
        [[affects]]
        paths = ["components/base/src/shared.rs"]
        crates = ["leaf"]
        reason = "embedded"
        [[affects]]
        paths = ["Cargo.toml"]
        crates = ["*"]
        reason = "shared"
    "#;

    #[test]
    fn affects_rules_add_crates_areas_and_everything() {
        let s = sel(&["resources/a.svg"], RULES);
        assert_eq!(names(&s), ["otto"]);
        assert_eq!(s.packaging.as_deref(), Some("resources/a.svg"));

        let s = sel(&["resources/bin/tool", "docs/x/y.md"], RULES);
        assert!(s.packages.is_empty() && s.everything.is_none());
        assert!(s.packaging.is_some());

        // A crate's own README still reaches the crate.
        let s = sel(&["components/leaf/README.md"], RULES);
        assert_eq!(names(&s), ["leaf"]);

        // Rules add up: the owner, and the crate the rule names.
        let s = sel(&["components/base/src/shared.rs"], RULES);
        assert_eq!(
            s.packages["leaf"],
            Why::Rule {
                file: "components/base/src/shared.rs".into(),
                reason: "embedded".into()
            }
        );
        assert!(s.packages.contains_key("base"));

        let s = sel(&["Cargo.toml"], RULES);
        assert!(s.everything.is_some());
        assert_eq!(s.packages.len(), ws().len());
    }

    #[test]
    fn a_file_nothing_covers_affects_everything() {
        let s = sel(&["src/lib.rs", "new-top-level-file"], "");
        assert!(s
            .everything
            .as_deref()
            .unwrap()
            .contains("new-top-level-file"));
        assert_eq!(s.packages.len(), ws().len());
        assert!(s.packaging.is_some());
    }

    #[test]
    fn globs() {
        use crate::rules::path_matches as m;
        assert!(m("otto_config*.toml", "otto_config.example.toml"));
        assert!(!m("otto_config*.toml", "sub/otto_config.toml"));
        assert!(m("**/*.md", "README.md"));
        assert!(m("**/*.md", "components/x/docs/a.md"));
        assert!(!m("*.md", "docs/a.md"));
        assert!(m("docs/", "docs/a/b.png"));
        assert!(!m("docs/", "docsx/a"));
        assert!(m("PKGBUILD*", "PKGBUILD-git"));
        assert!(!m("Cargo.toml", "components/x/Cargo.toml"));
        assert!(m("a.b", "a.b") && !m("a.b", "axb"));
    }

    const BASE_LOCK: &str = r#"
version = 4
[[package]]
name = "app"
version = "1.0.0"
dependencies = ["kit", "serde"]
[[package]]
name = "kit"
version = "1.0.0"
dependencies = ["log 0.4.1"]
[[package]]
name = "leaf"
version = "1.0.0"
dependencies = ["log 0.4.2 (registry+https://github.com/rust-lang/crates.io-index)"]
[[package]]
name = "log"
version = "0.4.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaa"
[[package]]
name = "log"
version = "0.4.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbb"
[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "ccc"
"#;

    fn lock_diff(head: &str) -> Vec<String> {
        let members = ["app", "kit", "leaf"].map(String::from);
        lock_changes(BASE_LOCK, head, &members)
            .unwrap()
            .into_keys()
            .collect()
    }

    #[test]
    fn lockfile_changes_reach_the_members_whose_closure_holds_them() {
        assert!(lock_diff(BASE_LOCK).is_empty());

        // log 0.4.1 is kit's, and so app's; leaf uses 0.4.2.
        let checksum = BASE_LOCK.replace("\"aaa\"", "\"zzz\"");
        assert_eq!(lock_diff(&checksum), ["app", "kit"]);
        let members = ["kit".to_owned()];
        let why = lock_changes(BASE_LOCK, &checksum, &members).unwrap();
        assert_eq!(why["kit"], "log 0.4.1");

        // serde is app's alone.
        let bump = BASE_LOCK.replace(
            "name = \"serde\"\nversion = \"1.0.0\"",
            "name = \"serde\"\nversion = \"1.0.1\"",
        );
        assert_eq!(lock_diff(&bump), ["app"]);

        // A new edge counts too.
        let edge = BASE_LOCK.replace("[\"log 0.4.2", "[\"serde\", \"log 0.4.2");
        assert_eq!(lock_diff(&edge), ["leaf"]);

        assert!(lock_changes("not toml [", BASE_LOCK, &[]).is_err());
    }

    #[test]
    fn lockfile_is_diffed_and_its_failure_selects_everything() {
        let files = vec![LOCKFILE.to_owned()];
        let s = select(&ws(), &rules(""), &files, || {
            Ok(BTreeMap::from([(
                "base".to_owned(),
                "log 0.4.1".to_owned(),
            )]))
        });
        assert_eq!(names(&s), ["app", "base", "emoji", "kit", "leaf", "otto"]);
        assert_eq!(s.chain("leaf"), "leaf <- base <- Cargo.lock (log 0.4.1)");

        let s = select(&ws(), &rules(""), &files, || Err("no base lockfile".into()));
        assert!(s.everything.unwrap().contains("no base lockfile"));
    }

    #[test]
    fn outputs_derive_areas_and_package_lists() {
        let ws = vec![
            member("otto", &["src/"], true, &[("otto-kit", false)]),
            member("otto-kit", &["components/otto-kit/"], true, &[]),
            member(
                "otto-bar",
                &["components/otto-bar/"],
                false,
                &[("otto-kit", false)],
            ),
            member(
                "otto-files",
                &["components/otto-files/"],
                true,
                &[("otto-kit", false)],
            ),
            member("otto-rdp", &["components/otto-rdp/"], false, &[]),
            member("otto-arch", &["tools/otto-arch/"], false, &[]),
        ];
        let run = |files: &[&str]| {
            let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
            let s = select(&ws, &rules(""), &files, || Ok(BTreeMap::new()));
            outputs(&s, &ws)
                .into_iter()
                .map(|(k, v)| format!("{k}={v}\n"))
                .collect::<String>()
        };
        assert_eq!(
            run(&["components/otto-bar/src/main.rs"]),
            "all=false\npackages=otto-bar\ncompositor=false\nrdp=false\nportal=false\n\
             files=false\nui=true\npackaging=false\nrust=true\nclippy_args=-p otto-bar\n\
             test_lib_args=\ntest_bins_args=-p otto-bar\n"
        );
        let kit = run(&["components/otto-kit/src/lib.rs"]);
        assert!(
            kit.contains("packages=otto otto-bar otto-files otto-kit\n"),
            "{kit}"
        );
        assert!(kit.contains("compositor=true\n") && kit.contains("files=true\n"));
        assert!(kit.contains("clippy_args=-p otto-bar -p otto-files -p otto-kit\n"));
        // otto-files' tests run in a step of their own.
        assert!(kit.contains("test_lib_args=-p otto -p otto-kit\n"), "{kit}");

        // The tool alone: the architecture job checks it, the check job
        // has nothing to do.
        let tool = run(&["tools/otto-arch/src/main.rs"]);
        assert!(tool.contains("ui=false\n") && tool.contains("rust=false\n"));
        assert!(tool.contains("clippy_args=\n"));

        let none = run(&[]);
        assert!(none.contains("packages=\n") && none.contains("rust=false\n"));
        assert_eq!(test_args(&sel(&["components/leaf/x"], "")), "-p leaf");
    }
}
