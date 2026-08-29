//! Hand-rolled MCP JSON-RPC 2.0 wire types (stdio transport).
//!
//! LLD-08 §11 Q4 flags the Rust MCP SDK ecosystem (`rmcp`) as PROVISIONAL
//! and names a ~400 LOC hand-rolled server as the fallback if it isn't
//! production-ready. This binary takes that fallback directly rather than
//! spending build-time/version-risk budget evaluating an early-ecosystem
//! crate: the surface this server needs is small (`initialize`, `tools/list`,
//! `tools/call`, plus graceful shutdown) and is fully specified by LLD-08 §3.
//!
//! Framing: newline-delimited JSON, one JSON-RPC message per line (LLD-08
//! §4 — "line-delimited JSON, not LSP Content-Length; MCP's own
//! convention"), not the LSP-style `Content-Length` header framing used
//! elsewhere in this codebase for the worker/runner IPC.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Absent on a notification (e.g. `notifications/initialized`) — no
    /// response is sent for those.
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// Standard JSON-RPC 2.0 codes this server actually emits. Tool-level
/// failures (validation, not-found, rate-limit, timeout) are never
/// JSON-RPC protocol errors — LLD-08 §6.1: they're structured
/// `isError: true` tool results so the calling model can recover. The only
/// protocol-level error this server returns is `METHOD_NOT_FOUND`, which
/// doubles as the read-only enforcement mechanism (LLD-08 §7): there is no
/// write-shaped method registered at all, so any client guess 404s.
pub mod error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const METHOD_NOT_FOUND: i64 = -32601;
}
