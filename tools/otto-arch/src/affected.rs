//! `otto-arch affected`: the workspace members a change reaches, so CI
//! checks those and skips the rest.
//!
//! The selection is [determinator]'s: it compares the package graphs
//! `cargo metadata` resolves at the base and at the head, matches every
//! changed file to a member, and closes the result over dependents through
//! simulated Cargo builds (dev-dependencies included for the member's own
//! tests, not for the members that depend on it). A changed third-party
//! version, source or feature set reaches exactly the members whose builds
//! include it, which is how a `Cargo.lock` edit is scoped.
//!
//! What determinator cannot know, which crates a file outside every member
//! reaches, comes from the `[[affects]]` rules in `architecture.toml`,
//! translated into determinator path rules by [`determinator_rules`]. A file
//! neither a member nor a rule covers reaches everything.
//!
//! Any failure (no base to diff against, a git or cargo error, a graph
//! guppy cannot build) selects everything: the answer may be too large,
//! never too small.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use determinator::rules::{
    DeterminatorMarkChanged, DeterminatorPostRule, DeterminatorRules, PathMatch, PathRule,
};
use determinator::Determinator;
use guppy::graph::{DependencyDirection, PackageGraph, PackageMetadata};
use guppy::CargoMetadata;

use crate::rules::{self, Rules};
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
}

/// Members from `cargo metadata`. A component owns its whole directory;
/// the root package's directory is the repository, so it owns only where
/// its targets live (`src/`, `tests/`, `examples/`, `build.rs`, …).
pub fn members(root: &Path, packages: &[Package]) -> Vec<Member> {
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
            Member {
                name: p.name.clone(),
                owns,
                dir,
                lib: p.has_target("lib"),
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
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The member owning `path`: the longest owned prefix wins, so a file under
/// `components/x/` is `x`'s even though the root package's directory holds
/// it too. This is determinator's nearest-ancestor match, limited for the
/// root package to its target directories.
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

// ---------------------------------------------------------------------------
// [[affects]] -> determinator rules
// ---------------------------------------------------------------------------

/// The `[[affects]]` rules as determinator path rules, applied in order to
/// each changed file:
///
/// 1. every `[[affects]]` rule that names crates, marking them and falling
///    through, so the rules add up whatever their order (`"*"` marks
///    everything and stops);
/// 2. the members' own paths: no more rules, match the file to its nearest
///    member (the root package owns only its target directories, so a
///    top-level file does not land on the compositor);
/// 3. any other file an `[[affects]]` rule covers: done, nothing more;
/// 4. anything else, a file no member and no rule covers: everything.
///
/// determinator's default rules are off: step 4 matches every path before
/// they would apply, and `architecture.toml` says what each file reaches
/// (they would ignore `clippy.toml`, which CI lints with, and do not know
/// `rust-toolchain.toml`).
pub fn determinator_rules(rules: &Rules, members: &[Member]) -> Result<DeterminatorRules, String> {
    let mut out = DeterminatorRules::parse("use-default-rules = false")
        .map_err(|e| format!("determinator rules: {e}"))?;
    for a in rules.affects.iter().filter(|a| !a.crates.is_empty()) {
        out.path_rules.push(PathRule {
            globs: a.globs(),
            mark_changed: if a.names_every_crate() {
                DeterminatorMarkChanged::All
            } else {
                DeterminatorMarkChanged::Packages(a.crates.clone())
            },
            post_rule: DeterminatorPostRule::Fallthrough,
        });
    }
    out.path_rules.push(PathRule {
        globs: members
            .iter()
            .flat_map(|m| &m.owns)
            .map(|o| rules::glob(o))
            .collect(),
        mark_changed: DeterminatorMarkChanged::Packages(Vec::new()),
        post_rule: DeterminatorPostRule::SkipRules,
    });
    out.path_rules.push(PathRule {
        globs: rules.affects.iter().flat_map(|a| a.globs()).collect(),
        mark_changed: DeterminatorMarkChanged::Packages(Vec::new()),
        post_rule: DeterminatorPostRule::Skip,
    });
    out.path_rules.push(PathRule {
        globs: vec!["**".into()],
        mark_changed: DeterminatorMarkChanged::All,
        post_rule: DeterminatorPostRule::Skip,
    });
    Ok(out)
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// Why a member is affected.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// A changed file inside the member.
    File(String),
    /// A changed file an `[[affects]]` rule maps to the member.
    Rule { file: String, reason: String },
    /// A simulated build of the member resolves different packages or
    /// features: a `Cargo.lock` or manifest change.
    Deps,
    /// It depends on an affected member through these members, nearest
    /// first; `dev` when the first edge is a dev-dependency.
    Via { path: Vec<String>, dev: bool },
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
    /// Every member, for a change nothing can be worked out for.
    pub fn everything(members: &[Member], reason: String) -> Self {
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
        let mut at = name.to_owned();
        for _ in 0..self.packages.len() + 1 {
            match self.packages.get(&at) {
                Some(Why::Via { path, dev }) => {
                    for (i, p) in path.iter().enumerate() {
                        let tag = if i == 0 && *dev { "(dev) " } else { "" };
                        let _ = write!(out, " <- {tag}{p}");
                    }
                    match path.last() {
                        Some(last) => at = last.clone(),
                        None => break,
                    }
                }
                Some(Why::File(f)) => return format!("{out} <- {f}"),
                Some(Why::Rule { file, reason }) => {
                    return format!("{out} <- {file} ([[affects]]: {reason})")
                }
                Some(Why::Deps) => {
                    return format!(
                        "{out} <- its resolved dependencies changed (Cargo.lock or a manifest)"
                    )
                }
                None => break,
            }
        }
        out
    }
}

/// `cargo metadata` JSON as a guppy graph.
pub fn graph(json: &str) -> Result<PackageGraph, String> {
    CargoMetadata::parse_json(json)
        .and_then(|m| m.build_graph())
        .map_err(|e| format!("building the package graph: {e}"))
}

/// What `files` reach, from the graphs at the base (`old`) and the head
/// (`new`). `members` and `rules` are the head's.
pub fn select(
    old: &PackageGraph,
    new: &PackageGraph,
    members: &[Member],
    rules: &Rules,
    files: &[String],
) -> Result<Selection, String> {
    let mut det = Determinator::new(old, new);
    det.set_rules(&determinator_rules(rules, members)?)
        .map_err(|e| format!("determinator rules: {e}"))?;
    det.add_changed_paths(files);

    let mut sel = Selection {
        packaging: files
            .iter()
            .find(|f| {
                rules
                    .affects
                    .iter()
                    .any(|r| !r.areas.is_empty() && r.matches(f))
            })
            .cloned(),
        ..Selection::default()
    };

    // Why each path-changed member is affected, and whether a file reaches
    // everything. match_path runs the same rules compute() does.
    for f in files {
        let mut hit = Vec::new();
        match det.match_path(f, |id| hit.push(id)) {
            PathMatch::RuleMatchedAll | PathMatch::NoMatches => {
                let reason = rules
                    .affects
                    .iter()
                    .find(|r| r.names_every_crate() && r.matches(f))
                    .map(|r| format!("{f}: {}", r.reason))
                    .unwrap_or_else(|| {
                        format!("{f} is in no crate and no [[affects]] rule in architecture.toml")
                    });
                return Ok(Selection::everything(members, reason));
            }
            PathMatch::RuleMatched(_) | PathMatch::AncestorMatched => {}
        }
        for id in hit {
            let name = new
                .metadata(id)
                .map_err(|e| format!("package graph: {e}"))?
                .name();
            let why = if owner(members, f) == Some(name) {
                Why::File(f.clone())
            } else {
                rules
                    .affects
                    .iter()
                    .find(|r| r.matches(f) && r.crates.iter().any(|c| c == name))
                    .map(|r| Why::Rule {
                        file: f.clone(),
                        reason: r.reason.clone(),
                    })
                    .unwrap_or_else(|| Why::File(f.clone()))
            };
            sel.packages.entry(name.to_owned()).or_insert(why);
        }
    }

    let set = det.compute();
    for p in set
        .summary_changed_set
        .packages(DependencyDirection::Forward)
    {
        sel.packages.entry(p.name().to_owned()).or_insert(Why::Deps);
    }
    let seeds: BTreeSet<String> = sel.packages.keys().cloned().collect();
    for p in set.affected_set.packages(DependencyDirection::Forward) {
        if !sel.packages.contains_key(p.name()) {
            let why = via(p, &seeds).unwrap_or(Why::Via {
                path: Vec::new(),
                dev: false,
            });
            sel.packages.insert(p.name().to_owned(), why);
        }
    }
    Ok(sel)
}

/// The shortest chain of workspace dependencies from `from` to a member in
/// `seeds`, for `--explain`. Only the first edge may be a dev-dependency,
/// as in the builds determinator simulates.
fn via(from: PackageMetadata<'_>, seeds: &BTreeSet<String>) -> Option<Why> {
    let mut parent: BTreeMap<&str, (&str, bool)> = BTreeMap::new();
    let mut queue = VecDeque::from([from]);
    while let Some(at) = queue.pop_front() {
        let first = at.id() == from.id();
        for link in at.direct_links() {
            let to = link.to();
            if !to.in_workspace()
                || (!first && link.dev_only())
                || to.id() == from.id()
                || parent.contains_key(to.name())
            {
                continue;
            }
            parent.insert(to.name(), (at.name(), first && link.dev_only()));
            if seeds.contains(to.name()) {
                let mut path = vec![to.name().to_owned()];
                let mut cur = to.name();
                let mut dev = false;
                while let Some(&(p, d)) = parent.get(cur) {
                    if p == from.name() {
                        dev = d;
                        break;
                    }
                    path.push(p.to_owned());
                    cur = p;
                }
                path.reverse();
                return Some(Why::Via { path, dev });
            }
            queue.push_back(to);
        }
    }
    None
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

/// Members that do not make the `check` job run on their own: the areas
/// with flags of their own, and the tool the `architecture` job checks.
const NOT_UI: &[&str] = &[
    "otto",
    "otto-render",
    "otto-rdp",
    "xdg-desktop-portal-otto",
    "otto-arch",
];

/// The components clippy run when everything is affected: what the step ran
/// before the selection existed. ci.yml's fallback writes the same lines.
const EVERYTHING_CLIPPY: &str =
    "--workspace --exclude otto --exclude otto-rdp --exclude xdg-desktop-portal-otto --exclude submenu-gtk";
/// The library tests when everything is affected, as above.
const EVERYTHING_TEST_LIB: &str = "--workspace --exclude otto-files";

/// `key=value` lines for `$GITHUB_OUTPUT`. `all` is read by the steps gated
/// on one crate being in `packages` (the toolkit doctests, the agent
/// service), so a selection of everything that could not list the members
/// still runs them.
pub fn outputs(sel: &Selection, members: &[Member]) -> Vec<(&'static str, String)> {
    let flag = |b: bool| if b { "true" } else { "false" }.to_owned();
    let packages = sel.packages.keys().cloned().collect::<Vec<_>>().join(" ");
    if sel.everything.is_some() {
        return everything_outputs(packages);
    }
    let affected = |n: &str| sel.has(n);
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
        ("all", flag(false)),
        ("packages", packages),
        ("compositor", flag(compositor)),
        ("rdp", flag(rdp)),
        ("portal", flag(portal)),
        ("files", flag(files)),
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

/// Every flag on and the workspace-wide argument lists: the same lines
/// ci.yml's fallback writes when this tool cannot run (a test keeps the two
/// in step).
fn everything_outputs(packages: String) -> Vec<(&'static str, String)> {
    let t = || "true".to_owned();
    vec![
        ("all", t()),
        ("packages", packages),
        ("compositor", t()),
        ("rdp", t()),
        ("portal", t()),
        ("files", t()),
        ("packaging", t()),
        ("rust", t()),
        ("clippy_args", EVERYTHING_CLIPPY.to_owned()),
        ("test_lib_args", EVERYTHING_TEST_LIB.to_owned()),
        (
            "test_bins_args",
            BIN_TESTS
                .iter()
                .map(|n| format!("-p {n}"))
                .collect::<Vec<_>>()
                .join(" "),
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

/// Tracked files, for validating the `[[affects]]` rules.
pub fn tracked_files(root: &Path) -> Result<Vec<String>, String> {
    git(root, &["ls-files", "-z"]).map(nul_separated)
}

/// A detached worktree of `rev` in a temporary directory, removed on drop:
/// the base (or an explicit head) needs its own `cargo metadata`. It shares
/// the cargo registry and git caches, so it downloads only what the other
/// revision does not already have.
pub struct Checkout {
    repo: PathBuf,
    pub dir: PathBuf,
}

impl Checkout {
    pub fn new(repo: &Path, rev: &str, label: &str) -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("otto-arch-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.to_string_lossy().into_owned();
        git(
            repo,
            &["worktree", "add", "--detach", "--quiet", &path, rev],
        )?;
        Ok(Checkout {
            repo: repo.to_path_buf(),
            dir,
        })
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        let path = self.dir.to_string_lossy().into_owned();
        if git(&self.repo, &["worktree", "remove", "--force", &path]).is_err() {
            let _ = std::fs::remove_dir_all(&self.dir);
            let _ = git(&self.repo, &["worktree", "prune"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, owns: &[&str], lib: bool) -> Member {
        Member {
            name: name.into(),
            dir: owns[0].trim_end_matches('/').into(),
            owns: owns.iter().map(|s| s.to_string()).collect(),
            lib,
        }
    }

    fn rules(text: &str) -> Rules {
        toml::from_str(&format!("layer = []\n{text}")).unwrap()
    }

    #[test]
    fn files_map_to_the_longest_owning_prefix() {
        let ws = vec![
            member("kit", &["components/kit/"], true),
            member("otto", &["src/", "tests/", "build.rs"], true),
        ];
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
    fn affects_rules_translate_into_ordered_path_rules() {
        let r = rules(
            r#"
            [[affects]]
            paths = ["resources/", "otto_config*.toml"]
            crates = ["otto"]
            areas = ["packaging"]
            reason = "r"
            [[affects]]
            paths = ["Cargo.toml"]
            crates = ["*"]
            reason = "shared"
            [[affects]]
            paths = ["docs/"]
            crates = []
            reason = "prose"
            "#,
        );
        let ws = vec![
            member("kit", &["components/kit/"], true),
            member("otto", &["src/", "build.rs"], true),
        ];
        let d = determinator_rules(&r, &ws).unwrap();
        let shape: Vec<(Vec<String>, DeterminatorMarkChanged, DeterminatorPostRule)> = d
            .path_rules
            .iter()
            .map(|p| (p.globs.clone(), p.mark_changed.clone(), p.post_rule))
            .collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let pkgs = |v: &[&str]| DeterminatorMarkChanged::Packages(s(v));
        use DeterminatorPostRule::*;
        assert_eq!(
            shape,
            vec![
                (
                    s(&["resources/**", "otto_config*.toml"]),
                    pkgs(&["otto"]),
                    Fallthrough
                ),
                (
                    s(&["Cargo.toml"]),
                    DeterminatorMarkChanged::All,
                    Fallthrough
                ),
                (
                    s(&["components/kit/**", "src/**", "build.rs"]),
                    pkgs(&[]),
                    SkipRules
                ),
                (
                    s(&["resources/**", "otto_config*.toml", "Cargo.toml", "docs/**"]),
                    pkgs(&[]),
                    Skip
                ),
                (s(&["**"]), DeterminatorMarkChanged::All, Skip),
            ]
        );
        // The rules round-trip through determinator's own TOML schema, with
        // its defaults off.
        let text = toml::to_string(&d).unwrap();
        assert!(text.contains("use-default-rules = false"), "{text}");
    }

    #[test]
    fn globs_match_as_determinator_matches() {
        use crate::rules::path_matches as m;
        assert!(m("otto_config*.toml", "otto_config.example.toml"));
        assert!(m("**/*.md", "README.md"));
        assert!(m("**/*.md", "components/x/docs/a.md"));
        // globset's default: `*` crosses directories.
        assert!(m("*.md", "docs/a.md"));
        assert!(m("docs/", "docs/a/b.png"));
        assert!(!m("docs/", "docsx/a"));
        assert!(!m("docs/", "docs"));
        assert!(m("PKGBUILD*", "PKGBUILD-git"));
        assert!(!m("Cargo.toml", "components/x/Cargo.toml"));
        assert!(m("a.b", "a.b") && !m("a.b", "axb"));
    }

    #[test]
    fn chains_explain_each_member() {
        let mut sel = Selection::default();
        sel.packages.insert(
            "kit".into(),
            Why::Rule {
                file: "protocols/p.xml".into(),
                reason: "generated".into(),
            },
        );
        sel.packages.insert(
            "bar".into(),
            Why::Via {
                path: vec!["auth".into(), "kit".into()],
                dev: false,
            },
        );
        sel.packages.insert(
            "otto".into(),
            Why::Via {
                path: vec!["bar".into()],
                dev: true,
            },
        );
        sel.packages.insert("rdp".into(), Why::Deps);
        assert_eq!(
            sel.chain("otto"),
            "otto <- (dev) bar <- auth <- kit <- protocols/p.xml ([[affects]]: generated)"
        );
        assert!(sel.chain("rdp").contains("resolved dependencies changed"));
        let text = explain(&sel);
        assert!(
            text.contains("  bar <- auth <- kit <- protocols/p.xml"),
            "{text}"
        );
    }

    fn render(out: Vec<(&'static str, String)>) -> String {
        out.into_iter().map(|(k, v)| format!("{k}={v}\n")).collect()
    }

    #[test]
    fn outputs_derive_areas_and_package_lists() {
        let ws = vec![
            member("otto", &["src/"], true),
            member("otto-kit", &["components/otto-kit/"], true),
            member("otto-bar", &["components/otto-bar/"], false),
            member("otto-files", &["components/otto-files/"], true),
            member("otto-rdp", &["components/otto-rdp/"], false),
            member("otto-arch", &["tools/otto-arch/"], false),
        ];
        let run = |names: &[&str], packaging: bool| {
            let sel = Selection {
                everything: None,
                packages: names
                    .iter()
                    .map(|n| (n.to_string(), Why::File("f".into())))
                    .collect(),
                packaging: packaging.then(|| "PKGBUILD".to_owned()),
            };
            render(outputs(&sel, &ws))
        };
        assert_eq!(
            run(&["otto-bar"], false),
            "all=false\npackages=otto-bar\ncompositor=false\nrdp=false\nportal=false\n\
             files=false\npackaging=false\nrust=true\nclippy_args=-p otto-bar\n\
             test_lib_args=\ntest_bins_args=-p otto-bar\n"
        );
        let kit = run(&["otto", "otto-bar", "otto-files", "otto-kit"], false);
        assert!(kit.contains("compositor=true\n") && kit.contains("files=true\n"));
        assert!(kit.contains("clippy_args=-p otto-bar -p otto-files -p otto-kit\n"));
        // otto-files' tests run in a step of their own.
        assert!(kit.contains("test_lib_args=-p otto -p otto-kit\n"), "{kit}");

        // The tool alone: the architecture job checks it, the check job
        // has nothing to do.
        let tool = run(&["otto-arch"], false);
        assert!(tool.contains("rust=false\n") && tool.contains("clippy_args=\n"));

        let none = run(&[], true);
        assert!(none.contains("packages=\n") && none.contains("rust=false\n"));
        assert!(none.contains("packaging=true\n"));
    }

    /// The tool's "everything" and ci.yml's fallback for when the tool
    /// cannot run write the same lines.
    #[test]
    fn everything_matches_the_ci_fallback() {
        let ws = vec![member("otto", &["src/"], true)];
        let sel = Selection::everything(&ws, "test".into());
        assert_eq!(test_args(&sel), "-p otto");
        let tool = render(outputs(&sel, &ws)).replace("packages=otto\n", "packages=\n");

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        let start = ci
            .find("<<'EVERYTHING'\n")
            .expect("ci.yml writes the fallback outputs with <<'EVERYTHING'")
            + "<<'EVERYTHING'\n".len();
        let block: String = ci[start..]
            .lines()
            .map(str::trim)
            .take_while(|l| *l != "EVERYTHING")
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(block, tool);
    }
}
