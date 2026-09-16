//! Guards the lockstep between the vendored spec (`spec/upstream.env`) and the
//! `ahp-types` crate the server is compiled against.

use std::collections::HashMap;
use std::path::PathBuf;

fn spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("spec")
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()))
}

fn upstream_pin() -> HashMap<String, String> {
    read(spec_dir().join("upstream.env"))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn vendored_spec_matches_ahp_types_version() {
    let pin = upstream_pin();
    assert_eq!(
        pin.get("AHP_PROTOCOL_VERSION").map(String::as_str),
        Some(ahp_types::PROTOCOL_VERSION),
        "spec/upstream.env and the ahp-types dependency disagree: \
         re-sync the spec with scripts/sync-spec.sh or bump the AHP crates in Cargo.toml",
    );
}

#[test]
fn vendored_json_schemas_parse() {
    for name in ["state", "actions", "commands", "notifications", "errors"] {
        let path = spec_dir().join(format!("upstream/schema/{name}.schema.json"));
        let text = read(path.clone());
        serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()));
    }
}
