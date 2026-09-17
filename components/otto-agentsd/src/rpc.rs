//! JSON-RPC 2.0 framing: classifying inbound messages and building responses.
//!
//! Responses are built as raw [`Value`]s instead of the `ahp_types::messages`
//! structs because those fix `id` to `u64`, while JSON-RPC also allows string
//! ids and requires a `null` id when a request cannot be parsed.

use ahp_types::errors::json_rpc_error_codes;
use serde_json::{Value, json};

/// A parsed inbound JSON-RPC message.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Value,
    },
}

/// An error sent back to the peer as a JSON-RPC error response.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(json_rpc_error_codes::INVALID_REQUEST, message)
    }

    pub fn invalid_params(err: impl std::fmt::Display) -> Self {
        Self::new(
            json_rpc_error_codes::INVALID_PARAMS,
            format!("invalid params: {err}"),
        )
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            json_rpc_error_codes::METHOD_NOT_FOUND,
            format!("method not found: {method}"),
        )
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::new(json_rpc_error_codes::INTERNAL_ERROR, err.to_string())
    }
}

/// Parses one text frame. On failure, returns the error response to send.
pub fn parse(text: &str) -> Result<Incoming, Value> {
    let value: Value = serde_json::from_str(text).map_err(|err| {
        error_response(
            Value::Null,
            &RpcError::new(json_rpc_error_codes::PARSE_ERROR, err.to_string()),
        )
    })?;
    let Value::Object(mut message) = value else {
        return Err(error_response(
            Value::Null,
            &RpcError::invalid_request("expected a JSON-RPC object"),
        ));
    };

    let id = message.remove("id");
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        let err = RpcError::invalid_request(r#"jsonrpc must be "2.0""#);
        return Err(error_response(id.unwrap_or(Value::Null), &err));
    }

    let params = message.remove("params").unwrap_or(Value::Null);
    match (message.remove("method"), id) {
        (Some(Value::String(method)), Some(id)) => Ok(Incoming::Request { id, method, params }),
        (Some(Value::String(method)), None) => Ok(Incoming::Notification { method, params }),
        (None, Some(id)) if message.contains_key("result") || message.contains_key("error") => {
            Ok(Incoming::Response { id })
        }
        (_, id) => Err(error_response(
            id.unwrap_or(Value::Null),
            &RpcError::invalid_request("missing method"),
        )),
    }
}

pub fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Value, err: &RpcError) -> Value {
    let mut error = json!({ "code": err.code, "message": err.message });
    if let Some(data) = &err.data {
        error["data"] = data.clone();
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_requests_and_notifications() {
        assert_eq!(
            parse(
                r#"{"jsonrpc":"2.0","id":"a","method":"ping","params":{"channel":"ahp-root://"}}"#
            ),
            Ok(Incoming::Request {
                id: json!("a"),
                method: "ping".into(),
                params: json!({ "channel": "ahp-root://" }),
            })
        );
        assert_eq!(
            parse(r#"{"jsonrpc":"2.0","method":"unsubscribe"}"#),
            Ok(Incoming::Notification {
                method: "unsubscribe".into(),
                params: Value::Null
            })
        );
        assert_eq!(
            parse(r#"{"jsonrpc":"2.0","id":3,"result":null}"#),
            Ok(Incoming::Response { id: json!(3) })
        );
    }

    #[test]
    fn malformed_input_yields_error_responses() {
        let parse_error = parse("{").unwrap_err();
        assert_eq!(parse_error["id"], Value::Null);
        assert_eq!(
            parse_error["error"]["code"],
            json_rpc_error_codes::PARSE_ERROR
        );

        let wrong_version = parse(r#"{"jsonrpc":"1.0","id":7,"method":"ping"}"#).unwrap_err();
        assert_eq!(wrong_version["id"], 7);
        assert_eq!(
            wrong_version["error"]["code"],
            json_rpc_error_codes::INVALID_REQUEST
        );
    }
}
