//! The crate graph in `docs/developer/architecture.md`, generated from
//! `architecture.toml` and the manifests so it cannot drift from either.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use crate::rules::Rules;
use crate::workspace::Package;

pub const DOC: &str = "docs/developer/architecture.md";
const BEGIN: &str = "<!-- otto-arch:graph:begin (generated: cargo run -p otto-arch -- --bless) -->";
const END: &str = "<!-- otto-arch:graph:end -->";

/// The generated section: one subgraph per layer, an edge per dependency
/// between workspace crates. Layers with `graph = false` (sample clients,
/// tools: standalone by definition) are left out to keep the picture readable.
fn graph(rules: &Rules, crates: &[Package]) -> String {
    let shown: Vec<_> = rules.layers.iter().filter(|l| l.graph).collect();
    let names: BTreeSet<&str> = shown
        .iter()
        .flat_map(|l| l.crates.iter().map(String::as_str))
        .collect();

    let mut out = String::from("```mermaid\nflowchart BT\n");
    for l in &shown {
        let _ = writeln!(out, "  subgraph {}[\"{}\"]", l.name, l.name);
        for c in &l.crates {
            let _ = writeln!(out, "    {}[{c}]", id(c));
        }
        out.push_str("  end\n");
    }
    for c in crates.iter().filter(|c| names.contains(c.name.as_str())) {
        let deps: BTreeSet<&str> = c
            .dependencies
            .iter()
            .filter(|d| !d.is_dev() && names.contains(d.name.as_str()))
            .map(|d| d.name.as_str())
            .collect();
        for d in deps {
            let _ = writeln!(out, "  {} --> {}", id(&c.name), id(d));
        }
    }
    out.push_str("```\n");
    out
}

fn id(name: &str) -> String {
    name.replace('-', "_")
}

fn render(current: &str, rules: &Rules, crates: &[Package]) -> Result<String, String> {
    let (head, rest) = current
        .split_once(BEGIN)
        .ok_or_else(|| format!("{DOC} has no `{BEGIN}` marker"))?;
    let (_, tail) = rest
        .split_once(END)
        .ok_or_else(|| format!("{DOC} has no `{END}` marker"))?;
    Ok(format!(
        "{head}{BEGIN}\n{}{END}{tail}",
        graph(rules, crates)
    ))
}

/// A problem when the committed graph differs from the generated one.
pub fn check(path: &Path, rules: &Rules, crates: &[Package]) -> Result<Option<String>, String> {
    let current =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok((render(&current, rules, crates)? != current).then(|| {
        format!("{DOC} is out of date\n  Run `cargo run -p otto-arch -- --bless` and commit it.")
    }))
}

pub fn write(path: &Path, rules: &Rules, crates: &[Package]) -> Result<(), String> {
    let current =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let next = render(&current, rules, crates)?;
    if next != current {
        std::fs::write(path, next).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    Ok(())
}
