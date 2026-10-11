//! Checks the workspace against `architecture.toml`: crate layers, the
//! compositor's module boundaries and single-home source patterns. See the
//! rules file for what each rule means, and `docs/developer/architecture.md`
//! for the picture.
//!
//! ```sh
//! cargo run -p otto-arch            # check
//! cargo run -p otto-arch -- --bless # also regenerate the layer graph in the docs
//! ```

mod check;
mod docs;
mod rules;
mod workspace;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let bless = std::env::args().skip(1).any(|a| a == "--bless");
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
    let doc = root.join(docs::DOC);
    if bless {
        docs::write(&doc, &rules, &crates)?;
    } else if let Some(p) = docs::check(&doc, &rules, &crates)? {
        problems.push(p);
    }
    Ok(problems)
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
