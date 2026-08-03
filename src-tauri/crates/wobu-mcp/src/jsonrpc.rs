//! JSON-RPC 2.0, only as much of it as MCP uses.
//!
//! Two things here are worth saying out loud, because both are places a naive
//! implementation gets it wrong in a way that looks fine against one client and
//! hangs against another.
//!
//! A message with no `id` is a **notification** and must never be answered —
//! not even with an error. MCP sends `notifications/initialized` immediately
//! after the handshake, so a server that replies to notifications produces an
//! unsolicited response on the very first exchange. [`Response`] cannot be
//! built for one: [`Request::is_notification`] is the only way to ask, and the
//! dispatcher returns `Option<Response>`.
//!
//! An `id` is `string | number` and travels back **unchanged**. Parsing it into
//! a `u64` would work with every client that numbers its requests and break the
//! ones that use strings or start at zero after a reconnect, so it is carried
//! as an opaque [`Value`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: &str = "2.0";

// The subset of the standard codes this crate can actually produce. `-32000` and
// below are the implementation-defined band; nothing here needs one, because a
// failure inside a tool is a *result* rather than a protocol error — see the
// module header on `dispatch`.
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub jsonrpc: String,
    /// Absent for a notification. `null` is *also* absent as far as JSON-RPC is
    /// concerned, and some clients send it, so both land here as `None`.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl Request {
    pub fn is_notification(&self) -> bool {
        self.id.is_none() || self.id.as_ref().is_some_and(Value::is_null)
    }

    /// `params` as an object, or an empty one.
    ///
    /// MCP always sends an object when it sends anything, and a missing
    /// `params` for a method whose arguments are all optional is legal. A
    /// caller that needs a particular field asks for it and reports its own
    /// [`INVALID_PARAMS`]; this only removes the `Option` dance from every one
    /// of them.
    pub fn params(&self) -> &serde_json::Map<String, Value> {
        static EMPTY: std::sync::LazyLock<serde_json::Map<String, Value>> =
            std::sync::LazyLock::new(serde_json::Map::new);
        self.params.as_ref().and_then(Value::as_object).unwrap_or(&EMPTY)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn result(id: Value, result: Value) -> Response {
        Response { jsonrpc: VERSION, id, result: Some(result), error: None }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Response {
        Response {
            jsonrpc: VERSION,
            id,
            result: None,
            error: Some(RpcError { code, message: message.into(), data: None }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// One POST may carry a single message or an array of them.
///
/// Untagged rather than a hand-written `Deserialize` so that a malformed member
/// of a batch fails the whole batch loudly instead of being silently dropped —
/// a client that gets fewer answers than it sent questions waits forever.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Single(Request),
    Batch(Vec<Request>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_without_an_id_is_a_notification_and_one_with_a_null_id_is_too() {
        let notification: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .unwrap();
        assert!(notification.is_notification());

        // Sent by more than one client in the wild, and answering it produces a
        // response the client never asked for.
        let nulled: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#).unwrap();
        assert!(nulled.is_notification());

        // Zero is a perfectly good id and must not be read as absent.
        let zero: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#).unwrap();
        assert!(!zero.is_notification());
    }

    #[test]
    fn a_string_id_travels_back_as_the_same_string() {
        let request: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":"tools-1","method":"ping"}"#).unwrap();
        let response = Response::result(request.id.clone().unwrap(), serde_json::json!({}));
        assert_eq!(serde_json::to_value(&response).unwrap()["id"], "tools-1");
    }

    #[test]
    fn an_answer_carries_exactly_one_of_result_and_error() {
        let ok = serde_json::to_value(Response::result(1.into(), serde_json::json!({}))).unwrap();
        assert!(ok.get("error").is_none(), "{ok}");

        let bad =
            serde_json::to_value(Response::error(1.into(), METHOD_NOT_FOUND, "nope")).unwrap();
        assert!(bad.get("result").is_none(), "{bad}");
        assert_eq!(bad["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn one_post_may_carry_a_batch() {
        let batch: Incoming = serde_json::from_str(
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"x"}]"#,
        )
        .unwrap();
        match batch {
            Incoming::Batch(messages) => assert_eq!(messages.len(), 2),
            Incoming::Single(_) => panic!("an array is a batch"),
        }
    }
}
