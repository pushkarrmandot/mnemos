//! The `mnemos_tool` wrapper — validation, correlation id,
//! rate limiting, timing, error mapping, and a no-content analytics event,
//! run in that order around every tool handler.
//!
//! A `#[mnemos_tool(...)]` attribute macro (the
//! Rust equivalent of Superset's `defineTool`) was considered, but a real
//! attribute macro needs its own `proc-macro = true` crate, which forces a
//! Cargo workspace — directly contradicting the "single crate, not
//! a workspace" mandate for the whole Rust side (justified there by v1 LOC
//! scale). This binary gets the identical *runtime* behavior — every
//! responsibility executed in the same order — from a
//! plain higher-order function instead of a compile-time macro. Adding a
//! tool is still "one function + one registration entry"; it just isn't
//! `inventory::submit!`-collected at compile time.

use std::future::Future;
use std::time::{Duration, Instant};

use mnemos_tauri_lib::error::AppError;
use mnemos_tauri_lib::metrics::{
    events, properties::EventProperties, properties::PropertyValue, Metrics,
};

use crate::rate_limit::RateLimiter;

#[derive(Debug)]
pub enum ToolError {
    Validation {
        message: String,
        field: Option<String>,
    },
    NotFound {
        message: String,
    },
    Storage {
        message: String,
    },
    Internal {
        message: String,
    },
}

impl ToolError {
    pub fn field(message: impl Into<String>, field: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            field: Some(field.into()),
        }
    }

    /// User-facing message — no correlation id in this
    /// string; it's already in the log line that produced the error.
    fn user_message(&self) -> String {
        match self {
            Self::Validation { message, field } => match field {
                Some(f) => format!("validation error on field '{f}': {message}"),
                None => format!("validation error: {message}"),
            },
            Self::NotFound { message } => message.clone(),
            Self::Storage { .. } => "temporary storage error".to_string(),
            Self::Internal { .. } => "internal error".to_string(),
        }
    }

    /// Full detail (unlike [`Self::user_message`]) — logged, never sent to
    /// the client: no correlation id in the user-visible
    /// message — the model doesn't need it; it's in the log.
    fn log_detail(&self) -> &str {
        match self {
            Self::Validation { message, .. }
            | Self::NotFound { message }
            | Self::Storage { message }
            | Self::Internal { message } => message,
        }
    }
}

/// `AppError` -> `ToolError` error-mapping.
impl From<AppError> for ToolError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::NotFound { entity, id } => Self::NotFound {
                message: format!("{entity} '{id}' not found"),
            },
            AppError::Validation { message, field } => Self::Validation { message, field },
            AppError::Storage { message, .. } => Self::Storage { message },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
    /// 15s for search/list variants, 5s for single-entity gets.
    pub timeout: Duration,
}

/// Runs `handler` with the full `#[mnemos_tool]`-equivalent wrapper
/// (argument parsing, which callers do
/// themselves via `serde_json::from_value` before calling this — see
/// `tools.rs`). Returns the MCP `tools/call` result shape directly:
/// `(structured content, isError)`.
pub async fn run_tool<F, Fut>(
    spec: &ToolSpec,
    limiter: &RateLimiter,
    metrics: &Metrics,
    handler: F,
) -> (serde_json::Value, bool)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<serde_json::Value, ToolError>>,
{
    let correlation_id = uuid::Uuid::new_v4().to_string();
    let _span = tracing::info_span!("mcp.tool", tool = spec.name, correlation_id = %correlation_id)
        .entered();

    if let Err(retry_after_ms) = limiter.try_acquire(spec.name) {
        tracing::warn!(retry_after_ms, "rate limit hit");
        return (
            serde_json::json!({
                "error": "rate_limited",
                "message": format!(
                    "rate limit: 60 calls/minute for {}; retry in {retry_after_ms}ms", spec.name
                ),
                "retry_after_ms": retry_after_ms,
            }),
            true,
        );
    }

    let start = Instant::now();
    let outcome = tokio::time::timeout(spec.timeout, handler()).await;
    let latency_ms = start.elapsed().as_millis();

    let (value, is_error, error_kind) = match outcome {
        Ok(Ok(value)) => (value, false, None),
        Ok(Err(tool_err)) => {
            let message = tool_err.user_message();
            let kind = error_kind_of(&tool_err);
            tracing::warn!(kind, detail = tool_err.log_detail(), "tool call failed");
            (
                serde_json::json!({ "error": kind, "message": message }),
                true,
                Some(kind),
            )
        }
        Err(_elapsed) => {
            tracing::warn!("tool call timed out");
            (
                serde_json::json!({
                    "error": "timeout",
                    "message": "tool timed out; try narrower filters",
                }),
                true,
                Some("timeout"),
            )
        }
    };

    // Fire-and-forget, no content, opt-out respected
    // (`Metrics::track` no-ops outright when the `metrics.enabled` setting
    // is off). `Metrics::track` also always logs a debug echo of exactly
    // these fields before attempting to send, so this remains the
    // verification surface the original stand-in log line was.
    metrics.track(
        events::MCP_TOOL_CALL,
        EventProperties::from([
            ("tool", PropertyValue::Enum(spec.name)),
            ("latency_ms", PropertyValue::UInt(latency_ms as u64)),
            ("success", PropertyValue::Bool(!is_error)),
            (
                "error_kind",
                PropertyValue::EnumOwned(error_kind.unwrap_or("none").to_string()),
            ),
        ]),
    );

    (value, is_error)
}

fn error_kind_of(err: &ToolError) -> &'static str {
    match err {
        ToolError::Validation { .. } => "validation",
        ToolError::NotFound { .. } => "not_found",
        ToolError::Storage { .. } => "storage",
        ToolError::Internal { .. } => "internal",
    }
}
