//! herdr-core's API, spoken on its socket (plan 0016): JSON-RPC 2.0, one
//! message per line, at `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`. The
//! session is `HC_SESSION`, else `default`, as `hc` picks it.
//!
//! [`call`] makes one request on a connection shared by the whole process.
//! [`subscribe`] opens a connection of its own for `events.subscribe`: a
//! snapshot of the terminals it matches, then each change as an [`Event`].
//! [`Event::apply`] keeps a terminal's facts, as `terminal.get` gives them,
//! current from the changes alone.

use std::collections::HashMap;
use std::io::{BufRead, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};

/// The protocol generation this speaks.
const GENERATION: u64 = 1;

/// How long a call made where nothing may wait for long, such as at startup,
/// waits for herdr-core before giving up.
const BLOCKING_PATIENCE: Duration = Duration::from_secs(2);

/// What went wrong with a call: herdr-core's own `data.kind` (`not_found`,
/// `invalid_params`, …), or `unreachable` when it could not be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub kind: String,
    pub message: String,
}

impl CoreError {
    fn unreachable(message: impl Into<String>) -> Self {
        Self {
            kind: "unreachable".to_owned(),
            message: message.into(),
        }
    }

    fn from_reply(error: &Value) -> Self {
        Self {
            kind: error
                .pointer("/data/kind")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_owned(),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "herdr-core: {} ({})", self.message, self.kind)
    }
}

impl std::error::Error for CoreError {}

/// herdr-core's socket for this session.
pub fn socket() -> Option<PathBuf> {
    let session = std::env::var("HC_SESSION")
        .ok()
        .filter(|session| !session.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    Some(
        otto_foundations::xdg::runtime_dir()?
            .join("herdr-core")
            .join(format!("{session}.sock")),
    )
}

/// The calls waiting for their reply on a connection, by id.
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CoreError>>>>>;

/// One connection to herdr-core, after the handshake.
struct Connection {
    lines: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
}

impl Connection {
    /// Connects and shakes hands. Notifications, such as a subscription's
    /// events, come out of the receiver; it closes with the connection.
    async fn open() -> Result<(Self, mpsc::UnboundedReceiver<Value>), CoreError> {
        let path = socket().ok_or_else(|| CoreError::unreachable("no runtime directory"))?;
        let stream = match UnixStream::connect(&path).await {
            Ok(stream) => stream,
            // herdr-core starts on first use; `hc` is what starts it.
            Err(_) => {
                let _ = tokio::process::Command::new("hc")
                    .args(["server", "start"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                UnixStream::connect(&path).await.map_err(|error| {
                    CoreError::unreachable(format!("{}: {error}", path.display()))
                })?
            }
        };
        let (read, mut write) = stream.into_split();
        let (lines, mut outgoing) = mpsc::unbounded_channel::<String>();
        let (notify, notifications) = mpsc::unbounded_channel::<Value>();
        let pending: Pending = Arc::default();
        tokio::spawn(async move {
            while let Some(mut line) = outgoing.recv().await {
                line.push('\n');
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let replies = pending.clone();
        tokio::spawn(async move {
            let mut incoming = BufReader::new(read).lines();
            while let Ok(Some(line)) = incoming.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match message.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        let waiting = replies
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&id);
                        if let Some(waiting) = waiting {
                            let _ = waiting.send(reply(message));
                        }
                    }
                    None => {
                        let _ = notify.send(message);
                    }
                }
            }
            // Closed: every call still waiting fails, and the notifications
            // end with `notify` dropped here.
            let waiting: Vec<_> = replies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain()
                .collect();
            for (_, waiting) in waiting {
                let _ = waiting.send(Err(CoreError::unreachable("the connection closed")));
            }
        });
        let connection = Self {
            lines,
            pending,
            next: AtomicU64::new(1),
        };
        connection
            .call("hello", json!({ "generation": GENERATION }))
            .await?;
        Ok((connection, notifications))
    }

    fn is_closed(&self) -> bool {
        self.lines.is_closed()
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, wait) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, reply);
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if self.lines.send(request.to_string()).is_err() {
            self.pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            return Err(CoreError::unreachable("the connection closed"));
        }
        wait.await
            .unwrap_or_else(|_| Err(CoreError::unreachable("the connection closed")))
    }
}

/// A reply's result, or its error.
fn reply(message: Value) -> Result<Value, CoreError> {
    match message.get("error") {
        Some(error) => Err(CoreError::from_reply(error)),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// The connection [`call`] shares, opened on first use and again after it
/// closed.
static SHARED: tokio::sync::Mutex<Option<Arc<Connection>>> = tokio::sync::Mutex::const_new(None);

/// Calls `method` with `params` on herdr-core.
pub async fn call(method: &str, params: Value) -> Result<Value, CoreError> {
    let connection = {
        let mut shared = SHARED.lock().await;
        match shared.as_ref().filter(|connection| !connection.is_closed()) {
            Some(connection) => connection.clone(),
            None => {
                let (connection, _notifications) = Connection::open().await?;
                let connection = Arc::new(connection);
                *shared = Some(connection.clone());
                connection
            }
        }
    };
    connection.call(method, params).await
}

/// [`call`] for code that cannot wait on the runtime, such as the backend's
/// synchronous questions: a connection of its own, given up after a moment.
pub fn call_blocking(method: &str, params: Value) -> Result<Value, CoreError> {
    let path = socket().ok_or_else(|| CoreError::unreachable("no runtime directory"))?;
    let stream = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|error| CoreError::unreachable(format!("{}: {error}", path.display())))?;
    let unreachable = |error: std::io::Error| CoreError::unreachable(error.to_string());
    stream
        .set_read_timeout(Some(BLOCKING_PATIENCE))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(BLOCKING_PATIENCE))
        .map_err(unreachable)?;
    let mut writer = stream.try_clone().map_err(unreachable)?;
    let hello = json!({ "jsonrpc": "2.0", "id": 1, "method": "hello", "params": { "generation": GENERATION } });
    let request = json!({ "jsonrpc": "2.0", "id": 2, "method": method, "params": params });
    writeln!(writer, "{hello}\n{request}").map_err(unreachable)?;
    for line in std::io::BufReader::new(stream).lines() {
        let message: Value = serde_json::from_str(&line.map_err(unreachable)?)
            .map_err(|error| CoreError::unreachable(error.to_string()))?;
        if message.get("id").and_then(Value::as_u64) == Some(2) {
            return reply(message);
        }
    }
    Err(CoreError::unreachable("the connection closed"))
}

/// A change `events.subscribe` reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A fact of one terminal changed: `kind` is `created`, `status`,
    /// `labels`, `title`, `foreground`, `finished`, `removed` and the rest,
    /// `data` what it is now.
    Changed {
        terminal: String,
        kind: String,
        data: Value,
    },
    /// The subscriber fell behind: the terminals it matches, afresh.
    Missed(Vec<Value>),
}

impl Event {
    /// Brings `item`, one terminal's facts as `terminal.get` gives them, up to
    /// date with a change to it.
    pub fn apply(item: &mut Value, kind: &str, data: &Value) {
        let Some(facts) = item.as_object_mut() else {
            return;
        };
        match kind {
            "foreground" => {
                facts.insert("foreground".to_owned(), data.clone());
            }
            "status" => {
                let status = data.get("status").unwrap_or(data);
                facts.insert("status".to_owned(), status.clone());
            }
            "finished" => {
                facts.insert("state".to_owned(), json!("finished"));
                merge(facts, data);
            }
            "lost" => {
                facts.insert("state".to_owned(), json!("lost"));
                merge(facts, data);
            }
            "restarted" => {
                facts.insert("state".to_owned(), json!("live"));
                merge(facts, data);
            }
            _ => merge(facts, data),
        }
    }
}

fn merge(facts: &mut serde_json::Map<String, Value>, data: &Value) {
    if let Some(data) = data.as_object() {
        for (key, value) in data {
            facts.insert(key.clone(), value.clone());
        }
    }
}

/// A subscription: the terminals it matched when it began, then each change.
pub struct Subscription {
    pub snapshot: Vec<Value>,
    notifications: mpsc::UnboundedReceiver<Value>,
    // Held so the connection stays open while the subscription is read.
    _connection: Connection,
}

impl Subscription {
    /// The next change; `None` once herdr-core is gone.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            let message = self.notifications.recv().await?;
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            match message.get("method").and_then(Value::as_str) {
                Some("event") => {
                    let (Some(terminal), Some(kind)) = (
                        params.get("terminal").and_then(Value::as_str),
                        params.get("type").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    return Some(Event::Changed {
                        terminal: terminal.to_owned(),
                        kind: kind.to_owned(),
                        data: params.get("data").cloned().unwrap_or(Value::Null),
                    });
                }
                Some("missed") => {
                    return Some(Event::Missed(terminals(&params)));
                }
                _ => {}
            }
        }
    }
}

/// The terminals a list, a snapshot or a `missed` carries.
fn terminals(value: &Value) -> Vec<Value> {
    value
        .get("terminals")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Subscribes to the terminals `filter` matches (`terminal`, `labels`, or
/// nothing for all), on a connection of its own.
pub async fn subscribe(filter: Value) -> Result<Subscription, CoreError> {
    let (connection, notifications) = Connection::open().await?;
    let snapshot = connection.call("events.subscribe", filter).await?;
    Ok(Subscription {
        snapshot: terminals(&snapshot),
        notifications,
        _connection: connection,
    })
}

/// The terminals `terminal.list` gives for `params`.
pub async fn list(params: Value) -> Result<Vec<Value>, CoreError> {
    Ok(terminals(&call("terminal.list", params).await?))
}

/// Sets labels on `terminal`; an empty value removes one.
pub async fn label<'a>(
    terminal: &str,
    labels: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), CoreError> {
    let labels: serde_json::Map<String, Value> = labels
        .into_iter()
        .map(|(key, value)| (key.to_owned(), json!(value)))
        .collect();
    call(
        "terminal.label",
        json!({ "terminal": terminal, "labels": labels }),
    )
    .await
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_stays_current_from_its_changes() {
        let mut item = json!({ "address": "t1", "state": "live", "labels": { "otto": "agent" } });
        Event::apply(
            &mut item,
            "labels",
            &json!({ "labels": { "otto": "agent", "otto.ask.a": "{}" } }),
        );
        assert_eq!(item["labels"]["otto.ask.a"], "{}");
        Event::apply(&mut item, "title", &json!({ "title": "Fix the dock" }));
        assert_eq!(item["title"], "Fix the dock");
        Event::apply(
            &mut item,
            "status",
            &json!({ "app": "claude-code", "state": "working" }),
        );
        assert_eq!(item["status"]["state"], "working");
        Event::apply(
            &mut item,
            "foreground",
            &json!({ "name": "claude", "pid": 7, "argv": ["claude"], "cwd": "/tmp" }),
        );
        assert_eq!(item["foreground"]["name"], "claude");
        Event::apply(&mut item, "finished", &json!({ "exit_code": 0 }));
        assert_eq!(item["state"], "finished");
        assert_eq!(item["exit_code"], 0);
        assert_eq!(item["address"], "t1");
    }

    #[test]
    fn an_error_carries_herdrs_kind() {
        let error = reply(
            json!({ "id": 2, "error": { "code": -32000, "message": "not_found", "data": { "kind": "not_found" } } }),
        );
        assert_eq!(error.unwrap_err().kind, "not_found");
        assert_eq!(
            reply(json!({ "id": 2, "result": { "ok": true } })).unwrap()["ok"],
            true
        );
    }
}
