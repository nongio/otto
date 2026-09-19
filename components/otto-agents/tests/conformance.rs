//! The vendored AHP conformance corpora, run against the types and reducers
//! this service is built on.
//!
//! `spec_pin.rs` proves we are *compiled against* the protocol version we
//! vendored. These two corpora are what upstream ships so an implementation
//! can prove it *agrees* with the spec: every reducer case is a state, a run
//! of actions and the state they must leave behind, and every round-trip case
//! is a wire payload that must decode and re-encode to one canonical form.
//!
//! The comparison is key-order-independent but key-presence-sensitive: `null`
//! is not absent and absent is not `null`, which is exactly what a JSON value
//! comparison gives. Group B cases carry extra unmodelled wire keys; a typed
//! decoder like this one drops them, and asserts the dropped form — so this
//! file asserts `acceptableOutputs[0]` throughout and never `preservedOutput`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use ahp::reducers::{
    apply_action_to_annotations, apply_action_to_automation, apply_action_to_automation_run,
    apply_action_to_changeset, apply_action_to_chat, apply_action_to_resource_watch,
    apply_action_to_root, apply_action_to_session, apply_action_to_terminal,
};
use ahp_types::actions::StateAction;

fn corpus(kind: &str) -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("spec/upstream/types/test-cases")
        .join(kind);
    let mut cases: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("reading {}: {err}", dir.display()))
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
            let value = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()));
            (name_of(&path), value)
        })
        .collect();
    assert!(!cases.is_empty(), "no cases in {}", dir.display());
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    cases
}

/// A value with whole-number floats written as integers.
///
/// Rust holds the spec's `number` fields as `f64`, so a `min` that arrived as
/// `0` re-encodes as `0.0`. Upstream's own Rust corpus harness normalises the
/// two together for exactly this reason, and names this case; it is a
/// representation of the same JSON number, not a different value.
fn normalised(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), normalised(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(normalised).collect()),
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.as_i64().is_none() && float.fract() == 0.0 => {
                match serde_json::Number::from_f64(float) {
                    Some(_) if float.abs() < 9.007_199_254_740_992e15 => {
                        Value::Number((float as i64).into())
                    }
                    _ => value.clone(),
                }
            }
            _ => value.clone(),
        },
        other => other.clone(),
    }
}

/// [`normalised`], and with every explicit `null` member dropped as well.
///
/// The reducer corpus writes a state's absent members out as `null`, the way
/// the TypeScript reference reducer encodes them. Rust holds them as `Option`
/// and serde leaves `None` out, so the two encodings differ in form for every
/// state that has an empty member. What the corpus is checking here is which
/// state a run of actions lands on, and that is what this compares; wire
/// fidelity, where `null` and absent are genuinely different, is the
/// round-trip corpus's job and is asserted exactly there.
fn substance(value: &Value) -> Value {
    match normalised(value) {
        Value::Object(members) => Value::Object(
            members
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key, substance(&value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(substance).collect()),
        other => other,
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned()
}

/// Run every action over `initial` with `$apply`, and give back the state that
/// leaves behind, as JSON to compare with the case's `expected`.
macro_rules! reduced {
    ($state:ty, $apply:path, $case:expr) => {{
        let mut state: $state = serde_json::from_value($case["initial"].clone())
            .map_err(|err| format!("initial state: {err}"))?;
        for action in $case["actions"].as_array().expect("an actions array") {
            let action: StateAction = serde_json::from_value(action.clone())
                .map_err(|err| format!("action {action}: {err}"))?;
            $apply(&mut state, &action);
        }
        serde_json::to_value(&state).map_err(|err| format!("re-encoding state: {err}"))?
    }};
}

/// Every reducer case upstream ships, over all nine reducers — including the
/// six this service does not drive itself, which are still part of what a
/// compliant host is expected to get right.
#[test]
fn the_reducer_corpus_lands_on_the_state_it_expects() {
    use ahp_types::state::{
        AnnotationsState, AutomationRunState, AutomationState, ChangesetState, ChatState,
        ResourceWatchState, RootState, SessionState, TerminalState,
    };

    let mut failures = Vec::new();
    for (name, case) in corpus("reducers") {
        let kind = case["reducer"].as_str().expect("a reducer name").to_owned();
        let found: Result<Value, String> = (|| {
            Ok(match kind.as_str() {
                "root" => reduced!(RootState, apply_action_to_root, case),
                "session" => reduced!(SessionState, apply_action_to_session, case),
                "chat" => reduced!(ChatState, apply_action_to_chat, case),
                "terminal" => reduced!(TerminalState, apply_action_to_terminal, case),
                "changeset" => reduced!(ChangesetState, apply_action_to_changeset, case),
                "annotations" => reduced!(AnnotationsState, apply_action_to_annotations, case),
                "resourceWatch" => {
                    reduced!(ResourceWatchState, apply_action_to_resource_watch, case)
                }
                "automation" => reduced!(AutomationState, apply_action_to_automation, case),
                "automationRun" => {
                    reduced!(AutomationRunState, apply_action_to_automation_run, case)
                }
                other => return Err(format!("no reducer for {other:?}")),
            })
        })();

        match found {
            Err(err) => failures.push(format!("{name}: {err}")),
            Ok(found) if substance(&found) != substance(&case["expected"]) => {
                failures.push(format!(
                    "{name} ({}): {}\n  expected {}\n  found    {}",
                    kind,
                    case["description"].as_str().unwrap_or(""),
                    case["expected"],
                    found,
                ))
            }
            Ok(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} reducer cases disagree with the spec:\n{}",
        failures.len(),
        failures.join("\n"),
    );
}

/// Decode `$type` from the case's `input` and hand back its re-encoding.
macro_rules! round_tripped {
    ($type:ty, $case:expr) => {{
        let decoded: $type = serde_json::from_value($case["input"].clone())
            .map_err(|err| format!("decoding: {err}"))?;
        serde_json::to_value(&decoded).map_err(|err| format!("re-encoding: {err}"))?
    }};
}

/// Every wire type the round-trip corpus covers, decoded and re-encoded.
#[test]
fn the_round_trip_corpus_re_encodes_to_the_canonical_form() {
    use ahp_types::actions::ActionEnvelope;
    use ahp_types::commands::{
        ChangesetOperationTarget, ChatSource, Implementation, InitializeResult,
    };
    use ahp_types::common::StringOrMarkdown;
    use ahp_types::messages::JsonRpcMessage;
    use ahp_types::notifications::{PartialSessionSummary, SessionAddedParams};
    use ahp_types::state::{
        ChatInputQuestion, Customization, SessionStatus, SessionSummary, Snapshot,
    };

    let mut failures = Vec::new();
    for (name, case) in corpus("round-trips") {
        let kind = case["type"].as_str().expect("a type name").to_owned();
        let found: Result<Value, String> = (|| {
            Ok(match kind.as_str() {
                "ActionEnvelope" => round_tripped!(ActionEnvelope, case),
                "StateAction" => round_tripped!(StateAction, case),
                "ChangesetOperationTarget" => round_tripped!(ChangesetOperationTarget, case),
                "ChatSource" => round_tripped!(ChatSource, case),
                "Implementation" => round_tripped!(Implementation, case),
                "InitializeResult" => round_tripped!(InitializeResult, case),
                "StringOrMarkdown" => round_tripped!(StringOrMarkdown, case),
                "JsonRpcMessage" => round_tripped!(JsonRpcMessage, case),
                "PartialSessionSummary" => round_tripped!(PartialSessionSummary, case),
                "SessionAddedParams" => round_tripped!(SessionAddedParams, case),
                "ChatInputQuestion" => round_tripped!(ChatInputQuestion, case),
                "Customization" => round_tripped!(Customization, case),
                "SessionStatus" => round_tripped!(SessionStatus, case),
                "SessionSummary" => round_tripped!(SessionSummary, case),
                "Snapshot" => round_tripped!(Snapshot, case),
                other => return Err(format!("no type for {other:?}")),
            })
        })();

        // One canonical form, by the corpus's own rule: a second entry would
        // cement an observed-but-wrong divergence as acceptable.
        let acceptable = case["acceptableOutputs"]
            .as_array()
            .expect("acceptableOutputs");
        assert_eq!(acceptable.len(), 1, "{name}: not a single canonical form");

        match found {
            Err(err) => failures.push(format!("{name}: {err}")),
            Ok(found) if normalised(&found) != normalised(&acceptable[0]) => {
                failures.push(format!(
                    "{name} ({kind}):\n  expected {}\n  found    {}",
                    acceptable[0], found,
                ))
            }
            Ok(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} round-trip cases disagree with the spec:\n{}",
        failures.len(),
        failures.join("\n"),
    );
}
