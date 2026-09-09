//! `run_agent_extraction` reverse-RPC handler.
//!
//! The Python worker calls back into Rust to run one extraction turn; this
//! handler spawns a fresh, ephemeral `ClaudeRunner`, drains it to
//! `Complete`, parses the buffered text as JSON, and returns it. Registered
//! by `lib.rs` via `WorkerSupervisor::register_reverse_rpc` — the only
//! real handler attached to the generic dispatch mechanism; tests register
//! their own dummy handlers instead.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio_stream::StreamExt;

use crate::error::AppError;
use crate::ipc::python::{ReverseRpcError, ReverseRpcHandler};
use crate::ipc::runner::claude::{ClaudeRunner, MODEL_IDS};
use crate::ipc::runner::{
    AgentEvent, AgentRunner, ApprovalPolicy, PromptRequest, RunnerConfig, UserContent,
};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// At most 2 concurrent
/// `run_agent_extraction` dispatches. Two concurrent `claude` subprocesses
/// is well within resource budget.
const MAX_CONCURRENT_EXTRACTIONS: usize = 2;

#[derive(Debug, Deserialize)]
struct RunAgentExtractionParams {
    prompt: String,
    system_prompt: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// v1 extraction is always stateless — this
    /// param is always null on the wire. Accepted-but-ignored so the wire
    /// shape stays stable without this handler pretending to resume
    /// anything.
    #[allow(dead_code)]
    #[serde(default)]
    session_id: Option<String>,
}

pub struct ExtractionRpcHandler {
    concurrency: Arc<Semaphore>,
}

impl ExtractionRpcHandler {
    pub fn new() -> Self {
        Self {
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_EXTRACTIONS)),
        }
    }
}

impl Default for ExtractionRpcHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ReverseRpcHandler for ExtractionRpcHandler {
    async fn handle(&self, params: Value) -> Result<Value, ReverseRpcError> {
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| ReverseRpcError {
                code: -32000,
                message: "extraction concurrency semaphore closed".to_string(),
                data: None,
            })?;

        let params: RunAgentExtractionParams =
            serde_json::from_value(params).map_err(|e| ReverseRpcError {
                code: -32602,
                message: format!("invalid run_agent_extraction params: {e}"),
                data: None,
            })?;
        let timeout_ms = params.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

        run_extraction(params.prompt, params.system_prompt, timeout_ms)
            .await
            .map_err(to_rpc_err)
    }
}

async fn run_extraction(
    prompt: String,
    system_prompt: String,
    timeout_ms: u64,
) -> Result<Value, AppError> {
    let mut runner: Box<dyn AgentRunner> = Box::new(ClaudeRunner::new());
    if let Err(e) = runner
        .start(RunnerConfig {
            model: MODEL_IDS.extraction.to_string(),
            timeout_ms: Some(timeout_ms),
            system_prompt: Some(system_prompt),
            tools: vec![], // extraction has no tools in v1.
            approval_policy: ApprovalPolicy::AutoDenyDestructive,
            mcp: None, // extraction is self-contained — no MCP config.
            // Ephemeral: nothing ever resumes an extraction call.
            resume: None,
        })
        .await
    {
        let _ = runner.dispose().await;
        return Err(e);
    }

    let mut stream = match runner
        .prompt(PromptRequest {
            content: vec![UserContent::Text(prompt)],
            turn_id: None,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            let _ = runner.dispose().await;
            return Err(e);
        }
    };

    let mut buffer = String::new();
    let mut terminal_error = None;
    while let Some(ev) = stream.next().await {
        match ev {
            AgentEvent::TokenDelta { text, .. } => buffer.push_str(&text),
            AgentEvent::Complete { .. } => break,
            AgentEvent::Error { error, .. } => {
                terminal_error = Some(error);
                break;
            }
            _ => {}
        }
    }
    drop(stream);
    let _ = runner.dispose().await;

    if let Some(err) = terminal_error {
        return Err(err);
    }

    serde_json::from_str(strip_code_fence(buffer.trim())).map_err(|e| AppError::Runner {
        runner: "claude".to_string(),
        message: format!("agent_json_parse: {e}"),
        correlation_id: crate::error::correlation_id(),
    })
}

/// Defense-in-depth against a model wrapping its JSON in a markdown code
/// fence (```` ```json ... ``` ````) despite the system prompt explicitly
/// forbidding it — smaller models follow "no code fences" less reliably
/// than Sonnet did. A no-op on already-bare JSON.
fn strip_code_fence(text: &str) -> &str {
    let Some(after_open) = text.strip_prefix("```") else {
        return text;
    };
    let after_open = after_open.strip_prefix("json").unwrap_or(after_open);
    let after_open = after_open.trim_start_matches(['\n', '\r']);
    match after_open.rfind("```") {
        Some(close) => after_open[..close].trim(),
        None => text,
    }
}

/// Maps an `AppError` from the extraction turn into the RPC error code +
/// `kind` the Python worker's `agent_call.py` dispatches on.
fn to_rpc_err(err: AppError) -> ReverseRpcError {
    match &err {
        AppError::WorkerUnavailable { .. } => ReverseRpcError {
            code: -32000,
            message: "claude CLI not on PATH".to_string(),
            data: Some(serde_json::json!({"kind": "cli_missing"})),
        },
        // A usage-limit refusal is not a schema problem and must never be
        // retried with the "your JSON was malformed" nudge — that spends a
        // second call against an already-exhausted quota and then reports
        // the failure as bad model output. `agent_call.py` keys off this
        // `kind` to fail fast; `-32023` carries the user-facing message
        // back through `map_json_rpc_error` intact.
        AppError::RunnerBlocked {
            message,
            resets_at,
            correlation_id,
            ..
        } => ReverseRpcError {
            code: -32023,
            message: message.clone(),
            data: Some(serde_json::json!({
                "kind": "agent_blocked",
                "resets_at": resets_at,
                "correlation_id": correlation_id,
            })),
        },
        AppError::Runner {
            message,
            correlation_id,
            ..
        } => match message.as_str() {
            "timeout" => ReverseRpcError {
                code: -32020,
                message: "extraction timed out".to_string(),
                data: Some(serde_json::json!({"correlation_id": correlation_id})),
            },
            "cli_not_logged_in" => ReverseRpcError {
                code: -32000,
                message: "claude CLI not logged in".to_string(),
                data: Some(
                    serde_json::json!({"kind": "cli_not_logged_in", "correlation_id": correlation_id}),
                ),
            },
            "stream_corrupt" => ReverseRpcError {
                code: -32000,
                message: "claude CLI stream corrupted".to_string(),
                data: Some(
                    serde_json::json!({"kind": "stream_corrupt", "correlation_id": correlation_id}),
                ),
            },
            m if m.starts_with("agent_json_parse") => ReverseRpcError {
                code: -32603,
                message: m.to_string(),
                data: Some(serde_json::json!({"correlation_id": correlation_id})),
            },
            m => ReverseRpcError {
                code: -32000,
                message: m.to_string(),
                data: Some(
                    serde_json::json!({"kind": "cli_error", "correlation_id": correlation_id}),
                ),
            },
        },
        other => ReverseRpcError {
            code: -32000,
            message: other.to_string(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in end-to-end proof against the REAL `claude` CLI —
    /// not run in CI, and not part of
    /// the regular `cargo test` run. Requires `claude` on `PATH` and an
    /// active login. Run with:
    /// `MNEMOS_LIVE_CLAUDE=1 cargo test --offline -- --ignored run_extraction_against_the_real_claude_cli`
    #[tokio::test]
    #[ignore]
    async fn run_extraction_against_the_real_claude_cli() {
        if std::env::var("MNEMOS_LIVE_CLAUDE").as_deref() != Ok("1") {
            eprintln!("skipping: set MNEMOS_LIVE_CLAUDE=1 to run against the real claude CLI");
            return;
        }
        let result = run_extraction(
            "The meeting covered the Q3 budget.".to_string(),
            r#"Output ONLY valid JSON matching {"summary": string}. No prose, no markdown fences."#
                .to_string(),
            30_000,
        )
        .await
        .expect("real claude CLI extraction call");
        assert!(result.get("summary").and_then(Value::as_str).is_some());
    }

    #[tokio::test]
    async fn missing_binary_maps_to_cli_missing() {
        let _guard = crate::ipc::runner::claude::path_env_test_lock()
            .lock()
            .await;
        let dir = tempfile::tempdir().unwrap();
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", dir.path()) };
        let result = run_extraction("hi".to_string(), "sys".to_string(), 5_000).await;
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        let err = result.unwrap_err();
        let rpc = to_rpc_err(err);
        assert_eq!(rpc.data.unwrap()["kind"], "cli_missing");
    }

    #[test]
    fn strip_code_fence_unwraps_a_json_fenced_block() {
        let wrapped = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_code_fence(wrapped), "{\"a\": 1}");
    }

    #[test]
    fn strip_code_fence_unwraps_a_bare_fenced_block() {
        let wrapped = "```\n{\"a\": 1}\n```";
        assert_eq!(strip_code_fence(wrapped), "{\"a\": 1}");
    }

    #[test]
    fn strip_code_fence_is_a_noop_on_bare_json() {
        let bare = "{\"a\": 1}";
        assert_eq!(strip_code_fence(bare), bare);
    }

    #[test]
    fn agent_json_parse_maps_to_dash32603() {
        let err = AppError::Runner {
            runner: "claude".to_string(),
            message: "agent_json_parse: expected value at line 1".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let rpc = to_rpc_err(err);
        assert_eq!(rpc.code, -32603);
    }

    #[test]
    fn timeout_maps_to_dash32020() {
        let err = AppError::Runner {
            runner: "claude".to_string(),
            message: "timeout".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let rpc = to_rpc_err(err);
        assert_eq!(rpc.code, -32020);
    }
}
