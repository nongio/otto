//! `otto-preview --mcp`: the document tools, as an MCP server an agent starts.
//!
//! The chat beside a document creates its session asking otto-agents to give
//! the agent this server (see `_meta.otto.mcpServers` in plan 0016), with the
//! document's path in `OTTO_PREVIEW_DOC`. The agent starts it and speaks MCP
//! over its stdin and stdout; each tool is one call to the running Preview
//! over `org.otto.Preview1`, which acts on the window showing the document.
//! The server holds no state of its own and never touches the display.
//!
//! JSON-RPC by hand, one message per line, as MCP's stdio transport is: a
//! handful of methods does not need a library (plan 0014, *No MCP crate*).

// Rust guideline compliant 2026-02-21

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{json, Value};
use zbus::blocking::Connection;

use crate::instance::{DBUS_NAME, DBUS_PATH};

/// The environment variable naming the document.
pub const DOC_ENV: &str = "OTTO_PREVIEW_DOC";
/// The MCP revision answered when the client names none.
const PROTOCOL: &str = "2025-06-18";

/// What an agent working on `file` beside the chat is told: by the session
/// when it starts, and by this server when the agent connects. Only which
/// file it is: how to work on it is the Studio agent's own definition,
/// `resources/plugins/otto/agents/studio.md`.
pub fn instructions(file: &Path) -> String {
    format!(
        "The file in the Studio window beside this chat is {}.",
        file.display()
    )
}

/// Serve MCP on stdin and stdout until the client hangs up.
pub fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let doc = std::env::var_os(DOC_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{DOC_ENV} names no document"))?;
    let bus = Connection::session()?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = answer(&bus, &doc, &line) else {
            continue;
        };
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}

/// The reply to one message, `None` for a notification.
fn answer(bus: &Connection, doc: &Path, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => return Some(error(Value::Null, -32700, &format!("parse error: {err}"))),
    };
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "otto-preview", "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions(doc),
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call(bus, doc, &params)),
        _ => Err((-32601, format!("unknown method {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, text)) => error(id, code, &text),
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The tools, with what they take.
fn tools() -> Value {
    let shape = json!({
        "type": "object",
        "description": "{\"kind\": \"rect\"|\"ellipse\"|\"arrow\", \"from\": [x, y], \"to\": [x, y]}, {\"kind\": \"path\", \"points\": [[x, y], ...]}, or {\"kind\": \"image\", \"path\": \"/absolute/picture.png\", \"from\": [x, y], \"to\": [x, y], \"opacity\": 0..1} to lay a picture over the box (a variant, a logo, a crop to compare)",
        "properties": {
            "kind": { "type": "string", "enum": ["rect", "ellipse", "arrow", "path", "image"] },
            "path": { "type": "string", "description": "For an image: the picture file, an absolute path." },
            "opacity": { "type": "number", "description": "For an image: 0 to 1, default 1." },
            "from": { "type": "array", "items": { "type": "number" } },
            "to": { "type": "array", "items": { "type": "number" } },
            "points": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } }
        },
        "required": ["kind"]
    });
    let nothing = json!({ "type": "object", "properties": {} });
    json!([
        {
            "name": "preview_info",
            "description": "What the Preview window beside the chat shows: the file, its kind (picture, pages), its size in pixels, the page, the zoom, and the person's marks waiting to be sent.",
            "inputSchema": nothing,
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "preview_reload",
            "description": "Show the file again after changing it on disk, and keep it as a version the person can step back from. Call after every edit with a short note of what changed; the window keeps its zoom and place.",
            "inputSchema": {
                "type": "object",
                "properties": { "note": { "type": "string", "description": "What this change did, in a few words: \"background removed\"." } }
            }
        },
        {
            "name": "preview_versions",
            "description": "The file's versions, oldest first: the file as it was before the chat, then each change, with its note and which one the file is at now.",
            "inputSchema": nothing,
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "preview_revert",
            "description": "Put an earlier (or later) version back in place of the file; the window shows it. Versions after it stay until the file changes again.",
            "inputSchema": {
                "type": "object",
                "properties": { "version": { "type": "integer", "description": "The version's number, from preview_versions." } },
                "required": ["version"]
            }
        },
        {
            "name": "preview_render",
            "description": "The picture as the window shows it, as an image, with the marks drawn on it unless marks is false. Pictures only for now.",
            "inputSchema": {
                "type": "object",
                "properties": { "marks": { "type": "boolean", "description": "Draw the marks (default true)." } }
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "preview_marks",
            "description": "Every mark on the file: the person's (numbered as on screen) and yours, with shapes and bounds in the picture's pixels or PDF points per page.",
            "inputSchema": nothing,
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "preview_draw",
            "description": "Draw marks on the file to point at things: boxes, ellipses, arrows or paths with short labels, or pictures laid over a box, in the picture's pixels (or PDF points with \"page\"). The person can move a mark by its label and delete it. Replaces the layer of the same name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "layer": { "type": "string", "description": "A name for this set of marks, e.g. \"dust\"; drawing it again replaces it." },
                    "marks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "shape": shape,
                                "label": { "type": "string" },
                                "page": { "type": "integer", "description": "0-based page, for a document of pages." }
                            },
                            "required": ["shape"]
                        }
                    }
                },
                "required": ["layer", "marks"]
            }
        },
        {
            "name": "preview_clear",
            "description": "Take away your layer of marks, all your marks when layer is empty, or the person's marks when layer is \"person\".",
            "inputSchema": {
                "type": "object",
                "properties": { "layer": { "type": "string" } }
            }
        }
    ])
}

/// Run one tool. Failures are results the agent reads, not protocol errors.
fn call(bus: &Connection, doc: &Path, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let path = doc.to_string_lossy().into_owned();
    let text = |text: String| json!({ "content": [{ "type": "text", "text": text }] });
    let outcome: Result<Value, zbus::Error> = (|| {
        Ok(match name {
            "preview_info" => text(dbus::<String>(bus, "Info", &(path.as_str(),))?),
            "preview_marks" => text(dbus::<String>(bus, "Marks", &(path.as_str(),))?),
            "preview_reload" => {
                let note = arguments.get("note").and_then(Value::as_str).unwrap_or("");
                dbus::<()>(bus, "Reload", &(path.as_str(), note))?;
                text(
                    "Reloaded: the window shows the file as it is on disk now, kept as a version."
                        .into(),
                )
            }
            "preview_versions" => text(dbus::<String>(bus, "Versions", &(path.as_str(),))?),
            "preview_revert" => {
                let Some(version) = arguments.get("version").and_then(Value::as_u64) else {
                    return Ok(
                        json!({ "content": [{ "type": "text", "text": "Say which version." }], "isError": true }),
                    );
                };
                dbus::<()>(bus, "Revert", &(path.as_str(), version as u32))?;
                text(format!("Version {version} is back in place of the file."))
            }
            "preview_draw" => {
                let layer = arguments
                    .get("layer")
                    .and_then(Value::as_str)
                    .unwrap_or("agent");
                let marks = arguments.get("marks").cloned().unwrap_or_else(|| json!([]));
                let drawn: u32 = dbus(
                    bus,
                    "Draw",
                    &(path.as_str(), layer, marks.to_string().as_str()),
                )?;
                text(format!("Drew {drawn} marks in the layer \"{layer}\"."))
            }
            "preview_clear" => {
                let layer = arguments.get("layer").and_then(Value::as_str).unwrap_or("");
                let gone: u32 = dbus(bus, "Clear", &(path.as_str(), layer))?;
                text(format!("Took away {gone} marks."))
            }
            "preview_render" => {
                let marks = arguments
                    .get("marks")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let png: String = dbus(bus, "Render", &(path.as_str(), marks))?;
                match std::fs::read(&png) {
                    Ok(bytes) => json!({ "content": [
                        { "type": "image", "mimeType": "image/png",
                          "data": base64::engine::general_purpose::STANDARD.encode(bytes) },
                        { "type": "text", "text": format!("Also written to {png}.") }
                    ] }),
                    Err(err) => text(format!(
                        "Rendered to {png}, but it could not be read: {err}"
                    )),
                }
            }
            _ => {
                return Ok(
                    json!({ "content": [{ "type": "text", "text": format!("No tool {name}.") }], "isError": true }),
                )
            }
        })
    })();
    outcome.unwrap_or_else(|err| {
        let message = match &err {
            zbus::Error::MethodError(_, Some(detail), _) => detail.clone(),
            zbus::Error::MethodError(name, None, _)
                if name.as_str().ends_with("ServiceUnknown") =>
            {
                "Preview isn't running: the window beside the chat was closed.".into()
            }
            other => other.to_string(),
        };
        json!({ "content": [{ "type": "text", "text": message }], "isError": true })
    })
}

/// Call `method` on the running Preview.
fn dbus<R>(
    bus: &Connection,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> zbus::Result<R>
where
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d> + zbus::zvariant::Type,
{
    let reply = bus.call_method(Some(DBUS_NAME), DBUS_PATH, Some(DBUS_NAME), method, body)?;
    reply.body().deserialize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_a_description_and_a_schema() {
        let tools = tools();
        let tools = tools.as_array().unwrap();
        assert_eq!(tools.len(), 8);
        // The Studio agent runs with a list of the tools it may use: one
        // missing there is a tool it never sees.
        let studio = include_str!("../../../resources/plugins/otto/agents/studio.md");
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(
                studio.contains(&format!("mcp__preview__{name}")),
                "{name} is not in studio.md's tools"
            );
            assert!(tool["name"].as_str().unwrap().starts_with("preview_"));
            assert!(!tool["description"].as_str().unwrap().is_empty());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn the_instructions_name_the_file() {
        let text = instructions(Path::new("/home/me/photo.jpg"));
        assert!(text.contains("/home/me/photo.jpg"));
    }
}
