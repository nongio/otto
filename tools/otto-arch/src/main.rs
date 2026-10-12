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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use affected::{Member, Selection};

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
files included. Without a usable --base, everything is affected.
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
    let (members, files, sel) = match affected(root, base.as_deref(), head.as_deref()) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("affected: {e}");
            return ExitCode::FAILURE;
        }
    };
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

/// The members, the changed files and what they reach. Anything that keeps
/// the change from being worked out selects everything instead; only not
/// knowing the members at all is an error.
fn affected(
    root: &Path,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<(Vec<Member>, Vec<String>, Selection), String> {
    let members = match workspace::members(root) {
        Ok(packages) => affected::members(root, &packages),
        Err(e) => {
            let members = affected::members_from_manifests(root)?;
            let sel = affected::everything(&members, format!("cargo metadata: {e}"));
            return Ok((members, Vec::new(), sel));
        }
    };
    let everything = |reason: String| {
        let sel = affected::everything(&members, reason);
        Ok((members.clone(), Vec::new(), sel))
    };
    let rules = match rules::load(&root.join("architecture.toml")) {
        Ok(r) => r,
        Err(e) => return everything(e),
    };
    let Some(base) = base else {
        return everything("no diff base".into());
    };
    if let Err(e) = affected::check_base(root, base) {
        return everything(e);
    }
    let files = match affected::changed_files(root, base, head) {
        Ok(f) => f,
        Err(e) => return everything(e),
    };
    let names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
    let sel = affected::select(&members, &rules, &files, || {
        let old = affected::lockfile(root, Some(base))?;
        let new = affected::lockfile(root, head)?;
        affected::lock_changes(&old, &new, &names)
    });
    Ok((members, files, sel))
}

#[cfg(test)]
mod tests {
    /// The real workspace passes, so `cargo test` catches a violation too.
    #[test]
    fn workspace_follows_architecture_toml() {
        let problems = super::run(&super::repo_root(), false).unwrap();
        assert!(problems.is_empty(), "\n{}", problems.join("\n\n"));
    }
}
