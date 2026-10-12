//! Checks the workspace against `architecture.toml`: crate layers, the
//! compositor's module boundaries and single-home source patterns. See the
//! rules file for what each rule means, and `docs/developer/architecture.md`
//! for the picture.
//!
//! ```sh
//! cargo run -p otto-arch            # check
//! cargo run -p otto-arch -- --bless # also regenerate the layer graph in the docs
//! cargo run -q -p otto-arch -- affected --base origin/main [--head REV] [--explain]
//!                                   # the members a change reaches, as CI outputs
//! cargo run -q -p otto-arch -- affected --base origin/main --test-args | xargs -r cargo test
//! ```

mod affected;
mod check;
mod docs;
mod rules;
mod workspace;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use affected::{Checkout, Member, Selection};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("affected") {
        return affected_main(&repo_root(), &args[1..]);
    }
    let bless = args.iter().any(|a| a == "--bless");
    match run(&repo_root(), bless) {
        Ok(problems) if problems.is_empty() => {
            println!("architecture: ok");
            ExitCode::SUCCESS
        }
        Ok(problems) => {
            for p in &problems {
                eprintln!("{p}\n");
            }
            eprintln!(
                "architecture: {} problem(s). docs/developer/architecture.md says what each \
                 rule protects and how to move the code.",
                problems.len()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("architecture: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root: two levels above this crate.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("tools/otto-arch sits two levels below the workspace root")
        .to_path_buf()
}

fn run(root: &Path, bless: bool) -> Result<Vec<String>, String> {
    let rules = rules::load(&root.join("architecture.toml"))?;
    let (crates, graph) = workspace::load(root)?;
    let mut problems = check::all(root, &rules, &crates, &graph)?;
    let members = affected::members(root, &crates);
    let tracked = affected::tracked_files(root)?;
    problems.extend(check::affects(root, &rules, &members, &tracked)?);
    let doc = root.join(docs::DOC);
    if bless {
        docs::write(&doc, &rules, &crates)?;
    } else if let Some(p) = docs::check(&doc, &rules, &crates)? {
        problems.push(p);
    }
    Ok(problems)
}

const AFFECTED_USAGE: &str =
    "usage: otto-arch affected [--base REV] [--head REV] [--explain] [--test-args]

Prints which workspace members the change from REV reaches, as key=value
lines for $GITHUB_OUTPUT. --head defaults to the working tree, untracked
files included. Without a usable --base, or on any error, everything is
affected.
  --explain    also print, on stderr, why each member is affected
  --test-args  print only `-p <member>` for each affected member";

fn affected_main(root: &Path, args: &[String]) -> ExitCode {
    let (mut base, mut head, mut explain, mut test_args) = (None, None, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base" => base = it.next().cloned(),
            "--head" => head = it.next().cloned(),
            "--explain" => explain = true,
            "--test-args" => test_args = true,
            _ => {
                eprintln!("{AFFECTED_USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    // A panic in the selection (guppy and determinator `expect` on graphs
    // they do not understand) selects everything like any other failure.
    let found = std::panic::catch_unwind(AssertUnwindSafe(|| {
        affected(root, base.as_deref(), head.as_deref())
    }));
    let (members, files, sel) = found.unwrap_or_else(|_| {
        everything(
            affected::members_from_manifests(root).unwrap_or_default(),
            "otto-arch panicked working out the selection".into(),
        )
    });
    if explain {
        if sel.everything.is_none() {
            eprintln!("changed files:");
            for f in &files {
                eprintln!("  {f}");
            }
            eprintln!("affected:");
        }
        eprint!("{}", affected::explain(&sel));
    }
    if test_args {
        println!("{}", affected::test_args(&sel));
    } else {
        for (k, v) in affected::outputs(&sel, &members) {
            println!("{k}={v}");
        }
    }
    ExitCode::SUCCESS
}

type Found = (Vec<Member>, Vec<String>, Selection);

fn everything(members: Vec<Member>, reason: String) -> Found {
    let sel = Selection::everything(&members, reason);
    (members, Vec::new(), sel)
}

/// The members, the changed files and what they reach. Anything that keeps
/// the change from being worked out selects everything instead; not even
/// knowing the members still selects everything, with the workspace-wide
/// argument lists.
fn affected(root: &Path, base: Option<&str>, head: Option<&str>) -> Found {
    let from_manifests = |dir: &Path| affected::members_from_manifests(dir).unwrap_or_default();
    let Some(base) = base else {
        return everything(from_manifests(root), "no diff base".into());
    };
    if let Err(e) = affected::check_base(root, base) {
        return everything(from_manifests(root), e);
    }
    // The head's tree: the working tree, or a checkout of --head.
    let head_checkout = match head.map(|h| Checkout::new(root, h, "head")).transpose() {
        Ok(c) => c,
        Err(e) => return everything(from_manifests(root), e),
    };
    let new_root = head_checkout.as_ref().map_or(root, |c| c.dir.as_path());
    let new_json = match workspace::metadata_json(new_root) {
        Ok(j) => j,
        Err(e) => return everything(from_manifests(new_root), e),
    };
    let members = match workspace::parse(&new_json) {
        Ok((packages, _)) => affected::members(new_root, &packages),
        Err(e) => return everything(from_manifests(new_root), e),
    };
    let fail = |e: String| everything(members.clone(), e);
    let rules = match rules::load(&new_root.join("architecture.toml")) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let files = match affected::changed_files(root, base, head) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    let old_json = match Checkout::new(root, base, "base").and_then(|c| {
        workspace::metadata_json(&c.dir).map_err(|e| format!("at the base {base}: {e}"))
    }) {
        Ok(j) => j,
        Err(e) => return fail(e),
    };
    let sel = affected::graph(&old_json)
        .and_then(|old| Ok((old, affected::graph(&new_json)?)))
        .and_then(|(old, new)| affected::select(&old, &new, &members, &rules, &files));
    match sel {
        Ok(sel) => (members, files, sel),
        Err(e) => fail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real workspace passes, so `cargo test` catches a violation too.
    #[test]
    fn workspace_follows_architecture_toml() {
        let problems = run(&repo_root(), false).unwrap();
        assert!(problems.is_empty(), "\n{}", problems.join("\n\n"));
    }

    /// The translated rules on the real workspace: where representative
    /// files land.
    #[test]
    fn translated_rules_place_real_files() {
        use determinator::rules::PathMatch;
        let root = repo_root();
        let json = workspace::metadata_json(&root).unwrap();
        let (packages, _) = workspace::parse(&json).unwrap();
        let members = affected::members(&root, &packages);
        let rules = rules::load(&root.join("architecture.toml")).unwrap();
        let graph = affected::graph(&json).unwrap();
        let mut det = determinator::Determinator::new(&graph, &graph);
        det.set_rules(&affected::determinator_rules(&rules, &members).unwrap())
            .unwrap();
        let place = |path: &str| {
            let mut names = Vec::new();
            let m = det.match_path(path, |id| {
                names.push(graph.metadata(id).unwrap().name().to_owned())
            });
            names.sort();
            names.dedup();
            match m {
                PathMatch::RuleMatchedAll | PathMatch::NoMatches => vec!["*".to_owned()],
                _ => names,
            }
        };
        assert_eq!(place("src/main.rs"), ["otto"]);
        assert_eq!(place("crates/otto-render/src/lib.rs"), ["otto-render"]);
        assert_eq!(
            place("components/otto-launcher/src/main.rs"),
            ["otto-launcher"]
        );
        // Rules add up with the file's own crate.
        assert_eq!(
            place("components/otto-agents/src/config.rs"),
            ["otto-agents", "otto-kit"]
        );
        assert_eq!(place("components/otto-lock/otto-lock.pam"), ["otto-lock"]);
        // A top-level file does not land on the root package.
        assert!(place("README.md").is_empty());
        assert!(place("docs/developer/rendering.md").is_empty());
        assert!(place("Cargo.lock").is_empty());
        assert_eq!(place("resources/locales/en.ftl").len(), 2);
        // Shared files, the rules and the tool reach everything; so does a
        // file nothing covers.
        for f in [
            "Cargo.toml",
            ".github/workflows/ci.yml",
            "architecture.toml",
            "tools/otto-arch/src/affected.rs",
            "tools/otto-arch/Cargo.toml",
            "a-new-top-level-file",
            "components/no-such-crate/src/lib.rs",
        ] {
            assert_eq!(place(f), ["*"], "{f}");
        }
    }

    /// No base, or one that is not a commit: everything, not an error.
    #[test]
    fn a_missing_base_selects_everything() {
        let root = repo_root();
        for base in [None, Some(""), Some("0000000000"), Some("no-such-rev")] {
            let (members, _, sel) = affected(&root, base, None);
            assert!(sel.everything.is_some(), "{base:?}");
            assert!(members.iter().any(|m| m.name == "otto"));
            let out = affected::outputs(&sel, &members);
            assert!(out.contains(&("all", "true".to_owned())));
        }
    }
}
