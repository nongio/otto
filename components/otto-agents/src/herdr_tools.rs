//! What a herdr session's agent runs to reach Otto (plan 0016): the `ask` MCP
//! server and Claude Code's `PermissionRequest` hook.
//!
//! Both run inside the session's terminal, where herdr sets `HC_TERMINAL_ID`.
//! Each writes its request as JSON in a label on that terminal and waits for
//! the answer label, with the same id, that the herdr backend writes back
//! from Ask or a dialog:
//!
//! | Request | Answer |
//! |---|---|
//! | `otto.ask` `{id, questions}` | `otto.answer` `{id, answers}` |
//! | `otto.permission` `{id, tool_name, tool_input, cwd, suggestions}` | `otto.permission.answer` `{id, allow, always}` |
//!
//! `suggestions` are Claude's `permission_suggestions`, the "don't ask again"
//! choices its own prompt offers; `always` is the index of the one picked.
//!
//! Both labels are cleared once the wait ends, answered or not.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// How long the permission hook waits for Otto. Claude gives the hook ten
/// minutes; giving up first lets Claude ask in the terminal instead.
const PERMISSION_PATIENCE: Duration = Duration::from_secs(540);

/// The terminal this runs in, as herdr names it.
fn terminal() -> Option<String> {
    std::env::var("HC_TERMINAL_ID")
        .ok()
        .filter(|terminal| !terminal.is_empty())
}

fn hc() -> Command {
    let mut command = Command::new("hc");
    command.stdin(Stdio::null()).kill_on_drop(true);
    command
}

/// A short id that pairs a request with its answer.
fn request_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_owned()
}

async fn label(terminal: &str, pairs: &[String]) {
    let _ = hc()
        .arg("label")
        .arg(terminal)
        .args(pairs)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// The answer under `key` on `terminal` whose id is `id`, if one is there.
fn answer_in(labels: Option<&Value>, key: &str, id: &str) -> Option<Value> {
    let raw = labels?.get(key)?.as_str()?;
    let answer: Value = serde_json::from_str(raw).ok()?;
    (answer.get("id").and_then(Value::as_str) == Some(id)).then_some(answer)
}

/// Writes `request` (which carries `id`) as `request_key` on `terminal`, and
/// waits for the answer under `answer_key` with the same id. Both labels are
/// cleared when the wait ends.
async fn ask_through_labels(
    terminal: &str,
    request_key: &str,
    answer_key: &str,
    id: &str,
    request: &Value,
) -> Option<Value> {
    label(terminal, &[format!("{request_key}={request}")]).await;
    let answer = wait_for(terminal, answer_key, id).await;
    clear(terminal, request_key, answer_key).await;
    answer
}

async fn clear(terminal: &str, request_key: &str, answer_key: &str) {
    label(
        terminal,
        &[format!("{request_key}="), format!("{answer_key}=")],
    )
    .await;
}

/// Follows `terminal`'s events until a label `key` answers `id`: the
/// snapshot first, then each change of labels. `None` when the terminal
/// goes, or herdr cannot be followed.
async fn wait_for(terminal: &str, key: &str, id: &str) -> Option<Value> {
    let mut child = hc()
        .args(["events", "--json", "--terminal", terminal])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(snapshot) = event.get("terminals").and_then(Value::as_array) {
            let mine = snapshot
                .iter()
                .find(|item| item.get("address").and_then(Value::as_str) == Some(terminal));
            if let Some(answer) = answer_in(mine.and_then(|item| item.get("labels")), key, id) {
                return Some(answer);
            }
            continue;
        }
        let params = event.get("params");
        match params
            .and_then(|params| params.get("type"))
            .and_then(Value::as_str)
        {
            Some("labels") => {
                let output = hc()
                    .args(["get", terminal, "--json"])
                    .stderr(Stdio::null())
                    .output()
                    .await
                    .ok()?;
                let item: Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
                if let Some(answer) = answer_in(item.get("labels"), key, id) {
                    return Some(answer);
                }
            }
            Some("removed") => return None,
            _ => {}
        }
    }
    None
}

/// The `ask` tool, with `AskUserQuestion`'s input shape so the agent's habits
/// carry over.
fn ask_tool() -> Value {
    json!({
        "name": "ask",
        "description": "Ask the person a question and wait for their answer. Use this whenever you need the person to choose or decide something.",
        "inputSchema": {
            "type": "object",
            "required": ["questions"],
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["question"],
                        "properties": {
                            "question": {"type": "string"},
                            "header": {"type": "string"},
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "required": ["label"],
                                    "properties": {
                                        "label": {"type": "string"},
                                        "description": {"type": "string"}
                                    }
                                }
                            },
                            "multiSelect": {"type": "boolean"}
                        }
                    }
                }
            }
        }
    })
}

/// Asks `questions` through the terminal's labels: the tool's result text,
/// and whether it failed.
async fn ask(questions: Value, id: &str) -> (String, bool) {
    let Some(terminal) = terminal() else {
        return (
            "Not running in a herdr terminal: ask in the terminal instead.".to_owned(),
            true,
        );
    };
    let request = json!({ "id": id, "questions": questions });
    match ask_through_labels(&terminal, "otto.ask", "otto.answer", id, &request).await {
        Some(answer) => (
            answer
                .get("answers")
                .cloned()
                .unwrap_or_else(|| json!({}))
                .to_string(),
            false,
        ),
        None => (
            "No answer came: ask in the terminal instead.".to_owned(),
            true,
        ),
    }
}

/// `otto-agents herdr-ask`: an MCP server on standard input and output with
/// one tool, `ask`. Each call waits on its own, so pings and cancellations
/// are answered meanwhile.
pub async fn serve_ask() -> anyhow::Result<()> {
    let (replies, mut outgoing) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = outgoing.recv().await {
            let mut line = message.to_string();
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    // Each call in hand, by its JSON-RPC id, with the id of its question.
    let mut calls: HashMap<String, (tokio::task::JoinHandle<()>, String)> = HashMap::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        calls.retain(|_, (call, _)| !call.is_finished());
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").cloned().unwrap_or_default();
        if method == Some("notifications/cancelled") {
            let cancelled = params.get("requestId").map(Value::to_string);
            if let Some((call, question)) = cancelled.and_then(|id| calls.remove(&id)) {
                call.abort();
                if let Some(terminal) = terminal() {
                    // Only this call's question: another may be waiting too.
                    let asked = hc()
                        .args(["get", &terminal, "--json"])
                        .output()
                        .await
                        .ok()
                        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
                    let still = asked
                        .as_ref()
                        .and_then(|item| item.pointer("/labels/otto.ask"))
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                        .is_some_and(|ask| {
                            ask.get("id").and_then(Value::as_str) == Some(&question)
                        });
                    if still {
                        clear(&terminal, "otto.ask", "otto.answer").await;
                    }
                }
            }
            continue;
        }
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let reply = |result: Result<Value, (i64, &str)>| match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, text)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } })
            }
        };
        let answer = match method {
            Some("initialize") => reply(Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or_else(|| json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") },
            }))),
            Some("tools/list") => reply(Ok(json!({ "tools": [ask_tool()] }))),
            Some("ping") => reply(Ok(json!({}))),
            Some("tools/call") if params.get("name").and_then(Value::as_str) == Some("ask") => {
                let questions = params
                    .pointer("/arguments/questions")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let question = request_id();
                let replies = replies.clone();
                let call = {
                    let question = question.clone();
                    tokio::spawn(async move {
                        let (text, failed) = ask(questions, &question).await;
                        let _ = replies.send(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "content": [{ "type": "text", "text": text }], "isError": failed },
                        }));
                    })
                };
                calls.insert(message["id"].to_string(), (call, question));
                continue;
            }
            Some("tools/call") => reply(Err((-32602, "no such tool"))),
            _ => reply(Err((-32601, "unknown method"))),
        };
        let _ = replies.send(answer);
    }
    // Standard input closed: the agent is gone, and so are its questions.
    for (call, _) in calls.into_values() {
        call.abort();
    }
    if let Some(terminal) = terminal() {
        clear(&terminal, "otto.ask", "otto.answer").await;
    }
    drop(replies);
    let _ = writer.await;
    Ok(())
}

/// What the hook prints for Otto's answer: Claude's `PermissionRequest`
/// decision. `always` is the suggestion picked with it, which Claude applies
/// as its own "don't ask again" choice.
fn decision(allow: bool, always: Option<&Value>) -> Value {
    let decision = if allow {
        match always {
            Some(update) => json!({ "behavior": "allow", "updatedPermissions": [update] }),
            None => json!({ "behavior": "allow" }),
        }
    } else {
        json!({ "behavior": "deny", "message": "The person declined this in Otto." })
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
}

/// `otto-agents herdr-permission`: Claude Code's `PermissionRequest` hook,
/// answered from Otto. With no answer in time, or outside herdr, it prints
/// nothing and Claude asks in the terminal as usual.
pub async fn permission_hook() -> anyhow::Result<()> {
    let Some(terminal) = terminal() else {
        return Ok(());
    };
    let mut input = String::new();
    tokio::io::stdin().read_to_string(&mut input).await?;
    let Ok(event) = serde_json::from_str::<Value>(&input) else {
        return Ok(());
    };
    let id = request_id();
    let request = json!({
        "id": id,
        "tool_name": event.get("tool_name").cloned().unwrap_or_else(|| json!("")),
        "tool_input": event.get("tool_input").cloned().unwrap_or_else(|| json!({})),
        "cwd": event.get("cwd").cloned().unwrap_or_else(|| json!("")),
        "suggestions": event.get("permission_suggestions").cloned().unwrap_or_else(|| json!([])),
    });
    let waited = tokio::time::timeout(
        PERMISSION_PATIENCE,
        ask_through_labels(
            &terminal,
            "otto.permission",
            "otto.permission.answer",
            &id,
            &request,
        ),
    )
    .await;
    let answer = match waited {
        Ok(answer) => answer,
        // Given up: the labels are still up, and the wait is gone.
        Err(_) => {
            clear(&terminal, "otto.permission", "otto.permission.answer").await;
            None
        }
    };
    if let Some(answer) = answer {
        let allow = answer.get("allow").and_then(Value::as_bool) == Some(true);
        let always = answer
            .get("always")
            .and_then(Value::as_u64)
            .and_then(|index| request["suggestions"].get(index as usize));
        println!("{}", decision(allow, always));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_counts_only_for_its_own_request() {
        let labels = json!({ "otto.answer": r#"{"id":"abc","answers":{"Pick":"A"}}"# });
        assert!(answer_in(Some(&labels), "otto.answer", "abc").is_some());
        assert!(answer_in(Some(&labels), "otto.answer", "xyz").is_none());
        assert!(answer_in(Some(&json!({})), "otto.answer", "abc").is_none());
        let garbled = json!({ "otto.answer": "not json" });
        assert!(answer_in(Some(&garbled), "otto.answer", "abc").is_none());
    }

    #[test]
    fn the_hook_decides_as_claude_reads_it() {
        assert_eq!(
            decision(true, None),
            json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } } })
        );
        let rule = json!({ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "ls" }], "behavior": "allow", "destination": "localSettings" });
        assert_eq!(
            decision(true, Some(&rule))
                .pointer("/hookSpecificOutput/decision/updatedPermissions/0"),
            Some(&rule)
        );
        assert_eq!(
            decision(false, Some(&rule)).pointer("/hookSpecificOutput/decision/behavior"),
            Some(&json!("deny"))
        );
    }
}
